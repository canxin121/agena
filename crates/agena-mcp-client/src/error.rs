//! Errors surfaced by the MCP client.

use std::io;

use thiserror::Error;

use crate::protocol::JsonRpcError;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("transport closed")]
    TransportClosed,

    #[error("transport error: {0}")]
    Transport(String),

    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("http error: {0}")]
    Http(String),

    #[error("server returned error: code={code} message={message}")]
    Rpc { code: i64, message: String },

    #[error("server returned malformed response: {0}")]
    Malformed(String),

    #[error("request timed out")]
    Timeout,

    #[error("client shutting down")]
    Shutdown,

    #[error("sampling not supported by this client")]
    SamplingUnsupported,

    #[error("server '{0}' not connected")]
    ServerNotConnected(String),
}

impl From<JsonRpcError> for McpError {
    fn from(e: JsonRpcError) -> Self {
        Self::Rpc {
            code: e.code,
            message: e.message,
        }
    }
}

impl From<reqwest::Error> for McpError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e.to_string())
    }
}

pub type McpResult<T> = Result<T, McpError>;
