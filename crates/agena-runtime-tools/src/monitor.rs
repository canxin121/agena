//! Runtime background-process registry.
//!
//! A background process runs a long-lived shell command and captures every
//! stdout/stderr line as a numbered event. The public model-visible tool
//! surface is `shell` with four actions:
//!
//! * `run`       — spawn a child with `background = true`, return a stable `process_id`
//! * `list`      — enumerate active and recently-finished background processes
//! * `logs`      — pull events with `seq > since_seq`, optionally blocking
//!   up to `wait_ms` for fresh output
//! * `stop`      — kill a running child
//!
//! Captured events live in a ring buffer (default 1000 lines) so the model can
//! walk forward through history without losing recent activity. Lines that get
//! evicted are counted in `dropped_lines` so the model knows it missed
//! something.
//!
//! # Concurrency model
//!
//! Tool execution is synchronous (`ToolExecutor::execute_tool_payload_*`) but the
//! runner is async. The registry caches a `tokio::runtime::Handle` for process
//! startup. Synchronous log reads wait with a short polling loop so they work
//! from regular threads, tokio workers, and single-thread runtimes.

use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use chrono::Utc;
use regex::Regex;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use uuid::Uuid;

use agena_domain::{ProcessEvent, ProcessStatus, ProcessStream, ProcessSummary};

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
    #[error("background process '{0}' not found")]
    NotFound(String),
    #[error("invalid background process input: {0}")]
    Invalid(String),
    #[error("invalid include_pattern: {0}")]
    InvalidPattern(#[from] regex::Error),
    #[error("background process registry not attached to a tokio runtime")]
    RuntimeMissing,
}

#[derive(Debug, Clone)]
pub struct StartParams {
    pub command: String,
    pub description: String,
    pub workdir: std::path::PathBuf,
    pub timeout_ms: Option<u64>,
    pub persistent: bool,
    pub monitored: bool,
    pub include_pattern: Option<String>,
    pub success_pattern: Option<String>,
    pub failure_pattern: Option<String>,
    pub quiet_period_ms: Option<u64>,
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
    pub status: ProcessStatus,
    pub events: Vec<ProcessEvent>,
    pub last_seq: u64,
    pub has_more: bool,
    pub dropped_lines: u64,
    pub exit_code: Option<i32>,
    pub completion_reason: Option<String>,
}

/// Outcome of a `start` action.
#[derive(Debug, Clone)]
pub struct MonitorStart {
    pub summary: ProcessSummary,
}

/// Observer hook called when a background process starts or reaches a
/// terminal state. Lets the runtime surface shell processes through the
/// unified background-activity registry without coupling the monitor to it.
#[allow(unused_variables)]
pub trait MonitorListener: Send + Sync + std::fmt::Debug {
    fn on_started(&self, summary: &ProcessSummary) {}
    fn on_finished(&self, summary: &ProcessSummary) {}
}

#[derive(Debug, Clone)]
pub struct MonitorStopOutcome {
    pub summary: ProcessSummary,
}

/// Trait so callers (and tests) can swap implementations.
pub trait MonitorService: Send + Sync + std::fmt::Debug {
    fn start(&self, params: StartParams) -> Result<MonitorStart, MonitorError>;
    fn list(&self) -> Vec<ProcessSummary>;
    fn read(&self, params: ReadParams) -> Result<MonitorRead, MonitorError>;
    fn stop(&self, monitor_id: &str) -> Result<MonitorStopOutcome, MonitorError>;
}

#[derive(Debug)]
struct MonitorState {
    monitor_id: String,
    command: String,
    description: String,
    started_at_ms: i64,
    monitored: bool,
    last_activity_ms: AtomicI64,
    capacity: usize,
    /// Latest assigned seq (0 means no events yet).
    last_seq: AtomicU64,
    /// Cumulative count of evicted lines.
    dropped_lines: AtomicU64,
    inner: Mutex<MonitorInner>,
    notify: Notify,
    /// Optional observer notified on start/finish transitions.
    listener: Option<Arc<dyn MonitorListener>>,
}

