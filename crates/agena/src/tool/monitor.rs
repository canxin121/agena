//! Background process monitor — analogue of Claude Code's `Monitor` tool.
//!
//! A monitor runs a long-lived shell command in the background and captures
//! every stdout/stderr line as a numbered event. The model interacts with it
//! through four actions on a single `monitor` builtin tool:
//!
//! * `start`     — spawn a child, return a stable `monitor_id`
//! * `list`      — enumerate active and recently-finished monitors
//! * `read`      — pull events with `seq > since_seq`, optionally blocking
//!                 up to `wait_ms` for fresh output
//! * `stop`      — kill a running child
//!
//! Captured events live in a ring buffer (default 1000 lines) so the model can
//! walk forward through history without losing recent activity. Lines that get
//! evicted are counted in `dropped_lines` so the model knows it missed
//! something.
//!
//! # Concurrency model
//!
//! Tool execution is synchronous (`ToolExecutor::execute_builtin_*`) but the
//! runner is async. The registry caches a `tokio::runtime::Handle` at
//! construction time and uses `block_in_place` + `Handle::block_on` when a
//! sync caller needs to wait for new events. This requires the multi-thread
//! tokio runtime that `agena` already mandates.

use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use chrono::Utc;
use regex::Regex;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::time::Instant;
use uuid::Uuid;

use crate::message::{MonitorEvent, MonitorStatus, MonitorStream, MonitorSummary};

