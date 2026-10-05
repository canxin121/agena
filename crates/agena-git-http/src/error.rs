//! Adapt the workbench's Git failures to the public API contract. Keep the
//! existing Git codes and recovery details understood by browser clients.

use agena_api::error::ApiError;
use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, RecoveryDirective,
    RetryDirective, UserPresentation,
};
use axum::{
    Json,
    body::{Body, to_bytes},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

/// Response middleware for Git endpoints shared by the browser and Rust SDK.
/// Successful responses pass through without collecting their bodies.
pub async fn git_error_envelope(response: Response) -> Response {
    let status = response.status();
    if !status.is_client_error() && !status.is_server_error() {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let bytes = match to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::error!(%error, "failed to read Git error response");
            return (
                status,
                Json(ApiError::internal(
                    "Git returned an unreadable error response.",
                )),
            )
                .into_response();
        }
    };
    // Preserve failures that already conform, including their correlation id.
    if serde_json::from_slice::<ApiError>(&bytes).is_ok() {
        return Response::from_parts(parts, Body::from(bytes));
    }
    let mut value = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let code = value
        .get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(if status.is_server_error() {
            "git_failed"
        } else {
            "git_invalid_request"
        });
    let message = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
    let (category, retry, recovery) = match status {
        StatusCode::CONFLICT => (
            FailureCategory::Conflict,
            RetryDirective::AfterRefresh,
            RecoveryDirective::Refresh,
        ),
        StatusCode::UNAUTHORIZED => (
            FailureCategory::AuthenticationRequired,
            RetryDirective::AfterUserAction,
            RecoveryDirective::Reauthenticate,
        ),
        StatusCode::FORBIDDEN => (
            FailureCategory::PermissionDenied,
            RetryDirective::AfterUserAction,
            RecoveryDirective::None,
        ),
        StatusCode::NOT_FOUND => (
            FailureCategory::NotFound,
            RetryDirective::AfterRefresh,
            RecoveryDirective::Refresh,
        ),
        StatusCode::TOO_MANY_REQUESTS => (
            FailureCategory::RateLimited,
            RetryDirective::Backoff,
            RecoveryDirective::Retry,
        ),
        _ if status.is_server_error() => (
            FailureCategory::Internal,
            RetryDirective::Unknown,
            RecoveryDirective::Retry,
        ),
        _ => (
            FailureCategory::InvalidInput,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
        ),
    };
    let api = ApiError::from_failure(Failure::new(
        FailureCode::new(code),
        category,
        if status.is_server_error() {
            FailureResponsibility::System
        } else {
            FailureResponsibility::Caller
        },
        retry,
        recovery,
        FailureImpact::RequestRejected,
        UserPresentation::validated_with_context("git-request-failed", &message),
    ));
    value["error"] = serde_json::Value::String(api.to_string());
    value["problem"] =
        serde_json::to_value(api.problem).expect("public Git problem is serializable");
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        "application/json".parse().expect("valid content type"),
    );
    let (_, body) = Json(value).into_response().into_parts();
    Response::from_parts(parts, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn non_repository_error_preserves_git_code_and_is_decodable_by_the_sdk() {
        let response = git_error_envelope(
            (
                StatusCode::CONFLICT,
                Json(
                    serde_json::json!({ "error": "Not a git repository", "code": "not_git_repo" }),
                ),
            )
                .into_response(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let error: ApiError = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error.problem.code.as_str(), "not_git_repo");
        assert_eq!(error.problem.category, FailureCategory::Conflict);
        assert!(error.to_string().contains("Not a git repository"));
        let legacy: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(legacy["code"], "not_git_repo");
    }

    #[tokio::test]
    async fn invalid_queries_and_git_diagnostics_use_shared_failures() {
        for response in [
            (
                StatusCode::BAD_REQUEST,
                "Invalid query: limit is not a number",
            )
                .into_response(),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "git failed token=secret: disk full", "code": "git_diff_failed"
                })),
            )
                .into_response(),
        ] {
            let status = response.status();
            let response = git_error_envelope(response).await;
            assert_eq!(response.status(), status);
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            let error: ApiError = serde_json::from_slice(&bytes).unwrap();
            assert!(!error.to_string().contains("token=secret"));
            assert!(!String::from_utf8_lossy(&bytes).contains("token=secret"));
        }
    }

    #[tokio::test]
    async fn existing_problems_keep_their_identity_and_successes_keep_their_body() {
        let error = ApiError::bad_request("Invalid path.");
        let id = error.problem.id;
        let response =
            git_error_envelope((StatusCode::BAD_REQUEST, Json(error)).into_response()).await;
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<ApiError>(&bytes)
                .unwrap()
                .problem
                .id,
            id
        );
        let response = git_error_envelope(
            Json(serde_json::json!({ "files": [], "totalFiles": 0 })).into_response(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["totalFiles"], 0);
        assert!(value.get("problem").is_none());
    }
}
