//! Public failure envelope.
//!
//! Machine semantics travel on the wire so clients can choose recovery
//! actions. Presentations must render `problem.user`, never the machine code
//! or an internal error chain.

use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, RecoveryDirective,
    RetryDirective, UserPresentation, UserProblem,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiError {
    pub problem: UserProblem,
}

impl ApiError {
    pub fn from_failure(problem: Failure) -> Self {
        Self {
            problem: problem.into(),
        }
    }

    pub fn bad_request(message: &'static str) -> Self {
        Self::new(
            "request.invalid",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            UserPresentation::new("request-invalid", message),
        )
    }

    pub fn not_found(message: &'static str) -> Self {
        Self::new(
            "resource.not_found",
            FailureCategory::NotFound,
            FailureResponsibility::Caller,
            RetryDirective::AfterRefresh,
            RecoveryDirective::Refresh,
            UserPresentation::new("resource-not-found", message),
        )
    }

    pub fn conflict(message: &'static str) -> Self {
        Self::new(
            "resource.conflict",
            FailureCategory::Conflict,
            FailureResponsibility::Caller,
            RetryDirective::AfterRefresh,
            RecoveryDirective::Refresh,
            UserPresentation::new("resource-conflict", message),
        )
    }

    /// Creates an unexpected public failure. `diagnostic` is intentionally
    /// ignored here: callers must record it in the diagnostic channel before
    /// constructing the wire value.
    pub fn internal(_diagnostic: impl Into<String>) -> Self {
        Self::new(
            "internal.unexpected",
            FailureCategory::Internal,
            FailureResponsibility::System,
            RetryDirective::Unknown,
            RecoveryDirective::Retry,
            UserPresentation::new("internal-unexpected", "Something went wrong."),
        )
    }

    pub fn protocol(_diagnostic: impl Into<String>) -> Self {
        Self::new(
            "protocol.invalid_message",
            FailureCategory::ProtocolFailure,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            UserPresentation::new(
                "protocol-invalid-message",
                "The client sent an invalid protocol message.",
            ),
        )
    }

    pub fn service_unavailable(_diagnostic: impl Into<String>) -> Self {
        Self::new(
            "service.unavailable",
            FailureCategory::DependencyUnavailable,
            FailureResponsibility::Dependency,
            RetryDirective::Backoff,
            RecoveryDirective::Retry,
            UserPresentation::new(
                "service-unavailable",
                "The service is temporarily unavailable. Try again shortly.",
            ),
        )
    }

    fn new(
        code: &'static str,
        category: FailureCategory,
        responsibility: FailureResponsibility,
        retry: RetryDirective,
        recovery: RecoveryDirective,
        user: UserPresentation,
    ) -> Self {
        Self {
            problem: Failure::new(
                FailureCode::new(code),
                category,
                responsibility,
                retry,
                recovery,
                FailureImpact::RequestRejected,
                user,
            )
            .into(),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.problem.user.fallback.as_str())?;
        if self.problem.is_unexpected() {
            write!(f, " Reference: {}", self.problem.id)?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
mod tests {
    use super::ApiError;
    use agena_failure::{FailureCategory, ModelFeedback};

    #[test]
    fn internal_diagnostic_is_not_serialized_or_displayed() {
        let diagnostic = "database error: token=secret Custom Error";
        let error = ApiError::internal(diagnostic);
        let json = serde_json::to_string(&error).expect("serialize api failure");
        let display = error.to_string();

        assert_eq!(error.problem.category, FailureCategory::Internal);
        assert!(!json.contains(diagnostic));
        assert!(!json.contains("token=secret"));
        assert!(!display.contains(diagnostic));
        assert!(display.contains("Something went wrong."));
        assert!(display.contains("Reference:"));
    }

    #[test]
    fn machine_code_is_not_part_of_display_text() {
        let error = ApiError::not_found("Session not found.");
        assert_eq!(error.to_string(), "Session not found.");
        assert!(!error.to_string().contains(error.problem.code.as_str()));
    }

    #[test]
    fn api_projection_never_contains_model_feedback() {
        let internal = agena_failure::Failure::new(
            agena_failure::FailureCode::new("tool.invalid_input"),
            FailureCategory::InvalidInput,
            agena_failure::FailureResponsibility::Caller,
            agena_failure::RetryDirective::CorrectInput,
            agena_failure::RecoveryDirective::None,
            agena_failure::FailureImpact::OperationFailed,
            agena_failure::UserPresentation::new(
                "tool-invalid-input",
                "The tool input is invalid.",
            ),
        )
        .with_model_feedback(ModelFeedback::invalid_input());
        let id = internal.id;
        let failure = ApiError::from_failure(internal);
        let json = serde_json::to_string(&failure).expect("serialize api error");
        assert!(!json.contains("model"));
        assert!(!json.contains("Review the tool schema"));
        assert_eq!(failure.problem.id, id);
    }
}
