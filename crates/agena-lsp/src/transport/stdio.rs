//! Stdio transport — spawn an LSP server child and exchange Content-Length
//! framed JSON-RPC over its stdin/stdout. stderr is line-tee'd to tracing
//! so server logs land in the agena log without interfering with framing.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use agena_process::ManagedChild;
use agena_stdio_codec::ContentLengthCodec;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc};
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::error::{LspError, LspResult};
use crate::protocol::InboundMessage;

use super::LspTransport;

/// Stdio transport for an LSP server.
pub struct StdioTransport {
    server_name: String,
    writer: mpsc::Sender<WriteRequest>,
    inbox: Mutex<mpsc::Receiver<LspResult<InboundMessage>>>,
    _child: Mutex<Option<ManagedChild>>,
}

struct WriteRequest {
    body: Bytes,
    completion: tokio::sync::oneshot::Sender<LspResult<()>>,
}

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

impl StdioTransport {
    pub async fn spawn(
        name: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: Option<&Path>,
    ) -> LspResult<Arc<Self>> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = agena_process::spawn(cmd)?;
        let stdin = child
            .stdin()
            .take()
            .ok_or_else(|| LspError::Transport("child stdin closed".into()))?;
        let stdout = child
            .stdout()
            .take()
            .ok_or_else(|| LspError::Transport("child stdout closed".into()))?;
        let stderr = child
            .stderr()
            .take()
            .ok_or_else(|| LspError::Transport("child stderr closed".into()))?;

        let (tx, rx) = mpsc::channel(256);
        spawn_stdout_reader(name.to_string(), stdout, tx.clone());
        spawn_stderr_reader(name.to_string(), stderr);

        let (writer, mut writes) = mpsc::channel::<WriteRequest>(64);
        tokio::spawn(async move {
            let mut stdin = FramedWrite::new(stdin, ContentLengthCodec::new(MAX_FRAME_BYTES));
            while let Some(request) = writes.recv().await {
                let result = tokio::time::timeout(WRITE_TIMEOUT, stdin.send(request.body))
                    .await
                    .map_err(|error| {
                        LspError::Transport(
                            agena_failure::diagnostic::format_error_chain_with_context(
                                "child stdin write timed out",
                                &error,
                            ),
                        )
                    })
                    .and_then(|result| result.map_err(|error| LspError::transport_error(&error)));
                let failed = result.is_err();
                if request.completion.send(result).is_err() {
                    tracing::debug!(
                        "LSP write result receiver was dropped before completion could be delivered"
                    );
                }
                if failed {
                    break;
                }
            }
        });

        Ok(Arc::new(Self {
            server_name: name.to_string(),
            writer,
            inbox: Mutex::new(rx),
            _child: Mutex::new(Some(child)),
        }))
    }

    pub fn name(&self) -> &str {
        &self.server_name
    }
}

#[async_trait]
impl LspTransport for StdioTransport {
    async fn send(&self, payload: Value) -> LspResult<()> {
        let body = Bytes::from(serde_json::to_vec(&payload)?);
        let (completion, result) = tokio::sync::oneshot::channel();
        self.writer
            .send(WriteRequest { body, completion })
            .await
            .map_err(|error| {
                LspError::transport_closed("failed to enqueue an outbound LSP stdio frame", &error)
            })?;
        result.await.map_err(|error| {
            LspError::transport_closed(
                "LSP stdio writer exited before confirming the outbound frame",
                &error,
            )
        })?
    }

    async fn recv(&self) -> LspResult<InboundMessage> {
        let mut g = self.inbox.lock().await;
        g.recv().await.unwrap_or_else(|| {
            Err(LspError::transport_closed_without_source(
                "LSP stdio reader channel closed before another frame arrived",
            ))
        })
    }

    async fn close(&self) -> LspResult<()> {
        let child = self._child.lock().await.take();
        if let Some(mut child) = child {
            child
                .terminate(std::time::Duration::from_millis(250))
                .await
                .map_err(|error| LspError::transport_error(&error))?;
        }
        Ok(())
    }
}

fn spawn_stdout_reader(
    name: String,
    stdout: tokio::process::ChildStdout,
    tx: mpsc::Sender<LspResult<InboundMessage>>,
) {
    tokio::spawn(async move {
        let mut reader = FramedRead::new(stdout, ContentLengthCodec::new(MAX_FRAME_BYTES));
        while let Some(frame) = reader.next().await {
            let body = match frame {
                Ok(body) => body,
                Err(error) => {
                    if tx
                        .send(Err(LspError::protocol_error(&error)))
                        .await
                        .is_err()
                    {
                        tracing::debug!(
                            server = %name,
                            "LSP frame decode failure could not be delivered because the receiver closed"
                        );
                    }
                    return;
                }
            };
            let value = match serde_json::from_slice(body.as_ref()) {
                Ok(value) => value,
                Err(error) => {
                    if tx
                        .send(Err(LspError::protocol_error(&error)))
                        .await
                        .is_err()
                    {
                        tracing::debug!(
                            server = %name,
                            "LSP JSON decode failure could not be delivered because the receiver closed"
                        );
                    }
                    return;
                }
            };
            match InboundMessage::from_value(value) {
                Ok(message) => {
                    if tx.send(Ok(message)).await.is_err() {
                        return;
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        target: "agena_lsp::stdio",
                        server = %name,
                        "failed to classify inbound frame: {error}"
                    );
                }
            }
        }
        if let Err(error) = tx
            .send(Err(LspError::transport_closed_without_source(
                "LSP stdio input reached EOF",
            )))
            .await
        {
            tracing::debug!(
                target: "agena_lsp::stdio",
                server = %name,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to report LSP stdio EOF because the client receiver was closed",
                    &error,
                ),
                "LSP stdio EOF notification was not delivered"
            );
        }
    });
}

fn spawn_stderr_reader(name: String, stderr: tokio::process::ChildStderr) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::debug!(target: "agena_lsp::stderr", server = %name, "{line}");
        }
    });
}
