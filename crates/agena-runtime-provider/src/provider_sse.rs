//! SSE parsing helpers for streaming provider responses.

use async_stream::try_stream;
use futures_core::Stream;
use futures_util::StreamExt;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
/// Error reading a provider JSON stream.
pub enum ProviderJsonStreamError {
    #[error("HTTP stream error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid {format} payload: {source}")]
    InvalidJson {
        format: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("provider stream frame exceeds the {limit}-byte limit")]
    FrameLimit { limit: usize },
    #[error("provider stream decoder worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
}

/// Decode the longest UTF-8 prefix of `bytes` and return the leftover
/// trailing bytes (which may form a valid character once more bytes
/// arrive). Naive `String::from_utf8_lossy` per chunk would replace any
/// multi-byte character split across a chunk boundary with U+FFFD.
fn decode_utf8_prefix(bytes: &[u8]) -> (String, Vec<u8>) {
    let mut text = String::with_capacity(bytes.len());
    let mut consumed = 0;
    while consumed < bytes.len() {
        let remaining = &bytes[consumed..];
        match std::str::from_utf8(remaining) {
            Ok(valid) => {
                text.push_str(valid);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                // SAFETY: the decoder identified this entire prefix as UTF-8.
                text.push_str(unsafe { std::str::from_utf8_unchecked(&remaining[..valid]) });
                consumed += valid;
                match error.error_len() {
                    Some(length) => {
                        text.push('\u{FFFD}');
                        consumed += length;
                    }
                    None => return (text, bytes[consumed..].to_vec()),
                }
            }
        }
    }
    (text, Vec::new())
}

#[derive(Debug, PartialEq)]
/// Payload of a provider JSON event.
pub enum JsonEventPayload {
    Event(Value),
    Done,
}

fn parse_json_event_payload(payload: &str) -> Result<JsonEventPayload, ProviderJsonStreamError> {
    let payload = payload.trim();
    if payload == "[DONE]" {
        return Ok(JsonEventPayload::Done);
    }

    let value = serde_json::from_str::<Value>(payload).map_err(|source| {
        ProviderJsonStreamError::InvalidJson {
            format: "SSE JSON",
            source,
        }
    })?;
    Ok(JsonEventPayload::Event(value))
}

fn flush_json_event_data_lines(
    data_lines: &mut Vec<String>,
) -> Result<Option<JsonEventPayload>, ProviderJsonStreamError> {
    if data_lines.is_empty() {
        return Ok(None);
    }

    let payload = data_lines.join("\n");
    data_lines.clear();
    parse_json_event_payload(payload.as_str()).map(Some)
}

fn starts_new_json_event(data: &str) -> bool {
    let data = data.trim_start();
    data == "[DONE]" || data.starts_with('{') || data.starts_with('[')
}

fn consume_json_event_line(
    line: &str,
    data_lines: &mut Vec<String>,
) -> Result<Option<JsonEventPayload>, ProviderJsonStreamError> {
    if line.is_empty() {
        return flush_json_event_data_lines(data_lines);
    }

    if let Some(data) = line.strip_prefix("data:") {
        let data = data.trim_start();
        // Only probe completion when another frame begins, and reuse the
        // parsed value instead of decoding the same complete JSON twice.
        let flushed = if !data_lines.is_empty() && starts_new_json_event(data) {
            match parse_json_event_payload(&data_lines.join("\n")) {
                Ok(payload) => {
                    data_lines.clear();
                    Some(payload)
                }
                Err(_) => None,
            }
        } else {
            None
        };

        data_lines.push(data.to_owned());
        return Ok(flushed);
    }

    Ok(None)
}

const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
const INLINE_DECODE_BYTES: usize = 64 * 1024;
static STREAM_DECODERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

type FrameResult = Result<JsonEventPayload, ProviderJsonStreamError>;

struct FrameDecoder {
    buffer: String,
    byte_carry: Vec<u8>,
    data_lines: Vec<String>,
    data_bytes: usize,
    sse: bool,
    done: bool,
}

impl FrameDecoder {
    fn new(sse: bool) -> Self {
        Self {
            buffer: String::new(),
            byte_carry: Vec::new(),
            data_lines: Vec::new(),
            data_bytes: 0,
            sse,
            done: false,
        }
    }

