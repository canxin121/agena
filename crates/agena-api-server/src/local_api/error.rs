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
            AppError::Cancelled => Self::conflict("execution cancelled"),
            AppError::ExecutionAlreadyActive(session_id) => Self::conflict(format!(
                "session {session_id} already has an active execution"
            )),
            AppError::NoActiveExecution(session_id) => {
                Self::conflict(format!("session {session_id} has no active execution"))
            }
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
