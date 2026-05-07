use async_stream::try_stream;
use futures_core::Stream;
use futures_util::StreamExt;
use serde_json::Value;

use crate::error::AppError;

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

pub fn json_events(
    response: reqwest::Response,
) -> std::pin::Pin<Box<dyn Stream<Item = Result<Value, AppError>> + Send>> {
    Box::pin(try_stream! {
        let mut buffer = String::new();
        let mut byte_carry: Vec<u8> = Vec::new();
        let mut data_lines: Vec<String> = Vec::new();
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

                if line.is_empty() {
                    if data_lines.is_empty() {
                        continue;
                    }

                    let payload = data_lines.join("\n");
                    data_lines.clear();

                    if payload.trim() == "[DONE]" {
                        continue;
                    }

                    let value: Value = serde_json::from_str(payload.as_str())
                        .map_err(|e| AppError::Provider(format!("invalid sse json payload: {e}")))?;
                    yield value;
                    continue;
                }

                if let Some(data) = line.strip_prefix("data:") {
                    data_lines.push(data.trim_start().to_owned());
                }
            }
        }

        if !data_lines.is_empty() {
            let payload = data_lines.join("\n");
            if payload.trim() != "[DONE]" {
                let value: Value = serde_json::from_str(payload.as_str())
                    .map_err(|e| AppError::Provider(format!("invalid sse json payload: {e}")))?;
                yield value;
            }
        }
    })
}

#[allow(dead_code)]
pub fn json_lines(
    response: reqwest::Response,
) -> std::pin::Pin<Box<dyn Stream<Item = Result<Value, AppError>> + Send>> {
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

                let value = serde_json::from_str::<Value>(line)
                    .map_err(|e| AppError::Provider(format!("invalid json line payload: {e}")))?;
                yield value;
            }
        }

        let remaining = buffer.trim();
        if !remaining.is_empty() {
            let value = serde_json::from_str::<Value>(remaining)
                .map_err(|e| AppError::Provider(format!("invalid json line payload: {e}")))?;
            yield value;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_utf8_prefix_holds_back_partial_multibyte_tail() {
        // Chinese "你" is E4 BD A0 in UTF-8. Split mid-character.
        let bytes = b"hi \xe4\xbd";
        let (text, carry) = decode_utf8_prefix(bytes);
        assert_eq!(text, "hi ");
        assert_eq!(carry, vec![0xe4, 0xbd]);

        // Now feed the rest plus more.
        let mut combined = carry.clone();
        combined.extend_from_slice(b"\xa0!");
        let (text, carry) = decode_utf8_prefix(&combined);
        assert_eq!(text, "你!");
        assert!(carry.is_empty());
    }

    #[test]
    fn decode_utf8_prefix_passes_through_invalid_bytes_as_replacement() {
        let bytes = b"a\xffb";
        let (text, carry) = decode_utf8_prefix(bytes);
        assert_eq!(text, "a\u{FFFD}b");
        assert!(carry.is_empty());
    }
}
