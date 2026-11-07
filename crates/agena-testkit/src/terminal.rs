//! Real terminal workbench PTY and SSE fixtures. These APIs do not emulate a
//! terminal; all input, resize, process exit and output pass through Agena.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use reqwest::Method;
use serde_json::{Value, json};

use crate::TestCase;

#[derive(Clone)]
pub struct TerminalDriver {
    case: TestCase,
}

#[derive(Clone)]
pub struct TerminalHandle {
    case: TestCase,
    id: String,
}

/// A stream checkpoint obtained before an action. It belongs to one terminal
/// session and excludes every event in that session's previous replay tail.
#[derive(Clone, Debug)]
pub struct TerminalOutputCheckpoint {
    session_id: String,
    last_seq: u64,
}

impl TestCase {
    pub fn terminal(&self) -> TerminalDriver {
        TerminalDriver { case: self.clone() }
    }
}

impl TerminalDriver {
    pub async fn create(&self, cwd: &Path) -> Result<TerminalHandle> {
        let root = tokio::fs::canonicalize(&self.case.paths().root).await?;
        let directory = tokio::fs::canonicalize(cwd).await?;
        ensure!(
            directory.starts_with(&root),
            "terminal fixture working directory must belong to this test case: {}",
            cwd.display()
        );
        let cwd = cwd
            .to_str()
            .context("terminal working directory is not UTF-8")?;
        let output: Value = self
            .case
            .http()
            .await
            .send_json(
                Method::POST,
                "/api/v1/workbench/terminal/create",
                &json!({"cwd": cwd, "cols": 80, "rows": 24}),
            )
            .await?
            .require_success()?
            .json()?;
        let id = output["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .context("terminal create response contained no session ID")?;
        ensure!(
            output["cols"] == 80 && output["rows"] == 24,
            "terminal dimensions changed: {output}"
        );
        Ok(TerminalHandle {
            case: self.case.clone(),
            id: id.to_owned(),
        })
    }
}

impl TerminalHandle {
    pub fn id(&self) -> &str {
        &self.id
    }
    fn path(&self, operation: &str) -> String {
        format!("/api/v1/workbench/terminal/{}{operation}", self.id)
    }

    pub async fn info(&self) -> Result<Value> {
        self.case
            .http()
            .await
            .get(&self.path(""))
            .await?
            .require_success()?
            .json()
    }

    pub async fn input(&self, bytes: impl Into<Vec<u8>>) -> Result<()> {
        self.case
            .http()
            .await
            .send_bytes(
                Method::POST,
                &self.path("/input"),
                Some("application/octet-stream"),
                bytes,
            )
            .await?
            .require_success()?;
        Ok(())
    }

    pub async fn resize(&self, cols: u16, rows: u16) -> Result<Value> {
        self.case
            .http()
            .await
            .send_json(
                Method::POST,
                &self.path("/resize"),
                &json!({"cols":cols,"rows":rows}),
            )
            .await?
            .require_success()?
            .json()
    }

    pub async fn stop(&self) -> Result<()> {
        self.case
            .http()
            .await
            .send_json(Method::POST, &self.path("/stop"), &json!({}))
            .await?
            .require_success()?;
        Ok(())
    }

    /// Restore the stopped session through the production PTY restore path.
    pub async fn start(&self) -> Result<Value> {
        self.case
            .http()
            .await
            .send_json(Method::POST, &self.path("/start"), &json!({}))
            .await?
            .require_success()?
            .json()
    }

    /// Replace a session with a new production PTY and server-issued ID.
    pub async fn restart(&self, cwd: &Path) -> Result<TerminalHandle> {
        let root = tokio::fs::canonicalize(&self.case.paths().root).await?;
        let directory = tokio::fs::canonicalize(cwd).await?;
        ensure!(
            directory.starts_with(&root),
            "terminal restart directory is outside test case"
        );
        let cwd = cwd
            .to_str()
            .context("terminal restart directory is not UTF-8")?;
        let response: Value = self
            .case
            .http()
            .await
            .send_json(
                Method::POST,
                &self.path("/restart"),
                &json!({"cwd": cwd, "cols": 80, "rows": 24}),
            )
            .await?
            .require_success()?
            .json()?;
        let id = response["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .context("terminal restart response missing session ID")?;
        ensure!(
            id != self.id,
            "terminal restart reused an existing session ID"
        );
        Ok(TerminalHandle {
            case: self.case.clone(),
            id: id.to_owned(),
        })
    }

    pub async fn delete(&self) -> Result<()> {
        let http = self.case.http().await;
        http.execute(http.request(Method::DELETE, &self.path(""))?)
            .await?
            .require_success()?;
        Ok(())
    }

    /// Capture the terminal's last emitted SSE sequence before sending input.
    /// Unlike a tail search, this supplies evidence that later output is new.
    /// Capture another checkpoint after stopping or restarting the PTY.
    pub async fn checkpoint(&self) -> Result<TerminalOutputCheckpoint> {
        let response = self.open_output_stream(None).await?;
        let mut stream = response.bytes_stream();
        let mut pending = Vec::new();
        let read = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.context("terminal checkpoint SSE stream failed")?;
                ensure!(
                    pending.len().saturating_add(chunk.len()) <= 512 * 1024,
                    "terminal checkpoint SSE frame exceeded 512 KiB"
                );
                pending.extend_from_slice(&chunk);
                while let Some(end) = pending.windows(2).position(|bytes| bytes == b"\n\n") {
                    let frame = pending.drain(..end + 2).collect::<Vec<_>>();
                    if let Some(event) = terminal_event(&frame)?
                        && event["type"] == "connected"
                    {
                        return event["lastSeq"]
                            .as_u64()
                            .context("terminal connected event missing lastSeq");
                    }
                }
            }
            bail!("terminal SSE closed without a connected checkpoint event")
        };
        let timeout = self.case.timeout().min(Duration::from_secs(5));
        let last_seq = tokio::time::timeout(timeout, read)
            .await
            .with_context(|| format!("terminal {} checkpoint timed out", self.id))??;
        Ok(TerminalOutputCheckpoint {
            session_id: self.id.clone(),
            last_seq,
        })
    }

