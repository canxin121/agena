use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use dashmap::{DashMap, mapref::entry::Entry};
use portable_pty::{ChildKiller, CommandBuilder, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, RwLock, broadcast, watch};

const TERMINAL_SESSION_REGISTRY_FILENAME: &str = "sessions.json";
const TERMINAL_HISTORY_MAX_BYTES: usize = 512 * 1024;
const TERMINAL_INITIAL_SNAPSHOT_MAX_BYTES: usize = 32 * 1024;
const TERMINAL_HEARTBEAT: Duration = Duration::from_secs(15);
const MAX_TERMINAL_INPUT_BYTES: usize = 1024 * 1024;

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

struct RegistryMutation<T> {
    value: T,
    persist: bool,
}

impl<T> RegistryMutation<T> {
    fn persist(value: T) -> Self {
        Self {
            value,
            persist: true,
        }
    }

    fn skip(value: T) -> Self {
        Self {
            value,
            persist: false,
        }
    }
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

    async fn get(&self, session_id: &str) -> Option<PersistedTerminalSession> {
        let registry = self.read().await;
        registry.sessions.get(session_id).cloned()
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

    async fn mutate_registry<T, F>(&self, mutate: F) -> Result<T, String>
    where
        F: FnOnce(&mut PersistedTerminalRegistry) -> Result<RegistryMutation<T>, String>,
    {
        let _guard = self.write_lock.lock().await;
        let mut registry = self.load_from_disk().await;
        let mutation = mutate(&mut registry)?;
        if !mutation.persist {
            return Ok(mutation.value);
        }
        registry.version = registry.version.saturating_add(1);
        self.persist_to_disk(&registry).await?;
        self.write_cache(registry).await;
        Ok(mutation.value)
    }

    async fn upsert(
        &self,
        session_id: &str,
        cwd: &str,
        cols: u16,
        rows: u16,
        running: bool,
    ) -> Result<(), String> {
        self.mutate_registry(|registry| {
            let now = now_millis();
            let created_at = registry
                .sessions
                .get(session_id)
                .map(|entry| entry.created_at)
                .unwrap_or(now);
            registry.sessions.insert(
                session_id.to_string(),
                PersistedTerminalSession {
                    cwd: cwd.to_string(),
                    cols,
                    rows,
                    running,
                    created_at,
                    updated_at: now,
                },
            );
            Ok(RegistryMutation::persist(()))
        })
        .await
    }

    async fn remove(&self, session_id: &str) -> Result<bool, String> {
        self.mutate_registry(|registry| {
            let removed = registry.sessions.remove(session_id).is_some();
            Ok(if removed {
                RegistryMutation::persist(true)
            } else {
                RegistryMutation::skip(false)
            })
        })
        .await
    }

    async fn set_running(&self, session_id: &str, running: bool) -> Result<bool, String> {
        self.mutate_registry(|registry| {
            let Some(entry) = registry.sessions.get_mut(session_id) else {
                return Ok(RegistryMutation::skip(false));
            };
            if entry.running == running {
                return Ok(RegistryMutation::skip(true));
            }
            entry.running = running;
            entry.updated_at = now_millis();
            Ok(RegistryMutation::persist(true))
        })
        .await
    }

    async fn update_dimensions(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<bool, String> {
        self.mutate_registry(|registry| {
            let Some(entry) = registry.sessions.get_mut(session_id) else {
                return Ok(RegistryMutation::skip(false));
            };
            if entry.cols == cols && entry.rows == rows {
                return Ok(RegistryMutation::skip(true));
            }
            entry.cols = cols;
            entry.rows = rows;
            entry.updated_at = now_millis();
            Ok(RegistryMutation::persist(true))
        })
        .await
    }
}

#[derive(Debug)]
enum TerminalError {
    InvalidWorkingDirectory,
    NotFound,
    Spawn(String),
    Internal(String),
}

#[derive(Debug, Clone)]
enum TerminalEvent {
    Data { seq: u64, data: String },
    Exit { exit_code: Option<i32> },
}

#[derive(Debug, Default)]
struct TerminalHistory {
    chunks: VecDeque<(u64, String)>,
    bytes: usize,
}

pub(crate) struct TerminalSession {
    cwd: String,
    last_activity: Mutex<Instant>,
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    tx: broadcast::Sender<TerminalEvent>,
    exit_state: watch::Sender<bool>,
    seq: AtomicU64,
    history: Mutex<TerminalHistory>,
}

impl TerminalSession {
    fn spawn(cwd: String, cols: u16, rows: u16) -> Result<Arc<Self>, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;

        let shell = default_shell();
        let mut command = CommandBuilder::new(shell);
        command.cwd(&cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        let killer = child.clone_killer();
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;
        let master = pair.master;

        let (tx, _rx) = broadcast::channel::<TerminalEvent>(1024);
        let (exit_state, _exit_rx) = watch::channel(false);

        let session = Arc::new(Self {
            cwd,
            last_activity: Mutex::new(Instant::now()),
            master: Mutex::new(master),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            tx,
            exit_state,
            seq: AtomicU64::new(0),
            history: Mutex::new(TerminalHistory::default()),
        });

        Self::spawn_reader_task(session.clone(), reader);
        Self::spawn_wait_task(session.clone(), child);

        Ok(session)
    }

    fn spawn_reader_task(session: Arc<Self>, mut reader: Box<dyn Read + Send>) {
        tokio::task::spawn_blocking(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        *session.last_activity.lock().unwrap() = Instant::now();

                        let seq = session.seq.fetch_add(1, Ordering::Relaxed) + 1;
                        {
                            let mut history = session.history.lock().unwrap();
                            history.bytes += chunk.len();
                            history.chunks.push_back((seq, chunk.clone()));
                            while history.bytes > TERMINAL_HISTORY_MAX_BYTES {
                                if let Some((_seq, discarded)) = history.chunks.pop_front() {
                                    history.bytes = history.bytes.saturating_sub(discarded.len());
                                } else {
                                    break;
                                }
                            }
                        }

                        let _ = session.tx.send(TerminalEvent::Data { seq, data: chunk });
                    }
                    Err(_) => break,
                }
            }
        });
    }

    fn spawn_wait_task(session: Arc<Self>, mut child: Box<dyn portable_pty::Child + Send + Sync>) {
        tokio::task::spawn_blocking(move || {
            let exit_code = child.wait().ok().map(|status| status.exit_code() as i32);
            let _ = session.exit_state.send(true);
            let _ = session.tx.send(TerminalEvent::Exit { exit_code });
        });
    }

    fn subscribe(&self) -> broadcast::Receiver<TerminalEvent> {
        self.tx.subscribe()
    }

    fn subscribe_exit(&self) -> watch::Receiver<bool> {
        self.exit_state.subscribe()
    }

    fn snapshot_history_chunks(&self) -> Vec<(u64, String)> {
        let history = self.history.lock().unwrap();
        history.chunks.iter().cloned().collect()
    }

    fn write(&self, data: Bytes) -> Result<(), String> {
        *self.last_activity.lock().unwrap() = Instant::now();
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(&data).map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        *self.last_activity.lock().unwrap() = Instant::now();
        let master = self.master.lock().unwrap();
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())
    }

    fn kill(&self) {
        let _ = self.killer.lock().unwrap().kill();
    }
}

