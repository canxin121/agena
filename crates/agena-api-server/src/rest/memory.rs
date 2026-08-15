pub async fn list_memories(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(async { state.service().list_memories() }).await
}

pub async fn get_memory_overview(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let workspace_root = state
        .application()
        .runtime_status_response()
        .await
        .workspace_root;
    let directory = state.service().memory_directory();
    let items = state.service().list_memories().map_err(ServerError::from)?;
    Ok(Json(serde_json::json!({
        "workspace_root": workspace_root,
        "directory": directory,
        "items": items,
    })))
}

pub async fn ensure_memory_index(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let path = state
        .service()
        .memory_index_path()
        .map_err(ServerError::from)?;
    Ok(Json(serde_json::json!({ "path": path })))
}

pub async fn get_memory(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(async { state.service().get_memory(name.as_str()) }).await
}

pub async fn save_memory(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<MemoryWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(async { state.service().save_memory(name.as_str(), request) }).await
}

pub async fn delete_memory(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(async { state.service().delete_memory(name.as_str()) }).await
}

use super::{
    AppState, IntoResponse, Json, MemoryWriteRequest, Path, ServerError, State, json_http,
};
