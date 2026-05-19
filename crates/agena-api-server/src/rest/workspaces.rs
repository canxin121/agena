use super::*;

pub async fn list_workspaces(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<WorkspaceListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_workspaces(query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let workspace = state
        .service()
        .get_workspace(workspace_id)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("workspace not found: {workspace_id}")))?;
    Ok(Json(workspace))
}

pub async fn create_workspace(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .create_workspace(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn resolve_workspace(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceResolveRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .resolve_workspace(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn replace_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    Json(request): Json<WorkspaceWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .replace_workspace(workspace_id, request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn delete_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .delete_workspace(workspace_id)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn list_workspace_files(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    AxumQuery(query): AxumQuery<WorkspaceFileTreeQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_workspace_files(workspace_id, query)
            .await
            .map_err(server_error_from_http)?,
    ))
}
