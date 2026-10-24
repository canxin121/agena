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
