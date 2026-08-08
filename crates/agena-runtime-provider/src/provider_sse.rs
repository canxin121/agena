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
}

/// Decode the longest UTF-8 prefix of `bytes` and return the leftover
/// trailing bytes (which may form a valid character once more bytes
/// arrive). Naive `String::from_utf8_lossy` per chunk would replace any
/// multi-byte character split across a chunk boundary with U+FFFD.
fn decode_utf8_prefix(bytes: &[u8]) -> (String, Vec<u8>) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), Vec::new()),
        Err(err) => {
            let valid_up_to = err.valid_up_to();
            // SAFETY: valid_up_to is the length of the longest valid UTF-8 prefix.
            let head = unsafe { std::str::from_utf8_unchecked(&bytes[..valid_up_to]) }.to_owned();
            match err.error_len() {
                // Truncated trailing sequence — keep it for the next chunk.
                None => (head, bytes[valid_up_to..].to_vec()),
                // A genuinely invalid byte — replace with U+FFFD and discard it.
                Some(invalid_len) => {
                    let mut out = head;
                    out.push('\u{FFFD}');
                    let tail_start = valid_up_to + invalid_len;
                    let (tail_text, leftover) = decode_utf8_prefix(&bytes[tail_start..]);
                    out.push_str(tail_text.as_str());
                    (out, leftover)
                }
            }
        }
    }
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

fn payload_is_complete_json_or_done(payload: &str) -> bool {
    let payload = payload.trim();
    payload == "[DONE]" || serde_json::from_str::<Value>(payload).is_ok()
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
        let current_payload = data_lines.join("\n");
        let should_flush = !current_payload.is_empty()
            && payload_is_complete_json_or_done(current_payload.as_str())
            && starts_new_json_event(data);
        let flushed = if should_flush {
            flush_json_event_data_lines(data_lines)?
        } else {
            None
        };

        data_lines.push(data.to_owned());
        return Ok(flushed);
    }

    Ok(None)
}

pub fn json_events_with_done(
    response: reqwest::Response,
) -> std::pin::Pin<Box<dyn Stream<Item = Result<JsonEventPayload, ProviderJsonStreamError>> + Send>>
{
    Box::pin(try_stream! {
        let mut buffer = String::new();
        let mut byte_carry: Vec<u8> = Vec::new();
        let mut data_lines: Vec<String> = Vec::new();
        let mut stream = response.bytes_stream();

        let mut done = false;
        'body: while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let combined = if byte_carry.is_empty() {
                chunk.to_vec()
            } else {
                let mut merged = std::mem::take(&mut byte_carry);
                merged.extend_from_slice(&chunk);
                merged
            };
            let (text, leftover) = decode_utf8_prefix(&combined);
            byte_carry = leftover;
            buffer.push_str(text.as_str());

            while let Some(idx) = buffer.find('\n') {
                let mut line = buffer[..idx].to_owned();
                buffer = buffer[idx + 1..].to_owned();

                if let Some(stripped) = line.strip_suffix('\r') {
                    line = stripped.to_owned();
                }

                if let Some(payload) = consume_json_event_line(line.as_str(), &mut data_lines)? {
                    match payload {
                        JsonEventPayload::Event(value) => {
                            yield JsonEventPayload::Event(value);
                        }
                        JsonEventPayload::Done => {
                            done = true;
                            yield JsonEventPayload::Done;
                            break 'body;
                        }
                    }
                }
            }
        }

        if !done && !buffer.is_empty() {
            if let Some(stripped) = buffer.strip_suffix('\r') {
                buffer = stripped.to_owned();
            }

            if let Some(payload) = consume_json_event_line(buffer.as_str(), &mut data_lines)? {
                match payload {
                    JsonEventPayload::Event(value) => {
                        yield JsonEventPayload::Event(value);
                    }
                    JsonEventPayload::Done => {
                        done = true;
                        yield JsonEventPayload::Done;
                    }
                }
            }
        }

        if !done {
            match flush_json_event_data_lines(&mut data_lines)? {
                Some(JsonEventPayload::Event(value)) => {
                    yield JsonEventPayload::Event(value);
                }
                Some(JsonEventPayload::Done) | None => {}
            }
        }
    })
}

pub fn json_events(
    response: reqwest::Response,
) -> std::pin::Pin<Box<dyn Stream<Item = Result<Value, ProviderJsonStreamError>> + Send>> {
    let mut events = json_events_with_done(response);
    Box::pin(try_stream! {
        while let Some(event) = events.next().await {
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
    Box::pin(try_stream! {
        let mut buffer = String::new();
        let mut byte_carry: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let combined = if byte_carry.is_empty() {
                chunk.to_vec()
            } else {
                let mut merged = std::mem::take(&mut byte_carry);
                merged.extend_from_slice(&chunk);
                merged
            };
            let (text, leftover) = decode_utf8_prefix(&combined);
            byte_carry = leftover;
            buffer.push_str(text.as_str());

            while let Some(idx) = buffer.find('\n') {
                let mut line = buffer[..idx].to_owned();
                buffer = buffer[idx + 1..].to_owned();

                if let Some(stripped) = line.strip_suffix('\r') {
                    line = stripped.to_owned();
                }

                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let value = serde_json::from_str::<Value>(line).map_err(|source| {
                    ProviderJsonStreamError::InvalidJson {
                        format: "JSON line",
                        source,
                    }
                })?;
                yield value;
            }
        }

        let remaining = buffer.trim();
        if !remaining.is_empty() {
            let value = serde_json::from_str::<Value>(remaining).map_err(|source| {
                ProviderJsonStreamError::InvalidJson {
                    format: "JSON line",
                    source,
                }
            })?;
            yield value;
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
