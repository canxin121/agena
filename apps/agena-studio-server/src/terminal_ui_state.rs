use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use dashmap::{DashMap, mapref::entry::Entry};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, RwLock};

const DEFAULT_FOLDER_ID: &str = "terminal-default";
const DEFAULT_FOLDER_NAME: &str = "Default";
const TERMINAL_UI_STATE_FILENAME: &str = "terminal-ui-state.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalUiFolder {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalUiSessionMeta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub pinned: Option<bool>,
    #[serde(default)]
    pub folder_id: Option<String>,
    #[serde(default)]
    pub last_used_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalUiState {
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub active_session_id: Option<String>,
    #[serde(default)]
    pub session_ids: Vec<String>,
    #[serde(default)]
    pub session_meta_by_id: BTreeMap<String, TerminalUiSessionMeta>,
    #[serde(default)]
    pub folders: Vec<TerminalUiFolder>,
}

impl Default for TerminalUiState {
    fn default() -> Self {
        Self {
            version: 0,
            updated_at: 0,
            active_session_id: None,
            session_ids: Vec::new(),
            session_meta_by_id: BTreeMap::new(),
            folders: vec![TerminalUiFolder {
                id: DEFAULT_FOLDER_ID.to_string(),
                name: DEFAULT_FOLDER_NAME.to_string(),
            }],
        }
    }
}

struct TerminalUiStateStore {
    path: PathBuf,
    cache: RwLock<Option<TerminalUiState>>,
    put_lock: AsyncMutex<()>,
}

impl TerminalUiStateStore {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            cache: RwLock::new(None),
            put_lock: AsyncMutex::new(()),
        }
    }

    async fn read(&self) -> TerminalUiState {
        {
            let guard = self.cache.read().await;
            if let Some(state) = guard.as_ref() {
                return state.clone();
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

    async fn replace(&self, body: TerminalUiState) -> Result<TerminalUiState, String> {
        let _guard = self.put_lock.lock().await;
        let current = self.load_from_disk().await;
        self.write_cache(current.clone()).await;

        let mut next = sanitize_state(body);
        next.version = current.version.saturating_add(1);
        next.updated_at = now_millis();

        self.persist_to_disk(&next).await?;
        self.write_cache(next.clone()).await;
        Ok(next)
    }

    async fn load_from_disk(&self) -> TerminalUiState {
        let raw = match tokio::fs::read_to_string(&self.path).await {
            Ok(raw) => raw,
            Err(_) => return TerminalUiState::default(),
        };

        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return TerminalUiState::default();
        }

        serde_json::from_str::<TerminalUiState>(trimmed)
            .map(sanitize_state)
            .unwrap_or_default()
    }

    async fn persist_to_disk(&self, state: &TerminalUiState) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| format!("create terminal ui state dir: {error}"))?;
        }

        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|error| format!("serialize terminal ui state: {error}"))?;
        tokio::fs::write(&self.path, bytes)
            .await
            .map_err(|error| format!("persist terminal ui state: {error}"))
    }

    async fn write_cache(&self, state: TerminalUiState) {
        let mut guard = self.cache.write().await;
        *guard = Some(state);
    }
}

static TERMINAL_UI_STATE_STORES: LazyLock<DashMap<PathBuf, Arc<TerminalUiStateStore>>> =
    LazyLock::new(DashMap::new);

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn workspace_terminal_ui_state_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join(".agena-studio")
        .join("ui")
        .join(TERMINAL_UI_STATE_FILENAME)
}

fn store_for_workspace(workspace_root: &Path) -> Arc<TerminalUiStateStore> {
    let path = workspace_terminal_ui_state_path(workspace_root);
    match TERMINAL_UI_STATE_STORES.entry(path.clone()) {
        Entry::Occupied(entry) => entry.get().clone(),
        Entry::Vacant(entry) => {
            let store = Arc::new(TerminalUiStateStore::new(path));
            entry.insert(store.clone());
            store
        }
    }
}

fn clip_chars(input: String, max_len: usize) -> String {
    input.chars().take(max_len).collect()
}

