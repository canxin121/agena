use std::{
    collections::{BTreeMap, VecDeque},
    error::Error as _,
    io::{Read, Write},
    path::Path,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use portable_atomic::AtomicU64;
use std::sync::atomic::Ordering;

use axum::{
    Json,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use bytes::Bytes;
use dashmap::DashMap;
use portable_pty::ChildKiller;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Semaphore, broadcast, watch};

use crate::{ApiResult, AppError};

use crate::server::persistence::db;

const MAX_TERMINAL_SESSIONS: usize = 20;
const TERMINAL_IDLE_TIMEOUT_ENV: &str = "AGENA_SERVER_TERMINAL_IDLE_TIMEOUT_SECS";
const TERMINAL_CLEANUP_INTERVAL: Duration = Duration::from_secs(5 * 60);
const TERMINAL_HEARTBEAT: Duration = Duration::from_secs(15);
const TERMINAL_SESSION_FILE_VERSION: u64 = 1;
const TMUX_SESSION_PREFIX: &str = "agena-";

// Keep a bounded recent scrollback for resumable streams.
const TERMINAL_HISTORY_MAX_BYTES: usize = 512 * 1024;

// For a fresh stream connection (no `since` cursor), only paint a compact
// tail snapshot to avoid replaying long historical output that visibly races
// upward before settling at the current prompt/screen.
const TERMINAL_INITIAL_SNAPSHOT_MAX_BYTES: usize = 32 * 1024;

fn recover_terminal_mutex<'a, T>(mutex: &'a Mutex<T>, name: &str) -> MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(error) => {
            tracing::error!(
                lock = name,
                diagnostic = %error,
                "terminal state lock was poisoned; recovering its inner state"
            );
            error.into_inner()
        }
    }
}

fn body_error_contains_length_limit(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(cause) = current {
        if cause.is::<http_body_util::LengthLimitError>() {
            return true;
        }
        current = cause.source();
    }
    false
}

