pub async fn list_workspaces(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<WorkspaceListQuery>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    let key = if query.include_session_count {
        "workspaces"
    } else {
        "workspaces:catalog"
    };
    let read = crate::revisions::ConditionalRead::new(&state, key).await?;
    if let Some(response) = read.not_modified(&headers) {
        return Ok(response);
    }
    Ok(read.headers(
        json_http(state.service().list_workspaces(query))
            .await?
            .into_response(),
    ))
}

#[derive(serde::Deserialize)]
pub struct WorkspaceStatsQuery {
    ids: String,
}

/// Only requested directories are counted. Multiple dirty directories share
/// one bounded request without retransmitting the workspace catalog.
pub async fn workspace_session_stats(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<WorkspaceStatsQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let mut ids = query
        .ids
        .split(',')
        .map(str::parse::<i64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ServerError::bad_request("ids must contain positive workspace ids"))?;
    if ids.len() > 128 || ids.iter().any(|id| *id <= 0) {
        return Err(ServerError::bad_request(
            "At most 128 positive workspace ids are allowed",
        ));
    }
    ids.sort_unstable();
    ids.dedup();
    let revisions = state.revisions()?;
    revisions.refresh_durable(&state).await?;
    // Capture each token before the projection to preserve a trailing update
    // if a mutation races this batch.
    let tokens = ids
        .iter()
        .map(|id| (*id, revisions.token(&format!("workspace:{id}:stats"))))
        .collect::<std::collections::HashMap<_, _>>();
    let stats = state
        .session_store()?
        .workspace_session_stats(&ids)
        .await
        .map_err(crate::rest::server_error_from_store)?;
    Ok(Json(serde_json::json!({"items": ids.into_iter().map(|id| {
        let row = stats.get(&id).copied().unwrap_or_default();
        serde_json::json!({"workspace_id": id, "revision": tokens[&id], "stats": {
            "total": row.total, "roots": row.roots, "pinned": row.pinned,
            "running": row.running, "attention": row.attention,
        }})
    }).collect::<Vec<_>>()})))
}

pub async fn get_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    json_http_found(state.service().get_workspace(workspace_id), || {
        format!("workspace not found: {workspace_id}")
    })
    .await
}

pub async fn create_workspace(
    State(state): State<AppState>,
    Json(request): Json<WorkspacePathRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let revisions = state.revisions()?;
    revisions.refresh_durable(&state).await?;
    let result = json_http(state.service().create_workspace(request)).await?;
    revisions.refresh_workspaces(&state).await?;
    Ok(result)
}

pub async fn resolve_workspace(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceResolveRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let revisions = state.revisions()?;
    revisions.refresh_durable(&state).await?;
    let result = json_http(state.service().resolve_workspace(request)).await?;
    revisions.refresh_workspaces(&state).await?;
    Ok(result)
}

pub async fn replace_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    Json(request): Json<WorkspacePathRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let revisions = state.revisions()?;
    revisions.refresh_durable(&state).await?;
    let result = json_http(state.service().replace_workspace(workspace_id, request)).await?;
    revisions.refresh_workspaces(&state).await?;
    Ok(result)
}

pub async fn delete_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let revisions = state.revisions()?;
    revisions.refresh_durable(&state).await?;
    let result = json_http(state.service().delete_workspace(workspace_id)).await?;
    revisions.refresh_workspaces(&state).await?;
    Ok(result)
}

pub async fn list_workspace_files(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    AxumQuery(query): AxumQuery<WorkspaceFileTreeQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().list_workspace_files(workspace_id, query)).await
}

pub async fn upload_workspace_file(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    Json(request): Json<WorkspaceFileUploadRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().upload_workspace_file(workspace_id, request)).await
}

pub async fn download_workspace_file(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    AxumQuery(query): AxumQuery<WorkspaceFileDownloadQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let (filename, bytes) = state
        .service()
        .read_workspace_file(workspace_id, query)
        .await
        .map_err(server_error_from_application)?;
    Ok((
        [
            ("content-type", "application/octet-stream".to_string()),
            (
                "content-disposition",
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        bytes,
    ))
}

pub async fn download_session_media_file(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<WorkspaceFileDownloadQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let (filename, bytes) = state
        .service()
        .read_session_media_file(session_id, query)
        .await
        .map_err(server_error_from_application)?;
    Ok((
        [
            ("content-type", "application/octet-stream".to_string()),
            (
                "content-disposition",
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        bytes,
    ))
}
use super::{
    AppState, AxumQuery, IntoResponse, Json, Path, ServerError, State, WorkspaceFileDownloadQuery,
    WorkspaceFileTreeQuery, WorkspaceFileUploadRequest, WorkspaceListQuery, WorkspacePathRequest,
    WorkspaceResolveRequest, json_http, json_http_found, server_error_from_application,
};
