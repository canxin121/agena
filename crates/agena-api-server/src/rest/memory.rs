pub async fn list_memories(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(async {
        state
            .application()
            .run_blocking(|app| app.service().list_memories())
            .await?
    })
    .await
}

pub async fn get_memory_overview(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let workspace_root = state
        .application()
        .runtime_diagnostics()
        .await
        .workspace_root;
    let (directory, items) = state
        .application()
        .run_blocking(|app| {
            Ok::<_, agena_application::ApplicationError>((
                app.service().memory_directory(),
                app.service().list_memories()?,
            ))
        })
        .await
        .map_err(ServerError::from)?
        .map_err(ServerError::from)?;
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
        .application()
        .run_blocking(|app| app.service().memory_index_path())
        .await
        .map_err(ServerError::from)?
        .map_err(ServerError::from)?;
    Ok(Json(serde_json::json!({ "path": path })))
}

pub async fn get_memory(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(async {
        state
            .application()
            .run_blocking(move |app| app.service().get_memory(&name))
            .await?
    })
    .await
}

pub async fn save_memory(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<MemoryWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(async {
        state
            .application()
            .run_blocking(move |app| app.service().save_memory(&name, request))
            .await?
    })
    .await
}

pub async fn delete_memory(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(async {
        state
            .application()
            .run_blocking(move |app| app.service().delete_memory(&name))
            .await?
    })
    .await
}

use super::{
    AppState, IntoResponse, Json, MemoryWriteRequest, Path, ServerError, State, json_http,
};