#[derive(Debug, Error)]
pub enum TerminalError {
    #[error("Maximum terminal sessions reached")]
    LimitReached,
    #[error("Invalid working directory")]
    InvalidWorkingDirectory,
    #[error("Failed to inspect the terminal working directory")]
    WorkingDirectory(#[source] std::io::Error),
    #[error("Terminal session not found")]
    NotFound,
    #[error("Failed to create terminal session")]
    Spawn(#[source] anyhow::Error),
    #[error("Failed to stop terminal session")]
    Kill(#[source] anyhow::Error),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
enum PersistedTerminalBackend {
    #[default]
    Shell,
    Tmux,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedTerminalSession {
    cwd: String,
    cols: u16,
    rows: u16,
    backend: PersistedTerminalBackend,
    updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedTerminalRegistry {
    #[serde(default)]
    version: u64,
    #[serde(default)]
    sessions: BTreeMap<String, PersistedTerminalSession>,
}

impl Default for PersistedTerminalRegistry {
    fn default() -> Self {
        Self {
            version: TERMINAL_SESSION_FILE_VERSION,
            sessions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum TerminalBackend {
    Shell,
    Tmux,
}

impl TerminalBackend {
    fn to_persisted(self) -> PersistedTerminalBackend {
        match self {
            Self::Shell => PersistedTerminalBackend::Shell,
            Self::Tmux => PersistedTerminalBackend::Tmux,
        }
    }
}

impl From<PersistedTerminalBackend> for TerminalBackend {
    fn from(value: PersistedTerminalBackend) -> Self {
        match value {
            PersistedTerminalBackend::Shell => Self::Shell,
            PersistedTerminalBackend::Tmux => Self::Tmux,
        }
    }
}

static TMUX_AVAILABLE: LazyLock<bool> =
    LazyLock::new(
        || match std::process::Command::new("tmux").arg("-V").output() {
            Ok(output) => output.status.success(),
            Err(error) => {
                tracing::debug!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "probe tmux availability",
                        &error,
                    ),
                    "tmux is unavailable"
                );
                false
            }
        },
    );

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn terminal_idle_timeout() -> Option<Duration> {
    let raw = match std::env::var(TERMINAL_IDLE_TIMEOUT_ENV) {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => return None,
        Err(error) => {
            tracing::warn!(
                variable = TERMINAL_IDLE_TIMEOUT_ENV,
                diagnostic = %error,
                "terminal idle timeout environment variable could not be decoded"
            );
            return None;
        }
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let Ok(secs) = trimmed.parse::<u64>() else {
        tracing::warn!(
            terminal_idle_timeout_secs = trimmed,
            "invalid terminal idle timeout value; disabling timeout"
        );
        return None;
    };

    if secs == 0 {
        return None;
    }

    Some(Duration::from_secs(secs))
}

fn tmux_session_name(session_id: &str) -> String {
    format!("{TMUX_SESSION_PREFIX}{session_id}")
}

fn tmux_has_session(session_name: &str) -> bool {
    match std::process::Command::new("tmux")
        .arg("has-session")
        .arg("-t")
        .arg(session_name)
        .output()
    {
        Ok(output) => output.status.success(),
        Err(error) => {
            tracing::warn!(
                tmux_session = session_name,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "query a tmux terminal session",
                    &error,
                ),
                "tmux terminal session availability could not be determined"
            );
            false
        }
    }
}

fn tmux_kill_session(session_name: &str) -> anyhow::Result<()> {
    let output = std::process::Command::new("tmux")
        .arg("kill-session")
        .arg("-t")
        .arg(session_name)
        .output()
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to execute tmux while stopping terminal session `{session_name}`: {}",
                agena_failure::diagnostic::format_error_chain(&error)
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "tmux could not stop terminal session `{session_name}` (status {}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn normalize_persisted_registry(
    mut registry: PersistedTerminalRegistry,
) -> PersistedTerminalRegistry {
    if registry.version == 0 {
        registry.version = TERMINAL_SESSION_FILE_VERSION;
    }
    registry
}

async fn load_session_registry_from_store(
    db: &crate::server::persistence::db::ServerStateDb,
) -> PersistedTerminalRegistry {
    match db
        .get_json::<PersistedTerminalRegistry>(
            crate::server::persistence::db::KV_KEY_TERMINAL_SESSION_REGISTRY,
        )
        .await
    {
        Ok(Some(registry)) => return normalize_persisted_registry(registry),
        Ok(None) => {}
        Err(error) => {
            tracing::error!(
                diagnostic = %error,
                "failed to load the persisted terminal session registry; starting with an empty registry"
            );
        }
    }

    let registry = PersistedTerminalRegistry::default();
    if let Err(error) = db
        .set_json(
            crate::server::persistence::db::KV_KEY_TERMINAL_SESSION_REGISTRY,
            &registry,
        )
        .await
    {
        tracing::error!(
            diagnostic = %error,
            "failed to initialize the persisted terminal session registry"
        );
    }
    registry
}

const TERMINAL_REGISTRY_FLUSH_DEBOUNCE: Duration = Duration::from_millis(180);
const TERMINAL_REGISTRY_FLUSH_RETRY_DELAY: Duration = Duration::from_millis(1200);

#[derive(Debug, Default)]
struct TerminalRegistryFlushQueue {
    pending: Option<PersistedTerminalRegistry>,
    worker_running: bool,
}

#[derive(Clone)]
pub struct TerminalManager {
    db: Arc<crate::server::persistence::db::ServerStateDb>,
    sessions: Arc<DashMap<String, Arc<TerminalSession>>>,
    session_registry: Arc<Mutex<PersistedTerminalRegistry>>,
    registry_flush_queue: Arc<Mutex<TerminalRegistryFlushQueue>>,
    restore_lock: Arc<Mutex<()>>,
    idle_timeout: Option<Duration>,
    prefer_tmux: bool,
}

impl TerminalManager {
    pub async fn new(db: Arc<crate::server::persistence::db::ServerStateDb>) -> Self {
        let session_registry = load_session_registry_from_store(db.as_ref()).await;
        let prefer_tmux = *TMUX_AVAILABLE;
        let idle_timeout = terminal_idle_timeout();

        if prefer_tmux {
            tracing::info!("terminal persistence backend: tmux");
        } else {
            tracing::warn!(
                "tmux not found; terminal sessions will be restored as fresh shells after restart"
            );
        }

        Self {
            db,
            sessions: Arc::new(DashMap::new()),
            session_registry: Arc::new(Mutex::new(session_registry)),
            registry_flush_queue: Arc::new(Mutex::new(TerminalRegistryFlushQueue::default())),
            restore_lock: Arc::new(Mutex::new(())),
            idle_timeout,
            prefer_tmux,
        }
    }

    fn queue_registry_flush(&self, registry: PersistedTerminalRegistry) {
        let mut should_spawn = false;
        let mut queue = self.registry_flush_queue.lock().unwrap_or_else(|error| {
            tracing::error!(
                diagnostic = %error,
                "terminal registry flush queue lock is poisoned; recovering queued persistence state"
            );
            error.into_inner()
        });
        queue.pending = Some(registry);
        if !queue.worker_running {
            queue.worker_running = true;
            should_spawn = true;
        }
        drop(queue);

        if !should_spawn {
            return;
        }

        let db = self.db.clone();
        let queue = self.registry_flush_queue.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(TERMINAL_REGISTRY_FLUSH_DEBOUNCE).await;

                let pending = queue
                    .lock()
                    .unwrap_or_else(|error| {
                        tracing::error!(
                            diagnostic = %error,
                            "terminal registry flush queue lock is poisoned; recovering pending persistence state"
                        );
                        error.into_inner()
                    })
                    .pending
                    .take();

                let Some(candidate) = pending else {
                    let mut q = queue.lock().unwrap_or_else(|error| {
                        tracing::error!(
                            diagnostic = %error,
                            "terminal registry flush queue lock is poisoned while stopping the worker; recovering state"
                        );
                        error.into_inner()
                    });
                    if q.pending.is_none() {
                        q.worker_running = false;
                        break;
                    }
                    continue;
                };

                if let Err(error) = db
                    .set_json(
                        crate::server::persistence::db::KV_KEY_TERMINAL_SESSION_REGISTRY,
                        &candidate,
                    )
                    .await
                {
                    tracing::warn!(
                        target: "agena.terminal",
                        error = %error,
                        "failed to persist terminal session registry; will retry"
                    );

                    {
                        let mut q = queue.lock().unwrap_or_else(|lock_error| {
                            tracing::error!(
                                diagnostic = %lock_error,
                                "terminal registry flush queue lock is poisoned while scheduling a retry; recovering state"
                            );
                            lock_error.into_inner()
                        });
                        if q.pending.is_none() {
                            q.pending = Some(candidate);
                        }
                    }

                    tokio::time::sleep(TERMINAL_REGISTRY_FLUSH_RETRY_DELAY).await;
                    continue;
                }
            }
        });
    }

    fn persist_registry_with<F>(&self, mutator: F) -> bool
    where
        F: FnOnce(&mut PersistedTerminalRegistry) -> bool,
    {
        let snapshot = {
            let mut guard = self.session_registry.lock().unwrap_or_else(|error| {
                tracing::error!(
                    diagnostic = %error,
                    "terminal session registry lock is poisoned; recovering persisted state"
                );
                error.into_inner()
            });
            if !mutator(&mut guard) {
                return false;
            }
            if guard.version == 0 {
                guard.version = TERMINAL_SESSION_FILE_VERSION;
            }
            guard.clone()
        };

        self.queue_registry_flush(snapshot);

        true
    }

    fn persisted_session(&self, session_id: &str) -> Option<PersistedTerminalSession> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return None;
        }
        let guard = recover_terminal_mutex(&self.session_registry, "persisted session registry");
        guard.sessions.get(sid).cloned()
    }

    fn upsert_persisted_session(
        &self,
        session_id: &str,
        cwd: &str,
        cols: u16,
        rows: u16,
        backend: PersistedTerminalBackend,
    ) {
        let sid = session_id.trim();
        if sid.is_empty() {
            return;
        }

        let sid = sid.to_string();
        let cwd = cwd.to_string();
        self.persist_registry_with(move |registry| {
            let next = PersistedTerminalSession {
                cwd,
                cols,
                rows,
                backend,
                updated_at: now_millis(),
            };

            match registry.sessions.get(&sid) {
                Some(current)
                    if current.cwd == next.cwd
                        && current.cols == next.cols
                        && current.rows == next.rows
                        && current.backend == next.backend =>
                {
                    false
                }
                _ => {
                    registry.sessions.insert(sid.clone(), next);
                    true
                }
            }
        });
    }

    fn remove_persisted_session(&self, session_id: &str) -> bool {
        let sid = session_id.trim();
        if sid.is_empty() {
            return false;
        }

        let sid = sid.to_string();
        self.persist_registry_with(move |registry| registry.sessions.remove(&sid).is_some())
    }

    fn handle_session_exit(&self, session_id: &str, session: &TerminalSession) {
        self.sessions.remove(session_id);
        if session.keep_persisted_entry_after_exit() {
            return;
        }
        self.remove_persisted_session(session_id);
    }

    fn track_session_lifecycle(&self, session_id: String, session: Arc<TerminalSession>) {
        let manager = self.clone();
        let mut exit_rx = session.subscribe_exit();
        tokio::spawn(async move {
            if *exit_rx.borrow() {
                manager.handle_session_exit(&session_id, session.as_ref());
                return;
            }

            while exit_rx.changed().await.is_ok() {
                if *exit_rx.borrow() {
                    manager.handle_session_exit(&session_id, session.as_ref());
                    break;
                }
            }
        });
    }

    fn try_restore_session(&self, session_id: &str) -> Option<Arc<TerminalSession>> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return None;
        }

        if self.sessions.len() >= MAX_TERMINAL_SESSIONS {
            return None;
        }

        let _restore_guard = recover_terminal_mutex(&self.restore_lock, "session restore");

        if let Some(existing) = self.sessions.get(sid) {
            return Some(existing.value().clone());
        }

        let persisted = self.persisted_session(sid)?;
        let cwd = persisted.cwd.clone();

        let cwd_meta = match std::fs::metadata(&cwd) {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(
                    session_id = sid,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "inspect a persisted terminal working directory during restore",
                        &error,
                    ),
                    "persisted terminal session could not be restored and will be removed"
                );
                self.remove_persisted_session(sid);
                return None;
            }
        };
        if !cwd_meta.is_dir() {
            self.remove_persisted_session(sid);
            return None;
        }

        let session = match TerminalSession::spawn(
            sid.to_string(),
            cwd.clone(),
            persisted.cols,
            persisted.rows,
            self.prefer_tmux,
        ) {
            Ok(session) => session,
            Err(error) => {
                tracing::warn!(session_id = sid, error = %error, "failed to restore terminal session");
                return None;
            }
        };

        self.sessions.insert(sid.to_string(), session.clone());
        self.track_session_lifecycle(sid.to_string(), session.clone());
        self.upsert_persisted_session(
            sid,
            &cwd,
            persisted.cols,
            persisted.rows,
            session.backend().to_persisted(),
        );
        Some(session)
    }

    pub fn spawn_cleanup_task(self: Arc<Self>) {
        let Some(idle_timeout) = self.idle_timeout else {
            tracing::info!("terminal idle timeout disabled");
            return;
        };

        let manager = Arc::downgrade(&self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(TERMINAL_CLEANUP_INTERVAL);
            loop {
                ticker.tick().await;
                let Some(manager) = manager.upgrade() else {
                    break;
                };
                let now = Instant::now();
                let mut to_remove = Vec::new();
                for entry in manager.sessions.iter() {
                    let idle = {
                        let last = recover_terminal_mutex(
                            &entry.value().last_activity,
                            "session last activity",
                        );
                        now.duration_since(*last)
                    };
                    if idle > idle_timeout {
                        to_remove.push(entry.key().clone());
                    }
                }

                for id in to_remove {
                    if let Some((_, session)) = manager.sessions.remove(&id) {
                        tracing::info!("Cleaning up idle terminal session: {}", id);
                        if let Err(error) = session.kill() {
                            tracing::error!(
                                session_id = id,
                                diagnostic = %agena_failure::diagnostic::format_error_chain(error.as_ref()),
                                "failed to stop an idle terminal session"
                            );
                        }
                        manager.remove_persisted_session(&id);
                    }
                }
            }
        });
    }

    pub fn get(&self, session_id: &str) -> Option<Arc<TerminalSession>> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return None;
        }

        if let Some(existing) = self.sessions.get(sid) {
            return Some(existing.value().clone());
        }

        self.try_restore_session(sid)
    }

    pub fn peek_info(&self, session_id: &str) -> Option<(String, bool)> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return None;
        }

        if let Some(existing) = self.sessions.get(sid) {
            return Some((existing.value().cwd.clone(), true));
        }

        let persisted = self.persisted_session(sid)?;
        match std::fs::metadata(&persisted.cwd) {
            Ok(meta) if !meta.is_dir() => {
                self.remove_persisted_session(sid);
                return None;
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    session_id = sid,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "inspect a persisted terminal working directory for session metadata",
                        &error,
                    ),
                    "persisted terminal metadata is unavailable and will be removed"
                );
                self.remove_persisted_session(sid);
                return None;
            }
        }

        Some((persisted.cwd, false))
    }

    pub async fn create(
        &self,
        cwd: String,
        cols: u16,
        rows: u16,
    ) -> Result<TerminalCreateResponse, TerminalError> {
        if self.sessions.len() >= MAX_TERMINAL_SESSIONS {
            return Err(TerminalError::LimitReached);
        }

        let meta = tokio::fs::metadata(&cwd)
            .await
            .map_err(TerminalError::WorkingDirectory)?;
        if !meta.is_dir() {
            return Err(TerminalError::InvalidWorkingDirectory);
        }

        let session_id = crate::server::issue_token();

        let session = TerminalSession::spawn(
            session_id.clone(),
            cwd.clone(),
            cols,
            rows,
            self.prefer_tmux,
        )
        .map_err(TerminalError::Spawn)?;

        let backend = session.backend().to_persisted();

        self.sessions.insert(session_id.clone(), session.clone());
        self.track_session_lifecycle(session_id.clone(), session);
        self.upsert_persisted_session(&session_id, &cwd, cols, rows, backend);

        Ok(TerminalCreateResponse {
            session_id,
            cols,
            rows,
        })
    }

    pub fn remember_dimensions(&self, session_id: &str, cols: u16, rows: u16) {
        let sid = session_id.trim();
        if sid.is_empty() {
            return;
        }

        let sid = sid.to_string();
        self.persist_registry_with(move |registry| {
            let Some(entry) = registry.sessions.get_mut(&sid) else {
                return false;
            };

            if entry.cols == cols && entry.rows == rows {
                return false;
            }

            entry.cols = cols;
            entry.rows = rows;
            entry.updated_at = now_millis();
            true
        });
    }

    pub fn kill_session(&self, session_id: &str) -> Result<(), TerminalError> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return Err(TerminalError::NotFound);
        }

        if let Some((_, session)) = self.sessions.remove(sid) {
            session.kill().map_err(TerminalError::Kill)?;
            self.remove_persisted_session(sid);
            return Ok(());
        }

        let Some(persisted) = self.persisted_session(sid) else {
            return Err(TerminalError::NotFound);
        };

        if persisted.backend == PersistedTerminalBackend::Tmux {
            tmux_kill_session(&tmux_session_name(sid)).map_err(TerminalError::Kill)?;
        }

        self.remove_persisted_session(sid);
        Ok(())
    }

    pub fn stop_session(&self, session_id: &str) -> Result<(), TerminalError> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return Err(TerminalError::NotFound);
        }

        if let Some((_, session)) = self.sessions.remove(sid) {
            session.stop_runtime().map_err(TerminalError::Kill)?;
            return Ok(());
        }

        // If we still have a persisted entry, treat stop as a no-op.
        if self.persisted_session(sid).is_some() {
            return Ok(());
        }

        Err(TerminalError::NotFound)
    }
}

