pub async fn list_workspaces(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<WorkspaceListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().list_workspaces(query)).await
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
    json_http(state.service().create_workspace(request)).await
}

pub async fn resolve_workspace(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceResolveRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().resolve_workspace(request)).await
}

pub async fn replace_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    Json(request): Json<WorkspacePathRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().replace_workspace(workspace_id, request)).await
}

pub async fn delete_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().delete_workspace(workspace_id)).await
}

pub async fn list_workspace_files(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    AxumQuery(query): AxumQuery<WorkspaceFileTreeQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().list_workspace_files(workspace_id, query)).await
}
use super::{
    AppState, AxumQuery, IntoResponse, Json, Path, ServerError, State, WorkspaceFileTreeQuery,
    WorkspaceListQuery, WorkspacePathRequest, WorkspaceResolveRequest, json_http, json_http_found,
};
