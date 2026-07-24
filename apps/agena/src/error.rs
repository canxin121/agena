#[derive(Debug, thiserror::Error)]
pub(crate) enum AgenaProcessError {
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("internal error: {0}")]
    Internal(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub(crate) type Result<T> = std::result::Result<T, AgenaProcessError>;
