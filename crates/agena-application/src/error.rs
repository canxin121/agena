use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, RecoveryDirective,
    RetryDirective, UserPresentation,
};

/// Product-level failure with a safe cross-layer projection and an optional
/// process-local diagnostic. Presentation adapters must use `failure` only.
#[derive(Debug)]
pub struct ApplicationError {
    pub failure: Box<Failure>,
    diagnostic: Option<String>,
}

impl ApplicationError {
    pub fn from_failure(failure: Failure) -> Self {
        Self {
            failure: Box::new(failure),
            diagnostic: None,
        }
    }

    pub fn from_failure_with_diagnostic(
        failure: Failure,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        Self {
            failure: Box::new(failure),
            diagnostic: Some(diagnostic.to_string()),
        }
    }

    pub fn bad_request(message: &'static str) -> Self {
        Self::expected(
            "request.invalid",
            FailureCategory::InvalidInput,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            message,
        )
    }

    pub fn not_found(message: &'static str) -> Self {
        Self::expected(
            "resource.not_found",
            FailureCategory::NotFound,
            RetryDirective::AfterRefresh,
            RecoveryDirective::Refresh,
            message,
        )
    }

    pub fn conflict(message: &'static str) -> Self {
        Self::expected(
            "resource.conflict",
            FailureCategory::Conflict,
            RetryDirective::AfterRefresh,
            RecoveryDirective::Refresh,
            message,
        )
    }

    pub fn bad_request_with_diagnostic(
        message: &'static str,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        Self::expected_with_diagnostic(
            "request.invalid",
            FailureCategory::InvalidInput,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            message,
            diagnostic,
        )
    }

    /// Build an invalid-input failure from a typed error while retaining its
    /// complete operator chain and showing the scrubbed, actionable cause to
    /// the user instead of a static "operation failed" wrapper.
    pub fn bad_request_error(error: &(dyn std::error::Error + 'static)) -> Self {
        let diagnostic = agena_failure::diagnostic::format_error_chain(error);
        let mut result = Self::bad_request(
            "The request diagnostic contained no safely displayable details; review the diagnostic log.",
        );
        result.failure.user = UserPresentation::validated_with_context(
            "request-invalid-diagnostic",
            diagnostic.as_str(),
        );
        result.diagnostic = Some(diagnostic);
        result
    }

    pub fn not_found_with_diagnostic(
        message: &'static str,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        Self::expected_with_diagnostic(
            "resource.not_found",
            FailureCategory::NotFound,
            RetryDirective::AfterRefresh,
            RecoveryDirective::Refresh,
            message,
            diagnostic,
        )
    }

    pub fn conflict_with_diagnostic(
        message: &'static str,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        Self::expected_with_diagnostic(
            "resource.conflict",
            FailureCategory::Conflict,
            RetryDirective::AfterRefresh,
            RecoveryDirective::Refresh,
            message,
            diagnostic,
        )
    }

    pub fn service_unavailable(diagnostic: impl std::fmt::Display) -> Self {
        Self::diagnostic(
            "service.unavailable",
            FailureCategory::DependencyUnavailable,
            FailureResponsibility::Dependency,
            RetryDirective::Backoff,
            RecoveryDirective::Retry,
            "The service is temporarily unavailable. Try again shortly.",
            diagnostic,
        )
    }

    /// The runtime has already confirmed the command. A failed follow-up
    /// projection must never ask a caller to execute that command again.
    pub fn committed_command_projection_unavailable(
        outcome: agena_runtime::SessionExecutionCommandOutcome,
        cause: Self,
    ) -> Self {
        let code = "session.command_committed_projection_unavailable";
        Self {
            failure: Box::new(Failure::new(
                FailureCode::new(code),
                FailureCategory::DependencyUnavailable,
                FailureResponsibility::System,
                RetryDirective::Never,
                RecoveryDirective::Refresh,
                FailureImpact::PartialSuccess,
                UserPresentation::validated(
                    code,
                    format!(
                        "The command for session {} was committed, but its current state could not be loaded. Refresh the session; do not submit the command again.",
                        outcome.session_id
                    ),
                ),
            )),
            diagnostic: Some(format!(
                "confirmed session command {outcome:?}; follow-up projection failed: {}: {}",
                cause.failure.code.as_str(),
                cause
                    .diagnostic_message()
                    .unwrap_or(cause.failure.user.fallback.as_str()),
            )),
        }
    }

    pub fn internal(diagnostic: impl std::fmt::Display) -> Self {
        let diagnostic = diagnostic.to_string();
        Self {
            failure: Box::new(Failure::new(
                FailureCode::new("internal.unexpected"),
                FailureCategory::Internal,
                FailureResponsibility::System,
                RetryDirective::Unknown,
                RecoveryDirective::Retry,
                FailureImpact::RequestRejected,
                // Surface the scrubbed root cause instead of a generic
                // "Something went wrong." so the user sees a real message.
                UserPresentation::validated_with_context("internal-unexpected", &diagnostic),
            )),
            diagnostic: Some(diagnostic),
        }
    }

    /// Build an internal application failure from a typed error without
    /// truncating its source chain at this transport-neutral boundary.
    pub fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::internal(agena_failure::diagnostic::format_error_chain(error))
    }

    /// Add an operation label while retaining every typed source.
    pub fn internal_error_with_context(
        context: impl AsRef<str>,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::internal(agena_failure::diagnostic::format_error_chain_with_context(
            context, error,
        ))
    }

    /// Retain an operation label without erasing a stopped runtime's retryable
    /// service-unavailability classification.
    pub fn runtime_control_error_with_context(
        context: impl AsRef<str>,
        error: &agena_runtime::RuntimeControlServiceError,
    ) -> Self {
        let diagnostic = agena_failure::diagnostic::format_error_chain_with_context(context, error);
        match error {
            agena_runtime::RuntimeControlServiceError::Shutdown => {
                Self::service_unavailable(diagnostic)
            }
            agena_runtime::RuntimeControlServiceError::Operation { .. } => {
                Self::internal(diagnostic)
            }
        }
    }

    pub fn diagnostic_message(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }

    pub fn is_not_found(&self) -> bool {
        self.failure.category == FailureCategory::NotFound
    }

    fn expected(
        code: &'static str,
        category: FailureCategory,
        retry: RetryDirective,
        recovery: RecoveryDirective,
        message: &'static str,
    ) -> Self {
        Self {
            failure: Box::new(Failure::new(
                FailureCode::new(code),
                category,
                FailureResponsibility::Caller,
                retry,
                recovery,
                FailureImpact::RequestRejected,
                UserPresentation::new(code, message),
            )),
            diagnostic: None,
        }
    }

    fn expected_with_diagnostic(
        code: &'static str,
        category: FailureCategory,
        retry: RetryDirective,
        recovery: RecoveryDirective,
        message: &'static str,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        let mut error = Self::expected(code, category, retry, recovery, message);
        error.diagnostic = Some(diagnostic.to_string());
        error
    }

    fn diagnostic(
        code: &'static str,
        category: FailureCategory,
        responsibility: FailureResponsibility,
        retry: RetryDirective,
        recovery: RecoveryDirective,
        fallback: &'static str,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        Self {
            failure: Box::new(Failure::new(
                FailureCode::new(code),
                category,
                responsibility,
                retry,
                recovery,
                FailureImpact::RequestRejected,
                UserPresentation::new(code, fallback),
            )),
            diagnostic: Some(diagnostic.to_string()),
        }
    }
}

