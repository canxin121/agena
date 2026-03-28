use thiserror::Error;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("invalid sandbox request: {0}")]
    InvalidRequest(String),
    #[error("sandbox denied execution: {0}")]
    Denied(String),
    #[error("sandbox audit failed: {0}")]
    AuditFailed(String),
    #[error("sandbox is unavailable on this platform: {0}")]
    Unavailable(String),
    #[error("windows sandbox error: {0}")]
    Windows(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
