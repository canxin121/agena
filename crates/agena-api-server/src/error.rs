use agena::AppError;
use agena_api::error::{ApiError, ErrorCode};
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("internal: {0}")]
    Internal(String),
    #[error(transparent)]
    Core(#[from] AppError),
}

impl ServerError {
    pub fn into_api(self) -> ApiError {
        match self {
            ServerError::BadRequest(m) => ApiError::bad_request(m),
            ServerError::NotFound(m) => ApiError::not_found(m),
            ServerError::Conflict(m) => ApiError::conflict(m),
            ServerError::ServiceUnavailable(m) => ApiError::service_unavailable(m),
            ServerError::Internal(m) => ApiError::internal(m),
            ServerError::Core(err) => map_core_error(err),
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            ServerError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ServerError::NotFound(_) => StatusCode::NOT_FOUND,
            ServerError::Conflict(_) => StatusCode::CONFLICT,
            ServerError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            ServerError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ServerError::Core(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

fn map_core_error(err: AppError) -> ApiError {
    let message = err.to_string();
    ApiError {
        code: ErrorCode::Internal,
        message,
        details: None,
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = self.into_api();
        (status, Json(body)).into_response()
    }
}
