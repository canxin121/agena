pub async fn list_memories(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(async { state.service().list_memories() }).await
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