impl std::fmt::Display for ApplicationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The fallback already carries a scrubbed, human-readable root cause.
        formatter.write_str(self.failure.user.fallback.as_str())
    }
}

impl std::error::Error for ApplicationError {}

impl From<agena_runtime::RuntimeControlServiceError> for ApplicationError {
    fn from(error: agena_runtime::RuntimeControlServiceError) -> Self {
        match error {
            error @ agena_runtime::RuntimeControlServiceError::Shutdown => {
                Self::service_unavailable(error)
            }
            error @ agena_runtime::RuntimeControlServiceError::Operation { .. } => {
                Self::internal_error(&error)
            }
        }
    }
}

impl From<agena_runtime::RuntimeBackgroundTaskControlError> for ApplicationError {
    fn from(error: agena_runtime::RuntimeBackgroundTaskControlError) -> Self {
        use agena_runtime::RuntimeBackgroundTaskControlError as Error;
        match error {
            error @ (Error::Shutdown | Error::Capacity { .. } | Error::ExecutorUnavailable) => {
                Self::service_unavailable(error)
            }
            Error::NotFound(task_id) => Self::not_found_with_diagnostic(
                "The background task was not found.",
                format!("background task `{task_id}` not found"),
            ),
            Error::NotRunning(task_id) => Self::conflict_with_diagnostic(
                "The background task is not running.",
                format!("background task `{task_id}` is not running"),
            ),
            Error::NotCancellable(task_id) => Self::conflict_with_diagnostic(
                "The background task cannot be cancelled.",
                format!("background task `{task_id}` cannot be cancelled"),
            ),
        }
    }
}
