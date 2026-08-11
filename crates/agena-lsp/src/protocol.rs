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
//! Byte framing lives in `agena-stdio-codec`; this module owns only LSP's
//! typed JSON-RPC messages.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSONRPC_VERSION: &str = "2.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
/// Identifier of a JSON-RPC request.
pub enum RequestId {
    Number(i64),
    // Strings appear in the wild though we never produce them.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A JSON-RPC request.
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A JSON-RPC notification.
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A JSON-RPC response.
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A JSON-RPC error.
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone)]
/// A message received from an LSP server.
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