#[derive(Clone)]
struct TerminalManager {
    store: Arc<TerminalSessionStore>,
    sessions: Arc<DashMap<String, Arc<TerminalSession>>>,
    restore_lock: Arc<AsyncMutex<()>>,
}

impl TerminalManager {
    fn new(store: Arc<TerminalSessionStore>) -> Self {
        Self {
            store,
            sessions: Arc::new(DashMap::new()),
            restore_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    async fn create(
        &self,
        cwd: String,
        cols: u16,
        rows: u16,
    ) -> Result<TerminalCreateResponse, TerminalError> {
        validate_working_directory(cwd.as_str())
            .await
            .map_err(|_| TerminalError::InvalidWorkingDirectory)?;

        let session_id = loop {
            let candidate = crate::issue_token();
            if self.sessions.contains_key(&candidate) {
                continue;
            }
            if self.store.get(&candidate).await.is_none() {
                break candidate;
            }
        };

        let session =
            TerminalSession::spawn(cwd.clone(), cols, rows).map_err(TerminalError::Spawn)?;
        self.sessions.insert(session_id.clone(), session.clone());
        self.track_session_exit(session_id.clone(), session);
        self.store
            .upsert(&session_id, &cwd, cols, rows, true)
            .await
            .map_err(TerminalError::Internal)?;

        Ok(TerminalCreateResponse {
            session_id,
            cols,
            rows,
        })
    }

    async fn peek_info(&self, session_id: &str) -> Option<TerminalInfoResponse> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return None;
        }

        if let Some(session) = self.sessions.get(sid) {
            return Some(TerminalInfoResponse {
                session_id: sid.to_string(),
                cwd: session.cwd.clone(),
                running: true,
            });
        }

        let entry = self.store.get(sid).await?;
        Some(TerminalInfoResponse {
            session_id: sid.to_string(),
            cwd: entry.cwd,
            running: entry.running,
        })
    }

