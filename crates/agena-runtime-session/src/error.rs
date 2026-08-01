use agena_provider::ProviderErrorKind;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error(transparent)]
    ConfigErr(Box<agena_runtime_config::ConfigError>),
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
    #[error("operation blocked by permission policy: {}", .0.reason)]
    PolicyDenied(Box<agena_domain::PolicyDeniedResult>),
    #[error("permission request declined by user")]
    UserDeclined(Box<agena_domain::UserDeclinedResult>),
    #[error("required execution capability is unavailable: {}", .0.reason)]
    CapabilityUnavailable(Box<agena_domain::CapabilityUnavailableResult>),
    #[error("tool is unavailable: {}", .0.reason)]
    ToolUnavailable(Box<agena_domain::ToolUnavailableResult>),
    #[error("tool execution error: {0}")]
    Tool(Box<crate::tool::ToolError>),
    #[error("subtask usage budget exceeded: {0}")]
    SubtaskBudgetExceeded(String),
    #[error("session {0} already has an active execution")]
    ExecutionAlreadyActive(i64),
    #[error("session {0} has no active execution")]
    NoActiveExecution(i64),
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    /// Safe, audience-neutral summary for durable user activity. The original
    /// `Display` remains diagnostic and must not be persisted or rendered.
    pub fn public_message(&self) -> &'static str {
        match self {
            Self::Config(_) | Self::ConfigErr(_) => {
                "The runtime configuration is invalid. Review the settings."
            }
            Self::ProviderClassified {
                kind: ProviderErrorKind::ContextOverflow,
                ..
            } => {
                "The conversation is too large for the selected model. Compact the conversation or choose a model with a larger context window."
            }
            Self::ProviderClassified {
                kind: ProviderErrorKind::Authentication,
                ..
            } => "Provider authentication failed. Sign in again or review the provider settings.",
            Self::ProviderClassified {
                kind: ProviderErrorKind::RateLimited,
                ..
            } => "The provider is rate-limiting requests. Try again shortly.",
            Self::ProviderClassified {
                kind: ProviderErrorKind::QuotaExceeded,
                ..
            } => {
                "The provider quota has been reached. Review the provider plan or choose another provider."
            }
            Self::ProviderClassified {
                kind: ProviderErrorKind::Timeout | ProviderErrorKind::Connection,
                ..
            } => "The provider could not be reached in time. Check the connection and try again.",
            Self::ProviderClassified {
                kind: ProviderErrorKind::InvalidRequest,
                ..
            } => {
                "The provider rejected the request. Review the model settings or choose another model."
            }
            Self::ProviderClassified {
                kind: ProviderErrorKind::Misconfiguration,
                ..
            } => "The provider is not configured correctly. Review the provider settings.",
            Self::HttpStatus { status, .. }
                if *status == reqwest::StatusCode::UNAUTHORIZED
                    || *status == reqwest::StatusCode::FORBIDDEN =>
            {
                "Provider authentication failed. Review the provider settings."
            }
            Self::HttpStatus { status, .. }
                if *status == reqwest::StatusCode::TOO_MANY_REQUESTS =>
            {
                "The provider is rate-limiting requests. Try again shortly."
            }
            Self::Provider(_)
            | Self::ProviderClassified { .. }
            | Self::HttpStatus { .. }
            | Self::Http(_) => {
                "The provider could not complete the response. Try again or choose another model."
            }
            Self::Conflict { .. } => {
                "The session changed while the request was running. Refresh and try again."
            }
            Self::Cancelled => "Response cancelled.",
            Self::SubtaskBudgetExceeded(_) => {
                "The subtask reached its usage limit before it could finish."
            }
            Self::ExecutionAlreadyActive(_) => "This session is already running a response.",
            Self::NoActiveExecution(_) => "This session has no active response.",
            Self::PolicyDenied(_)
            | Self::UserDeclined(_)
            | Self::CapabilityUnavailable(_)
            | Self::ToolUnavailable(_)
            | Self::Tool(_)
            | Self::Database(_)
            | Self::SerdeJson(_)
            | Self::Io(_)
            | Self::StorageConfig(_)
            | Self::Internal(_) => "Something went wrong.",
        }
    }

    /// Project an internal execution error into the safe command boundary.
    /// The caller must log `self` together with the returned id; diagnostic
    /// source data intentionally does not become part of this value.
    pub fn failure(&self) -> agena_failure::Failure {
        use agena_failure::{
            Failure, FailureCategory as Category, FailureCode, FailureImpact,
            FailureResponsibility as Responsibility, RecoveryDirective as Recovery,
            RetryDirective as Retry, UserPresentation,
        };

        let (code, category, responsibility, retry, recovery) = match self {
            Self::Config(_) | Self::ConfigErr(_) => (
                "configuration.invalid",
                Category::InvalidInput,
                Responsibility::Caller,
                Retry::CorrectInput,
                Recovery::OpenSettings,
            ),
            Self::ProviderClassified {
                kind: ProviderErrorKind::ContextOverflow,
                ..
            } => (
                "provider.context_overflow",
                Category::InvalidInput,
                Responsibility::Caller,
                Retry::AfterUserAction,
                Recovery::ChooseAlternative,
            ),
            Self::ProviderClassified {
                kind: ProviderErrorKind::Authentication,
                ..
            } => (
                "provider.authentication_required",
                Category::AuthenticationRequired,
                Responsibility::Caller,
                Retry::AfterUserAction,
                Recovery::Reauthenticate,
            ),
            Self::ProviderClassified {
                kind: ProviderErrorKind::RateLimited,
                ..
            } => (
                "provider.rate_limited",
                Category::RateLimited,
                Responsibility::Dependency,
                Retry::Backoff,
                Recovery::Retry,
            ),
            Self::ProviderClassified {
                kind: ProviderErrorKind::QuotaExceeded,
                ..
            } => (
                "provider.quota_exceeded",
                Category::QuotaExceeded,
                Responsibility::Caller,
                Retry::AfterUserAction,
                Recovery::ChooseAlternative,
            ),
            Self::ProviderClassified {
                kind: ProviderErrorKind::Timeout | ProviderErrorKind::Connection,
                ..
            } => (
                "provider.connection_failed",
                Category::Timeout,
                Responsibility::Dependency,
                Retry::Backoff,
                Recovery::Retry,
            ),
            Self::ProviderClassified {
                kind: ProviderErrorKind::InvalidRequest,
                ..
            } => (
                "provider.invalid_request",
                Category::InvalidInput,
                Responsibility::Caller,
                Retry::AfterUserAction,
                Recovery::ChooseAlternative,
            ),
            Self::ProviderClassified {
                kind: ProviderErrorKind::Misconfiguration,
                ..
            } => (
                "provider.misconfigured",
                Category::InvalidInput,
                Responsibility::Caller,
                Retry::CorrectInput,
                Recovery::OpenSettings,
            ),
            Self::HttpStatus { status, .. }
                if *status == reqwest::StatusCode::UNAUTHORIZED
                    || *status == reqwest::StatusCode::FORBIDDEN =>
            {
                (
                    "provider.authentication_required",
                    Category::AuthenticationRequired,
                    Responsibility::Caller,
                    Retry::AfterUserAction,
                    Recovery::Reauthenticate,
                )
            }
            Self::HttpStatus { status, .. }
                if *status == reqwest::StatusCode::TOO_MANY_REQUESTS =>
            {
                (
                    "provider.rate_limited",
                    Category::RateLimited,
                    Responsibility::Dependency,
                    Retry::Backoff,
                    Recovery::Retry,
                )
            }
            Self::Provider(_)
            | Self::ProviderClassified { .. }
            | Self::HttpStatus { .. }
            | Self::Http(_) => (
                "provider.request_failed",
                Category::DependencyUnavailable,
                Responsibility::Dependency,
                Retry::Backoff,
                Recovery::ChooseAlternative,
            ),
            Self::Conflict { .. } | Self::ExecutionAlreadyActive(_) => (
                "session.conflict",
                Category::Conflict,
                Responsibility::Caller,
                Retry::AfterRefresh,
                Recovery::Refresh,
            ),
            Self::Cancelled => (
                // Cancellation is a terminal outcome. If it reaches a
                // failure-only boundary, report the invariant violation
                // safely instead of reintroducing a cancelled error kind.
                "internal.cancelled_as_failure",
                Category::Internal,
                Responsibility::System,
                Retry::Unknown,
                Recovery::Retry,
            ),
            Self::SubtaskBudgetExceeded(_) => (
                "subtask.budget_exceeded",
                Category::QuotaExceeded,
                Responsibility::Caller,
                Retry::AfterUserAction,
                Recovery::None,
            ),
            Self::NoActiveExecution(_) => (
                "execution.not_active",
                Category::NotFound,
                Responsibility::Caller,
                Retry::AfterRefresh,
                Recovery::Refresh,
            ),
            Self::PolicyDenied(_)
            | Self::UserDeclined(_)
            | Self::CapabilityUnavailable(_)
            | Self::ToolUnavailable(_)
            | Self::Tool(_)
            | Self::Database(_)
            | Self::SerdeJson(_)
            | Self::Io(_)
            | Self::StorageConfig(_)
            | Self::Internal(_) => (
                "internal.unexpected",
                Category::Internal,
                Responsibility::System,
                Retry::Unknown,
                Recovery::Retry,
            ),
        };
        Failure::new(
            FailureCode::new(code),
            category,
            responsibility,
            retry,
            recovery,
            FailureImpact::RequestRejected,
            UserPresentation::new(code, self.public_message()),
        )
    }

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