const DEFAULT_BUFFER_LINES: usize = 1_000;
const MAX_BUFFER_LINES: usize = 10_000;
const DEFAULT_TIMEOUT_MS: u64 = 300_000;
const MAX_TIMEOUT_MS: u64 = 3_600_000;
const DEFAULT_READ_LIMIT: usize = 200;
const MAX_READ_LIMIT: usize = 2_000;
const MAX_WAIT_MS: u64 = 60_000;
const READER_LINE_BYTE_CAP: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum MonitorError {
    #[error("monitor '{0}' not found")]
    NotFound(String),
    #[error("invalid monitor input: {0}")]
    Invalid(String),
    #[error("invalid include_pattern: {0}")]
    InvalidPattern(#[from] regex::Error),
    #[error("monitor registry not attached to a tokio runtime")]
    RuntimeMissing,
    #[error("failed to spawn monitor command: {0}")]
    Spawn(String),
}

#[derive(Debug, Clone)]
pub struct StartParams {
    pub command: String,
    pub description: String,
    pub workdir: std::path::PathBuf,
    pub timeout_ms: Option<u64>,
    pub persistent: bool,
    pub include_pattern: Option<String>,
    pub max_buffered_lines: Option<u32>,
    pub capture_stderr: bool,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ReadParams {
    pub monitor_id: String,
    pub since_seq: u64,
    pub limit: Option<u32>,
    pub wait_ms: u64,
}

/// Outcome of a `read` action.
#[derive(Debug, Clone)]
pub struct MonitorRead {
    pub monitor_id: String,
    pub status: MonitorStatus,
    pub events: Vec<MonitorEvent>,
    pub last_seq: u64,
    pub has_more: bool,
    pub dropped_lines: u64,
    pub exit_code: Option<i32>,
}

/// Outcome of a `start` action.
#[derive(Debug, Clone)]
pub struct MonitorStart {
    pub summary: MonitorSummary,
}

#[derive(Debug, Clone)]
pub struct MonitorStopOutcome {
    pub summary: MonitorSummary,
}

/// Trait so callers (and tests) can swap implementations.
pub trait MonitorService: Send + Sync + std::fmt::Debug {
    fn start(&self, params: StartParams) -> Result<MonitorStart, MonitorError>;
    fn list(&self) -> Vec<MonitorSummary>;
    fn read(&self, params: ReadParams) -> Result<MonitorRead, MonitorError>;
    fn stop(&self, monitor_id: &str) -> Result<MonitorStopOutcome, MonitorError>;
}

#[derive(Debug)]
struct MonitorState {
    monitor_id: String,
    command: String,
    description: String,
    persistent: bool,
    started_at_ms: i64,
    capacity: usize,
    /// Latest assigned seq (0 means no events yet).
    last_seq: AtomicU64,
    /// Cumulative count of evicted lines.
    dropped_lines: AtomicU64,
    inner: Mutex<MonitorInner>,
    notify: Notify,
}

#[derive(Debug)]
struct MonitorInner {
    buffer: VecDeque<MonitorEvent>,
    status: MonitorStatus,
    exit_code: Option<i32>,
    ended_at_ms: Option<i64>,
    abort: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MonitorState {
    fn snapshot(&self) -> MonitorSummary {
        let inner = self.inner.lock().unwrap();
        MonitorSummary {
            monitor_id: self.monitor_id.clone(),
            command: self.command.clone(),
            description: self.description.clone(),
            status: inner.status,
            persistent: self.persistent,
            started_at_ms: self.started_at_ms,
            ended_at_ms: inner.ended_at_ms,
            buffered_lines: inner.buffer.len() as u32,
            last_seq: self.last_seq.load(Ordering::Acquire),
            dropped_lines: self.dropped_lines.load(Ordering::Acquire),
            exit_code: inner.exit_code,
        }
    }
}

#[derive(Debug)]
pub struct MonitorRegistry {
    handle: Option<Handle>,
    monitors: Mutex<HashMap<String, Arc<MonitorState>>>,
}

impl Default for MonitorRegistry {
    fn default() -> Self {
        Self {
            handle: Handle::try_current().ok(),
            monitors: Mutex::new(HashMap::new()),
        }
    }
}

impl MonitorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_handle(handle: Handle) -> Self {
        Self {
            handle: Some(handle),
            monitors: Mutex::new(HashMap::new()),
        }
    }

    fn require_handle(&self) -> Result<Handle, MonitorError> {
        self.handle.clone().ok_or(MonitorError::RuntimeMissing)
    }

    fn lookup(&self, monitor_id: &str) -> Option<Arc<MonitorState>> {
        self.monitors.lock().unwrap().get(monitor_id).cloned()
    }
}

/// Build the registry that `ToolExecutor::new` installs by default. Returns
/// `None` when no tokio runtime is reachable from the current thread (so
/// non-async test scaffolding stays usable — callers can attach a registry
/// later via `with_monitor_registry`).
pub(super) fn default_registry() -> Option<Arc<dyn MonitorService>> {
    Handle::try_current()
        .ok()
        .map(|handle| Arc::new(MonitorRegistry::with_handle(handle)) as Arc<dyn MonitorService>)
}

impl MonitorService for MonitorRegistry {
    fn start(&self, params: StartParams) -> Result<MonitorStart, MonitorError> {
        if params.command.trim().is_empty() {
            return Err(MonitorError::Invalid(
                "monitor command must not be empty".into(),
            ));
        }
        let timeout_ms = params.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).min(MAX_TIMEOUT_MS);
        if !params.persistent && timeout_ms == 0 {
            return Err(MonitorError::Invalid(
                "non-persistent monitors must have timeout_ms > 0".into(),
            ));
        }
        let capacity = params
            .max_buffered_lines
            .map(|n| (n as usize).clamp(1, MAX_BUFFER_LINES))
            .unwrap_or(DEFAULT_BUFFER_LINES);
        let include = match params.include_pattern.as_deref() {
            Some(pattern) if !pattern.is_empty() => Some(Regex::new(pattern)?),
            _ => None,
        };

        let handle = self.require_handle()?;
        let (abort_tx, abort_rx) = tokio::sync::oneshot::channel::<()>();

        let state = Arc::new(MonitorState {
            monitor_id: format!("mon_{}", Uuid::new_v4().simple()),
            command: params.command.clone(),
            description: params.description.clone(),
            persistent: params.persistent,
            started_at_ms: Utc::now().timestamp_millis(),
            capacity,
            last_seq: AtomicU64::new(0),
            dropped_lines: AtomicU64::new(0),
            inner: Mutex::new(MonitorInner {
                buffer: VecDeque::with_capacity(capacity.min(256)),
                status: MonitorStatus::Running,
                exit_code: None,
                ended_at_ms: None,
                abort: Some(abort_tx),
            }),
            notify: Notify::new(),
        });

        let runner_state = Arc::clone(&state);
        let runner_workdir = params.workdir.clone();
        let runner_env = params.env.clone();
        let runner_command = params.command.clone();
        let runner_capture_stderr = params.capture_stderr;
        let runner_persistent = params.persistent;
        handle.spawn(async move {
            run_monitor(
                runner_state,
                runner_command,
                runner_workdir,
                runner_env,
                runner_capture_stderr,
                runner_persistent,
                timeout_ms,
                include,
                abort_rx,
            )
            .await;
        });

        let summary = state.snapshot();
        self.monitors
            .lock()
            .unwrap()
            .insert(state.monitor_id.clone(), state);
        Ok(MonitorStart { summary })
    }

    fn list(&self) -> Vec<MonitorSummary> {
        let guard = self.monitors.lock().unwrap();
        let mut out: Vec<MonitorSummary> = guard.values().map(|s| s.snapshot()).collect();
        out.sort_by(|a, b| a.started_at_ms.cmp(&b.started_at_ms));
        out
    }

    fn read(&self, params: ReadParams) -> Result<MonitorRead, MonitorError> {
        let state = self
            .lookup(&params.monitor_id)
            .ok_or_else(|| MonitorError::NotFound(params.monitor_id.clone()))?;
        let limit = params
            .limit
            .map(|n| (n as usize).clamp(1, MAX_READ_LIMIT))
            .unwrap_or(DEFAULT_READ_LIMIT);
        let wait_ms = params.wait_ms.min(MAX_WAIT_MS);

        // Fast path: try once.
        if let Some(read) = collect_events(&state, params.since_seq, limit) {
            if !read.events.is_empty() || wait_ms == 0 || read.status != MonitorStatus::Running {
                return Ok(read);
            }
        }

        if wait_ms == 0 {
            // No events and no wait requested: still return a valid (possibly empty) snapshot.
            return Ok(collect_events(&state, params.since_seq, limit).unwrap_or_else(|| {
                empty_read(&state, params.since_seq)
            }));
        }

        let handle = self.require_handle()?;
        let state_for_wait = Arc::clone(&state);
        let since_seq = params.since_seq;

        // We bridge async → sync with a one-shot channel: the wait runs as a
        // spawned tokio task and the calling thread blocks on `recv`. This
        // works regardless of whether the caller is on a tokio worker thread,
        // the blocking pool (`spawn_blocking`), or a plain OS thread, and it
        // never deadlocks on a single-thread runtime.
        let (tx, rx) = std::sync::mpsc::channel::<MonitorRead>();
        handle.spawn(async move {
            let deadline = Instant::now() + Duration::from_millis(wait_ms);
            loop {
                // Register the waiter BEFORE re-checking the condition.
                // Otherwise a `notify_waiters()` that fires between the
                // condition check and the `.await` below would be lost —
                // `Notify` only delivers wakeups to already-registered
                // waiters.
                let notified = state_for_wait.notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();

                if state_for_wait.last_seq.load(Ordering::Acquire) > since_seq
                    || state_for_wait.inner.lock().unwrap().status != MonitorStatus::Running
                {
                    break;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                if tokio::time::timeout(remaining, notified).await.is_err() {
                    break;
                }
            }
            let read = collect_events(&state_for_wait, since_seq, limit)
                .unwrap_or_else(|| empty_read(&state_for_wait, since_seq));
            let _ = tx.send(read);
        });

        let read = rx
            .recv()
            .map_err(|_| MonitorError::Invalid("monitor read task dropped".into()))?;
        Ok(read)
    }

    fn stop(&self, monitor_id: &str) -> Result<MonitorStopOutcome, MonitorError> {
        let state = self
            .lookup(monitor_id)
            .ok_or_else(|| MonitorError::NotFound(monitor_id.to_string()))?;
        {
            let mut inner = state.inner.lock().unwrap();
            if inner.status == MonitorStatus::Running {
                inner.status = MonitorStatus::Stopped;
                if let Some(tx) = inner.abort.take() {
                    let _ = tx.send(());
                }
            }
        }
        state.notify.notify_waiters();
        Ok(MonitorStopOutcome {
            summary: state.snapshot(),
        })
    }
}

fn empty_read(state: &MonitorState, since_seq: u64) -> MonitorRead {
    let summary = state.snapshot();
    MonitorRead {
        monitor_id: summary.monitor_id.clone(),
        status: summary.status,
        events: Vec::new(),
        last_seq: since_seq,
        has_more: summary.last_seq > since_seq,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
    }
}

fn collect_events(
    state: &MonitorState,
    since_seq: u64,
    limit: usize,
) -> Option<MonitorRead> {
    let inner = state.inner.lock().unwrap();
    let status = inner.status;
    let exit_code = inner.exit_code;
    let mut events = Vec::with_capacity(limit.min(64));
    for event in inner
        .buffer
        .iter()
        .filter(|e| e.seq > since_seq)
        .take(limit)
    {
        events.push(event.clone());
    }
    let last_seq_in_batch = events.last().map(|e| e.seq).unwrap_or(since_seq);
    let global_last = state.last_seq.load(Ordering::Acquire);
    let has_more = global_last > last_seq_in_batch;
    Some(MonitorRead {
        monitor_id: state.monitor_id.clone(),
        status,
        events,
        last_seq: last_seq_in_batch,
        has_more,
        dropped_lines: state.dropped_lines.load(Ordering::Acquire),
        exit_code,
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_monitor(
    state: Arc<MonitorState>,
    command: String,
    workdir: std::path::PathBuf,
    env: HashMap<String, String>,
    capture_stderr: bool,
    persistent: bool,
    timeout_ms: u64,
    include: Option<Regex>,
    abort_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut cmd = build_command(&command);
    cmd.current_dir(&workdir);
    cmd.env_clear();
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    if capture_stderr {
        cmd.stderr(Stdio::piped());
    } else {
        cmd.stderr(Stdio::null());
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            mark_failed(&state, format!("failed to spawn: {err}"));
            return;
        }
    };

    let stdout = child.stdout.take();
    let stderr = if capture_stderr { child.stderr.take() } else { None };

    let stdout_state = Arc::clone(&state);
    let stdout_include = include.clone();
    let stdout_task = stdout.map(|s| {
        tokio::spawn(stream_lines(
            stdout_state,
            s,
            MonitorStream::Stdout,
            stdout_include,
        ))
    });

    let stderr_state = Arc::clone(&state);
    let stderr_include = include.clone();
    let stderr_task = stderr.map(|s| {
        tokio::spawn(stream_lines(
            stderr_state,
            s,
            MonitorStream::Stderr,
            stderr_include,
        ))
    });

    let mut abort_rx = abort_rx;

    let final_status: MonitorStatus;
    let final_exit_code: Option<i32>;

    let timeout_sleep = if persistent {
        None
    } else {
        Some(tokio::time::sleep(Duration::from_millis(timeout_ms)))
    };
    tokio::pin!(timeout_sleep);

    let outcome = loop {
        // We re-poll `child.wait()` each iteration. `wait` is documented as
        // safe to call repeatedly; once it resolves the child is reaped.
        tokio::select! {
            biased;
            _ = &mut abort_rx => break TerminationCause::Stopped,
            _ = async {
                if let Some(sleep) = timeout_sleep.as_mut().as_pin_mut() {
                    sleep.await
                } else {
                    std::future::pending::<()>().await
                }
            } => break TerminationCause::TimedOut,
            result = child.wait() => match result {
                Ok(status) => break TerminationCause::Exited(status.code()),
                Err(err) => break TerminationCause::WaitError(err.to_string()),
            },
        }
    };

    match outcome {
        TerminationCause::Stopped => {
            kill_child(&mut child).await;
            final_exit_code = child.wait().await.ok().and_then(|s| s.code());
            final_status = MonitorStatus::Stopped;
        }
        TerminationCause::TimedOut => {
            kill_child(&mut child).await;
            final_exit_code = child.wait().await.ok().and_then(|s| s.code());
            final_status = MonitorStatus::TimedOut;
        }
        TerminationCause::Exited(code) => {
            final_exit_code = code;
            final_status = MonitorStatus::Exited;
        }
        TerminationCause::WaitError(reason) => {
            mark_failed(&state, format!("wait failed: {reason}"));
            return;
        }
    }

    if let Some(handle) = stdout_task {
        let _ = handle.await;
    }
    if let Some(handle) = stderr_task {
        let _ = handle.await;
    }

    {
        let mut inner = state.inner.lock().unwrap();
        // Don't downgrade an explicit Stopped from `stop()` to Exited if the
        // child happened to finish racing the kill.
        let preserve_stopped =
            matches!(inner.status, MonitorStatus::Stopped) && final_status == MonitorStatus::Exited;
        if !preserve_stopped {
            inner.status = final_status;
        }
        inner.exit_code = final_exit_code;
        inner.ended_at_ms = Some(Utc::now().timestamp_millis());
        inner.abort = None;
    }
    state.notify.notify_waiters();
}

async fn kill_child(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
}

enum TerminationCause {
    Stopped,
    TimedOut,
    Exited(Option<i32>),
    WaitError(String),
}

fn mark_failed(state: &MonitorState, reason: String) {
    push_event(state, MonitorStream::Stderr, reason);
    let mut inner = state.inner.lock().unwrap();
    inner.status = MonitorStatus::Failed;
    inner.ended_at_ms = Some(Utc::now().timestamp_millis());
    inner.abort = None;
    drop(inner);
    state.notify.notify_waiters();
}

async fn stream_lines<R>(
    state: Arc<MonitorState>,
    reader: R,
    stream: MonitorStream,
    include: Option<Regex>,
) where
    R: tokio::io::AsyncRead + Unpin + Send,
{
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::with_capacity(1024);
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) => break,
            Ok(_) => {
                if buf.ends_with(b"\n") {
                    buf.pop();
                    if buf.ends_with(b"\r") {
                        buf.pop();
                    }
                }
                if buf.len() > READER_LINE_BYTE_CAP {
                    buf.truncate(READER_LINE_BYTE_CAP);
                }
                let line = String::from_utf8_lossy(&buf).into_owned();
                if let Some(re) = include.as_ref()
                    && !re.is_match(&line)
                {
                    continue;
                }
                push_event(&state, stream, line);
            }
            Err(_) => break,
        }
    }
}

fn push_event(state: &MonitorState, stream: MonitorStream, line: String) {
    let seq = state.last_seq.fetch_add(1, Ordering::AcqRel) + 1;
    let event = MonitorEvent {
        seq,
        stream,
        ts_ms: Utc::now().timestamp_millis(),
        line,
    };
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.buffer.len() == state.capacity {
            inner.buffer.pop_front();
            state.dropped_lines.fetch_add(1, Ordering::AcqRel);
        }
        inner.buffer.push_back(event);
    }
    state.notify.notify_waiters();
}

