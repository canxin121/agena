//! MCP client error types.

use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
/// Error from the MCP client.
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

    #[error("MCP authentication error: {0}")]
    Auth(String),

    #[error("server returned error: code={code} message={message}")]
    Rpc { code: i64, message: String },

    #[error("server returned malformed response: {0}")]
    Malformed(String),

    #[error("request timed out: {0}")]
    Timeout(String),

    #[error("client shutting down")]
    Shutdown,

    #[error("sampling not supported by this client")]
    SamplingUnsupported,

    #[error("server '{0}' not connected")]
    ServerNotConnected(String),

    #[error("MCP tool '{tool}' is disallowed by configured policy for server '{server}'")]
    ToolDisallowed { server: String, tool: String },
}

impl McpError {
    pub fn transport_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Transport(agena_failure::diagnostic::format_error_chain(error))
    }

    pub fn http_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Http(agena_failure::diagnostic::format_error_chain(error))
    }

    pub fn auth_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Auth(agena_failure::diagnostic::format_error_chain(error))
    }

    pub fn timeout_error(
        context: impl AsRef<str>,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::Timeout(agena_failure::diagnostic::format_error_chain_with_context(
            context, error,
        ))
    }
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
            rmcp::service::ServiceError::TransportSend(error) => Self::transport_error(&error),
            rmcp::service::ServiceError::UnexpectedResponse => {
                Self::Malformed("unexpected MCP response".to_string())
            }
            rmcp::service::ServiceError::Cancelled { reason } => {
                Self::Transport(reason.unwrap_or_else(|| "request cancelled".to_string()))
            }
            timeout @ rmcp::service::ServiceError::Timeout { .. } => {
                Self::timeout_error("MCP service request timed out", &timeout)
            }
            other => Self::transport_error(&other),
        }
    }
}

impl From<rmcp::service::ClientInitializeError> for McpError {
    fn from(error: rmcp::service::ClientInitializeError) -> Self {
        match error {
            rmcp::service::ClientInitializeError::JsonRpcError(error) => error.into(),
            other => Self::transport_error(&other),
        }
    }
}

/// Result alias for MCP client operations.
pub type McpResult<T> = Result<T, McpError>;
