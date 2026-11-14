//! HTTP representation validators for the shared file-edit projection.
use crate::{error::ServerError, state::AppState};
pub use agena_api::extended_sessions::SessionFileChangesParams as FileChangesQuery;
use agena_api::resource::SessionFileChangesResource;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

pub async fn get_session_file_changes(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<FileChangesQuery>,
    headers: HeaderMap,
) -> Result<Response, ServerError> {
    let resource = format!("session:{id}:files");
    let read = crate::revisions::ConditionalRead::new(&state, &resource).await?;
    let projection = state.application().session_file_changes(id, query).await?;
    let revisions = state.revisions()?;
    revisions.refresh_durable(&state).await?;
    file_changes_response(read, projection, &headers, revisions.token(&resource)).await
}

/// The resource clock wakes readers; the representation validator determines
/// whether this particular summary/page/file actually needs any body bytes.
async fn file_changes_response(
    read: crate::revisions::ConditionalRead,
    projection: SessionFileChangesResource,
    headers: &HeaderMap,
    current_revision: String,
) -> Result<Response, ServerError> {
    let authority = read
        .token()
        .rsplit_once(':')
        .map_or(read.token(), |(epoch, _)| epoch)
        .to_owned();
    let (bytes, etag) = FILE_PROJECTIONS
        .run(move || {
            let bytes = serde_json::to_vec(&projection)?;
            let etag = format!("W/\"{authority}:{}\"", blake3::hash(&bytes).to_hex());
            Ok::<_, serde_json::Error>((bytes, etag))
        })
        .await
        .map_err(|error| ServerError::internal_error(&error))?
        .map_err(|error| ServerError::internal_error(&error))?;
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|tag| tag.trim().trim_start_matches("W/") == etag.trim_start_matches("W/"))
        });
    let response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, "application/json")], bytes).into_response()
    };
    let mut response = read.headers(response);
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("file representation ETag"),
    );
    response.headers_mut().insert(
        "x-resource-current-revision",
        HeaderValue::from_str(&current_revision).expect("current file resource revision"),
    );
    Ok(response)
}

static FILE_PROJECTIONS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
