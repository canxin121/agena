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

    #[error("permission for an MCP authentication endpoint was not granted")]
    PermissionDenied,

    #[error("client shutting down")]
    Shutdown,

    #[error("sampling not supported by this client")]
    SamplingUnsupported,

    #[error("server '{0}' not connected")]
    ServerNotConnected(String),

    #[error("MCP server '{server}' does not advertise {capability}")]
    CapabilityUnavailable { server: String, capability: String },

    #[error("MCP tool '{tool}' is disallowed by configured policy for server '{server}'")]
    ToolDisallowed { server: String, tool: String },
}

impl McpError {
    /// Public status and management surfaces use this projection rather than
    /// source diagnostics, which may contain credential-bearing endpoint URLs.
    pub fn user_message(&self) -> String {
        match self {
            Self::PermissionDenied => {
                "Permission for an MCP authentication endpoint was not granted.".into()
            }
            Self::Auth(_) => {
                "The MCP connection requires valid credentials or OAuth sign-in.".into()
            }
            Self::Timeout(_) => {
                "The MCP server did not respond before the configured timeout.".into()
            }
            Self::ServerNotConnected(_) | Self::TransportClosed => {
                "The MCP connection is disconnected.".into()
            }
            Self::Transport(_) | Self::Io(_) | Self::Http(_) => {
                "Could not communicate with the MCP server. Check its process or endpoint settings."
                    .into()
            }
            Self::Malformed(_) | Self::Serde(_) | Self::Rpc { .. } => {
                "The MCP server returned an incompatible protocol response.".into()
            }
            Self::Shutdown => "The MCP client is shutting down.".into(),
            Self::SamplingUnsupported | Self::CapabilityUnavailable { .. } => {
                "The MCP connection does not support this capability.".into()
            }
            Self::ToolDisallowed { .. } => {
                "The MCP tool is disabled by the connection's tool filters.".into()
            }
        }
    }

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