pub struct TerminalSession {
    pub cwd: String,
    pub last_activity: Mutex<Instant>,
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    blocking_io: Arc<Semaphore>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    tx: broadcast::Sender<TerminalEvent>,
    exit_state: watch::Sender<bool>,
    backend: TerminalBackend,
    tmux_session_name: Option<String>,

    // Keep a bounded history of recent output for new subscribers.
    seq: AtomicU64,
    history: Mutex<TerminalHistory>,
}

#[derive(Debug, Clone)]
enum TerminalEvent {
    Data {
        seq: u64,
        data: String,
    },
    Exit {
        exit_code: Option<i32>,
        signal: Option<i32>,
    },
}

#[derive(Debug, Default)]
struct TerminalHistory {
    chunks: VecDeque<(u64, String)>,
    bytes: usize,
}

impl TerminalSession {
    fn spawn(
        session_id: String,
        cwd: String,
        cols: u16,
        rows: u16,
        prefer_tmux: bool,
    ) -> Result<Arc<Self>, anyhow::Error> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut backend = TerminalBackend::Shell;
        let mut tmux_session_name_value = None;

        let child = if prefer_tmux {
            let tmux_name = tmux_session_name(&session_id);
            let mut tmux_cmd = CommandBuilder::new("tmux");
            tmux_cmd.arg("new-session");
            tmux_cmd.arg("-A");
            tmux_cmd.arg("-s");
            tmux_cmd.arg(&tmux_name);
            tmux_cmd.arg("-c");
            tmux_cmd.arg(&cwd);
            tmux_cmd.cwd(&cwd);
            tmux_cmd.env("TERM", "xterm-256color");
            tmux_cmd.env("COLORTERM", "truecolor");

            match pair.slave.spawn_command(tmux_cmd) {
                Ok(child) => {
                    backend = TerminalBackend::Tmux;
                    tmux_session_name_value = Some(tmux_name);
                    child
                }
                Err(error) => {
                    tracing::warn!(
                        session_id,
                        error = %error,
                        "failed to spawn tmux-backed terminal; falling back to shell"
                    );

                    let shell = default_shell();
                    let mut cmd = CommandBuilder::new(shell);
                    cmd.cwd(&cwd);
                    cmd.env("TERM", "xterm-256color");
                    cmd.env("COLORTERM", "truecolor");
                    pair.slave.spawn_command(cmd)?
                }
            }
        } else {
            let shell = default_shell();
            let mut cmd = CommandBuilder::new(shell);
            cmd.cwd(&cwd);

            // Parity-ish environment.
            cmd.env("TERM", "xterm-256color");
            cmd.env("COLORTERM", "truecolor");
            pair.slave.spawn_command(cmd)?
        };