fn collapse_spaces(input: &str) -> String {
    input
        .split_whitespace()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_session_id(raw: &str) -> String {
    raw.trim().to_string()
}

fn normalize_folder_id(raw: &str) -> String {
    clip_chars(raw.trim().to_string(), 80)
}

fn normalize_folder_name(raw: &str) -> String {
    clip_chars(collapse_spaces(raw), 40)
}

fn normalize_session_name(raw: &str) -> String {
    clip_chars(collapse_spaces(raw), 80)
}

fn sanitize_session_meta(
    input: TerminalUiSessionMeta,
    folder_ids: &HashSet<String>,
) -> Option<TerminalUiSessionMeta> {
    let mut out = TerminalUiSessionMeta::default();

    let name = normalize_session_name(input.name.as_deref().unwrap_or_default());
    if !name.is_empty() {
        out.name = Some(name);
    }

    if input.pinned.unwrap_or(false) {
        out.pinned = Some(true);
    }

    let folder_id = normalize_folder_id(input.folder_id.as_deref().unwrap_or_default());
    if !folder_id.is_empty() && folder_ids.contains(&folder_id) && folder_id != DEFAULT_FOLDER_ID {
        out.folder_id = Some(folder_id);
    }

    let last_used_at = input.last_used_at.unwrap_or(0);
    if last_used_at > 0 {
        out.last_used_at = Some(last_used_at);
    }

    if out.name.is_none()
        && out.pinned.is_none()
        && out.folder_id.is_none()
        && out.last_used_at.is_none()
    {
        return None;
    }

    Some(out)
}

fn sanitize_state(input: TerminalUiState) -> TerminalUiState {
    let mut session_ids = Vec::<String>::new();
    let mut session_seen = HashSet::<String>::new();
    for raw in input.session_ids {
        let session_id = normalize_session_id(&raw);
        if session_id.is_empty() {
            continue;
        }
        if session_seen.insert(session_id.clone()) {
            session_ids.push(session_id);
        }
    }

    let mut folders = Vec::<TerminalUiFolder>::new();
    let mut folder_ids = HashSet::<String>::new();
    for folder in input.folders {
        let id = normalize_folder_id(&folder.id);
        let name = normalize_folder_name(&folder.name);
        if id.is_empty() || name.is_empty() {
            continue;
        }
        if folder_ids.insert(id.clone()) {
            folders.push(TerminalUiFolder { id, name });
        }
    }

    if !folder_ids.contains(DEFAULT_FOLDER_ID) {
        folders.insert(
            0,
            TerminalUiFolder {
                id: DEFAULT_FOLDER_ID.to_string(),
                name: DEFAULT_FOLDER_NAME.to_string(),
            },
        );
        folder_ids.insert(DEFAULT_FOLDER_ID.to_string());
    }

    let mut session_meta_by_id = BTreeMap::<String, TerminalUiSessionMeta>::new();
    for session_id in &session_ids {
        let Some(meta) = input.session_meta_by_id.get(session_id).cloned() else {
            continue;
        };
        if let Some(compact) = sanitize_session_meta(meta, &folder_ids) {
            session_meta_by_id.insert(session_id.clone(), compact);
        }
    }

    let requested_active =
        normalize_session_id(input.active_session_id.as_deref().unwrap_or_default());
    let active_session_id =
        if !requested_active.is_empty() && session_seen.contains(&requested_active) {
            Some(requested_active)
        } else {
            session_ids.first().cloned()
        };

    TerminalUiState {
        version: input.version,
        updated_at: input.updated_at,
        active_session_id,
        session_ids,
        session_meta_by_id,
        folders,
    }
}

pub(crate) async fn terminal_ui_state_get(State(state): State<Arc<crate::AppState>>) -> Response {
    let store = store_for_workspace(state.runtime.workspace_root());
    Json(store.read().await).into_response()
}

pub(crate) async fn terminal_ui_state_put(
    State(state): State<Arc<crate::AppState>>,
    Json(body): Json<TerminalUiState>,
) -> Response {
    let store = store_for_workspace(state.runtime.workspace_root());
    match store.replace(body).await {
        Ok(next) => Json(next).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena::config::LoadConfigRequest;
    use agena::runtime::AgenaRuntime;
    use axum::{Router, body::Body, http::Request, routing::get};
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
            .route(
                "/api/ui/terminal/state",
                get(terminal_ui_state_get).put(terminal_ui_state_put),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn terminal_ui_state_route_round_trips_sanitized_state_to_disk() {
        let temp = tempdir().expect("tempdir should be created");
        let state = test_state(temp.path()).await;
        let router = test_router(state);

        let initial_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/ui/terminal/state")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(initial_response.status(), StatusCode::OK);
        let initial_body = initial_response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let initial_payload: TerminalUiState =
            serde_json::from_slice(&initial_body).expect("response should be valid json");
        assert_eq!(initial_payload, TerminalUiState::default());

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/ui/terminal/state")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "activeSessionId": "missing",
                            "sessionIds": ["  alpha  ", "alpha", "", "beta"],
                            "sessionMetaById": {
                                "alpha": {
                                    "name": "  Alpha   Session  ",
                                    "pinned": true,
                                    "folderId": " team ",
                                    "lastUsedAt": 42
                                },
                                "ignored": {
                                    "name": "nope"
                                }
                            },
                            "folders": [
                                {"id": " team ", "name": "  Team   Space "},
                                {"id": "", "name": "ignored"}
                            ]
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: TerminalUiState =
            serde_json::from_slice(&body).expect("response should be valid json");
        assert_eq!(payload.version, 1);
        assert!(payload.updated_at > 0);
        assert_eq!(payload.active_session_id.as_deref(), Some("alpha"));
        assert_eq!(
            payload.session_ids,
            vec!["alpha".to_string(), "beta".to_string()]
        );
        assert_eq!(
            payload
                .session_meta_by_id
                .get("alpha")
                .expect("alpha metadata should exist"),
            &TerminalUiSessionMeta {
                name: Some("Alpha Session".to_string()),
                pinned: Some(true),
                folder_id: Some("team".to_string()),
                last_used_at: Some(42),
            }
        );
        assert!(payload.session_meta_by_id.get("ignored").is_none());
        assert_eq!(
            payload.folders,
            vec![
                TerminalUiFolder {
                    id: DEFAULT_FOLDER_ID.to_string(),
                    name: DEFAULT_FOLDER_NAME.to_string(),
                },
                TerminalUiFolder {
                    id: "team".to_string(),
                    name: "Team Space".to_string(),
                },
            ]
        );

        let persisted_path = workspace_terminal_ui_state_path(temp.path());
        assert!(persisted_path.is_file());
        let persisted_raw =
            std::fs::read_to_string(&persisted_path).expect("persisted state should be readable");
        let persisted_state: TerminalUiState =
            serde_json::from_str(&persisted_raw).expect("persisted state should be valid json");
        assert_eq!(persisted_state, payload);

        let get_response = router
            .oneshot(
                Request::builder()
                    .uri("/api/ui/terminal/state")
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
        let get_payload: TerminalUiState =
            serde_json::from_slice(&get_body).expect("response should be valid json");
        assert_eq!(get_payload, payload);
    }
}