    /// Wait for data strictly newer than a captured checkpoint. An old replay
    /// or truncated history cannot satisfy the requested output assertion.
    /// The needle should not occur literally in the input command: a PTY can
    /// echo keystrokes, which are indistinguishable from stdout in the stream.
    pub async fn wait_output_after(
        &self,
        checkpoint: &TerminalOutputCheckpoint,
        needle: &str,
    ) -> Result<String> {
        ensure!(
            checkpoint.session_id == self.id,
            "terminal output checkpoint belongs to another session"
        );
        self.wait_output_from(Some(checkpoint.last_seq), needle)
            .await
    }

    async fn open_output_stream(&self, since: Option<u64>) -> Result<reqwest::Response> {
        let http = self.case.http().await;
        let mut path = self.path("/stream");
        if let Some(since) = since {
            path.push_str(&format!("?since={since}"));
        }
        let response = http.open_stream(http.request(Method::GET, &path)?).await?;
        ensure!(
            response.status().is_success(),
            "terminal SSE stream rejected with {}",
            response.status()
        );
        ensure!(
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|header| header.to_str().ok())
                .is_some_and(|value| value.starts_with("text/event-stream")),
            "terminal did not return a text/event-stream"
        );
        Ok(response)
    }

    /// Observe real SSE output. Sessions replay a bounded tail on connection;
    /// open the stream after sending input or before the next command. Neither
    /// the response nor its background PTY is buffered indefinitely.
    pub async fn wait_output_contains(&self, needle: &str) -> Result<String> {
        self.wait_output_from(None, needle).await
    }

    async fn wait_output_from(&self, since: Option<u64>, needle: &str) -> Result<String> {
        ensure!(!needle.is_empty(), "terminal output needle cannot be empty");
        let response = self.open_output_stream(since).await?;
        let mut stream = response.bytes_stream();
        let mut pending = Vec::new();
        let mut output = String::new();
        let bound = self.case.timeout().min(Duration::from_secs(15));
        let read = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.context("real terminal SSE stream failed")?;
                ensure!(
                    pending.len().saturating_add(chunk.len()) <= 512 * 1024,
                    "terminal SSE frame exceeded 512 KiB"
                );
                pending.extend_from_slice(&chunk);
                while let Some(end) = pending.windows(2).position(|bytes| bytes == b"\n\n") {
                    let frame = pending.drain(..end + 2).collect::<Vec<_>>();
                    if let Some(event) = terminal_event(&frame)? {
                        if event["type"] == "resync" {
                            bail!("terminal history was truncated since checkpoint {since:?}");
                        } else if event["type"] == "data" {
                            let seq = event["seq"]
                                .as_u64()
                                .context("terminal data event missing sequence")?;
                            if since.is_some_and(|cursor| seq <= cursor) {
                                continue;
                            }
                            let text = event["data"]
                                .as_str()
                                .context("missing terminal output text")?;
                            ensure!(
                                output.len().saturating_add(text.len()) <= 512 * 1024,
                                "terminal output exceeded the fixture 512 KiB budget"
                            );
                            output.push_str(text);
                            if output.contains(needle) {
                                return Ok(output);
                            }
                        } else if event["type"] == "exit" {
                            bail!(
                                "terminal exited before output {:?} appeared: {output:?}",
                                needle
                            );
                        }
                    }
                }
            }
            bail!(
                "terminal SSE closed before output {:?} appeared: {output:?}",
                needle
            )
        };
        tokio::time::timeout(bound, read).await.with_context(|| {
            format!(
                "terminal {} did not produce {:?} within {:?}",
                self.id, needle, bound
            )
        })?
    }
}

fn terminal_event(frame: &[u8]) -> Result<Option<Value>> {
    let frame = std::str::from_utf8(frame).context("terminal SSE was not UTF-8")?;
    for line in frame.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            return Ok(Some(
                serde_json::from_str(data).context("malformed terminal SSE event")?,
            ));
        }
    }
    Ok(None)
}
