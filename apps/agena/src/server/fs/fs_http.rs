//! Explicit body budgets for the two Workbench endpoints carrying file contents.

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{
        Query, Request,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, header},
};
use http_body_util::{BodyExt as _, Limited};
use tokio::sync::{Semaphore, SemaphorePermit};

use super::{
    ApiResult, AppError, MAX_UPLOAD_BYTES, ProjectDirQuery, SuccessPathResponse, UploadQuery,
    UploadResponse, WriteBody, fs_upload, fs_write,
};

#[cfg(test)]
mod tests;

// A decoded byte can occupy six JSON bytes (for example, \u0000). Keep the
// 50 MiB content contract even for fully escaped input, plus bounded overhead
// for the path, object syntax, and whitespace. The core handler also checks
// the decoded content size before creating directories or publishing a file.
const MAX_WRITE_BODY_BYTES: usize = 6 * MAX_UPLOAD_BYTES + 64 * 1024;

// Acquire before polling the body, and retain through parsing and writing.
// In particular, a cancelled waiter cannot release a running parser's slot.
static FILE_REQUESTS: Semaphore = Semaphore::const_new(2);

pub async fn fs_upload_http(
    query: Result<Query<UploadQuery>, QueryRejection>,
    request: Request,
) -> ApiResult<Json<UploadResponse>> {
    let query = query.map_err(|error| AppError::bad_request(error.body_text()))?;
    let (parts, body) = request.into_parts();
    check_declared_length(&parts.headers, MAX_UPLOAD_BYTES)?;
    let _permit = acquire_body_slot().await?;
    let bytes = read_body(body, MAX_UPLOAD_BYTES).await?;
    fs_upload(parts.headers, query, bytes).await
}

pub async fn fs_write_http(
    query: Result<Query<ProjectDirQuery>, QueryRejection>,
    request: Request,
) -> ApiResult<Json<SuccessPathResponse>> {
    let query = query.map_err(|error| AppError::bad_request(error.body_text()))?;
    let (parts, body) = request.into_parts();
    if !is_json_content_type(&parts.headers) {
        return Err(AppError::unsupported_media_type(
            "Expected request with Content-Type: application/json",
        ));
    }
    check_declared_length(&parts.headers, MAX_WRITE_BODY_BYTES)?;
    let permit = acquire_body_slot().await?;
    let bytes = read_body(body, MAX_WRITE_BODY_BYTES).await?;
    let (_permit, body) = process_body(permit, move || {
        Json::<WriteBody>::from_bytes(&bytes).map_err(json_error)
    })
    .await?;
    fs_write(parts.headers, query, body).await
}

async fn acquire_body_slot() -> ApiResult<SemaphorePermit<'static>> {
    FILE_REQUESTS.acquire().await.map_err(|error| {
        AppError::internal_error_with_context("acquire a filesystem request slot", &error)
    })
}

fn check_declared_length(headers: &HeaderMap, limit: usize) -> ApiResult<()> {
    if headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > limit as u64)
    {
        return Err(body_too_large(limit));
    }
    Ok(())
}

fn body_too_large(limit: usize) -> AppError {
    AppError::payload_too_large(format!("Request body exceeds the {limit}-byte limit"))
}

async fn read_body(body: Body, limit: usize) -> ApiResult<Bytes> {
    // Raw Body bypasses Axum's default limit. Limited counts actual bytes,
    // including bodies with no Content-Length, before data reaches our buffer.
    let mut body = Limited::new(body, limit);
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| {
            let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(error.as_ref());
            while let Some(error) = cause {
                if error.is::<http_body_util::LengthLimitError>() {
                    return body_too_large(limit);
                }
                cause = error.source();
            }
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "read a filesystem request body",
                    error.as_ref(),
                ),
                "filesystem request body could not be read"
            );
            AppError::bad_request("Failed to read request body")
        })?;
        if let Ok(data) = frame.into_data() {
            // Coalesce as we read: keeping every tiny frame until EOF can
            // consume far more memory than the payload limit. Grow geometrically
            // without reserving beyond the configured byte budget.
            let needed = bytes.len() + data.len();
            if needed > bytes.capacity() {
                let capacity = needed
                    .max(bytes.capacity().saturating_mul(2))
                    .max(64 * 1024)
                    .min(limit);
                bytes.reserve_exact(capacity - bytes.len());
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(Bytes::from(bytes))
}

async fn process_body<T: Send + 'static>(
    permit: SemaphorePermit<'static>,
    process: impl FnOnce() -> ApiResult<T> + Send + 'static,
) -> ApiResult<(SemaphorePermit<'static>, T)> {
    tokio::task::spawn_blocking(move || {
        let result = process();
        result.map(|body| (permit, body))
    })
    .await
    .map_err(|error| {
        AppError::internal_error_with_context("decode a filesystem request body", &error)
    })?
}

fn is_json_content_type(headers: &HeaderMap) -> bool {
    // Match Axum Json's media-type rules, including application/*+json and
    // MIME parameters, while performing the expensive decoding off the executor.
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<mime::Mime>().ok())
        .is_some_and(|mime| {
            mime.type_() == "application"
                && (mime.subtype() == "json"
                    || mime.suffix().is_some_and(|suffix| suffix == "json"))
        })
}

fn json_error(error: JsonRejection) -> AppError {
    match error {
        JsonRejection::JsonSyntaxError(error) => AppError::bad_request(error.body_text()),
        JsonRejection::JsonDataError(error) => AppError::unprocessable_entity(error.body_text()),
        error => AppError::internal_error_with_context("decode filesystem write JSON", &error),
    }
}
