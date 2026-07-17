use std::{
    collections::{BTreeMap, HashSet},
    convert::Infallible,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    #[serde(default)]
    pub folder_id: Option<String>,
    #[serde(default)]
    pub last_used_at: Option<u64>,
}

fn is_false(value: &bool) -> bool {
    !*value
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
        next.updated_at = crate::time::now_millis();

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
            "ts": crate::time::now_millis(),
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

fn workspace_terminal_ui_state_path(workspace_root: &Path) -> PathBuf {
    agena::project_paths::project_state_dir(workspace_root)
        .join("studio")
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
    let name = normalize_session_name(input.name.as_deref().unwrap_or_default());
    let name = (!name.is_empty()).then_some(name);
    let folder_id = normalize_folder_id(input.folder_id.as_deref().unwrap_or_default());
    let folder_id = (!folder_id.is_empty()
        && folder_ids.contains(&folder_id)
        && folder_id != DEFAULT_FOLDER_ID)
        .then_some(folder_id);

    let last_used_at = input.last_used_at.unwrap_or(0);
    let last_used_at = (last_used_at > 0).then_some(last_used_at);

    let out = TerminalUiSessionMeta {
        name,
        pinned: input.pinned,
        folder_id,
        last_used_at,
    };

    if out.name.is_none() && !out.pinned && out.folder_id.is_none() && out.last_used_at.is_none() {
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
