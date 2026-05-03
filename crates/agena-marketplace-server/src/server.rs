use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct RegistrySnapshot {
    pub index: Value,
    pub plugins: BTreeMap<String, Value>,
    pub artifacts: BTreeMap<(String, String, String), RegistryArtifact>,
}

#[derive(Debug, Clone)]
pub struct RegistryArtifact {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

pub fn router(snapshot: RegistrySnapshot) -> Router {
    Router::new()
        .route("/index.json", get(index))
        .route("/plugin/:plugin_id/manifest.json", get(plugin_manifest))
        .route(
            "/plugin/:plugin_id/versions/:version/:artifact",
            get(plugin_artifact),
        )
        .with_state(Arc::new(snapshot))
}

pub async fn serve(addr: SocketAddr, snapshot: RegistrySnapshot) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(snapshot)).await
}

async fn index(State(snapshot): State<Arc<RegistrySnapshot>>) -> Json<Value> {
    Json(snapshot.index.clone())
}

async fn plugin_manifest(
    State(snapshot): State<Arc<RegistrySnapshot>>,
    Path(plugin_id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    snapshot
        .plugins
        .get(plugin_id.as_str())
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn plugin_artifact(
    State(snapshot): State<Arc<RegistrySnapshot>>,
    Path((plugin_id, version, artifact)): Path<(String, String, String)>,
) -> Result<Response, StatusCode> {
    let artifact = snapshot
        .artifacts
        .get(&(plugin_id, version, artifact))
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, artifact.content_type.as_str())],
        Body::from(artifact.bytes.clone()),
    )
        .into_response())
}
