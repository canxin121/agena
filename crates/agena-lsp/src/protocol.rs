//! JSON-RPC 2.0 framing helpers shared by every LSP transport.
//!
//! LSP wraps each JSON-RPC frame with an HTTP-style header block:
//!
//! ```text
//! Content-Length: 67\r\n
//! \r\n
//! {"jsonrpc":"2.0","id":1,"method":"initialize","params":{...}}
//! ```
//!
//! We expose [`encode_frame`] / [`decode_frames`] so transports stay tiny.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSONRPC_VERSION: &str = "2.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    // Strings appear in the wild though we never produce them.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone)]
pub enum InboundMessage {
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
    /// Server-to-client request — rare in agena's usage; we ack with an
    /// error to keep the server moving.
    Request(JsonRpcRequest),
}

impl InboundMessage {
    pub fn from_value(value: Value) -> Result<Self, serde_json::Error> {
        if value.get("id").is_some() {
            if value.get("method").is_some() {
                Ok(Self::Request(serde_json::from_value(value)?))
            } else {
                Ok(Self::Response(serde_json::from_value(value)?))
            }
        } else {
            Ok(Self::Notification(serde_json::from_value(value)?))
        }
    }
}

/// Encode a JSON value with the LSP `Content-Length` header.
pub fn encode_frame(payload: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(payload).expect("Value::serialize is infallible");
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    out
}

/// Stateful frame parser. Feed bytes via [`feed`] and pull complete
/// JSON-RPC payloads via [`take`]. Handles partial reads.
#[derive(Debug, Default)]
pub struct FrameParser {
    buf: Vec<u8>,
}

impl FrameParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    /// Try to extract the next complete frame. Returns `Ok(None)` when
    /// more bytes are needed, `Err` on a malformed header.
    pub fn take(&mut self) -> Result<Option<Value>, String> {
        let Some(sep) = find_subseq(&self.buf, b"\r\n\r\n") else {
            return Ok(None);
        };
        let header =
            std::str::from_utf8(&self.buf[..sep]).map_err(|e| format!("non-utf8 header: {e}"))?;
        let mut content_length: Option<usize> = None;
        for line in header.split("\r\n") {
            let mut split = line.splitn(2, ':');
            let key = split.next().unwrap_or("").trim();
            let value = split.next().unwrap_or("").trim();
            if key.eq_ignore_ascii_case("Content-Length") {
                content_length = Some(
                    value
                        .parse()
                        .map_err(|e| format!("invalid Content-Length: {e}"))?,
                );
            }
        }
        let Some(len) = content_length else {
            return Err("frame missing Content-Length header".to_string());
        };
        let body_start = sep + 4;
        let body_end = body_start + len;
        if self.buf.len() < body_end {
            return Ok(None);
        }
        let body = self.buf[body_start..body_end].to_vec();
        self.buf.drain(..body_end);
        let value: Value =
            serde_json::from_slice(&body).map_err(|e| format!("invalid json body: {e}"))?;
        Ok(Some(value))
    }
}

fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encode_then_decode_round_trip() {
        let value = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
        let bytes = encode_frame(&value);
        let mut parser = FrameParser::new();
        parser.feed(&bytes);
        let parsed = parser.take().unwrap().unwrap();
        assert_eq!(parsed, value);
        // Buffer fully consumed.
        assert!(parser.take().unwrap().is_none());
    }

    #[test]
    fn frame_parser_handles_partial_reads() {
        let value = json!({"a": 1});
        let bytes = encode_frame(&value);
        let mut parser = FrameParser::new();
        // Feed one byte at a time.
        for b in &bytes {
            parser.feed(&[*b]);
        }
        let parsed = parser.take().unwrap().unwrap();
        assert_eq!(parsed, value);
    }

    #[test]
    fn frame_parser_decodes_two_back_to_back_frames() {
        let a = json!({"a": 1});
        let b = json!({"b": 2});
        let mut bytes = encode_frame(&a);
        bytes.extend_from_slice(&encode_frame(&b));
        let mut parser = FrameParser::new();
        parser.feed(&bytes);
        assert_eq!(parser.take().unwrap().unwrap(), a);
        assert_eq!(parser.take().unwrap().unwrap(), b);
        assert!(parser.take().unwrap().is_none());
    }

    #[test]
    fn frame_parser_rejects_missing_content_length() {
        let mut parser = FrameParser::new();
        parser.feed(b"X-Header: value\r\n\r\n{}");
        let err = parser.take().unwrap_err();
        assert!(err.contains("Content-Length"));
    }

    #[test]
    fn inbound_message_classifies_by_shape() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"x"});
        assert!(matches!(
            InboundMessage::from_value(req).unwrap(),
            InboundMessage::Request(_)
        ));
        let resp = json!({"jsonrpc":"2.0","id":1,"result":null});
        assert!(matches!(
            InboundMessage::from_value(resp).unwrap(),
            InboundMessage::Response(_)
        ));
        let note = json!({"jsonrpc":"2.0","method":"x"});
        assert!(matches!(
            InboundMessage::from_value(note).unwrap(),
            InboundMessage::Notification(_)
        ));
    }
}