#[derive(Debug)]
struct MonitorInner {
    buffer: VecDeque<ProcessEvent>,
    status: ProcessStatus,
    exit_code: Option<i32>,
    ended_at_ms: Option<i64>,
    completion_reason: Option<String>,
    abort: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MonitorState {
    fn snapshot(&self) -> ProcessSummary {
        let inner = self.inner.lock().unwrap();
        ProcessSummary {
            process_id: self.monitor_id.clone(),
            command: self.command.clone(),
            description: self.description.clone(),
            status: inner.status,
            background: true,
            monitored: self.monitored,
            started_at_ms: self.started_at_ms,
            ended_at_ms: inner.ended_at_ms,
            buffered_lines: inner.buffer.len() as u32,
            last_seq: self.last_seq.load(Ordering::Acquire),
            dropped_lines: self.dropped_lines.load(Ordering::Acquire),
            exit_code: inner.exit_code,
            completion_reason: inner.completion_reason.clone(),
        }
    }
}

#[derive(Debug)]
pub struct MonitorRegistry {
    handle: Option<Handle>,
    monitors: Mutex<HashMap<String, Arc<MonitorState>>>,
    listener: Option<Arc<dyn MonitorListener>>,
}

impl Default for MonitorRegistry {
    fn default() -> Self {
        Self {
            handle: Handle::try_current().ok(),
            monitors: Mutex::new(HashMap::new()),
            listener: None,
        }
    }
}

impl MonitorRegistry {
    pub fn from_handle(handle: Handle) -> Self {
        Self {
            handle: Some(handle),
            monitors: Mutex::new(HashMap::new()),
            listener: None,
        }
    }

    /// Attach an observer notified when processes start or finish. Only one
    /// listener is supported per registry; later calls replace the previous
    /// one.
    pub fn with_monitor_listener(mut self, listener: Arc<dyn MonitorListener>) -> Self {
        self.listener = Some(listener);
        self
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
pub fn default_monitor_registry() -> Option<Arc<dyn MonitorService>> {
    Handle::try_current()
        .ok()
        .map(|handle| Arc::new(MonitorRegistry::from_handle(handle)) as Arc<dyn MonitorService>)
}

impl MonitorService for MonitorRegistry {
    fn start(&self, params: StartParams) -> Result<MonitorStart, MonitorError> {
        if params.command.trim().is_empty() {
            return Err(MonitorError::Invalid(
                "monitor command must not be empty".into(),
            ));
        }
        let timeout_ms = params
            .timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);
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
        let success = compile_optional_pattern(params.success_pattern.as_deref())?;
        let failure = compile_optional_pattern(params.failure_pattern.as_deref())?;
        let monitored = params.monitored
            || success.is_some()
            || failure.is_some()
            || params.quiet_period_ms.is_some();

        let handle = self.require_handle()?;
        let (abort_tx, abort_rx) = tokio::sync::oneshot::channel::<()>();

        let started_at_ms = Utc::now().timestamp_millis();
        let state = Arc::new(MonitorState {
            monitor_id: format!("proc_{}", Uuid::new_v4().simple()),
            command: params.command.clone(),
            description: params.description.clone(),
            started_at_ms,
            monitored,
            last_activity_ms: AtomicI64::new(started_at_ms),
            capacity,
            last_seq: AtomicU64::new(0),
            dropped_lines: AtomicU64::new(0),
            inner: Mutex::new(MonitorInner {
                buffer: VecDeque::with_capacity(capacity.min(256)),
                status: ProcessStatus::Running,
                exit_code: None,
                ended_at_ms: None,
                completion_reason: None,
                abort: Some(abort_tx),
            }),
            notify: Notify::new(),
            listener: self.listener.clone(),
        });

        let runner_state = Arc::clone(&state);
        let runner_workdir = params.workdir.clone();
        let runner_env = params.env.clone();
        let runner_command = params.command.clone();
        let runner_capture_stderr = params.capture_stderr;
        let runner_persistent = params.persistent;
        let runner_quiet_period_ms = params.quiet_period_ms;
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
                success,
                failure,
                runner_quiet_period_ms,
                abort_rx,
            )
            .await;
        });