impl From<agena_runtime_provider::ProviderError> for AppError {
    fn from(error: agena_runtime_provider::ProviderError) -> Self {
        match error {
            agena_runtime_provider::ProviderError::Config(message) => Self::Config(message),
            agena_runtime_provider::ProviderError::Provider(message) => Self::Provider(message),
            agena_runtime_provider::ProviderError::Database(message) => Self::Internal(message),
            agena_runtime_provider::ProviderError::SerdeJson(error) => Self::SerdeJson(error),
            agena_runtime_provider::ProviderError::Http(error) => Self::Http(error),
            agena_runtime_provider::ProviderError::Io(error) => Self::Io(error),
            agena_runtime_provider::ProviderError::HttpStatus {
                provider,
                status,
                body,
                kind,
                retryable,
            } => Self::HttpStatus {
                provider,
                status,
                body,
                kind,
                retryable,
            },
            agena_runtime_provider::ProviderError::ProviderClassified {
                provider,
                message,
                kind,
                retryable,
            } => Self::ProviderClassified {
                provider,
                message,
                kind,
                retryable,
            },
            agena_runtime_provider::ProviderError::Internal(message) => Self::Internal(message),
        }
    }
}

impl From<agena_runtime_provider::ProviderJsonStreamError> for AppError {
    fn from(error: agena_runtime_provider::ProviderJsonStreamError) -> Self {
        Self::Provider(error.to_string())
    }
}

