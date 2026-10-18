//! Raw JSONL transfers. Archive bytes never enter a JSON command envelope.

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio_util::io::ReaderStream;

use super::*;

const MEDIA_TYPE: &str = "application/x-ndjson";

#[derive(serde::Deserialize)]
#[serde(tag = "kind", content = "data", deny_unknown_fields)]
enum ArchiveTerminalRecord {
    #[serde(rename = "footer")]
    Footer(ArchiveFooterReport),
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveFooterReport {
    entries: u64,
    records: u64,
    bytes: u64,
    #[serde(rename = "sha256")]
    _sha256: [u8; 32],
}

#[derive(Default)]
struct ArchiveFooter {
    line: Vec<u8>,
    last_line: Vec<u8>,
    discarding: bool,
    terminated: bool,
}

impl ArchiveFooter {
    fn observe(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.terminated = bytes.ends_with(b"\n");
        for piece in bytes.split_inclusive(|byte| *byte == b'\n') {
            if self.line.len().saturating_add(piece.len()) > 4096 {
                self.discarding = true;
                self.line.clear();
            }
            if !self.discarding {
                self.line.extend_from_slice(piece);
            }
            if piece.ends_with(b"\n") {
                self.last_line = std::mem::take(&mut self.line);
                self.discarding = false;
            }
        }
    }

    fn finish(self) -> Result<(), ClientError> {
        let complete = self.terminated
            && serde_json::from_slice::<ArchiveTerminalRecord>(&self.last_line).is_ok_and(
                |ArchiveTerminalRecord::Footer(report)| {
                    report.records > report.entries && report.bytes > 0
                },
            );
        if complete {
            Ok(())
        } else {
            Err(ClientError::Protocol(
                "session archive ended without a complete success footer".to_owned(),
            ))
        }
    }
}

impl AgenaClient {
    /// Stream the complete current-format archive into the caller's sink.
    /// A failed transfer retains any partial output and returns an error.
    pub async fn export_session_archive<W: AsyncWrite + Unpin + Send>(
        &self,
        session_id: i64,
        target: &mut W,
    ) -> Result<(), ClientError> {
        let response = self
            .send_request(
                reqwest::Method::GET,
                self.endpoint(&format!("/api/v1/sessions/{session_id}/export")),
                None,
                Some(MEDIA_TYPE),
            )
            .await?;
        if !response.status().is_success() {
            return self.parse_json::<()>(response).await;
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next());
        if !content_type.is_some_and(|value| value.trim().eq_ignore_ascii_case(MEDIA_TYPE)) {
            return Err(ClientError::Protocol(
                "session export did not return application/x-ndjson".to_owned(),
            ));
        }
        let mut chunks = response.bytes_stream();
        let mut footer = ArchiveFooter::default();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            footer.observe(&chunk);
            target.write_all(&chunk).await.map_err(|error| {
                ClientError::Transport(format!("write session archive: {error}"))
            })?;
        }
        footer.finish()?;
        target
            .flush()
            .await
            .map_err(|error| ClientError::Transport(format!("flush session archive: {error}")))
    }

    /// Consume a raw JSONL reader once. Reuse the same request identity and
    /// reopen the source when retrying: the kernel compares its verified
    /// archive digest and never creates a second session for a matching key.
    pub async fn import_session_archive<R: AsyncRead + Unpin + Send + 'static>(
        &self,
        request_id: agena_domain::CommandId,
        source: R,
    ) -> Result<agena_api::resource::SessionArchiveImportResource, ClientError> {
        // Refresh before consuming a non-replayable source such as stdin.
        // A streamed body is never silently replayed after being consumed.
        if self.authentication.password.is_some() {
            let generation = self.authentication.generation.load(Ordering::Acquire);
            self.refresh_password_session(generation).await?;
        }
        let stream = ReaderStream::with_capacity(source, 64 * 1024);
        let response = self
            .request_builder(
                reqwest::Method::POST,
                self.endpoint("/api/v1/sessions/import"),
                None,
                Some("application/json"),
            )
            .header(reqwest::header::CONTENT_TYPE, MEDIA_TYPE)
            .header("idempotency-key", request_id.to_string())
            .body(reqwest::Body::wrap_stream(stream))
            .send()
            .await?;
        self.parse_json(response).await
    }
}