    async fn ensure_running(
        &self,
        session_id: &str,
    ) -> Result<Arc<TerminalSession>, TerminalError> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return Err(TerminalError::NotFound);
        }

        if let Some(session) = self.sessions.get(sid) {
            return Ok(session.value().clone());
        }

        let _guard = self.restore_lock.lock().await;
        if let Some(session) = self.sessions.get(sid) {
            return Ok(session.value().clone());
        }

        let entry = self.store.get(sid).await.ok_or(TerminalError::NotFound)?;
        validate_working_directory(entry.cwd.as_str())
            .await
            .map_err(|_| TerminalError::InvalidWorkingDirectory)?;

        let session = TerminalSession::spawn(entry.cwd.clone(), entry.cols, entry.rows)
            .map_err(TerminalError::Spawn)?;
        self.sessions.insert(sid.to_string(), session.clone());
        self.track_session_exit(sid.to_string(), session.clone());
        self.store
            .set_running(sid, true)
            .await
            .map_err(TerminalError::Internal)?;
        Ok(session)
    }

    async fn remember_dimensions(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), TerminalError> {
        self.store
            .update_dimensions(session_id, cols, rows)
            .await
            .map_err(TerminalError::Internal)?;
        Ok(())
    }

    async fn stop_session(&self, session_id: &str) -> Result<(), TerminalError> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return Err(TerminalError::NotFound);
        }

        if let Some((_, session)) = self.sessions.remove(sid) {
            session.kill();
            self.store
                .set_running(sid, false)
                .await
                .map_err(TerminalError::Internal)?;
            return Ok(());
        }

        if self.store.get(sid).await.is_some() {
            self.store
                .set_running(sid, false)
                .await
                .map_err(TerminalError::Internal)?;
            return Ok(());
        }

        Err(TerminalError::NotFound)
    }

    async fn kill_session(&self, session_id: &str) -> Result<(), TerminalError> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return Err(TerminalError::NotFound);
        }

        if let Some((_, session)) = self.sessions.remove(sid) {
            session.kill();
        }

        let removed = self
            .store
            .remove(sid)
            .await
            .map_err(TerminalError::Internal)?;
        if removed {
            Ok(())
        } else {
            Err(TerminalError::NotFound)
        }
    }

    fn track_session_exit(&self, session_id: String, session: Arc<TerminalSession>) {
        let manager = self.clone();
        let mut exit_rx = session.subscribe_exit();
        tokio::spawn(async move {
            if *exit_rx.borrow() {
                manager.mark_stopped(session_id.as_str()).await;
                return;
            }

            while exit_rx.changed().await.is_ok() {
                if *exit_rx.borrow() {
                    manager.mark_stopped(session_id.as_str()).await;
                    break;
                }
            }
        });
    }

    async fn mark_stopped(&self, session_id: &str) {
        self.sessions.remove(session_id);
        let _ = self.store.set_running(session_id, false).await;
    }
}