impl From<agena_runtime_config::ConfigError> for AppError {
    fn from(value: agena_runtime_config::ConfigError) -> Self {
        Self::ConfigErr(Box::new(value))
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn provider_body_is_diagnostic_only() {
        let error = AppError::HttpStatus {
            provider: "example".to_owned(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            body: "token=secret internal upstream stack".to_owned(),
            kind: agena_provider::ProviderErrorKind::Unavailable,
            retryable: true,
        };
        assert!(!error.public_message().contains("token=secret"));
        assert!(!error.public_message().contains("stack"));
        assert!(error.to_string().contains("token=secret"));
    }

    #[test]
    fn context_overflow_has_actionable_public_message() {
        let error = AppError::ProviderClassified {
            provider: "example".to_owned(),
            message: "raw provider message".to_owned(),
            kind: agena_provider::ProviderErrorKind::ContextOverflow,
            retryable: false,
        };
        let message = error.public_message();
        assert!(message.contains("Compact"));
        assert!(!message.contains("raw provider message"));
    }

    #[test]
    fn execution_command_failure_excludes_internal_source_chain() {
        let error = AppError::Internal(
            "database error: token=secret Custom Error: /private/agena.sqlite".to_owned(),
        );
        let failure = error.failure();
        let command_error = crate::SessionExecutionCommandError::from_failure(failure.clone());
        let wire = serde_json::to_string(&failure).expect("serialize safe command failure");
        let display = command_error.to_string();

        assert!(!wire.contains("token=secret"));
        assert!(!wire.contains("/private/agena.sqlite"));
        assert!(!display.contains("database error"));
        assert!(!display.contains(failure.code.as_str()));
        assert!(display.contains("Something went wrong."));
        assert!(display.contains("Reference:"));
    }
}