        let summary = state.snapshot();
        self.monitors
            .lock()
            .unwrap()
            .insert(state.monitor_id.clone(), state);
        if let Some(listener) = &self.listener {
            listener.on_started(&summary);
        }
        Ok(MonitorStart { summary })
    }

    fn list(&self) -> Vec<ProcessSummary> {
        let guard = self.monitors.lock().unwrap();
        let mut out: Vec<ProcessSummary> = guard.values().map(|s| s.snapshot()).collect();
        out.sort_by_key(|summary| summary.started_at_ms);
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
        if let Some(read) = collect_events(&state, params.since_seq, limit)
            && (!read.events.is_empty() || wait_ms == 0 || read.status != ProcessStatus::Running)
        {
            return Ok(read);
        }

        if wait_ms == 0 {
            // No events and no wait requested: still return a valid (possibly empty) snapshot.
            return Ok(collect_events(&state, params.since_seq, limit)
                .unwrap_or_else(|| empty_read(&state, params.since_seq)));
        }

        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        loop {
            if let Some(read) = collect_events(&state, params.since_seq, limit)
                && (!read.events.is_empty() || read.status != ProcessStatus::Running)
            {
                return Ok(read);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            std::thread::sleep(remaining.min(Duration::from_millis(20)));
        }

        Ok(collect_events(&state, params.since_seq, limit)
            .unwrap_or_else(|| empty_read(&state, params.since_seq)))
    }

    fn stop(&self, monitor_id: &str) -> Result<MonitorStopOutcome, MonitorError> {
        let state = self
            .lookup(monitor_id)
            .ok_or_else(|| MonitorError::NotFound(monitor_id.to_string()))?;
        {
            let mut inner = state.inner.lock().unwrap();
            if inner.status == ProcessStatus::Running {
                inner.status = ProcessStatus::Stopped;
                inner.completion_reason = Some("explicit_stop".to_string());
                if let Some(tx) = inner.abort.take() {
                    let _ = tx.send(());
                }
            }
        }
        state.notify.notify_waiters();
        if let Some(listener) = state.listener.as_ref() {
            listener.on_finished(&state.snapshot());
        }
        Ok(MonitorStopOutcome {
            summary: state.snapshot(),
        })
    }
}

fn empty_read(state: &MonitorState, since_seq: u64) -> MonitorRead {
    let summary = state.snapshot();
    MonitorRead {
        monitor_id: summary.process_id.clone(),
        status: summary.status,
        events: Vec::new(),
        last_seq: since_seq,
        has_more: summary.last_seq > since_seq,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
        completion_reason: summary.completion_reason,
    }
}

fn collect_events(state: &MonitorState, since_seq: u64, limit: usize) -> Option<MonitorRead> {
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
        completion_reason: inner.completion_reason.clone(),
    })
}

