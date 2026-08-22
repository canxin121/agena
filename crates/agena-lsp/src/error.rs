//! LSP client error types.

use thiserror::Error;

#[derive(Debug, Error)]
/// Error from the LSP subsystem.
pub enum LspError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("transport closed: {0}")]
    TransportClosed(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("LSP error response: code={code} message={message}")]
    Server { code: i64, message: String },
    #[error("{operation} timed out after {timeout_ms}ms: {source}")]
    Timeout {
        operation: String,
        timeout_ms: u64,
        #[source]
        source: tokio::time::error::Elapsed,
    },
    #[error("server not initialized")]
    NotInitialized,
    #[error("unknown LSP server: {0}")]
    UnknownServer(String),
}

impl LspError {
    pub(crate) fn transport_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Transport(agena_failure::diagnostic::format_error_chain(error))
    }

    pub(crate) fn protocol_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Protocol(agena_failure::diagnostic::format_error_chain(error))
    }

    pub(crate) fn transport_closed(
        context: impl AsRef<str>,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::TransportClosed(agena_failure::diagnostic::format_error_chain_with_context(
            context, error,
        ))
    }

    pub(crate) fn transport_closed_without_source(context: impl Into<String>) -> Self {
        Self::TransportClosed(context.into())
    }
}

/// Result alias for LSP operations.
pub type LspResult<T> = Result<T, LspError>;
