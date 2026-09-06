//! Bound each log segment even if a plugin never writes a newline. Invalid
//! UTF-8 is replaced for display and cannot stop draining the child's pipe.

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStderr;

use super::*;

const MAX_LOG_BYTES: usize = 16 * 1024;

impl Inner {
    pub(super) async fn drain_stderr(
        self: Arc<Self>,
        stderr: ChildStderr,
        stop: CancellationToken,
    ) {
        let mut reader = BufReader::new(stderr);
        let mut line = Vec::with_capacity(MAX_LOG_BYTES);
        let mut continuation = false;
        loop {
            let available = tokio::select! {
                biased;
                _ = stop.cancelled() => return,
                result = reader.fill_buf() => result,
            };
            let available = match available {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.record_log(
                        "warn",
                        "host",
                        format!("read plugin stderr: {error}"),
                        serde_json::Value::Null,
                    );
                    return;
                }
            };
            if available.is_empty() {
                if !line.is_empty() {
                    self.record_stderr(&line, continuation, false);
                }
                return;
            }
            let amount = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |offset| offset + 1)
                .min(MAX_LOG_BYTES - line.len());
            line.extend_from_slice(&available[..amount]);
            reader.consume(amount);
            if line.last() == Some(&b'\n') {
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                self.record_stderr(&line, continuation, false);
                line.clear();
                continuation = false;
            } else if line.len() == MAX_LOG_BYTES {
                // Retain an incomplete code point until the next segment,
                // including when earlier bytes in this segment were invalid.
                let end = complete_utf8_prefix(&line);
                self.record_stderr(&line[..end], continuation, true);
                line.drain(..end);
                continuation = true;
            }
        }
    }

    fn record_stderr(&self, bytes: &[u8], continuation: bool, continues: bool) {
        let message = String::from_utf8_lossy(bytes);
        tracing::info!(target: "agena_plugin_host::stdio_err", continuation, continues, "{message}");
        self.record_log(
            "info",
            "stderr",
            message.into_owned(),
            serde_json::json!({"continuation": continuation, "continues": continues}),
        );
    }
}

fn complete_utf8_prefix(bytes: &[u8]) -> usize {
    let mut offset = 0;
    while let Err(error) = std::str::from_utf8(&bytes[offset..]) {
        offset += error.valid_up_to();
        match error.error_len() {
            Some(length) => offset += length,
            None => return offset,
        }
    }
    bytes.len()
}
