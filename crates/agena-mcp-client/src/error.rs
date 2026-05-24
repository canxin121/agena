use std::io;

use thiserror::Error;

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

impl From<rmcp::ErrorData> for McpError {
    fn from(error: rmcp::ErrorData) -> Self {
        Self::Rpc {
            code: i64::from(error.code.0),
            message: error.message.to_string(),
        }
    }
}

impl From<rmcp::service::ServiceError> for McpError {
    fn from(error: rmcp::service::ServiceError) -> Self {
        match error {
            rmcp::service::ServiceError::McpError(error) => error.into(),
            rmcp::service::ServiceError::TransportClosed => Self::TransportClosed,
            rmcp::service::ServiceError::TransportSend(error) => Self::Transport(error.to_string()),
            rmcp::service::ServiceError::UnexpectedResponse => {
                Self::Malformed("unexpected MCP response".to_string())
            }
            rmcp::service::ServiceError::Cancelled { reason } => {
                Self::Transport(reason.unwrap_or_else(|| "request cancelled".to_string()))
            }
            rmcp::service::ServiceError::Timeout { .. } => Self::Timeout,
            other => Self::Transport(other.to_string()),
        }
    }
}

impl From<rmcp::service::ClientInitializeError> for McpError {
    fn from(error: rmcp::service::ClientInitializeError) -> Self {
        match error {
            rmcp::service::ClientInitializeError::JsonRpcError(error) => error.into(),
            other => Self::Transport(other.to_string()),
        }
    }
}

pub type McpResult<T> = Result<T, McpError>;