fn build_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/d", "/s", "/c", command]);
        cmd
    } else {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-lc", command]);
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn workdir() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn captures_stdout_lines_and_exits_cleanly() {
        let registry = MonitorRegistry::new();
        let started = registry
            .start(StartParams {
                command: "printf 'a\\nb\\nc\\n'".into(),
                description: "echo".into(),
                workdir: workdir(),
                timeout_ms: Some(5_000),
                persistent: false,
                include_pattern: None,
                max_buffered_lines: None,
                capture_stderr: true,
                env: std::env::vars().collect(),
            })
            .expect("start should succeed");

        let id = started.summary.monitor_id.clone();
        let read = registry
            .read(ReadParams {
                monitor_id: id.clone(),
                since_seq: 0,
                limit: None,
                wait_ms: 2_000,
            })
            .expect("read should succeed");
        let lines: Vec<&str> = read.events.iter().map(|e| e.line.as_str()).collect();
        assert!(
            lines.contains(&"a") && lines.contains(&"b") && lines.contains(&"c"),
            "unexpected lines: {lines:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_terminates_running_monitor() {
        let registry = MonitorRegistry::new();
        let started = registry
            .start(StartParams {
                command: "sleep 30".into(),
                description: "sleep".into(),
                workdir: workdir(),
                timeout_ms: Some(5_000),
                persistent: false,
                include_pattern: None,
                max_buffered_lines: None,
                capture_stderr: true,
                env: std::env::vars().collect(),
            })
            .expect("start should succeed");
        let id = started.summary.monitor_id.clone();
        registry.stop(&id).expect("stop should succeed");

        // Give the runner a brief moment to update status.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let summary = registry
            .list()
            .into_iter()
            .find(|s| s.monitor_id == id)
            .expect("monitor should be listed");
        assert_eq!(summary.status, MonitorStatus::Stopped);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn read_blocks_until_new_event_arrives() {
        let registry = Arc::new(MonitorRegistry::new());
        let started = registry
            .start(StartParams {
                command: "sleep 0.3 && printf 'hello\\n' && sleep 5".into(),
                description: "delayed".into(),
                workdir: workdir(),
                timeout_ms: Some(10_000),
                persistent: false,
                include_pattern: None,
                max_buffered_lines: None,
                capture_stderr: true,
                env: std::env::vars().collect(),
            })
            .expect("start should succeed");
        let id = started.summary.monitor_id.clone();

        let registry_for_read = Arc::clone(&registry);
        let id_for_read = id.clone();
        let read = tokio::task::spawn_blocking(move || {
            registry_for_read.read(ReadParams {
                monitor_id: id_for_read,
                since_seq: 0,
                limit: None,
                wait_ms: 5_000,
            })
        })
        .await
        .unwrap()
        .expect("read should succeed");

        assert_eq!(
            read.events.iter().map(|e| e.line.as_str()).collect::<Vec<_>>(),
            vec!["hello"],
        );
        assert_eq!(read.last_seq, 1);

        registry.stop(&id).expect("stop should succeed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_reports_running_and_finished_monitors() {
        let registry = MonitorRegistry::new();
        registry
            .start(StartParams {
                command: "true".into(),
                description: "quick".into(),
                workdir: workdir(),
                timeout_ms: Some(2_000),
                persistent: false,
                include_pattern: None,
                max_buffered_lines: None,
                capture_stderr: true,
                env: std::env::vars().collect(),
            })
            .expect("start should succeed");
        // Give the runner a moment to wait the child.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let monitors = registry.list();
        assert_eq!(monitors.len(), 1);
        assert!(matches!(
            monitors[0].status,
            MonitorStatus::Exited | MonitorStatus::Running
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn include_pattern_filters_lines() {
        let registry = MonitorRegistry::new();
        let started = registry
            .start(StartParams {
                command: "printf 'INFO ok\\nERROR boom\\nINFO done\\n'".into(),
                description: "filter".into(),
                workdir: workdir(),
                timeout_ms: Some(5_000),
                persistent: false,
                include_pattern: Some("ERROR".into()),
                max_buffered_lines: None,
                capture_stderr: true,
                env: std::env::vars().collect(),
            })
            .expect("start should succeed");
        let id = started.summary.monitor_id.clone();
        let read = registry
            .read(ReadParams {
                monitor_id: id,
                since_seq: 0,
                limit: None,
                wait_ms: 2_000,
            })
            .expect("read should succeed");
        let lines: Vec<&str> = read.events.iter().map(|e| e.line.as_str()).collect();
        assert_eq!(lines, vec!["ERROR boom"]);
    }
}