static TERMINAL_MANAGERS: LazyLock<DashMap<PathBuf, Arc<TerminalManager>>> =
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

fn manager_for_workspace(workspace_root: &Path) -> Arc<TerminalManager> {
    let path = workspace_terminal_session_registry_path(workspace_root);
    match TERMINAL_MANAGERS.entry(path.clone()) {
        Entry::Occupied(entry) => entry.get().clone(),
        Entry::Vacant(entry) => {
            let store = Arc::new(TerminalSessionStore::new(path));
            let manager = Arc::new(TerminalManager::new(store));
            entry.insert(manager.clone());
            manager
        }
    }
}

fn json_error(status: StatusCode, error: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": error.into() }))).into_response()
}

fn terminal_error_response(error: TerminalError) -> Response {
    match error {
        TerminalError::InvalidWorkingDirectory => {
            json_error(StatusCode::BAD_REQUEST, "Invalid working directory")
        }
        TerminalError::NotFound => json_error(StatusCode::NOT_FOUND, "Terminal session not found"),
        TerminalError::Spawn(message) | TerminalError::Internal(message) => {
            json_error(StatusCode::INTERNAL_SERVER_ERROR, message)
        }
    }
}

async fn validate_working_directory(raw: &str) -> Result<PathBuf, ()> {
    let path = tokio::fs::canonicalize(raw).await.map_err(|_| ())?;
    let metadata = tokio::fs::metadata(&path).await.map_err(|_| ())?;
    if metadata.is_dir() { Ok(path) } else { Err(()) }
}

fn default_shell() -> String {
    if cfg!(windows) {
        return "powershell.exe".to_string();
    }

    if let Ok(shell) = std::env::var("SHELL")
        && !shell.trim().is_empty()
    {
        return shell;
    }

    for candidate in ["/bin/bash", "/usr/bin/bash", "/bin/sh"] {
        if Path::new(candidate).is_file() {
            return candidate.to_string();
        }
    }

    "/bin/sh".to_string()
}

