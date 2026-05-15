use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use dashmap::{DashMap, mapref::entry::Entry};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, RwLock};

const TERMINAL_SESSION_REGISTRY_FILENAME: &str = "sessions.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PersistedTerminalSession {
    cwd: String,
    cols: u16,
    rows: u16,
    #[serde(default)]
    running: bool,
    #[serde(default)]
    created_at: u64,
    #[serde(default)]
    updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
struct PersistedTerminalRegistry {
    #[serde(default)]
    version: u64,
    #[serde(default)]
    sessions: BTreeMap<String, PersistedTerminalSession>,
}

struct TerminalSessionStore {
    path: PathBuf,
    cache: RwLock<Option<PersistedTerminalRegistry>>,
    write_lock: AsyncMutex<()>,
}

impl TerminalSessionStore {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            cache: RwLock::new(None),
            write_lock: AsyncMutex::new(()),
        }
    }

    async fn read(&self) -> PersistedTerminalRegistry {
        {
            let guard = self.cache.read().await;
            if let Some(registry) = guard.as_ref() {
                return registry.clone();
            }
        }

        let loaded = self.load_from_disk().await;
        let mut guard = self.cache.write().await;
        if let Some(existing) = guard.as_ref() {
            return existing.clone();
        }
        *guard = Some(loaded.clone());
        loaded
    }

    async fn load_from_disk(&self) -> PersistedTerminalRegistry {
        let raw = match tokio::fs::read_to_string(&self.path).await {
            Ok(raw) => raw,
            Err(_) => return PersistedTerminalRegistry::default(),
        };

        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return PersistedTerminalRegistry::default();
        }

        serde_json::from_str::<PersistedTerminalRegistry>(trimmed).unwrap_or_default()
    }

    async fn persist_to_disk(&self, registry: &PersistedTerminalRegistry) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| format!("create terminal session dir: {error}"))?;
        }

        let bytes = serde_json::to_vec_pretty(registry)
            .map_err(|error| format!("serialize terminal sessions: {error}"))?;
        tokio::fs::write(&self.path, bytes)
            .await
            .map_err(|error| format!("persist terminal sessions: {error}"))
    }

    async fn write_cache(&self, registry: PersistedTerminalRegistry) {
        let mut guard = self.cache.write().await;
        *guard = Some(registry);
    }

    async fn create(
        &self,
        cwd: String,
        cols: u16,
        rows: u16,
    ) -> Result<TerminalCreateResponse, String> {
        let _guard = self.write_lock.lock().await;
        let mut registry = self.load_from_disk().await;
        let now = now_millis();

        let session_id = loop {
            let candidate = crate::issue_token();
            if !registry.sessions.contains_key(&candidate) {
                break candidate;
            }
        };

        registry.version = registry.version.saturating_add(1);
        registry.sessions.insert(
            session_id.clone(),
            PersistedTerminalSession {
                cwd,
                cols,
                rows,
                running: false,
                created_at: now,
                updated_at: now,
            },
        );

        self.persist_to_disk(&registry).await?;
        self.write_cache(registry).await;

        Ok(TerminalCreateResponse {
            session_id,
            cols,
            rows,
        })
    }

    async fn get_info(&self, session_id: &str) -> Option<TerminalInfoResponse> {
        let registry = self.read().await;
        let entry = registry.sessions.get(session_id)?;
        Some(TerminalInfoResponse {
            session_id: session_id.to_string(),
            cwd: entry.cwd.clone(),
            running: entry.running,
        })
    }

    async fn delete(&self, session_id: &str) -> Result<bool, String> {
        let _guard = self.write_lock.lock().await;
        let mut registry = self.load_from_disk().await;
        let removed = registry.sessions.remove(session_id).is_some();
        if !removed {
            return Ok(false);
        }

        registry.version = registry.version.saturating_add(1);
        self.persist_to_disk(&registry).await?;
        self.write_cache(registry).await;
        Ok(true)
    }
}

static TERMINAL_SESSION_STORES: LazyLock<DashMap<PathBuf, Arc<TerminalSessionStore>>> =
    LazyLock::new(DashMap::new);

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn workspace_terminal_session_registry_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join(".agena-studio")
        .join("terminal")
        .join(TERMINAL_SESSION_REGISTRY_FILENAME)
}

fn store_for_workspace(workspace_root: &Path) -> Arc<TerminalSessionStore> {
    let path = workspace_terminal_session_registry_path(workspace_root);
    match TERMINAL_SESSION_STORES.entry(path.clone()) {
        Entry::Occupied(entry) => entry.get().clone(),
        Entry::Vacant(entry) => {
            let store = Arc::new(TerminalSessionStore::new(path));
            entry.insert(store.clone());
            store
        }
    }
}