        let killer = child.clone_killer();
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let master = pair.master;

        let (tx, _rx) = broadcast::channel::<TerminalEvent>(1024);
        let (exit_state, _exit_state_rx) = watch::channel(false);

        let session = Arc::new(Self {
            cwd,
            last_activity: Mutex::new(Instant::now()),
            master: Mutex::new(master),
            writer: Mutex::new(writer),
            blocking_io: Arc::new(Semaphore::new(1)),
            killer: Mutex::new(killer),
            tx,
            exit_state,
            backend,
            tmux_session_name: tmux_session_name_value,

            seq: AtomicU64::new(0),
            history: Mutex::new(TerminalHistory::default()),
        });

        Self::spawn_reader_task(session.clone(), reader);
        Self::spawn_wait_task(session.clone(), child);

        Ok(session)
    }

    fn backend(&self) -> TerminalBackend {
        self.backend
    }

    fn keep_persisted_entry_after_exit(&self) -> bool {
        match self.backend {
            // Shell sessions cannot survive a backend restart, but keep the
            // registry entry so we can transparently restore a new shell.
            TerminalBackend::Shell => true,
            TerminalBackend::Tmux => {
                let Some(name) = self.tmux_session_name.as_deref() else {
                    return false;
                };

                tmux_has_session(name)
            }
        }
    }

    fn spawn_reader_task(session: Arc<Self>, mut reader: Box<dyn Read + Send>) {
        tokio::task::spawn_blocking(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        *recover_terminal_mutex(&session.last_activity, "session last activity") =
                            Instant::now();

                        let seq = session.seq.fetch_add(1, Ordering::Relaxed) + 1;
                        {
                            let mut hist =
                                recover_terminal_mutex(&session.history, "session history");
                            hist.bytes += chunk.len();
                            hist.chunks.push_back((seq, chunk.clone()));
                            while hist.bytes > TERMINAL_HISTORY_MAX_BYTES {
                                if let Some((_s, old)) = hist.chunks.pop_front() {
                                    hist.bytes = hist.bytes.saturating_sub(old.len());
                                } else {
                                    break;
                                }
                            }
                        }

                        if session
                            .tx
                            .send(TerminalEvent::Data { seq, data: chunk })
                            .is_err()
                        {
                            tracing::debug!(
                                terminal_cwd = %session.cwd,
                                "terminal output had no active event subscribers; it remains available in history"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::error!(
                            terminal_cwd = %session.cwd,
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to read from the terminal PTY",
                                &error,
                            ),
                            "terminal PTY reader stopped after an I/O failure"
                        );
                        break;
                    }
                }
            }
        });
    }

    fn snapshot_history_chunks(&self) -> Vec<(u64, String)> {
        let hist = recover_terminal_mutex(&self.history, "session history");
        hist.chunks.iter().cloned().collect()
    }

    fn spawn_wait_task(session: Arc<Self>, mut child: Box<dyn portable_pty::Child + Send + Sync>) {
        tokio::task::spawn_blocking(move || {
            // Wait for the child process to exit.
            let status = child.wait();
            let (exit_code, signal) = match status {
                Ok(status) => {
                    // portable-pty exposes a numeric exit code but doesn't reliably expose
                    // signal details across platforms; keep it null.
                    (Some(status.exit_code() as i32), None)
                }
                Err(error) => {
                    tracing::error!(
                        terminal_cwd = %session.cwd,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "failed to wait for the terminal child process",
                            &error,
                        ),
                        "terminal child wait failed"
                    );
                    (None, None)
                }
            };
            if session.exit_state.send(true).is_err() {
                tracing::debug!(
                    terminal_cwd = %session.cwd,
                    "terminal exit state had no active receivers"
                );
            }
            if session
                .tx
                .send(TerminalEvent::Exit { exit_code, signal })
                .is_err()
            {
                tracing::debug!(
                    terminal_cwd = %session.cwd,
                    "terminal exit event had no active subscribers"
                );
            }
        });
    }

    pub async fn write(self: &Arc<Self>, data: Bytes) -> Result<(), anyhow::Error> {
        let permit = Arc::clone(&self.blocking_io)
            .acquire_owned()
            .await
            .map_err(|error| {
                anyhow::anyhow!(agena_failure::diagnostic::format_error_chain_with_context(
                    "acquire terminal write worker",
                    &error,
                ))
            })?;
        let session = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            session.write_blocking(data)
        })
        .await
        .map_err(|error| {
            anyhow::anyhow!(agena_failure::diagnostic::format_error_chain_with_context(
                "terminal write worker failed",
                &error,
            ))
        })?
    }

    fn write_blocking(&self, data: Bytes) -> Result<(), anyhow::Error> {
        *recover_terminal_mutex(&self.last_activity, "session last activity") = Instant::now();
        let mut writer = recover_terminal_mutex(&self.writer, "PTY writer");
        writer.write_all(&data)?;
        writer.flush()?;
        Ok(())
    }

    pub async fn resize(self: &Arc<Self>, cols: u16, rows: u16) -> Result<(), anyhow::Error> {
        let permit = Arc::clone(&self.blocking_io)
            .acquire_owned()
            .await
            .map_err(|error| {
                anyhow::anyhow!(agena_failure::diagnostic::format_error_chain_with_context(
                    "acquire terminal resize worker",
                    &error,
                ))
            })?;
        let session = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            session.resize_blocking(cols, rows)
        })
        .await
        .map_err(|error| {
            anyhow::anyhow!(agena_failure::diagnostic::format_error_chain_with_context(
                "terminal resize worker failed",
                &error,
            ))
        })?
    }

    fn resize_blocking(&self, cols: u16, rows: u16) -> Result<(), anyhow::Error> {
        *recover_terminal_mutex(&self.last_activity, "session last activity") = Instant::now();
        let master = recover_terminal_mutex(&self.master, "PTY master");
        master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    pub fn kill(&self) -> Result<(), anyhow::Error> {
        if let Some(name) = self.tmux_session_name.as_deref() {
            tmux_kill_session(name)?;
        }

        recover_terminal_mutex(&self.killer, "PTY child killer")
            .kill()
            .map_err(anyhow::Error::from)?;
        Ok(())
    }

    pub fn stop_runtime(&self) -> Result<(), anyhow::Error> {
        // Stop the attached runtime process. For tmux-backed sessions this terminates the
        // client while leaving the tmux session alive.
        recover_terminal_mutex(&self.killer, "PTY child killer")
            .kill()
            .map_err(anyhow::Error::from)?;
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<TerminalEvent> {
        self.tx.subscribe()
    }

    fn subscribe_exit(&self) -> watch::Receiver<bool> {
        self.exit_state.subscribe()
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct TerminalStreamQuery {
    pub since: Option<String>,
}

fn parse_seq(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok()
}

fn parse_last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
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
    let mut parts: Vec<&str> = Vec::new();

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
    let mut out = String::new();
    if let Some(id) = id {
        out.push_str("id: ");
        out.push_str(&id.to_string());
        out.push('\n');
    }
    out.push_str("data: ");
    out.push_str(&payload.to_string());
    out.push_str("\n\n");
    Bytes::from(out)
}

fn default_shell() -> String {
    if cfg!(windows) {
        return "powershell.exe".to_string();
    }

    // Prefer $SHELL when it exists.
    if let Ok(shell) = std::env::var("SHELL")
        && !shell.trim().is_empty()
    {
        return shell;
    }

    // Fallbacks that exist on most Linux distros.
    for candidate in ["/bin/bash", "/usr/bin/bash", "/bin/sh"] {
        if Path::new(candidate).is_file() {
            return candidate.to_string();
        }
    }

    "/bin/sh".to_string()
}

#[derive(Debug, Deserialize)]
pub struct TerminalCreateBody {
    pub cwd: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCreateResponse {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Deserialize)]
pub struct TerminalResizeBody {
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

#[derive(Debug, Serialize)]
pub(crate) struct TerminalSuccessResponse {
    success: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalResizeResponse {
    success: bool,
    cols: u16,
    rows: u16,
}

pub async fn terminal_create(
    State(state): State<Arc<crate::AppState>>,
    Json(body): Json<TerminalCreateBody>,
) -> ApiResult<Json<TerminalCreateResponse>> {
    let cwd = body
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::bad_request("cwd is required"))?
        .to_string();

    let cols = body.cols.unwrap_or(80);
    let rows = body.rows.unwrap_or(24);

    match state.terminal.create(cwd, cols, rows).await {
        Ok(resp) => Ok(Json(resp)),
        Err(TerminalError::LimitReached) => Err(AppError::too_many_requests(
            TerminalError::LimitReached.to_string(),
        )),
        Err(TerminalError::InvalidWorkingDirectory) => Err(AppError::bad_request(
            TerminalError::InvalidWorkingDirectory.to_string(),
        )),
        Err(TerminalError::WorkingDirectory(error)) => {
            let diagnostic = agena_failure::diagnostic::format_error_chain_with_context(
                "inspect the terminal working directory",
                &error,
            );
            tracing::warn!(%diagnostic, "terminal creation rejected an unreadable working directory");
            let public = agena_failure::diagnostic::user_message_with_context(&diagnostic, 240);
            Err(AppError::bad_request(if public.is_empty() {
                "The terminal working directory could not be accessed.".to_owned()
            } else {
                public
            }))
        }
        Err(err) => Err(AppError::internal_error(&err)),
    }
}

pub async fn terminal_stream(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<TerminalStreamQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let session = state
        .terminal
        .get(&session_id)
        .ok_or_else(|| AppError::not_found("Terminal session not found"))?;

    *recover_terminal_mutex(&session.last_activity, "session last activity") = Instant::now();
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

    let connected = {
        let payload = serde_json::json!({
            "type": "connected",
            "runtime": "rust",
            "ptyBackend": "portable-pty",
        });
        sse_json(payload, None)
    };

    let start = tokio::time::Instant::now() + TERMINAL_HEARTBEAT;
    let mut ticker = tokio::time::interval_at(start, TERMINAL_HEARTBEAT);

    let stream = async_stream::stream! {
        yield Ok::<Bytes, std::convert::Infallible>(connected);

        if needs_resync_notice {
            let payload = serde_json::json!({
                "type": "resync",
                "reason": "history_miss",
                "since": resume_since,
                "firstAvailableSeq": snapshot_first_seq,
                "lastSeq": snapshot_last_seq,
            });
            yield Ok(sse_json(payload, None));
        }

        // For a fresh stream, send a compact tail snapshot in one event so the
        // terminal paints immediately without replaying long historical chunks.
        if let Some((seq, data)) = initial_tail_snapshot.as_ref()
            && !data.is_empty()
        {
            let payload = serde_json::json!({"type": "data", "seq": seq, "data": data});
            yield Ok(sse_json(payload, Some(*seq)));
        }

        // Replay missed output for resumable streams.
        for (seq, chunk) in snapshot_chunks.iter() {
            if *seq < replay_from_seq {
                continue;
            }
            let payload = serde_json::json!({"type": "data", "seq": seq, "data": chunk});
            yield Ok(sse_json(payload, Some(*seq)));
        }

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    // Use SSE comments for heartbeats.
                    yield Ok(Bytes::from(": heartbeat\n\n"));
                }
                evt = rx.recv() => {
                    match evt {
                        Ok(TerminalEvent::Data { seq, data }) => {
                            // Skip any chunks that were included in the history snapshot.
                            if seq <= snapshot_last_seq {
                                continue;
                            }
                            let payload = serde_json::json!({"type": "data", "seq": seq, "data": data});
                            yield Ok(sse_json(payload, Some(seq)));
                        }
                        Ok(TerminalEvent::Exit { exit_code, signal }) => {
                            let payload = serde_json::json!({
                                "type": "exit",
                                "exitCode": exit_code,
                                "signal": signal,
                            });
                            yield Ok(sse_json(payload, None));
                            break;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    }
                }
            }
        }
    };

    let mut resp = Response::new(Body::from_stream(stream));
    *resp.status_mut() = StatusCode::OK;
    let headers = resp.headers_mut();
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
    Ok(resp)
}

pub async fn terminal_input(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
    body: Body,
) -> ApiResult<Json<TerminalSuccessResponse>> {
    let session = state
        .terminal
        .get(&session_id)
        .ok_or_else(|| AppError::not_found("Terminal session not found"))?;

    const MAX_TERMINAL_INPUT_BYTES: usize = 1024 * 1024;
    let bytes = match axum::body::to_bytes(body, MAX_TERMINAL_INPUT_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let diagnostic = agena_failure::diagnostic::format_error_chain_with_context(
                "failed to read terminal input request body",
                &error,
            );
            if body_error_contains_length_limit(&error) {
                tracing::warn!(
                    terminal_session_id = %session_id,
                    max_bytes = MAX_TERMINAL_INPUT_BYTES,
                    %diagnostic,
                    "rejected oversized terminal input"
                );
                return Err(AppError::payload_too_large(format!(
                    "Terminal input exceeds the {MAX_TERMINAL_INPUT_BYTES}-byte limit: {diagnostic}"
                )));
            }
            return Err(AppError::internal(diagnostic));
        }
    };

    session
        .write(bytes)
        .await
        .map_err(|err| AppError::internal(format!("{err:#}")))?;

    Ok(Json(TerminalSuccessResponse { success: true }))
}