#[derive(Debug, Deserialize)]
pub(crate) struct TerminalCreateBody {
    pub cwd: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TerminalResizeBody {
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct TerminalStreamQuery {
    pub since: Option<String>,
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

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalResizeResponse {
    success: bool,
    cols: u16,
    rows: u16,
}

fn parse_seq(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok()
}

fn parse_last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_seq)
}

fn tail_slice_at_char_boundary(chunk: &str, keep_last_bytes: usize) -> &str {
    if chunk.len() <= keep_last_bytes {
        return chunk;
    }

    let mut start = chunk.len().saturating_sub(keep_last_bytes);
    while start < chunk.len() && !chunk.is_char_boundary(start) {
        start += 1;
    }
    &chunk[start..]
}

fn build_initial_tail_snapshot(
    chunks: &[(u64, String)],
    max_bytes: usize,
) -> Option<(u64, String)> {
    if max_bytes == 0 {
        return None;
    }

    let last_seq = chunks.last().map(|(seq, _)| *seq)?;
    let mut remaining = max_bytes;
    let mut parts = Vec::<&str>::new();

    for (_, chunk) in chunks.iter().rev() {
        if remaining == 0 {
            break;
        }
        if chunk.is_empty() {
            continue;
        }

        if chunk.len() <= remaining {
            parts.push(chunk.as_str());
            remaining -= chunk.len();
        } else {
            parts.push(tail_slice_at_char_boundary(chunk, remaining));
            remaining = 0;
        }
    }

    if parts.is_empty() {
        return None;
    }

    parts.reverse();
    let mut merged = String::with_capacity(max_bytes.saturating_sub(remaining));
    for part in parts {
        merged.push_str(part);
    }
    Some((last_seq, merged))
}

fn sse_json(payload: serde_json::Value, id: Option<u64>) -> Bytes {
    let mut output = String::new();
    if let Some(id) = id {
        output.push_str("id: ");
        output.push_str(id.to_string().as_str());
        output.push('\n');
    }
    output.push_str("data: ");
    output.push_str(payload.to_string().as_str());
    output.push_str("\n\n");
    Bytes::from(output)
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

    let resolved = match validate_working_directory(cwd).await {
        Ok(path) => path,
        Err(()) => return json_error(StatusCode::BAD_REQUEST, "Invalid working directory"),
    };

    let cols = body.cols.unwrap_or(80);
    let rows = body.rows.unwrap_or(24);
    let manager = manager_for_workspace(state.runtime.workspace_root());
    match manager
        .create(resolved.display().to_string(), cols, rows)
        .await
    {
        Ok(response) => Json(response).into_response(),
        Err(error) => terminal_error_response(error),
    }
}

pub(crate) async fn terminal_get(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Response {
    let manager = manager_for_workspace(state.runtime.workspace_root());
    match manager.peek_info(session_id.as_str()).await {
        Some(info) => Json(info).into_response(),
        None => json_error(StatusCode::NOT_FOUND, "Terminal session not found"),
    }
}

pub(crate) async fn terminal_delete(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Response {
    let manager = manager_for_workspace(state.runtime.workspace_root());
    match manager.kill_session(session_id.as_str()).await {
        Ok(()) => Json(TerminalSuccessResponse { success: true }).into_response(),
        Err(error) => terminal_error_response(error),
    }
}

pub(crate) async fn terminal_start(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Response {
    let manager = manager_for_workspace(state.runtime.workspace_root());
    match manager.ensure_running(session_id.as_str()).await {
        Ok(session) => Json(TerminalInfoResponse {
            session_id,
            cwd: session.cwd.clone(),
            running: true,
        })
        .into_response(),
        Err(error) => terminal_error_response(error),
    }
}

pub(crate) async fn terminal_stop(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Response {
    let manager = manager_for_workspace(state.runtime.workspace_root());
    match manager.stop_session(session_id.as_str()).await {
        Ok(()) => Json(TerminalSuccessResponse { success: true }).into_response(),
        Err(error) => terminal_error_response(error),
    }
}

pub(crate) async fn terminal_restart(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(old_session_id): AxumPath<String>,
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

    let resolved = match validate_working_directory(cwd).await {
        Ok(path) => path,
        Err(()) => return json_error(StatusCode::BAD_REQUEST, "Invalid working directory"),
    };

    let cols = body.cols.unwrap_or(80);
    let rows = body.rows.unwrap_or(24);
    let manager = manager_for_workspace(state.runtime.workspace_root());
    let _ = manager.kill_session(old_session_id.as_str()).await;

    match manager
        .create(resolved.display().to_string(), cols, rows)
        .await
    {
        Ok(response) => Json(response).into_response(),
        Err(error) => terminal_error_response(error),
    }
}

pub(crate) async fn terminal_input(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
    body: Body,
) -> Response {
    let manager = manager_for_workspace(state.runtime.workspace_root());
    let session = match manager.ensure_running(session_id.as_str()).await {
        Ok(session) => session,
        Err(error) => return terminal_error_response(error),
    };

    let bytes = match axum::body::to_bytes(body, MAX_TERMINAL_INPUT_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return json_error(StatusCode::PAYLOAD_TOO_LARGE, "Input too large"),
    };

    match session.write(bytes) {
        Ok(()) => Json(TerminalSuccessResponse { success: true }).into_response(),
        Err(error) => json_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(crate) async fn terminal_resize(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<TerminalResizeBody>,
) -> Response {
    let (Some(cols), Some(rows)) = (body.cols, body.rows) else {
        return json_error(StatusCode::BAD_REQUEST, "cols and rows are required");
    };

    let manager = manager_for_workspace(state.runtime.workspace_root());
    let session = match manager.ensure_running(session_id.as_str()).await {
        Ok(session) => session,
        Err(error) => return terminal_error_response(error),
    };

    if let Err(error) = session.resize(cols, rows) {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, error);
    }
    if let Err(error) = manager
        .remember_dimensions(session_id.as_str(), cols, rows)
        .await
    {
        return terminal_error_response(error);
    }

    Json(TerminalResizeResponse {
        success: true,
        cols,
        rows,
    })
    .into_response()
}

pub(crate) async fn terminal_stream(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<TerminalStreamQuery>,
    headers: HeaderMap,
) -> Response {
    let manager = manager_for_workspace(state.runtime.workspace_root());
    let session = match manager.ensure_running(session_id.as_str()).await {
        Ok(session) => session,
        Err(error) => return terminal_error_response(error),
    };

    *session.last_activity.lock().unwrap() = Instant::now();
    let mut rx = session.subscribe();
    let snapshot_chunks = session.snapshot_history_chunks();
    let snapshot_last_seq = snapshot_chunks.last().map(|(seq, _)| *seq).unwrap_or(0);
    let snapshot_first_seq = snapshot_chunks.first().map(|(seq, _)| *seq);

    let resume_since = query
        .since
        .as_deref()
        .and_then(parse_seq)
        .or_else(|| parse_last_event_id(&headers));

    let initial_tail_snapshot = if resume_since.is_none() {
        build_initial_tail_snapshot(&snapshot_chunks, TERMINAL_INITIAL_SNAPSHOT_MAX_BYTES)
    } else {
        None
    };

    let replay_from_seq = match (resume_since, snapshot_first_seq) {
        (Some(since), Some(first_seq)) => {
            let wanted = since.saturating_add(1);
            if wanted <= first_seq {
                first_seq
            } else if wanted > snapshot_last_seq {
                snapshot_last_seq.saturating_add(1)
            } else {
                wanted
            }
        }
        (Some(_), None) => snapshot_last_seq.saturating_add(1),
        (None, Some(_)) => snapshot_last_seq.saturating_add(1),
        (None, None) => snapshot_last_seq.saturating_add(1),
    };

    let needs_resync_notice = match (resume_since, snapshot_first_seq) {
        (Some(since), Some(first_seq)) => {
            let wanted = since.saturating_add(1);
            wanted < first_seq && since < snapshot_last_seq
        }
        _ => false,
    };

    let connected = sse_json(
        serde_json::json!({
            "type": "connected",
            "runtime": "rust",
            "ptyBackend": "portable-pty",
        }),
        None,
    );

    let start = tokio::time::Instant::now() + TERMINAL_HEARTBEAT;
    let mut ticker = tokio::time::interval_at(start, TERMINAL_HEARTBEAT);

    let stream = async_stream::stream! {
        yield Ok::<Bytes, std::convert::Infallible>(connected);

        if needs_resync_notice {
            yield Ok(sse_json(
                serde_json::json!({
                    "type": "resync",
                    "reason": "history_miss",
                    "since": resume_since,
                    "firstAvailableSeq": snapshot_first_seq,
                    "lastSeq": snapshot_last_seq,
                }),
                None,
            ));
        }

        if let Some((seq, data)) = initial_tail_snapshot.as_ref()
            && !data.is_empty()
        {
            yield Ok(sse_json(
                serde_json::json!({
                    "type": "data",
                    "seq": seq,
                    "data": data,
                }),
                Some(*seq),
            ));
        }

        for (seq, chunk) in snapshot_chunks.iter() {
            if *seq < replay_from_seq {
                continue;
            }
            yield Ok(sse_json(
                serde_json::json!({
                    "type": "data",
                    "seq": seq,
                    "data": chunk,
                }),
                Some(*seq),
            ));
        }

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    yield Ok(Bytes::from(": heartbeat\n\n"));
                }
                event = rx.recv() => {
                    match event {
                        Ok(TerminalEvent::Data { seq, data }) => {
                            if seq <= snapshot_last_seq {
                                continue;
                            }
                            yield Ok(sse_json(
                                serde_json::json!({
                                    "type": "data",
                                    "seq": seq,
                                    "data": data,
                                }),
                                Some(seq),
                            ));
                        }
                        Ok(TerminalEvent::Exit { exit_code }) => {
                            yield Ok(sse_json(
                                serde_json::json!({
                                    "type": "exit",
                                    "exitCode": exit_code,
                                }),
                                None,
                            ));
                            break;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    }
                }
            }
        }
    };

    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        "text/event-stream".parse().unwrap(),
    );
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        "no-cache".parse().unwrap(),
    );
    headers.insert(
        axum::http::header::CONNECTION,
        "keep-alive".parse().unwrap(),
    );
    headers.insert("X-Accel-Buffering", "no".parse().unwrap());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, header},
        routing::{get, post},
    };
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
            .route("/api/terminal/create", post(terminal_create))
            .route(
                "/api/terminal/{session_id}",
                get(terminal_get).delete(terminal_delete),
            )
            .route("/api/terminal/{session_id}/stream", get(terminal_stream))
            .route("/api/terminal/{session_id}/input", post(terminal_input))
            .route("/api/terminal/{session_id}/resize", post(terminal_resize))
            .route("/api/terminal/{session_id}/start", post(terminal_start))
            .route("/api/terminal/{session_id}/stop", post(terminal_stop))
            .route("/api/terminal/{session_id}/restart", post(terminal_restart))
            .with_state(state)
    }

    async fn next_event_chunk(body: &mut Body) -> String {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("stream frame should arrive")
            .expect("stream should yield frame")
            .expect("stream should not error");
        let data = frame.into_data().expect("frame should contain data");
        String::from_utf8(data.to_vec()).expect("frame should be utf8")
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
        assert!(persisted.running);

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
                running: true,
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

    #[tokio::test]
    async fn terminal_runtime_routes_stream_resize_stop_start_and_restart() {
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
                            "cols": 100,
                            "rows": 24
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(create_response.status(), StatusCode::OK);
        let created: TerminalCreateResponse = serde_json::from_slice(
            &create_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");

        let stream_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/terminal/{}/stream", created.session_id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(stream_response.status(), StatusCode::OK);
        assert_eq!(
            stream_response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
        let mut stream_body = stream_response.into_body();
        let connected_chunk = next_event_chunk(&mut stream_body).await;
        assert!(connected_chunk.contains("\"type\":\"connected\""));

        let input_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/terminal/{}/input", created.session_id))
                    .header("content-type", "text/plain")
                    .body(Body::from("echo hello-terminal\n"))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(input_response.status(), StatusCode::OK);

        let resize_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/terminal/{}/resize", created.session_id))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "cols": 140, "rows": 36 }).to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(resize_response.status(), StatusCode::OK);
        let resized: TerminalResizeResponse = serde_json::from_slice(
            &resize_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            resized,
            TerminalResizeResponse {
                success: true,
                cols: 140,
                rows: 36,
            }
        );

        let mut saw_output = false;
        for _ in 0..20 {
            let chunk = next_event_chunk(&mut stream_body).await;
            if chunk.contains("hello-terminal") {
                saw_output = true;
                break;
            }
        }
        assert!(saw_output, "stream never contained echoed terminal output");

        let stop_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/terminal/{}/stop", created.session_id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(stop_response.status(), StatusCode::OK);

        let stopped_info = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/terminal/{}", created.session_id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let stopped: TerminalInfoResponse = serde_json::from_slice(
            &stopped_info
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(!stopped.running);

        let start_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/terminal/{}/start", created.session_id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(start_response.status(), StatusCode::OK);
        let started: TerminalInfoResponse = serde_json::from_slice(
            &start_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(started.running);

        let restart_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/terminal/{}/restart", created.session_id))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "cwd": temp.path(),
                            "cols": 90,
                            "rows": 20
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(restart_response.status(), StatusCode::OK);
        let restarted: TerminalCreateResponse = serde_json::from_slice(
            &restart_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_ne!(restarted.session_id, created.session_id);
        assert_eq!(restarted.cols, 90);
        assert_eq!(restarted.rows, 20);

        let delete_response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/terminal/{}", restarted.session_id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(delete_response.status(), StatusCode::OK);
    }
}