fn compile_optional_pattern(pattern: Option<&str>) -> Result<Option<Regex>, MonitorError> {
    pattern
        .filter(|pattern| !pattern.is_empty())
        .map(Regex::new)
        .transpose()
        .map_err(MonitorError::InvalidPattern)
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
    success: Option<Regex>,
    failure: Option<Regex>,
    quiet_period_ms: Option<u64>,
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
    let stderr = if capture_stderr {
        child.stderr.take()
    } else {
        None
    };
    let (condition_tx, mut condition_rx) = tokio::sync::mpsc::unbounded_channel();

    let stdout_state = Arc::clone(&state);
    let stdout_include = include.clone();
    let stdout_success = success.clone();
    let stdout_failure = failure.clone();
    let stdout_condition_tx = condition_tx.clone();
    let stdout_task = stdout.map(|s| {
        tokio::spawn(stream_lines(
            stdout_state,
            s,
            ProcessStream::Stdout,
            stdout_include,
            stdout_success,
            stdout_failure,
            stdout_condition_tx,
        ))
    });

    let stderr_state = Arc::clone(&state);
    let stderr_include = include.clone();
    let stderr_success = success;
    let stderr_failure = failure;
    let stderr_condition_tx = condition_tx.clone();
    let stderr_task = stderr.map(|s| {
        tokio::spawn(stream_lines(
            stderr_state,
            s,
            ProcessStream::Stderr,
            stderr_include,
            stderr_success,
            stderr_failure,
            stderr_condition_tx,
        ))
    });

    let mut abort_rx = abort_rx;

    let timeout_sleep = if persistent {
        None
    } else {
        Some(tokio::time::sleep(Duration::from_millis(timeout_ms)))
    };
    tokio::pin!(timeout_sleep);
    let quiet_wait = async {
        if persistent {
            std::future::pending::<()>().await;
        }
        let Some(quiet_period_ms) = quiet_period_ms else {
            std::future::pending::<()>().await;
            return;
        };
        wait_for_quiet(&state, quiet_period_ms).await;
    };
    tokio::pin!(quiet_wait);

    let outcome = tokio::select! {
        biased;
        _ = &mut abort_rx => TerminationCause::Stopped,
        Some(condition) = condition_rx.recv() => TerminationCause::Condition(condition),
        _ = &mut quiet_wait => TerminationCause::Quiet,
        _ = async {
            if let Some(sleep) = timeout_sleep.as_mut().as_pin_mut() {
                sleep.await
            } else {
                std::future::pending::<()>().await
            }
        } => TerminationCause::TimedOut,
        result = child.wait() => match result {
            Ok(status) => TerminationCause::Exited(status.code()),
            Err(err) => TerminationCause::WaitError(err.to_string()),
        },
    };

    let (final_status, final_exit_code, completion_reason) = match outcome {
        TerminationCause::Stopped => {
            kill_child(&mut child).await;
            let code = child.wait().await.ok().and_then(|s| s.code());
            (ProcessStatus::Stopped, code, "explicit_stop".to_string())
        }
        TerminationCause::TimedOut => {
            kill_child(&mut child).await;
            let code = child.wait().await.ok().and_then(|s| s.code());
            (ProcessStatus::TimedOut, code, "timeout".to_string())
        }
        TerminationCause::Condition(PatternOutcome::Success) => {
            kill_child(&mut child).await;
            let code = child.wait().await.ok().and_then(|s| s.code());
            (ProcessStatus::Exited, code, "success_pattern".to_string())
        }
        TerminationCause::Condition(PatternOutcome::Failure) => {
            kill_child(&mut child).await;
            let code = child.wait().await.ok().and_then(|s| s.code());
            (ProcessStatus::Failed, code, "failure_pattern".to_string())
        }
        TerminationCause::Quiet => {
            kill_child(&mut child).await;
            let code = child.wait().await.ok().and_then(|s| s.code());
            (ProcessStatus::Exited, code, "quiet_period".to_string())
        }
        TerminationCause::Exited(code) => (ProcessStatus::Exited, code, "process_exit".to_string()),
        TerminationCause::WaitError(reason) => {
            mark_failed(&state, format!("wait failed: {reason}"));
            return;
        }
    };

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
            matches!(inner.status, ProcessStatus::Stopped) && final_status == ProcessStatus::Exited;
        if !preserve_stopped {
            inner.status = final_status;
        }
        inner.exit_code = final_exit_code;
        if inner.completion_reason.is_none() {
            inner.completion_reason = Some(completion_reason);
        }
        inner.ended_at_ms = Some(Utc::now().timestamp_millis());
        inner.abort = None;
    }
    state.notify.notify_waiters();
    if let Some(listener) = state.listener.as_ref() {
        listener.on_finished(&state.snapshot());
    }
}

async fn kill_child(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
}

enum TerminationCause {
    Stopped,
    TimedOut,
    Condition(PatternOutcome),
    Quiet,
    Exited(Option<i32>),
    WaitError(String),
}

#[derive(Debug, Clone, Copy)]
enum PatternOutcome {
    Success,
    Failure,
}

fn mark_failed(state: &MonitorState, reason: String) {
    push_event(state, ProcessStream::Stderr, reason);
    let mut inner = state.inner.lock().unwrap();
    inner.status = ProcessStatus::Failed;
    inner.completion_reason = Some("runtime_failure".to_string());
    inner.ended_at_ms = Some(Utc::now().timestamp_millis());
    inner.abort = None;
    drop(inner);
    state.notify.notify_waiters();
    if let Some(listener) = state.listener.as_ref() {
        listener.on_finished(&state.snapshot());
    }
}

