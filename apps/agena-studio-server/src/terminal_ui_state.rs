use std::{
    collections::{BTreeMap, HashSet},
    convert::Infallible,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_stream::stream;
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use dashmap::{DashMap, mapref::entry::Entry};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, RwLock, broadcast};

const DEFAULT_FOLDER_ID: &str = "terminal-default";
const DEFAULT_FOLDER_NAME: &str = "Default";
const TERMINAL_UI_STATE_FILENAME: &str = "terminal-ui-state.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalUiFolder {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
struct SequencedTerminalUiStateEvent {
    seq: u64,
    payload: String,
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
    tx: broadcast::Sender<SequencedTerminalUiStateEvent>,
    next_seq: AtomicU64,
}

impl TerminalUiStateStore {
    fn new(path: PathBuf) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            path,
            cache: RwLock::new(None),
            put_lock: AsyncMutex::new(()),
            tx,
            next_seq: AtomicU64::new(1),
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
        self.publish_state_replace(&next);
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

    fn subscribe(&self) -> broadcast::Receiver<SequencedTerminalUiStateEvent> {
        self.tx.subscribe()
    }

    fn publish_state_replace(&self, state: &TerminalUiState) {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let payload = serde_json::to_string(&serde_json::json!({
            "type": "terminal-ui-state.patch",
            "seq": seq,
            "ts": now_millis(),
            "properties": {
                "ops": [
                    {
                        "type": "state.replace",
                        "state": state,
                    }
                ]
            }
        }))
        .unwrap_or_else(|_| "{}".to_string());

        let _ = self.tx.send(SequencedTerminalUiStateEvent { seq, payload });
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

fn snapshot_payload(state: &TerminalUiState) -> String {
    serde_json::to_string(&serde_json::json!({
        "type": "terminal-ui-state.snapshot",
        "state": state,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct TerminalUiStateEventsQuery {
    pub since: Option<String>,
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

pub(crate) async fn terminal_ui_state_events(
    State(state): State<Arc<crate::AppState>>,
    Query(query): Query<TerminalUiStateEventsQuery>,
) -> Response {
    let _ = query.since;
    let store = store_for_workspace(state.runtime.workspace_root());
    let mut rx = store.subscribe();
    let initial = snapshot_payload(&store.read().await);

    let sse_stream = stream! {
        yield Ok::<Event, Infallible>(Event::default().data(initial));

        loop {
            match rx.recv().await {
                Ok(event) => {
                    yield Ok::<Event, Infallible>(
                        Event::default()
                            .id(event.seq.to_string())
                            .data(event.payload),
                    );
                }
                Err(broadcast::error::RecvError::Lagged(_)) => break,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(sse_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("heartbeat"),
        )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::get};
    use http_body_util::BodyExt;
    use tempfile::tempdir;
    use tower::ServiceExt;

    async fn test_state(workspace_root: &Path) -> Arc<crate::AppState> {
        crate::test_support::build_test_app_state(
            workspace_root,
            crate::settings::Settings::default(),
        )
        .await
    }

    fn test_router(state: Arc<crate::AppState>) -> Router {
        Router::new()
            .route(
                "/api/ui/terminal/state",
                get(terminal_ui_state_get).put(terminal_ui_state_put),
            )
            .route(
                "/api/ui/terminal/state/events",
                get(terminal_ui_state_events),
            )
            .with_state(state)
    }

    async fn next_event_chunk(body: &mut Body) -> String {
        let frame = tokio::time::timeout(Duration::from_secs(1), body.frame())
            .await
            .expect("stream frame should arrive")
            .expect("stream should yield frame")
            .expect("stream should not error");
        let data = frame.into_data().expect("frame should contain data");
        String::from_utf8(data.to_vec()).expect("frame should be utf8")
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

    #[tokio::test]
    async fn terminal_ui_state_events_route_streams_snapshot_and_patch() {
        let temp = tempdir().expect("tempdir should be created");
        let state = test_state(temp.path()).await;
        let router = test_router(state);

        let events_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/ui/terminal/state/events")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(events_response.status(), StatusCode::OK);
        assert_eq!(
            events_response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );

        let mut events_body = events_response.into_body();
        let snapshot_chunk = next_event_chunk(&mut events_body).await;
        assert!(
            snapshot_chunk.contains("terminal-ui-state.snapshot"),
            "unexpected snapshot chunk: {snapshot_chunk:?}"
        );
        assert!(
            snapshot_chunk.contains("\"version\":0"),
            "unexpected snapshot chunk: {snapshot_chunk:?}"
        );

        let put_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/ui/terminal/state")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "activeSessionId": "alpha",
                            "sessionIds": ["alpha"],
                            "sessionMetaById": {
                                "alpha": {
                                    "name": "Alpha"
                                }
                            },
                            "folders": []
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(put_response.status(), StatusCode::OK);

        let patch_chunk = next_event_chunk(&mut events_body).await;
        assert!(
            patch_chunk.contains("id:1") || patch_chunk.contains("id: 1"),
            "unexpected patch chunk: {patch_chunk:?}"
        );
        assert!(
            patch_chunk.contains("terminal-ui-state.patch"),
            "unexpected patch chunk: {patch_chunk:?}"
        );
        assert!(
            patch_chunk.contains("\"type\":\"state.replace\""),
            "unexpected patch chunk: {patch_chunk:?}"
        );
        assert!(
            patch_chunk.contains("\"sessionIds\":[\"alpha\"]"),
            "unexpected patch chunk: {patch_chunk:?}"
        );
    }
}
