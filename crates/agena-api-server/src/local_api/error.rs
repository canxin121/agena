use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use agena::AppError;

#[derive(Debug, Clone)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    error: ApiErrorPayload,
}

#[derive(Debug, Serialize)]
struct ApiErrorPayload {
    code: String,
    message: String,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "conflict",
            message: message.into(),
        }
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "service_unavailable",
            message: message.into(),
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: message.into(),
        }
    }

    pub(crate) fn status_code(&self) -> StatusCode {
        self.status
    }

    pub(crate) fn message(&self) -> &str {
        self.message.as_str()
    }
}

impl From<AppError> for ApiError {
    fn from(value: AppError) -> Self {
        match value {
            AppError::Config(message) => Self::bad_request(message),
            AppError::ConfigErr(error) => Self::bad_request(error.to_string()),
            AppError::Database(error) => Self::internal(error.to_string()),
            AppError::SerdeJson(error) => Self::bad_request(error.to_string()),
            AppError::Http(error) => Self::internal(error.to_string()),
            AppError::Io(error) => Self::internal(error.to_string()),
            AppError::Provider(message)
            | AppError::InvalidRole(message)
            | AppError::Internal(message) => Self::internal(message),
            AppError::ProviderClassified { message, .. } => Self::bad_request(message),
            AppError::HttpStatus { body, status, .. } => Self {
                status,
                code: "provider_http_error",
                message: body,
            },
            AppError::Conflict {
                session_id,
                expected,
                current,
            } => Self::conflict(format!(
                "session {session_id} version conflict: expected {expected}, current {current}"
            )),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorBody {
            error: ApiErrorPayload {
                code: self.code.to_string(),
                message: self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

#[cfg(test)]
impl ApiError {
    /// Test-only accessor for `code`.
    pub(crate) fn error_code(&self) -> &'static str {
        self.code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_app_error_maps_to_bad_request() {
        let err: ApiError = AppError::Config("bad".into()).into();
        assert_eq!(err.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(err.error_code(), "bad_request");
    }

    #[test]
    fn database_app_error_maps_to_internal() {
        let err: ApiError = AppError::Database(sea_orm::DbErr::Custom("boom".into())).into();
        assert_eq!(err.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.error_code(), "internal_error");
    }

    #[test]
    fn invalid_role_maps_to_internal() {
        let err: ApiError = AppError::InvalidRole("ghost".into()).into();
        assert_eq!(err.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn http_status_app_error_preserves_status() {
        let err: ApiError = AppError::HttpStatus {
            provider: "anthropic".into(),
            status: StatusCode::TOO_MANY_REQUESTS,
            body: "rate limited".into(),
            kind: agena::error::ProviderErrorKind::ApiError,
            retryable: true,
        }
        .into();
        assert_eq!(err.status_code(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(err.error_code(), "provider_http_error");
    }

    #[test]
    fn conflict_app_error_maps_to_conflict() {
        let err: ApiError = AppError::Conflict {
            session_id: 1,
            expected: 1,
            current: 2,
        }
        .into();
        assert_eq!(err.status_code(), StatusCode::CONFLICT);
        assert_eq!(err.error_code(), "conflict");
    }
}
