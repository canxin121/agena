use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

pub type ApiResult<T> = Result<T, AppError>;

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{message}")]
    BadRequest { message: String },

    #[error("{message}")]
    Forbidden { message: String },

    #[error("{message}")]
    NotFound { message: String },

    #[error("{message}")]
    PayloadTooLarge { message: String },

    #[error("{message}")]
    TooManyRequests { message: String },

    #[error("{message}")]
    BadGateway { message: String },

    #[error("{message}")]
    Internal { message: String },
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest {
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden {
            message: message.into(),
        }
    }

    pub fn forbidden_error(
        context: impl AsRef<str>,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        let diagnostic = agena_failure::diagnostic::format_error_chain_with_context(context, error);
        tracing::error!(diagnostic, "server request was denied by an I/O boundary");
        let public = agena_failure::diagnostic::user_message_with_context(&diagnostic, 240);
        Self::forbidden(if public.is_empty() {
            "Access was denied by the operating system.".to_owned()
        } else {
            public
        })
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound {
            message: message.into(),
        }
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::PayloadTooLarge {
            message: message.into(),
        }
    }

    pub fn too_many_requests(message: impl Into<String>) -> Self {
        Self::TooManyRequests {
            message: message.into(),
        }
    }

    pub fn bad_gateway(message: impl Into<String>) -> Self {
        Self::BadGateway {
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    pub fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::internal(agena_failure::diagnostic::format_error_chain(error))
    }

    pub fn internal_error_with_context(
        context: impl AsRef<str>,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::internal(agena_failure::diagnostic::format_error_chain_with_context(
            context.as_ref(),
            error,
        ))
    }

    fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest { .. } => StatusCode::BAD_REQUEST,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::PayloadTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::TooManyRequests { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::BadGateway { .. } => StatusCode::BAD_GATEWAY,
            Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn body(&self) -> ErrorBody {
        match self {
            Self::BadRequest { message }
            | Self::Forbidden { message }
            | Self::NotFound { message }
            | Self::PayloadTooLarge { message }
            | Self::TooManyRequests { message } => {
                let safe = agena_failure::diagnostic::scrubbed_preserve(message, 240);
                ErrorBody {
                    error: if safe.is_empty() {
                        "The request was rejected because its details could not be displayed safely."
                            .to_owned()
                    } else {
                        safe
                    },
                }
            }
            Self::BadGateway { message } | Self::Internal { message } => {
                let failure_id = agena_failure::FailureId::new();
                tracing::error!(
                    %failure_id,
                    diagnostic = %message,
                    "server request failed"
                );
                let safe =
                    agena_failure::diagnostic::user_message_with_context(message.as_str(), 240);
                ErrorBody {
                    error: if safe.is_empty() {
                        format!(
                            "The request failed; review the server diagnostic log for reference {failure_id}."
                        )
                    } else {
                        safe
                    },
                }
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = self.body();
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn internal_http_body_keeps_a_scrubbed_root_cause() {
        let error = AppError::internal(
            "failed to load terminal registry: token=secret /private/agena.sqlite: disk full",
        );
        let body = error.body();

        assert!(body.error.contains("disk full"));
        assert!(!body.error.contains("token=secret"));
        assert!(!body.error.contains("/private/agena.sqlite"));
    }
}
