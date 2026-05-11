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
        .route("/plugin/{plugin_id}/manifest.json", get(plugin_manifest))
        .route(
            "/plugin/{plugin_id}/versions/{version}/{artifact}",
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::util::ServiceExt;

    fn test_snapshot() -> RegistrySnapshot {
        let mut plugins = BTreeMap::new();
        plugins.insert(
            "demo".to_string(),
            serde_json::json!({
                "id": "demo",
                "versions": [{"version": "1.0.0"}]
            }),
        );

        let mut artifacts = BTreeMap::new();
        artifacts.insert(
            (
                "demo".to_string(),
                "1.0.0".to_string(),
                "plugin.wasm".to_string(),
            ),
            RegistryArtifact {
                bytes: b"wasm-binary".to_vec(),
                content_type: "application/wasm".to_string(),
            },
        );

        RegistrySnapshot {
            index: serde_json::json!({"version": 1, "plugins": [{"id": "demo"}]}),
            plugins,
            artifacts,
        }
    }

    #[tokio::test]
    async fn routes_index_manifest_and_artifact() {
        let app = router(test_snapshot());

        let index = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/index.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(index.status(), StatusCode::OK);
        let index_body = index.into_body().collect().await.unwrap().to_bytes();
        let index_json: Value = serde_json::from_slice(&index_body).unwrap();
        assert_eq!(index_json["version"], 1);

        let manifest = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/plugin/demo/manifest.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(manifest.status(), StatusCode::OK);
        let manifest_body = manifest.into_body().collect().await.unwrap().to_bytes();
        let manifest_json: Value = serde_json::from_slice(&manifest_body).unwrap();
        assert_eq!(manifest_json["id"], "demo");

        let artifact = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/plugin/demo/versions/1.0.0/plugin.wasm")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(artifact.status(), StatusCode::OK);
        assert_eq!(
            artifact
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/wasm")
        );
        let artifact_body = artifact.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(artifact_body.as_ref(), b"wasm-binary");
    }

    #[tokio::test]
    async fn missing_manifest_and_artifact_return_not_found() {
        let app = router(test_snapshot());

        let missing_manifest = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/plugin/missing/manifest.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_manifest.status(), StatusCode::NOT_FOUND);

        let missing_artifact = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/plugin/demo/versions/9.9.9/plugin.wasm")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_artifact.status(), StatusCode::NOT_FOUND);
    }
}