pub async fn terminal_resize(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<TerminalResizeBody>,
) -> ApiResult<Json<TerminalResizeResponse>> {
    let session = state
        .terminal
        .get(&session_id)
        .ok_or_else(|| AppError::not_found("Terminal session not found"))?;

    let (Some(cols), Some(rows)) = (body.cols, body.rows) else {
        return Err(AppError::bad_request("cols and rows are required"));
    };

    session
        .resize(cols, rows)
        .await
        .map_err(|err| AppError::internal(format!("{err:#}")))?;
    state.terminal.remember_dimensions(&session_id, cols, rows);

    Ok(Json(TerminalResizeResponse {
        success: true,
        cols,
        rows,
    }))
}

pub async fn terminal_delete(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Json<TerminalSuccessResponse>> {
    match state.terminal.kill_session(&session_id) {
        Ok(()) => Ok(Json(TerminalSuccessResponse { success: true })),
        Err(TerminalError::NotFound) => Err(AppError::not_found("Terminal session not found")),
        Err(err) => Err(AppError::internal_error(&err)),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfoResponse {
    pub session_id: String,
    pub cwd: String,
    pub running: bool,
}

pub async fn terminal_get(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Json<TerminalInfoResponse>> {
    let (cwd, running) = state
        .terminal
        .peek_info(&session_id)
        .ok_or_else(|| AppError::not_found("Terminal session not found"))?;
    Ok(Json(TerminalInfoResponse {
        session_id,
        cwd,
        running,
    }))
}

pub async fn terminal_start(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Json<TerminalInfoResponse>> {
    let session = state
        .terminal
        .get(&session_id)
        .ok_or_else(|| AppError::not_found("Terminal session not found"))?;
    Ok(Json(TerminalInfoResponse {
        session_id,
        cwd: session.cwd.clone(),
        running: true,
    }))
}

pub async fn terminal_stop(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Json<TerminalSuccessResponse>> {
    match state.terminal.stop_session(&session_id) {
        Ok(()) => Ok(Json(TerminalSuccessResponse { success: true })),
        Err(TerminalError::NotFound) => Err(AppError::not_found("Terminal session not found")),
        Err(err) => Err(AppError::internal_error(&err)),
    }
}

pub async fn terminal_restart(
    State(state): State<Arc<crate::AppState>>,
    AxumPath(old_session_id): AxumPath<String>,
    Json(body): Json<TerminalCreateBody>,
) -> ApiResult<Json<TerminalCreateResponse>> {
    let cwd = body
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::bad_request("cwd is required"))?
        .to_string();
    let cols = body.cols.unwrap_or(80);
    let rows = body.rows.unwrap_or(24);

    // A missing old id is acceptable for an idempotent restart. Every actual
    // stop failure must abort the restart so the server never reports two
    // overlapping terminals as a successful replacement.
    if let Err(error) = state.terminal.kill_session(&old_session_id)
        && !matches!(error, TerminalError::NotFound)
    {
        return Err(AppError::internal_error(&error));
    }

    match state.terminal.create(cwd, cols, rows).await {
        Ok(resp) => Ok(Json(resp)),
        Err(TerminalError::LimitReached) => Err(AppError::too_many_requests(
            TerminalError::LimitReached.to_string(),
        )),
        Err(TerminalError::InvalidWorkingDirectory) => Err(AppError::bad_request(
            TerminalError::InvalidWorkingDirectory.to_string(),
        )),
        Err(TerminalError::WorkingDirectory(error)) => {
            let diagnostic = agena_failure::diagnostic::format_error_chain_with_context(
                "inspect the terminal working directory",
                &error,
            );
            tracing::warn!(%diagnostic, "terminal creation rejected an unreadable working directory");
            let public = agena_failure::diagnostic::user_message_with_context(&diagnostic, 240);
            Err(AppError::bad_request(if public.is_empty() {
                "The terminal working directory could not be accessed.".to_owned()
            } else {
                public
            }))
        }
        Err(err) => Err(AppError::internal_error(&err)),
    }
}
