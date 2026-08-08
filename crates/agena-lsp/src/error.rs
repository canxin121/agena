//! LSP client error types.

use thiserror::Error;

#[derive(Debug, Error)]
/// Error from the LSP subsystem.
pub enum LspError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("transport closed before response arrived")]
    TransportClosed,
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("LSP error response: code={code} message={message}")]
    Server { code: i64, message: String },
    #[error("request timed out after {0}ms")]
    Timeout(u64),
    #[error("server not initialized")]
    NotInitialized,
    #[error("unknown LSP server: {0}")]
    UnknownServer(String),
}

/// Result alias for LSP operations.
pub type LspResult<T> = Result<T, LspError>;
