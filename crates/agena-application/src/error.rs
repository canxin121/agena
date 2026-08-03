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

#[cfg(test)]
mod tests {
    use super::ApplicationError;

    #[test]
    fn internal_diagnostics_surface_scrubbed_root_cause() {
        let diagnostic = "session execution command failed: database error: Custom Error: response 123 is missing or already terminal";
        let error = ApplicationError::internal(diagnostic);

        // The user sees the operation context and the actionable root cause,
        // not a generic fallback.
        let display = error.to_string();
        assert!(display.contains("response 123 is missing or already terminal"));
        assert!(display.contains("session execution command failed"));
        assert!(!display.contains("Something went wrong."));
        // Nested wrapper noise is stripped from the user channel.
        assert!(!display.contains("Custom Error"));
        assert!(!display.contains("database error"));
        assert_eq!(error.diagnostic_message(), Some(diagnostic));
    }

    #[test]
    fn expected_input_message_remains_actionable() {
        let error = ApplicationError::bad_request("The model field is required.");
        assert_eq!(error.to_string(), "The model field is required.");
    }
}