async fn stream_lines<R>(
    state: Arc<MonitorState>,
    reader: R,
    stream: ProcessStream,
    include: Option<Regex>,
    success: Option<Regex>,
    failure: Option<Regex>,
    condition_tx: tokio::sync::mpsc::UnboundedSender<PatternOutcome>,
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
                state
                    .last_activity_ms
                    .store(Utc::now().timestamp_millis(), Ordering::Release);
                if failure
                    .as_ref()
                    .is_some_and(|pattern| pattern.is_match(&line))
                {
                    let _ = condition_tx.send(PatternOutcome::Failure);
                } else if success
                    .as_ref()
                    .is_some_and(|pattern| pattern.is_match(&line))
                {
                    let _ = condition_tx.send(PatternOutcome::Success);
                }
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

async fn wait_for_quiet(state: &MonitorState, quiet_period_ms: u64) {
    let quiet_period_ms = quiet_period_ms.max(1);
    loop {
        let now = Utc::now().timestamp_millis();
        let last = state.last_activity_ms.load(Ordering::Acquire);
        let elapsed = now.saturating_sub(last) as u64;
        if elapsed >= quiet_period_ms {
            return;
        }
        tokio::time::sleep(Duration::from_millis(
            quiet_period_ms.saturating_sub(elapsed).min(100),
        ))
        .await;
    }
}

fn push_event(state: &MonitorState, stream: ProcessStream, line: String) {
    let seq = state.last_seq.fetch_add(1, Ordering::AcqRel) + 1;
    let event = ProcessEvent {
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn start_params(command: &str) -> StartParams {
        StartParams {
            command: command.to_string(),
            description: "monitor test".to_string(),
            workdir: std::env::current_dir().expect("current dir"),
            timeout_ms: Some(2_000),
            persistent: false,
            monitored: true,
            include_pattern: None,
            success_pattern: None,
            failure_pattern: None,
            quiet_period_ms: None,
            max_buffered_lines: Some(32),
            capture_stderr: true,
            env: std::env::vars().collect(),
        }
    }

    async fn wait_for_terminal(registry: Arc<MonitorRegistry>, id: String) -> MonitorRead {
        tokio::task::spawn_blocking(move || {
            registry
                .read(ReadParams {
                    monitor_id: id,
                    since_seq: 0,
                    limit: Some(32),
                    wait_ms: 2_000,
                })
                .expect("read monitor")
        })
        .await
        .expect("join read")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn success_pattern_completes_managed_process() {
        let registry = Arc::new(MonitorRegistry::from_handle(Handle::current()));
        let mut params = start_params("printf 'READY\\n'; exec sleep 30");
        params.success_pattern = Some("^READY$".to_string());
        let id = registry.start(params).expect("start").summary.process_id;
        let read = wait_for_terminal(registry, id).await;
        assert_eq!(read.status, ProcessStatus::Exited);
        assert_eq!(read.completion_reason.as_deref(), Some("success_pattern"));
        assert!(read.events.iter().any(|event| event.line == "READY"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failure_pattern_marks_managed_process_failed() {
        let registry = Arc::new(MonitorRegistry::from_handle(Handle::current()));
        let mut params = start_params("printf 'FATAL\\n' >&2; exec sleep 30");
        params.failure_pattern = Some("^FATAL$".to_string());
        let id = registry.start(params).expect("start").summary.process_id;
        let read = wait_for_terminal(registry, id).await;
        assert_eq!(read.status, ProcessStatus::Failed);
        assert_eq!(read.completion_reason.as_deref(), Some("failure_pattern"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn quiet_period_completes_silent_managed_process() {
        let registry = Arc::new(MonitorRegistry::from_handle(Handle::current()));
        let mut params = start_params("exec sleep 30");
        params.quiet_period_ms = Some(50);
        let id = registry.start(params).expect("start").summary.process_id;
        let read = wait_for_terminal(registry, id).await;
        assert_eq!(read.status, ProcessStatus::Exited);
        assert_eq!(read.completion_reason.as_deref(), Some("quiet_period"));
    }
}
