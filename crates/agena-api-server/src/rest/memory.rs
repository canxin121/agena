pub use agena_api::business::{MemoryBatchDeleteRequest, MemoryDeleteQuery};

use super::{
    AppState, IntoResponse, Json, MemoryWriteRequest, Path, ServerError, State, json_http,
};

pub async fn list_memories(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().list_memories()).await
}

pub async fn get_memory_overview(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(state.application().memory_overview().await?))
}

pub async fn ensure_memory_index(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(state.application().ensure_memory_index().await?))
}

pub async fn get_memory(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().get_memory(name)).await
}

pub async fn save_memory(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<MemoryWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().save_memory(name, request)).await
}

pub async fn delete_memory(
    State(state): State<AppState>,
    Path(name): Path<String>,
    axum::extract::Query(query): axum::extract::Query<MemoryDeleteQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .delete_memory(name, query.expected_sha256),
    )
    .await
}

pub async fn delete_memories(
    State(state): State<AppState>,
    Json(request): Json<MemoryBatchDeleteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().delete_memories(request.records)).await
}
