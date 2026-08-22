use agena_provider::ProviderErrorKind;
use thiserror::Error;

/// SQLite busy errors are transient database-lock conflicts: another process
/// holds the write lock. They should be retried rather than reported as a
/// terminal internal error.
pub(crate) fn is_database_busy(error: &sea_orm::DbErr) -> bool {
    agena_storage_sqlite::is_sqlite_busy(error)
}

#[derive(Debug, Error)]
/// Top-level application error of the session runtime.
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
    #[error("automatic approval classification failed: {0}")]
    AutoApproveClassifyFailed(agena_permission::ClassifyFailure),
    #[error("subtask usage budget exceeded: {0}")]
    SubtaskBudgetExceeded(String),
    #[error("session {0} already has an active execution")]
    ExecutionAlreadyActive(i64),
    #[error("session {0} mutation queue is full or did not become available before its deadline")]
    SessionMutationBusy(i64),
    #[error(
        "nested session mutation is forbidden: task holds session {held_session_id} and requested session {requested_session_id}"
    )]
    NestedSessionMutation {
        held_session_id: i64,
        requested_session_id: i64,
    },
    #[error("model returned an empty response")]
    EmptyResponse,
    #[error("model-turn budget exhausted (max_turns={max_turns}); the run stopped")]
    ModelTurnBudgetExhausted { max_turns: usize },
    #[error("session {0} has no active execution")]
    NoActiveExecution(i64),
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn config_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Config(agena_failure::diagnostic::format_error_chain(error))
    }

    pub fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Internal(agena_failure::diagnostic::format_error_chain(error))
    }

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
            Self::AutoApproveClassifyFailed(_) => {
                "Automatic approval could not classify this request. Choose an option below."
            }
            Self::SubtaskBudgetExceeded(_) => {
                "The subtask reached its usage limit before it could finish."
            }
            Self::ExecutionAlreadyActive(_) => "This session is already running a response.",
            Self::SessionMutationBusy(_) => {
                "This session is busy applying another change. Try again shortly."
            }
            Self::EmptyResponse => {
                "The model returned an empty response. It may be temporarily unavailable or misconfigured; try again or choose another model."
            }
            Self::ModelTurnBudgetExhausted { .. } => {
                "The run reached the configured model-turn cap and stopped. Send a new message to continue, or raise the cap via `session.max_turns` in the config (`0` means unlimited)."
            }
            Self::NoActiveExecution(_) => "This session has no active response.",
            Self::Database(error) if is_database_busy(error) => {
                "The database is busy. Try again in a moment."
            }
            Self::PolicyDenied(_)
            | Self::UserDeclined(_)
            | Self::CapabilityUnavailable(_)
            | Self::ToolUnavailable(_)
            | Self::Tool(_)
            | Self::Database(_)
            | Self::SerdeJson(_)
            | Self::Io(_)
            | Self::StorageConfig(_)
            | Self::NestedSessionMutation { .. }
            | Self::Internal(_) => "The operation failed. Review the logs for details.",
        }
    }

    /// Project an internal execution error into the safe command boundary.
    /// The caller must log `self` together with the returned id; diagnostic
    /// source data intentionally does not become part of this value. Root
    /// causes are extracted and scrubbed for the user channel so a real,
    /// human-readable message survives instead of a generic fallback.
    pub fn failure(&self) -> agena_failure::Failure {
        use agena_failure::{
            Failure, FailureCategory as Category, FailureCode, FailureImpact,
            FailureResponsibility as Responsibility, RecoveryDirective as Recovery,
            RetryDirective as Retry,
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
            Self::Conflict { .. }
            | Self::ExecutionAlreadyActive(_)
            | Self::SessionMutationBusy(_) => (
                "session.conflict",
                Category::Conflict,
                Responsibility::Caller,
                Retry::AfterRefresh,
                Recovery::Refresh,
            ),
            Self::EmptyResponse => (
                "provider.empty_response",
                Category::DependencyUnavailable,
                Responsibility::Dependency,
                Retry::AfterUserAction,
                Recovery::ChooseAlternative,
            ),
            Self::ModelTurnBudgetExhausted { .. } => (
                "session.model_turn_budget_exhausted",
                Category::QuotaExceeded,
                Responsibility::Caller,
                Retry::AfterUserAction,
                Recovery::None,
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
            Self::AutoApproveClassifyFailed(_) => (
                "permission.auto_approve_classify_failed",
                Category::DependencyUnavailable,
                Responsibility::Dependency,
                Retry::Backoff,
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
            // A transient SQLite lock conflict ("database is locked") is a
            // dependency-level, retryable condition, not an internal error.
            // This guard must precede the `Database` catch-all arm below.
            Self::Database(error) if is_database_busy(error) => (
                "database.busy",
                Category::DependencyUnavailable,
                Responsibility::System,
                Retry::Backoff,
                Recovery::Retry,
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
            | Self::NestedSessionMutation { .. }
            | Self::Internal(_) => (
                "internal.unexpected",
                Category::Internal,
                Responsibility::System,
                Retry::Unknown,
                Recovery::Retry,
            ),
        };
        let failure = Failure::new(
            FailureCode::new(code),
            category,
            responsibility,
            retry,
            recovery,
            FailureImpact::RequestRejected,
            self.user_presentation(code),
        );
        self.attach_model_feedback(failure)
    }

    /// Presentation for the user channel. Classified/expected failures keep
    /// their reviewed static prose; diagnostics-bearing failures surface a
    /// scrubbed root cause so the user sees a real message.
    fn user_presentation(&self, code: &str) -> agena_failure::UserPresentation {
        use agena_failure::UserPresentation;
        match self {
            Self::Provider(_)
            | Self::Http(_)
            | Self::HttpStatus { .. }
            | Self::ProviderClassified { .. } => {
                let diagnostic = self.diagnostic_text();
                match diagnostic {
                    Some(diagnostic) => {
                        // Expected provider classification is already covered
                        // by `public_message`; only surface raw provider text
                        // when it is the best available signal (unclassified).
                        if matches!(
                            self,
                            Self::Provider(_) | Self::Http(_) | Self::HttpStatus { .. }
                        ) {
                            UserPresentation::validated_with_context(code, diagnostic)
                        } else {
                            UserPresentation::new(code, self.public_message())
                        }
                    }
                    None => UserPresentation::new(code, self.public_message()),
                }
            }
            Self::Tool(error) => match error.actionable_message() {
                Some(actionable) => UserPresentation::validated_with_context(code, actionable),
                None => UserPresentation::new(code, self.public_message()),
            },
            Self::Database(error) if is_database_busy(error) => {
                // A lock conflict is a transient condition with a stable,
                // localizable message — don't surface the internal SQLite
                // `(code: 5)` detail to the user.
                UserPresentation::new(code, self.public_message())
            }
            Self::Database(error) => UserPresentation::validated_with_context(
                code,
                agena_failure::diagnostic::format_error_chain(error),
            ),
            Self::SerdeJson(error) => UserPresentation::validated_with_context(
                code,
                agena_failure::diagnostic::format_error_chain(error),
            ),
            Self::Io(error) => UserPresentation::validated_with_context(
                code,
                agena_failure::diagnostic::format_error_chain(error),
            ),
            Self::StorageConfig(error) => UserPresentation::validated_with_context(
                code,
                agena_failure::diagnostic::format_error_chain(error),
            ),
            Self::Internal(diagnostic) => {
                UserPresentation::validated_with_context(code, diagnostic)
            }
            // ClassifyFailure is a display-only value with no Error/source
            // implementation, so Display is the full available diagnostic.
            Self::AutoApproveClassifyFailed(failure) => {
                UserPresentation::validated_with_context(code, failure.to_string())
            }
            _ => UserPresentation::new(code, self.public_message()),
        }
    }

    /// Best diagnostic text available for a raw-diagnostic variant.
    fn diagnostic_text(&self) -> Option<String> {
        match self {
            Self::Provider(message) => Some(message.clone()),
            Self::Http(error) => Some(agena_failure::diagnostic::format_error_chain(error)),
            Self::HttpStatus { body, .. } if !body.is_empty() => Some(body.clone()),
            Self::HttpStatus { .. } => None,
            Self::ProviderClassified { message, .. } if !message.is_empty() => {
                Some(message.clone())
            }
            Self::ProviderClassified { .. } => None,
            _ => None,
        }
    }

    /// Attach scrubbed model feedback for tool failures so the model can act
    /// on the real root cause rather than only a closed kind.
    fn attach_model_feedback(&self, failure: agena_failure::Failure) -> agena_failure::Failure {
        use agena_failure::{ModelFeedback, ModelFeedbackKind};
        let feedback = match self {
            Self::Tool(error) => match error.as_ref() {
                crate::tool::ToolError::InvalidPatch(d) => Some(
                    ModelFeedback::internal_tool_failure()
                        .with_text(d.to_string())
                        .with_kind(ModelFeedbackKind::InvalidInput),
                ),
                crate::tool::ToolError::InvalidInput { diagnostic, .. } => Some(
                    ModelFeedback::invalid_input_with_fields(error.field_issues().to_vec())
                        .with_text(diagnostic.to_string()),
                ),
                crate::tool::ToolError::InvalidGlobPattern(e) => {
                    Some(ModelFeedback::invalid_pattern().with_text(e.to_string()))
                }
                crate::tool::ToolError::InvalidRegexPattern(e) => {
                    Some(ModelFeedback::invalid_pattern().with_text(e.to_string()))
                }
                crate::tool::ToolError::Shell(e) => {
                    Some(ModelFeedback::internal_tool_failure().with_text(e.to_string()))
                }
                crate::tool::ToolError::Io(e) => {
                    Some(ModelFeedback::internal_tool_failure().with_text(e.to_string()))
                }
                crate::tool::ToolError::Plugin(p) => {
                    Some(ModelFeedback::plugin_failure().with_text(p.public.user.fallback.clone()))
                }
                crate::tool::ToolError::StaleToolCall { tool } => Some(
                    ModelFeedback::stale_tool_call()
                        .with_text(format!("Tool `{tool}` is stale; refresh the catalog.")),
                ),
                _ => None,
            },
            _ => None,
        };
        match feedback {
            Some(feedback) => failure.with_model_feedback(feedback),
            None => failure,
        }
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
    fn execution_command_failure_scrubs_but_preserves_root_cause() {
        let error = AppError::Internal(
            "database error: token=secret Custom Error: /private/agena.sqlite".to_owned(),
        );
        let failure = error.failure();
        let command_error = crate::SessionExecutionCommandError::from_failure(failure.clone());
        let wire = serde_json::to_string(&failure).expect("serialize safe command failure");
        let display = command_error.to_string();

        // Secrets and absolute paths never cross the boundary.
        assert!(!wire.contains("token=secret"));
        assert!(!wire.contains("/private/agena.sqlite"));
        assert!(!display.contains("token=secret"));
        assert!(!display.contains("/private/agena.sqlite"));
        // Machine code is not shown to the user.
        assert!(!display.contains(failure.code.as_str()));
        // A real, human-readable message replaces the old generic fallback,
        // with no correlation-id noise appended.
        assert!(!display.contains("Something went wrong."));
        assert!(display.contains("database error") || display.contains("<redacted>"));
        assert!(!display.contains("Reference:"));
    }

    #[test]
    fn sqlite_busy_is_classified_as_retryable_not_internal() {
        use agena_failure::{FailureCategory, FailureResponsibility, RetryDirective};

        let error = AppError::Database(busy_db_err());
        let failure = error.failure();

        assert_eq!(failure.code.as_str(), "database.busy");
        assert_eq!(failure.category, FailureCategory::DependencyUnavailable);
        assert_eq!(failure.responsibility, FailureResponsibility::System);
        assert_eq!(failure.retry, RetryDirective::Backoff);
        // The user-facing message is stable prose, not the internal code.
        let message = failure.user.fallback.as_str();
        assert!(!message.contains("(code:"));
        assert!(!message.contains("database is locked"));
        assert!(message.contains("busy"));
        assert!(error.public_message().contains("busy"));
    }

    #[test]
    fn ordinary_database_error_stays_internal_unexpected() {
        let error = AppError::Database(sea_orm::DbErr::Custom("something broke".to_owned()));
        let failure = error.failure();
        assert_eq!(failure.code.as_str(), "internal.unexpected");
    }

    #[test]
    fn empty_response_is_classified_as_provider_dependency_failure() {
        use agena_failure::{FailureCategory, FailureResponsibility, RetryDirective};

        let error = AppError::EmptyResponse;
        let failure = error.failure();

        assert_eq!(failure.code.as_str(), "provider.empty_response");
        assert_eq!(failure.category, FailureCategory::DependencyUnavailable);
        assert_eq!(failure.responsibility, FailureResponsibility::Dependency);
        assert_eq!(failure.retry, RetryDirective::AfterUserAction);
        assert!(!error.retryable());
        assert!(error.public_message().contains("empty response"));
        assert!(error.to_string().contains("empty response"));
    }

    /// Builds a `DbErr::Exec` wrapping a SQLite busy error through the public
    /// `DatabaseError` trait (the real `SqliteError` constructor is private).
    fn busy_db_err() -> sea_orm::DbErr {
        sea_orm::DbErr::Exec(sea_orm::RuntimeErr::SqlxError(
            sea_orm::sqlx::Error::Database(Box::new(BusyDbError)),
        ))
    }

    #[derive(Debug)]
    struct BusyDbError;

    impl std::fmt::Display for BusyDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "(code: 5) database is locked")
        }
    }

    impl std::error::Error for BusyDbError {}

    impl sea_orm::sqlx::error::DatabaseError for BusyDbError {
        fn message(&self) -> &str {
            "database is locked"
        }
        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            Some("5".into())
        }
        fn kind(&self) -> sea_orm::sqlx::error::ErrorKind {
            sea_orm::sqlx::error::ErrorKind::Other
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
    }
}
