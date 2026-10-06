use portable_atomic::AtomicU64;
use std::{
    collections::{BTreeMap, HashSet},
    convert::Infallible,
    hash::{Hash, Hasher},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, atomic::Ordering},
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
use tokio::sync::{Mutex as AsyncMutex, broadcast};

const DEFAULT_FOLDER_ID: &str = "terminal-default";
const DEFAULT_FOLDER_NAME: &str = "Default";
const TERMINAL_UI_STATE_FILENAME: &str = "terminal-ui-state.json";
const MAX_WORKSPACE_KEY_LEN: usize = 80;
const MAX_STATE_BYTES: usize = 8 * 1024 * 1024;
static STATE_READS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
static STATE_WRITES: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TerminalUiFolder {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
struct SequencedTerminalUiStateEvent {
    seq: u64,
    payload: Arc<str>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TerminalUiState {
    pub version: u64,
    pub updated_at: u64,
    pub active_session_id: Option<String>,
    pub session_ids: Vec<String>,
    pub session_meta_by_id: BTreeMap<String, TerminalUiSessionMeta>,
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

#[derive(Clone)]
struct TerminalUiStateStore {
    path: PathBuf,
    cache: Arc<Mutex<Option<Arc<TerminalUiState>>>>,
    put_lock: Arc<AsyncMutex<()>>,
    tx: broadcast::Sender<SequencedTerminalUiStateEvent>,
    next_seq: Arc<AtomicU64>,
}

impl TerminalUiStateStore {
    fn new(path: PathBuf) -> Self {
        // Every patch contains the complete state. Bound retained snapshots;
        // lagging clients already reconnect and receive the current snapshot.
        let (tx, _) = broadcast::channel(64);
        Self {
            path,
            cache: Arc::new(Mutex::new(None)),
            put_lock: Arc::new(AsyncMutex::new(())),
            tx,
            next_seq: Arc::new(AtomicU64::new(1)),
        }
    }

    fn cached(&self) -> Option<Arc<TerminalUiState>> {
        self.cache
            .lock()
            .expect("terminal state cache lock")
            .clone()
    }

    async fn read(&self) -> Result<Arc<TerminalUiState>, String> {
        if let Some(state) = self.cached() {
            return Ok(state);
        }
        // Share initialization with mutations. A started worker owns the gate
        // through publication even if the HTTP request is cancelled.
        let gate = Arc::clone(&self.put_lock).lock_owned().await;
        if let Some(state) = self.cached() {
            return Ok(state);
        }
        let store = self.clone();
        STATE_READS
            .run(move || {
                let _gate = gate;
                let loaded = Arc::new(store.load_from_disk()?);
                store.write_cache(Arc::clone(&loaded));
                Ok(loaded)
            })
            .await
            .map_err(|error| format!("terminal state read worker failed: {error}"))?
    }

    async fn replace(&self, body: TerminalUiState) -> Result<Arc<TerminalUiState>, String> {
        let gate = Arc::clone(&self.put_lock).lock_owned().await;
        let store = self.clone();
        STATE_WRITES
            .run(move || {
                let _gate = gate;
                let current = store.load_from_disk()?;
                let version = current.version;
                store.write_cache(Arc::new(current));
                let mut next = sanitize_state(body);
                next.version = version.saturating_add(1);
                next.updated_at = now_millis();
                store.persist_to_disk(&next)?;
                let next = Arc::new(next);
                store.write_cache(Arc::clone(&next));
                store.publish_state_replace(&next);
                Ok(next)
            })
            .await
            .map_err(|error| format!("terminal state write worker failed: {error}"))?
    }

    fn load_from_disk(&self) -> Result<TerminalUiState, String> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        // Opening a named pipe must not occupy a state worker indefinitely.
        // Inspect the opened handle so a concurrent rename cannot bypass the
        // regular-file check. O_NONBLOCK has no effect on regular files.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = match options.open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TerminalUiState::default());
            }
            Err(error) => {
                return Err(format!(
                    "read terminal UI state {}: {error}",
                    self.path.display()
                ));
            }
        };
        let metadata = file.metadata().map_err(|error| {
            format!("inspect terminal UI state {}: {error}", self.path.display())
        })?;
        if !metadata.is_file() {
            return Err("terminal UI state must be a regular file".to_owned());
        }
        if metadata.len() > MAX_STATE_BYTES as u64 {
            return Err("persisted terminal UI state exceeds the 8 MiB limit".to_owned());
        }
        let mut raw = Vec::new();
        file.take((MAX_STATE_BYTES + 1) as u64)
            .read_to_end(&mut raw)
            .map_err(|error| format!("read terminal UI state {}: {error}", self.path.display()))?;
        if raw.len() > MAX_STATE_BYTES {
            return Err("persisted terminal UI state exceeds the 8 MiB limit".to_owned());
        }
        if raw.iter().all(u8::is_ascii_whitespace) {
            return Err(format!(
                "persisted terminal UI state at {} is empty; delete it and recreate current state",
                self.path.display()
            ));
        }
        serde_json::from_slice::<TerminalUiState>(&raw).map(sanitize_state).map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                format!("persisted terminal UI state at {} does not match this Agena build; delete it and recreate current state", self.path.display()),
                &error,
            )
        })
    }

    fn persist_to_disk(&self, state: &TerminalUiState) -> Result<(), String> {
        // Preserve existing file aliases: atomically replace their target.
        let destination = match std::fs::canonicalize(&self.path) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => self.path.clone(),
            Err(error) => return Err(format!("resolve terminal UI state: {error}")),
        };
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create terminal UI state dir: {error}"))?;
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|error| format!("serialize terminal UI state: {error}"))?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err("terminal UI state exceeds the 8 MiB limit".to_owned());
        }
        let permissions = match std::fs::metadata(&self.path) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.permissions().readonly() {
                    return Err("terminal UI state must be a writable regular file".to_owned());
                }
                Some(metadata.permissions())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("inspect terminal UI state: {error}")),
        };
        let mut staged = tempfile::Builder::new()
            .prefix(".terminal-ui-state-")
            .tempfile_in(parent)
            .map_err(|error| format!("stage terminal UI state: {error}"))?;
        if let Some(permissions) = permissions {
            staged
                .as_file()
                .set_permissions(permissions)
                .map_err(|error| format!("set terminal UI state permissions: {error}"))?;
        }
        staged
            .write_all(&bytes)
            .map_err(|error| format!("write terminal UI state: {error}"))?;
        staged
            .as_file()
            .sync_all()
            .map_err(|error| format!("sync terminal UI state: {error}"))?;
        staged
            .persist(&destination)
            .map_err(|error| format!("publish terminal UI state: {error}"))?;
        Ok(())
    }

    fn write_cache(&self, state: Arc<TerminalUiState>) {
        let retired = self
            .cache
            .lock()
            .expect("terminal state cache lock")
            .replace(state);
        drop(retired);
    }

    fn subscribe(&self) -> broadcast::Receiver<SequencedTerminalUiStateEvent> {
        self.tx.subscribe()
    }

    fn publish_state_replace(&self, state: &TerminalUiState) {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let payload = match serde_json::to_string(&serde_json::json!({
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
        })) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::error!(
                    seq,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "serialize a terminal UI state replacement event",
                        &error,
                    ),
                    "terminal UI state event was not published"
                );
                return;
            }
        };

        if self
            .tx
            .send(SequencedTerminalUiStateEvent {
                seq,
                payload: payload.into(),
            })
            .is_err()
        {
            tracing::debug!(seq, "terminal UI state event had no active subscribers");
        }
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
    project_state_dir(workspace_root)
        .join("server")
        .join(TERMINAL_UI_STATE_FILENAME)
}

