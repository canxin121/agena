//! Optional typed mirror/server for an Agena marketplace. GitHub remains the
//! release and catalog source of truth; this server can expose the exact same
//! immutable index/release/artifact shapes for private networks or caches.

use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use agena_plugin_marketplace::{
    AGENA_MARKETPLACE_FILENAME, AGENA_RELEASE_MANIFEST_FILENAME, MarketplaceError,
    PluginReleaseManifest, RegistryIndex,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
/// Snapshot of a marketplace registry.
pub struct RegistrySnapshot {
    pub index: RegistryIndex,
    pub releases: BTreeMap<(String, String), PluginReleaseManifest>,
    pub artifacts: BTreeMap<(String, String, String), RegistryArtifact>,
}

impl Default for RegistrySnapshot {
    fn default() -> Self {
        Self {
            index: RegistryIndex::default(),
            releases: BTreeMap::new(),
            artifacts: BTreeMap::new(),
        }
    }
}

impl RegistrySnapshot {
    pub fn validate(&self) -> Result<(), MarketplaceError> {
        self.index.validate()?;
        for ((plugin_id, version), release) in &self.releases {
            release.validate()?;
            if release.id != *plugin_id || release.version != *version {
                return Err(MarketplaceError::Index(format!(
                    "release key {plugin_id}@{version} does not match manifest {}@{}",
                    release.id, release.version
                )));
            }
            for artifact in &release.artifacts {
                let key = (plugin_id.clone(), version.clone(), artifact.asset.clone());
                let stored = self.artifacts.get(&key).ok_or_else(|| {
                    MarketplaceError::Index(format!(
                        "release {plugin_id}@{version} is missing artifact `{}`",
                        artifact.asset
                    ))
                })?;
                let actual = hex::encode(Sha256::digest(&stored.bytes));
                if !actual.eq_ignore_ascii_case(&artifact.sha256) {
                    return Err(MarketplaceError::Sha256Mismatch {
                        plugin: plugin_id.clone(),
                        expected: artifact.sha256.clone(),
                        got: actual,
                    });
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
/// An artifact stored in the registry.
pub struct RegistryArtifact {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

pub fn router(snapshot: RegistrySnapshot) -> Result<Router, MarketplaceError> {
    snapshot.validate()?;
    Ok(Router::new()
        .route(&format!("/{AGENA_MARKETPLACE_FILENAME}"), get(index))
        .route(
            &format!(
                "/plugins/{{plugin_id}}/releases/{{version}}/{AGENA_RELEASE_MANIFEST_FILENAME}"
            ),
            get(plugin_release),
        )
        .route(
            "/plugins/{plugin_id}/releases/{version}/{artifact}",
            get(plugin_artifact),
        )
        .with_state(Arc::new(snapshot)))
}

pub async fn serve(addr: SocketAddr, snapshot: RegistrySnapshot) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let app = router(snapshot).map_err(std::io::Error::other)?;
    axum::serve(listener, app).await
}

async fn index(State(snapshot): State<Arc<RegistrySnapshot>>) -> Json<RegistryIndex> {
    Json(snapshot.index.clone())
}

async fn plugin_release(
    State(snapshot): State<Arc<RegistrySnapshot>>,
    Path((plugin_id, version)): Path<(String, String)>,
) -> Result<Json<PluginReleaseManifest>, StatusCode> {
    snapshot
        .releases
        .get(&(plugin_id, version))
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