fn json_error(status: StatusCode, error: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": error.into() }))).into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct TerminalCreateBody {
    pub cwd: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalCreateResponse {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalInfoResponse {
    pub session_id: String,
    pub cwd: String,
    pub running: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TerminalSuccessResponse {
    success: bool,
}

pub(crate) async fn terminal_create(
    State(state): State<Arc<crate::AppState>>,
    Json(body): Json<TerminalCreateBody>,
) -> Response {
    let cwd = match body
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => value,
        None => return json_error(StatusCode::BAD_REQUEST, "cwd is required"),
    };

    let resolved = match tokio::fs::canonicalize(cwd).await {
        Ok(path) => path,
        Err(_) => return json_error(StatusCode::BAD_REQUEST, "Invalid working directory"),
    };

    let metadata = match tokio::fs::metadata(&resolved).await {
        Ok(metadata) => metadata,
        Err(_) => return json_error(StatusCode::BAD_REQUEST, "Invalid working directory"),
    };
    if !metadata.is_dir() {
        return json_error(StatusCode::BAD_REQUEST, "Invalid working directory");
    }

    let cols = body.cols.unwrap_or(80);
    let rows = body.rows.unwrap_or(24);
    let store = store_for_workspace(state.runtime.workspace_root());
    match store
        .create(resolved.display().to_string(), cols, rows)
        .await
    {
        Ok(response) => Json(response).into_response(),
        Err(error) => json_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(crate) async fn terminal_get(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Response {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return json_error(StatusCode::NOT_FOUND, "Terminal session not found");
    }

    let store = store_for_workspace(state.runtime.workspace_root());
    match store.get_info(session_id).await {
        Some(info) => Json(info).into_response(),
        None => json_error(StatusCode::NOT_FOUND, "Terminal session not found"),
    }
}

pub(crate) async fn terminal_delete(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Response {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return json_error(StatusCode::NOT_FOUND, "Terminal session not found");
    }

    let store = store_for_workspace(state.runtime.workspace_root());
    match store.delete(session_id).await {
        Ok(true) => Json(TerminalSuccessResponse { success: true }).into_response(),
        Ok(false) => json_error(StatusCode::NOT_FOUND, "Terminal session not found"),
        Err(error) => json_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena::config::LoadConfigRequest;
    use agena::runtime::AgenaRuntime;
    use axum::{
        Router,
        body::Body,
        http::Request,
        routing::{get, post},
    };
    use axum_extra::extract::cookie::SameSite;
    use http_body_util::BodyExt;
    use tempfile::tempdir;
    use tower::ServiceExt;

    async fn test_state(workspace_root: &Path) -> Arc<crate::AppState> {
        let config_path = workspace_root.join("empty-agena.toml");
        std::fs::write(&config_path, "").expect("empty config should be written");
        let runtime = AgenaRuntime::builder()
            .with_load_request(LoadConfigRequest {
                config_path: Some(config_path),
                overrides: Vec::new(),
            })
            .with_workspace_root(workspace_root)
            .build()
            .await
            .expect("runtime should build");
        Arc::new(crate::AppState {
            ui_auth: crate::ui_auth::init_ui_auth(None),
            ui_cookie_same_site: SameSite::Strict,
            cors_allowed_origins: Vec::new(),
            cors_allow_all: false,
            runtime,
        })
    }

    fn test_router(state: Arc<crate::AppState>) -> Router {
        Router::new()
            .route("/api/terminal/create", post(terminal_create))
            .route(
                "/api/terminal/{session_id}",
                get(terminal_get).delete(terminal_delete),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn terminal_metadata_routes_round_trip_registry_entries() {
        let temp = tempdir().expect("tempdir should be created");
        let state = test_state(temp.path()).await;
        let router = test_router(state);

        let create_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal/create")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "cwd": temp.path(),
                            "cols": 132,
                            "rows": 28
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(create_response.status(), StatusCode::OK);
        let create_body = create_response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let created: TerminalCreateResponse =
            serde_json::from_slice(&create_body).expect("response should be valid json");
        assert!(!created.session_id.is_empty());
        assert_eq!(created.cols, 132);
        assert_eq!(created.rows, 28);

        let registry_path = workspace_terminal_session_registry_path(temp.path());
        assert!(registry_path.is_file());
        let registry_raw =
            std::fs::read_to_string(&registry_path).expect("registry should be readable");
        let registry: PersistedTerminalRegistry =
            serde_json::from_str(&registry_raw).expect("registry should be valid json");
        let persisted = registry
            .sessions
            .get(&created.session_id)
            .expect("created session should be persisted");
        assert_eq!(persisted.cwd, temp.path().display().to_string());
        assert_eq!(persisted.cols, 132);
        assert_eq!(persisted.rows, 28);
        assert!(!persisted.running);

        let get_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/terminal/{}", created.session_id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(get_response.status(), StatusCode::OK);
        let get_body = get_response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let info: TerminalInfoResponse =
            serde_json::from_slice(&get_body).expect("response should be valid json");
        assert_eq!(
            info,
            TerminalInfoResponse {
                session_id: created.session_id.clone(),
                cwd: temp.path().display().to_string(),
                running: false,
            }
        );

        let delete_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/terminal/{}", created.session_id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(delete_response.status(), StatusCode::OK);
        let delete_body = delete_response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let deleted: TerminalSuccessResponse =
            serde_json::from_slice(&delete_body).expect("response should be valid json");
        assert_eq!(deleted, TerminalSuccessResponse { success: true });

        let missing_response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/api/terminal/{}", created.session_id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(missing_response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn terminal_metadata_create_rejects_missing_or_invalid_cwd() {
        let temp = tempdir().expect("tempdir should be created");
        let state = test_state(temp.path()).await;
        let router = test_router(state);

        let missing_cwd = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal/create")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(missing_cwd.status(), StatusCode::BAD_REQUEST);

        let invalid_cwd = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal/create")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "cwd": temp.path().join("missing"),
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(invalid_cwd.status(), StatusCode::BAD_REQUEST);
    }
}