fn project_state_dir(workspace_root: &Path) -> PathBuf {
    agena_home_dir()
        .join("projects")
        .join(workspace_key(workspace_root))
}

fn agena_home_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("agena")
}

fn workspace_key(workspace_root: &Path) -> String {
    let normalized = workspace_root.to_string_lossy().replace('\\', "/");
    sanitize_path(&normalized)
}

fn sanitize_path(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();

    while sanitized.contains("--") {
        sanitized = sanitized.replace("--", "-");
    }
    sanitized = sanitized.trim_matches('-').to_string();
    if sanitized.is_empty() {
        sanitized = "workspace".to_string();
    }
    if sanitized.len() <= MAX_WORKSPACE_KEY_LEN {
        return sanitized;
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = format!("{:x}", hasher.finish());
    format!("{}-{hash}", &sanitized[..MAX_WORKSPACE_KEY_LEN])
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

fn snapshot_payload(state: &TerminalUiState) -> Result<String, serde_json::Error> {
    serde_json::to_string(&serde_json::json!({
        "type": "terminal-ui-state.snapshot",
        "state": state,
    }))
}

async fn state_response(state: Arc<TerminalUiState>) -> Response {
    match STATE_READS
        .run(move || serde_json::to_vec(state.as_ref()))
        .await
    {
        Ok(Ok(bytes)) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            bytes,
        )
            .into_response(),
        result => {
            let error = match result {
                Ok(Err(error)) => error.to_string(),
                Err(error) => format!("terminal state response worker failed: {error}"),
                Ok(Ok(_)) => unreachable!(),
            };
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct TerminalUiStateEventsQuery {
    pub since: Option<String>,
}

pub(crate) async fn terminal_ui_state_get(State(state): State<Arc<crate::AppState>>) -> Response {
    let store = store_for_workspace(state.application.workspace_root());
    match store.read().await {
        Ok(snapshot) => state_response(snapshot).await,
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

pub(crate) async fn terminal_ui_state_put(
    State(state): State<Arc<crate::AppState>>,
    Json(body): Json<TerminalUiState>,
) -> Response {
    let store = store_for_workspace(state.application.workspace_root());
    match store.replace(body).await {
        Ok(next) => state_response(next).await,
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
    let store = store_for_workspace(state.application.workspace_root());
    let mut rx = store.subscribe();
    let current = match store.read().await {
        Ok(current) => current,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };
    let initial = match STATE_READS
        .run(move || snapshot_payload(&current))
        .await
        .map_err(|error| format!("terminal state snapshot worker failed: {error}"))
        .and_then(|result| result.map_err(|error| error.to_string()))
    {
        Ok(initial) => initial,
        Err(error) => {
            let diagnostic = format!("serialize the initial terminal UI state snapshot: {error}");
            tracing::error!(
                diagnostic = %diagnostic,
                "terminal UI state event stream could not be initialized"
            );
            let public = agena_failure::diagnostic::user_message_with_context(&diagnostic, 320);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": if public.is_empty() {
                        "The terminal state stream could not be initialized. Try again."
                    } else {
                        public.as_str()
                    }
                })),
            )
                .into_response();
        }
    };

    let sse_stream = stream! {
        yield Ok::<Event, Infallible>(Event::default().data(initial));

        loop {
            tokio::task::consume_budget().await;
            match rx.recv().await {
                Ok(event) => {
                    let large = event.payload.len() >= 64 * 1024;
                    let build = move || Event::default()
                            .id(event.seq.to_string())
                            .data(event.payload.as_ref());
                    let event = if large {
                        match STATE_READS.run(build).await {
                            Ok(event) => event,
                            Err(error) => {
                                tracing::error!(%error, "terminal state event worker failed");
                                break;
                            }
                        }
                    } else { build() };
                    yield Ok::<Event, Infallible>(event);
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        skipped_events = skipped,
                        "terminal UI state event stream lagged; closing it so the client reconnects with a fresh snapshot"
                    );
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::debug!("terminal UI state event publisher closed");
                    break;
                }
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
    use super::{TerminalUiState, TerminalUiStateStore};

    #[test]
    fn persisted_terminal_ui_state_requires_the_current_complete_shape() {
        let current = serde_json::json!({
            "version": 0,
            "updatedAt": 0,
            "activeSessionId": null,
            "sessionIds": [],
            "sessionMetaById": {},
            "folders": [{"id":"terminal-default","name":"Default"}]
        });
        serde_json::from_value::<TerminalUiState>(current.clone())
            .expect("current terminal UI state must decode");

        let mut missing = current.clone();
        missing.as_object_mut().unwrap().remove("version");
        assert!(serde_json::from_value::<TerminalUiState>(missing).is_err());

        let mut extra = current;
        extra["obsolete"] = serde_json::json!(true);
        assert!(serde_json::from_value::<TerminalUiState>(extra).is_err());
    }

    #[tokio::test]
    async fn incompatible_persisted_terminal_state_is_not_silently_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("terminal-ui-state.json");
        tokio::fs::write(&path, r#"{"sessionIds":[]}"#)
            .await
            .unwrap();
        let store = TerminalUiStateStore::new(path);
        let error = store
            .read()
            .await
            .expect_err("incompatible persisted state must fail");
        assert!(error.contains("does not match this Agena build"), "{error}");
    }
}
