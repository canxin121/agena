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
/// Structured error returned by the runtime API, carrying a user-facing problem.
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

    /// Creates an unexpected public failure. The diagnostic is reduced to a
    /// scrubbed root cause so the client receives a real, human-readable
    /// message instead of a generic fallback. Callers should also log the full
    /// diagnostic in the diagnostic channel for correlation via [`FailureId`].
    pub fn internal(diagnostic: impl Into<String>) -> Self {
        let diagnostic = diagnostic.into();
        let user = UserPresentation::validated_with_context("internal-unexpected", &diagnostic);
        Self::new(
            "internal.unexpected",
            FailureCategory::Internal,
            FailureResponsibility::System,
            RetryDirective::Unknown,
            RecoveryDirective::Retry,
            user,
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

    pub fn authentication_required(message: impl AsRef<str>) -> Self {
        Self::new(
            "auth.required",
            FailureCategory::AuthenticationRequired,
            FailureResponsibility::Caller,
            RetryDirective::AfterUserAction,
            RecoveryDirective::Reauthenticate,
            UserPresentation::validated("auth-required", message),
        )
    }

    pub fn invalid_credentials(message: impl AsRef<str>) -> Self {
        Self::new(
            "auth.invalid_credentials",
            FailureCategory::AuthenticationRequired,
            FailureResponsibility::Caller,
            RetryDirective::AfterUserAction,
            RecoveryDirective::Reauthenticate,
            UserPresentation::validated("auth-invalid-credentials", message),
        )
    }

    pub fn rate_limited(message: impl AsRef<str>) -> Self {
        Self::new(
            "auth.rate_limited",
            FailureCategory::RateLimited,
            FailureResponsibility::Caller,
            RetryDirective::Backoff,
            RecoveryDirective::Retry,
            UserPresentation::validated("auth-rate-limited", message),
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
        // The fallback already carries a scrubbed, human-readable root cause.
        // The machine code and correlation id stay out of the user-visible text
        // (they are still available on the structured `problem`).
        f.write_str(self.problem.user.fallback.as_str())
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
mod tests {
    use super::ApiError;
    use agena_failure::{FailureCategory, ModelFeedback};

    #[test]
    fn internal_diagnostic_is_scrubbed_but_surfaced() {
        let diagnostic = "database error: token=secret Custom Error";
        let error = ApiError::internal(diagnostic);
        let json = serde_json::to_string(&error).expect("serialize api failure");
        let display = error.to_string();

        assert_eq!(error.problem.category, FailureCategory::Internal);
        // The raw chain and secret never cross the wire verbatim.
        assert!(!json.contains("token=secret"));
        assert!(!json.contains(diagnostic));
        assert!(!display.contains("token=secret"));
        assert!(!display.contains(diagnostic));
        // A scrubbed root cause replaces the generic fallback, with no
        // "Reference:" noise appended.
        assert!(!display.contains("Something went wrong."));
        assert!(!display.contains("Reference:"));
    }

    #[test]
    fn machine_code_is_not_part_of_display_text() {
        let error = ApiError::not_found("Session not found.");
        assert_eq!(error.to_string(), "Session not found.");
        assert!(!error.to_string().contains(error.problem.code.as_str()));
    }

    #[test]
    fn ui_auth_failures_use_the_shared_problem_taxonomy() {
        let required = ApiError::authentication_required("UI authentication is required.");
        assert_eq!(required.problem.code.as_str(), "auth.required");
        assert_eq!(
            required.problem.category,
            FailureCategory::AuthenticationRequired
        );
        assert_eq!(
            required.problem.recovery,
            agena_failure::RecoveryDirective::Reauthenticate
        );

        let invalid = ApiError::invalid_credentials("The server password is incorrect.");
        assert_eq!(invalid.problem.code.as_str(), "auth.invalid_credentials");
        assert_eq!(
            invalid.problem.category,
            FailureCategory::AuthenticationRequired
        );

        let limited = ApiError::rate_limited("Too many failed login attempts.");
        assert_eq!(limited.problem.code.as_str(), "auth.rate_limited");
        assert_eq!(limited.problem.category, FailureCategory::RateLimited);
        assert_eq!(
            limited.problem.retry,
            agena_failure::RetryDirective::Backoff
        );
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
