use thiserror::Error;

#[derive(Debug, Error)]
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

pub type RolloutResult<T> = Result<T, RolloutError>;
