use std::sync::Arc;

use axum::Json;
use serde::Serialize;

use crate::server::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerHealthResponse {
    status: &'static str,
    generation: u64,
    loaded_at: String,
    workspace_root: String,
    config_path: String,
    config_found: bool,
    provider_ids: Vec<String>,
    session_runtime_available: bool,
}

pub(crate) async fn health(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<ServerHealthResponse> {
    let snapshot = state.application.runtime_diagnostics().await;
    Json(ServerHealthResponse {
        status: "ok",
        generation: snapshot.generation,
        loaded_at: snapshot.loaded_at.to_rfc3339(),
        workspace_root: snapshot.workspace_root.display().to_string(),
        config_path: snapshot.config_path.display().to_string(),
        config_found: snapshot.config_found,
        provider_ids: snapshot.provider_ids,
        session_runtime_available: snapshot.session_runtime_available,
    })
}
