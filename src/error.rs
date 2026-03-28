use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    ApiError,
    ContextOverflow,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),
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
    #[error("invalid role value in storage: {0}")]
    InvalidRole(String),
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
