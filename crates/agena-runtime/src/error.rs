use agena_provider::ProviderErrorKind;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error(transparent)]
    ConfigErr(Box<agena_runtime::ConfigError>),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("serde json error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("http client error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    StorageConfig(#[from] agena_storage::StorageConfigError),
    #[error("{provider} provider error: {message} (kind={kind:?}, retryable={retryable})")]
    ProviderClassified {
        provider: String,
        message: String,
        kind: ProviderErrorKind,
        retryable: bool,
    },
    #[error(
        "{provider} api request failed with status {status}: {body} (kind={kind:?}, retryable={retryable})"
    )]
    HttpStatus {
        provider: String,
        status: reqwest::StatusCode,
        body: String,
        kind: ProviderErrorKind,
        retryable: bool,
    },
    #[error(
        "session {session_id} version conflict: expected {expected}, current {current} \
         (a concurrent writer raced ahead — reload and retry)"
    )]
    Conflict {
        session_id: i64,
        expected: i64,
        current: i64,
    },
    #[error("execution cancelled")]
    Cancelled,
    #[error("session {0} already has an active execution")]
    ExecutionAlreadyActive(i64),
    #[error("session {0} has no active execution")]
    NoActiveExecution(i64),
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn retryable(&self) -> bool {
        match self {
            Self::HttpStatus { retryable, .. } | Self::ProviderClassified { retryable, .. } => {
                *retryable
            }
            Self::Http(err) => err.is_timeout() || err.is_connect(),
            _ => false,
        }
    }

    pub fn provider_error_kind(&self) -> Option<ProviderErrorKind> {
        match self {
            Self::HttpStatus { kind, .. } | Self::ProviderClassified { kind, .. } => Some(*kind),
            _ => None,
        }
    }
}

impl From<agena_provider::ToolStreamError> for AppError {
    fn from(error: agena_provider::ToolStreamError) -> Self {
        Self::Provider(error.to_string())
    }
}

impl From<agena_provider::ProviderToolModeViolation> for AppError {
    fn from(error: agena_provider::ProviderToolModeViolation) -> Self {
        Self::Provider(error.to_string())
    }
}

impl From<agena_runtime::ProviderJsonStreamError> for AppError {
    fn from(error: agena_runtime::ProviderJsonStreamError) -> Self {
        Self::Provider(error.to_string())
    }
}

impl From<agena_runtime::ConfigError> for AppError {
    fn from(value: agena_runtime::ConfigError) -> Self {
        Self::ConfigErr(Box::new(value))
    }
}
