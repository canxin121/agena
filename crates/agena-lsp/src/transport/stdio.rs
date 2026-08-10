//! Stdio transport — spawn an LSP server child and exchange Content-Length
//! framed JSON-RPC over its stdin/stdout. stderr is line-tee'd to tracing
//! so server logs land in the agena log without interfering with framing.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc};

use crate::error::{LspError, LspResult};
use crate::protocol::{FrameParser, InboundMessage, encode_frame};

use super::LspTransport;

/// Stdio transport for an LSP server.
pub struct StdioTransport {
    server_name: String,
    writer: mpsc::Sender<WriteRequest>,
    inbox: Mutex<mpsc::Receiver<LspResult<InboundMessage>>>,
    _child: Mutex<Option<Child>>,
}

struct WriteRequest {
    bytes: Vec<u8>,
    completion: tokio::sync::oneshot::Sender<LspResult<()>>,
}

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
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| LspError::Transport("child stdin closed".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LspError::Transport("child stdout closed".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| LspError::Transport("child stderr closed".into()))?;

        let (tx, rx) = mpsc::channel(256);
        spawn_stdout_reader(name.to_string(), stdout, tx.clone());
        spawn_stderr_reader(name.to_string(), stderr);

        let (writer, mut writes) = mpsc::channel::<WriteRequest>(64);
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(request) = writes.recv().await {
                let result = async {
                    stdin.write_all(request.bytes.as_slice()).await?;
                    stdin.flush().await
                }
                .await
                .map_err(LspError::Io);
                let failed = result.is_err();
                let _ = request.completion.send(result);
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
        let bytes = encode_frame(&payload);
        let (completion, result) = tokio::sync::oneshot::channel();
        self.writer
            .send(WriteRequest { bytes, completion })
            .await
            .map_err(|_| LspError::TransportClosed)?;
        result.await.map_err(|_| LspError::TransportClosed)?
    }

    async fn recv(&self) -> LspResult<InboundMessage> {
        let mut g = self.inbox.lock().await;
        g.recv().await.unwrap_or(Err(LspError::TransportClosed))
    }

    async fn close(&self) -> LspResult<()> {
        let child = self._child.lock().await.take();
        if let Some(mut child) = child {
            let _ = child.kill().await;
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
        let mut reader = BufReader::new(stdout);
        let mut parser = FrameParser::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = match reader.read(&mut chunk).await {
                Ok(0) => {
                    let _ = tx.send(Err(LspError::TransportClosed)).await;
                    return;
                }
                Ok(n) => n,
                Err(err) => {
                    let _ = tx.send(Err(LspError::Io(err))).await;
                    return;
                }
            };
            parser.feed(&chunk[..n]);
            loop {
                match parser.take() {
                    Ok(Some(value)) => match InboundMessage::from_value(value) {
                        Ok(msg) => {
                            if tx.send(Ok(msg)).await.is_err() {
                                return;
                            }
                        }
                        Err(err) => {
                            tracing::warn!(
                                target: "agena_lsp::stdio",
                                server = %name,
                                "failed to classify inbound frame: {err}"
                            );
                        }
                    },
                    Ok(None) => break,
                    Err(err) => {
                        let _ = tx.send(Err(LspError::Protocol(err))).await;
                        return;
                    }
                }
            }
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