    fn line(&mut self, line: &str) -> Result<Option<JsonEventPayload>, ProviderJsonStreamError> {
        if line.len() > MAX_FRAME_BYTES {
            return Err(ProviderJsonStreamError::FrameLimit {
                limit: MAX_FRAME_BYTES,
            });
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        if !self.sse {
            let line = line.trim();
            if line.is_empty() {
                return Ok(None);
            }
            return serde_json::from_str(line)
                .map(JsonEventPayload::Event)
                .map(Some)
                .map_err(|source| ProviderJsonStreamError::InvalidJson {
                    format: "JSON line",
                    source,
                });
        }
        let payload = consume_json_event_line(line, &mut self.data_lines)?;
        if self.data_lines.is_empty() {
            self.data_bytes = 0;
        } else if let Some(data) = line.strip_prefix("data:") {
            let length = data.trim_start().len();
            self.data_bytes = if payload.is_some() {
                length
            } else {
                self.data_bytes.saturating_add(length).saturating_add(1)
            };
        }
        if self.data_bytes > MAX_FRAME_BYTES {
            return Err(ProviderJsonStreamError::FrameLimit {
                limit: MAX_FRAME_BYTES,
            });
        }
        Ok(payload)
    }

    fn push_payload(&mut self, output: &mut Vec<FrameResult>, payload: Option<JsonEventPayload>) {
        if let Some(payload) = payload {
            self.done = matches!(payload, JsonEventPayload::Done);
            output.push(Ok(payload));
        }
    }

    fn feed(&mut self, chunk: &[u8], eof: bool) -> Vec<FrameResult> {
        let mut output = Vec::new();
        if self.done {
            return output;
        }
        let result = (|| -> Result<(), ProviderJsonStreamError> {
            if chunk.len() > MAX_FRAME_BYTES {
                return Err(ProviderJsonStreamError::FrameLimit {
                    limit: MAX_FRAME_BYTES,
                });
            }
            let (text, leftover) = if self.byte_carry.is_empty() {
                decode_utf8_prefix(chunk)
            } else {
                let mut combined = std::mem::take(&mut self.byte_carry);
                combined.extend_from_slice(chunk);
                decode_utf8_prefix(&combined)
            };
            self.byte_carry = leftover;
            let mut buffer = std::mem::take(&mut self.buffer);
            buffer.push_str(&text);
            let mut consumed = 0;
            while let Some(offset) = buffer[consumed..].find('\n') {
                let end = consumed + offset;
                let payload = self.line(&buffer[consumed..end])?;
                self.push_payload(&mut output, payload);
                consumed = end + 1;
                if self.done {
                    break;
                }
            }
            if !self.done {
                // Shift the unfinished tail once per network chunk, not once
                // per line. A chunk of many tiny frames stays linear to scan.
                buffer.drain(..consumed);
                if buffer.len() > MAX_FRAME_BYTES {
                    return Err(ProviderJsonStreamError::FrameLimit {
                        limit: MAX_FRAME_BYTES,
                    });
                }
                self.buffer = buffer;
                if eof {
                    if !self.byte_carry.is_empty() {
                        self.buffer
                            .push_str(&String::from_utf8_lossy(&self.byte_carry));
                        self.byte_carry.clear();
                    }
                    let tail = std::mem::take(&mut self.buffer);
                    if !tail.is_empty() {
                        let payload = self.line(&tail)?;
                        self.push_payload(&mut output, payload);
                    }
                    if self.sse && !self.done {
                        let payload = flush_json_event_data_lines(&mut self.data_lines)?;
                        self.push_payload(&mut output, payload);
                    }
                    self.done = true;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.done = true;
            output.push(Err(error));
        }
        output
    }
}

async fn decode_chunk(
    mut decoder: FrameDecoder,
    chunk: bytes::Bytes,
    eof: bool,
) -> Result<(FrameDecoder, Vec<FrameResult>), ProviderJsonStreamError> {
    if chunk
        .len()
        .saturating_add(decoder.buffer.len())
        .saturating_add(decoder.data_bytes)
        < INLINE_DECODE_BYTES
    {
        let output = decoder.feed(&chunk, eof);
        return Ok((decoder, output));
    }
    Ok(STREAM_DECODERS
        .run(move || {
            let output = decoder.feed(&chunk, eof);
            (decoder, output)
        })
        .await?)
}

fn framed_json(
    response: reqwest::Response,
    sse: bool,
) -> std::pin::Pin<Box<dyn Stream<Item = FrameResult> + Send>> {
    Box::pin(try_stream! {
        let mut decoder = FrameDecoder::new(sse);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            tokio::task::consume_budget().await;
            let (next, events) = decode_chunk(decoder, chunk?, false).await?;
            decoder = next;
            for event in events {
                tokio::task::consume_budget().await;
                yield event?;
            }
            if decoder.done { return; }
        }
        let (_, events) = decode_chunk(decoder, bytes::Bytes::new(), true).await?;
        for event in events {
            tokio::task::consume_budget().await;
            yield event?;
        }
    })
}

pub fn json_events_with_done(
    response: reqwest::Response,
) -> std::pin::Pin<Box<dyn Stream<Item = FrameResult> + Send>> {
    framed_json(response, true)
}

pub fn json_events(
    response: reqwest::Response,
) -> std::pin::Pin<Box<dyn Stream<Item = Result<Value, ProviderJsonStreamError>> + Send>> {
    let mut events = json_events_with_done(response);
    Box::pin(try_stream! {
        while let Some(event) = events.next().await {
            tokio::task::consume_budget().await;
            match event? {
                JsonEventPayload::Event(value) => yield value,
                JsonEventPayload::Done => break,
            }
        }
    })
}

pub fn json_lines(
    response: reqwest::Response,
) -> std::pin::Pin<Box<dyn Stream<Item = Result<Value, ProviderJsonStreamError>> + Send>> {
    let mut frames = framed_json(response, false);
    Box::pin(try_stream! {
        while let Some(frame) = frames.next().await {
            tokio::task::consume_budget().await;
            if let JsonEventPayload::Event(value) = frame? { yield value; }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{JsonEventPayload, consume_json_event_line, parse_json_event_payload};
    use serde_json::json;

    #[test]
    fn done_marker_is_a_terminal_payload() {
        assert_eq!(
            parse_json_event_payload(" [DONE] ").expect("parse marker"),
            JsonEventPayload::Done
        );
    }

    #[test]
    fn consecutive_json_data_frames_are_not_coalesced() {
        let mut lines = Vec::new();
        assert!(
            consume_json_event_line("data: {\"sequence\":1}", &mut lines)
                .expect("first frame")
                .is_none()
        );
        assert_eq!(
            consume_json_event_line("data: {\"sequence\":2}", &mut lines).expect("second frame"),
            Some(JsonEventPayload::Event(json!({ "sequence": 1 })))
        );
        assert_eq!(
            consume_json_event_line("", &mut lines).expect("flush second frame"),
            Some(JsonEventPayload::Event(json!({ "sequence": 2 })))
        );
    }
}
