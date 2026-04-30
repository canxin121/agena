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
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, mpsc};

use crate::error::{LspError, LspResult};
use crate::protocol::{FrameParser, InboundMessage, encode_frame};

use super::LspTransport;

pub struct StdioTransport {
    server_name: String,
    stdin: Mutex<ChildStdin>,
    inbox: Mutex<mpsc::UnboundedReceiver<LspResult<InboundMessage>>>,
    _child: Mutex<Option<Child>>,
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
            .stderr(Stdio::piped());
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

        let (tx, rx) = mpsc::unbounded_channel();
        spawn_stdout_reader(name.to_string(), stdout, tx.clone());
        spawn_stderr_reader(name.to_string(), stderr);

        Ok(Arc::new(Self {
            server_name: name.to_string(),
            stdin: Mutex::new(stdin),
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
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&bytes).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn recv(&self) -> LspResult<InboundMessage> {
        let mut g = self.inbox.lock().await;
        g.recv().await.unwrap_or(Err(LspError::TransportClosed))
    }

    async fn close(&self) -> LspResult<()> {
        let mut guard = self._child.lock().await;
        if let Some(mut child) = guard.take() {
            let _ = child.kill().await;
        }
        Ok(())
    }
}

fn spawn_stdout_reader(
    name: String,
    stdout: tokio::process::ChildStdout,
    tx: mpsc::UnboundedSender<LspResult<InboundMessage>>,
) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut parser = FrameParser::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = match reader.read(&mut chunk).await {
                Ok(0) => {
                    let _ = tx.send(Err(LspError::TransportClosed));
                    return;
                }
                Ok(n) => n,
                Err(err) => {
                    let _ = tx.send(Err(LspError::Io(err)));
                    return;
                }
            };
            parser.feed(&chunk[..n]);
            loop {
                match parser.take() {
                    Ok(Some(value)) => match InboundMessage::from_value(value) {
                        Ok(msg) => {
                            if tx.send(Ok(msg)).is_err() {
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
                        let _ = tx.send(Err(LspError::Protocol(err)));
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
