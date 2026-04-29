//! Stdio transport — spawn a child process and exchange newline-delimited
//! JSON frames over its stdin/stdout.
//!
//! Why newline-delimited and not LSP `Content-Length`?  The MCP reference
//! servers (`@modelcontextprotocol/server-everything` etc.) all emit
//! one JSON object per stdout line.  LSP framing is permitted by the spec
//! but rare in practice; we currently support newline-delimited only.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, mpsc};

use crate::error::{McpError, McpResult};
use crate::protocol::InboundMessage;
use crate::transport::McpTransport;

pub struct StdioTransport {
    inner: Arc<Inner>,
}

struct Inner {
    stdin: Mutex<Option<ChildStdin>>,
    inbox: Mutex<mpsc::UnboundedReceiver<McpResult<InboundMessage>>>,
    /// Holding the child here keeps it alive until the transport is dropped.
    _child: Mutex<Option<Child>>,
}

impl StdioTransport {
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: Option<&PathBuf>,
    ) -> McpResult<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear()
            // Inherit a minimal viable env, then layer per-server overrides.
            .envs(std::env::vars().filter(|(k, _)| {
                matches!(
                    k.as_str(),
                    "PATH" | "HOME" | "USER" | "LANG" | "LC_ALL" | "TZ" | "TMPDIR" | "TEMP"
                )
            }))
            .envs(env);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }

        let mut child = cmd.spawn().map_err(McpError::Io)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Transport("child stdin missing".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Transport("child stdout missing".to_string()))?;
        let stderr = child.stderr.take();

        let (tx, rx) = mpsc::unbounded_channel();

        // stdout reader task — split into lines, parse each as a JSON frame.
        let tx_out = tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        let value: Value = match serde_json::from_str(line) {
                            Ok(v) => v,
                            Err(e) => {
                                let _ = tx_out.send(Err(McpError::Malformed(format!(
                                    "invalid JSON frame: {e}: {line}"
                                ))));
                                continue;
                            }
                        };
                        let frame = InboundMessage::from_value(value)
                            .map_err(|e| McpError::Malformed(e.to_string()));
                        if tx_out.send(frame).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        let _ = tx_out.send(Err(McpError::TransportClosed));
                        break;
                    }
                    Err(e) => {
                        let _ = tx_out.send(Err(McpError::Io(e)));
                        break;
                    }
                }
            }
        });

        // stderr reader task — surface as tracing logs (MCP servers commonly
        // log diagnostics to stderr).
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if !line.is_empty() {
                        tracing::debug!(target: "agena_mcp_client::stdio", "[server stderr] {line}");
                    }
                }
            });
        }

        Ok(Self {
            inner: Arc::new(Inner {
                stdin: Mutex::new(Some(stdin)),
                inbox: Mutex::new(rx),
                _child: Mutex::new(Some(child)),
            }),
        })
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, payload: Value) -> McpResult<()> {
        let mut buf = serde_json::to_vec(&payload)?;
        buf.push(b'\n');
        let mut guard = self.inner.stdin.lock().await;
        let stdin = guard.as_mut().ok_or(McpError::TransportClosed)?;
        stdin.write_all(&buf).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn recv(&self) -> McpResult<InboundMessage> {
        let mut guard = self.inner.inbox.lock().await;
        match guard.recv().await {
            Some(frame) => frame,
            None => Err(McpError::TransportClosed),
        }
    }

    async fn close(&self) -> McpResult<()> {
        let mut stdin = self.inner.stdin.lock().await;
        if let Some(s) = stdin.take() {
            drop(s);
        }
        Ok(())
    }
}
