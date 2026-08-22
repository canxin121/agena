//! Error types and response mapping for the API server transports.

use agena_api::error::ApiError;
use agena_application::ApplicationError;
use agena_failure::FailureCategory;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

/// HTTP/WS/IPC adapter for the shared application failure. It does not create
/// a second error identity or copy diagnostics into the protocol envelope.
#[derive(Debug)]
pub struct ServerError {
    application: ApplicationError,
}

impl ServerError {
    pub fn from_failure(failure: agena_failure::Failure) -> Self {
        ApplicationError::from_failure(failure).into()
    }

    pub fn from_failure_with_diagnostic(
        failure: agena_failure::Failure,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        ApplicationError::from_failure_with_diagnostic(failure, diagnostic).into()
    }

    pub fn bad_request(message: &'static str) -> Self {
        ApplicationError::bad_request(message).into()
    }

    pub fn bad_request_with_diagnostic(
        message: &'static str,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        ApplicationError::bad_request_with_diagnostic(message, diagnostic).into()
    }

    pub fn bad_request_error(error: &(dyn std::error::Error + 'static)) -> Self {
        ApplicationError::bad_request_error(error).into()
    }

    pub fn not_found(message: &'static str) -> Self {
        ApplicationError::not_found(message).into()
    }

    pub fn not_found_with_diagnostic(
        message: &'static str,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        ApplicationError::not_found_with_diagnostic(message, diagnostic).into()
    }

    pub fn conflict(message: &'static str) -> Self {
        ApplicationError::conflict(message).into()
    }

    pub fn conflict_with_diagnostic(
        message: &'static str,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        ApplicationError::conflict_with_diagnostic(message, diagnostic).into()
    }

    pub fn service_unavailable(diagnostic: impl std::fmt::Display) -> Self {
        ApplicationError::service_unavailable(diagnostic).into()
    }

    pub fn internal(diagnostic: impl std::fmt::Display) -> Self {
        ApplicationError::internal(diagnostic).into()
    }

    pub fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        ApplicationError::internal_error(error).into()
    }

    pub fn into_api(self) -> ApiError {
        if let Some(diagnostic) = self.application.diagnostic_message() {
            tracing::error!(
                failure_id = %self.application.failure.id,
                failure_code = %self.application.failure.code,
                diagnostic,
                "request failed"
            );
        }
        ApiError::from_failure(*self.application.failure)
    }

    pub fn status(&self) -> StatusCode {
        match self.application.failure.category {
            FailureCategory::InvalidInput | FailureCategory::ProtocolFailure => {
                StatusCode::BAD_REQUEST
            }
            FailureCategory::NotFound => StatusCode::NOT_FOUND,
            FailureCategory::Conflict => StatusCode::CONFLICT,
            FailureCategory::PermissionRequired | FailureCategory::PermissionDenied => {
                StatusCode::FORBIDDEN
            }
            FailureCategory::AuthenticationRequired => StatusCode::UNAUTHORIZED,
            FailureCategory::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            FailureCategory::QuotaExceeded => StatusCode::PAYMENT_REQUIRED,
            FailureCategory::Timeout => StatusCode::GATEWAY_TIMEOUT,
            FailureCategory::DependencyUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            FailureCategory::DataCorruption | FailureCategory::Internal => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.application.fmt(formatter)
    }
}

impl std::error::Error for ServerError {}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = self.into_api();
        (status, Json(body)).into_response()
    }
}

impl From<ApplicationError> for ServerError {
    fn from(application: ApplicationError) -> Self {
        Self { application }
    }
}

#[cfg(test)]
mod tests {
    use super::ServerError;
    use agena_application::ApplicationError;

    #[test]
    fn transport_preserves_failure_identity_and_excludes_diagnostic() {
        let application =
            ApplicationError::internal("database error token=secret /private/agena.sqlite");
        let failure_id = application.failure.id;
        let api = ServerError::from(application).into_api();
        let public = serde_json::to_string(&api).expect("serialize api error");

        assert_eq!(api.problem.id, failure_id);
        assert!(!public.contains("token=secret"));
        assert!(!public.contains("/private/agena.sqlite"));
    }
}
