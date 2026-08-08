//! Rollout error types.

use thiserror::Error;

#[derive(Debug, Error)]
/// Error from the rollout recorder.
pub enum RolloutError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("malformed frame: {0}")]
    Malformed(String),

    #[error("session not found: {0}")]
    NotFound(String),
}

/// Result alias for rollout operations.
pub type RolloutResult<T> = Result<T, RolloutError>;
