//! Runtime background-process / monitor registry.
//!
//! A background process runs a long-lived shell command and captures every
//! stdout/stderr line as a numbered event; a monitor may alternatively watch a
//! WebSocket feed whose text frames become events. The public model-visible
//! tool surface is `shell` (run/list/logs/stop) plus the `monitor` tool
//! (start/stop), both backed by this registry.
//!
//! Captured events live in a ring buffer (default 1000 lines) so the model can
//! walk forward through history without losing recent activity. Lines that get
//! evicted are counted in `dropped_lines` so the model knows it missed
//! something. A [`MonitorListener`] receives start/finish transitions and
//! **every** event as it arrives — the runtime's activity layer projects the
//! events into the transcript as `system_notification` parts (everything-is-a-
//! part), while the ring buffer stays the ephemeral "live projection".
//!
//! # Concurrency model
//!
//! The process runner, pipe readers, WebSocket reader, timeout, cancellation,
//! and child wait all run on the caller's Tokio runtime. The registry retains a
//! synchronous compatibility surface for non-async consumers; those reads sleep
//! on a condition variable and async callers isolate them with `spawn_blocking`.

use portable_atomic::{AtomicI64, AtomicU64};
use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::{Arc, Condvar, Mutex, atomic::Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use futures_util::StreamExt as _;
use regex::Regex;
use thiserror::Error;
use tokio::process::Command;
use tokio::runtime::Handle;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_util::codec::{FramedRead, LinesCodec};
use uuid::Uuid;

use agena_domain::{ProcessEvent, ProcessStatus, ProcessStream, ProcessSummary};
use agena_process::ManagedChild;

const DEFAULT_BUFFER_LINES: usize = 1_000;
const MAX_BUFFER_LINES: usize = 10_000;
const DEFAULT_TIMEOUT_MS: u64 = 300_000;
const MAX_TIMEOUT_MS: u64 = 3_600_000;
const DEFAULT_READ_LIMIT: usize = 200;
const MAX_READ_LIMIT: usize = 2_000;
const MAX_WAIT_MS: u64 = 60_000;
const READER_LINE_BYTE_CAP: usize = 64 * 1024;

#[derive(Debug, Error)]
/// Error from the process monitor.
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
/// Parameters for starting a monitored process.
pub struct StartParams {
    /// Stable id reserved by the durable background-operation coordinator.
    /// Callers outside a session launch may omit it and receive a UUID-based
    /// id. Reusing a reserved id is idempotent and returns the existing
    /// monitor instead of spawning a duplicate side effect.
    pub process_id: Option<String>,
    /// Shell command to run. Exactly one of `command` / `ws` must be set
    /// (`command` empty + `ws` `None` is invalid, both set is invalid).
    pub command: String,
    /// WebSocket endpoint to watch instead of a command; text frames become
    /// events.
    pub ws: Option<MonitorWsParams>,
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
/// WebSocket endpoint parameters for a monitor.
pub struct MonitorWsParams {
    pub url: String,
    pub protocols: Vec<String>,
}

#[derive(Debug, Clone)]
/// Parameters for reading process output.
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

/// Observer hook called when a background process starts, emits an event, or
/// reaches a terminal state. Lets the runtime surface shell processes through
/// the unified background-activity registry — and forward each captured event
/// to the transcript as a `system_notification` part — without coupling the
/// monitor to the storage layer. Everything-is-a-part: the durable truth is
/// the parts the listener projects; the ring buffer here is only the live
/// projection.
#[allow(unused_variables)]
pub trait MonitorListener: Send + Sync + std::fmt::Debug {
    fn on_started(&self, summary: &ProcessSummary) {}
    /// Called for every captured event (`include_pattern`-filtered), with the
    /// monitor's live summary for correlation. The runtime's activity bridge
    /// forwards these as per-event `system_notification` parts.
    fn on_event(&self, event: &ProcessEvent, summary: &ProcessSummary) {}
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
    /// The source line shown in the transcript / activity panel: the command,
    /// or `ws <url>` for a WebSocket monitor.
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
    changed: Condvar,
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
    worker: Option<tokio::task::AbortHandle>,
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
/// Registry of monitored processes.
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
        // Exactly one of `command` / `ws` must be provided.
        let has_command = !params.command.trim().is_empty();
        let has_ws = params.ws.is_some();
        match (has_command, has_ws) {
            (false, false) => {
                return Err(MonitorError::Invalid(
                    "monitor requires exactly one of `command` or `ws`".into(),
                ));
            }
            (true, true) => {
                return Err(MonitorError::Invalid(
                    "monitor accepts only one of `command` or `ws`, not both".into(),
                ));
            }
            _ => {}
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
        // A WebSocket monitor displays its endpoint as the "command" line so
        // the transcript and activity panel identify the source.
        let source = params
            .ws
            .as_ref()
            .map(|ws| format!("ws {}", ws.url))
            .unwrap_or_else(|| params.command.clone());
        let process_id = params
            .process_id
            .clone()
            .unwrap_or_else(|| format!("proc_{}", Uuid::new_v4().simple()));
        if process_id.trim().is_empty() {
            return Err(MonitorError::Invalid(
                "reserved background process id must not be empty".into(),
            ));
        }
        let state = Arc::new(MonitorState {
            monitor_id: process_id,
            command: source,
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
                worker: None,
            }),
            changed: Condvar::new(),
            listener: self.listener.clone(),
        });

        // Reserve the identity before the worker can emit either an event or
        // completion. This closes the old fast-process race where callbacks
        // arrived before the registry (and therefore the durable coordinator)
        // could resolve their owner. A replay with the same durable id is a
        // no-op rather than a second process.
        {
            let mut monitors = self.monitors.lock().unwrap();
            if let Some(existing) = monitors.get(&state.monitor_id) {
                return Ok(MonitorStart {
                    summary: existing.snapshot(),
                });
            }
            monitors.insert(state.monitor_id.clone(), Arc::clone(&state));
        }
        let summary = state.snapshot();
        if let Some(listener) = &self.listener {
            listener.on_started(&summary);
        }

        let runner_state = Arc::clone(&state);
        let worker = if let Some(ws) = params.ws.clone() {
            let runner_ws = ws;
            let runner_quiet_period_ms = params.quiet_period_ms;
            handle.spawn(async move {
                run_ws_monitor(
                    runner_state,
                    runner_ws,
                    runner_quiet_period_ms,
                    timeout_ms,
                    abort_rx,
                )
                .await;
            })
        } else {
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
            })
        };
        {
            let mut inner = state.inner.lock().unwrap();
            if inner.status == ProcessStatus::Running {
                inner.worker = Some(worker.abort_handle());
            }
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
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        let mut inner = state.inner.lock().unwrap();
        loop {
            let read = collect_events_locked(&state, &inner, params.since_seq, limit);
            if !read.events.is_empty() || wait_ms == 0 || read.status != ProcessStatus::Running {
                return Ok(read);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(read);
            }
            let (next_inner, timeout) = state
                .changed
                .wait_timeout(inner, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            inner = next_inner;
            if timeout.timed_out() {
                return Ok(collect_events_locked(
                    &state,
                    &inner,
                    params.since_seq,
                    limit,
                ));
            }
        }
    }

    fn stop(&self, monitor_id: &str) -> Result<MonitorStopOutcome, MonitorError> {
        let state = self
            .lookup(monitor_id)
            .ok_or_else(|| MonitorError::NotFound(monitor_id.to_string()))?;
        {
            let mut inner = state.inner.lock().unwrap_or_else(|error| {
                tracing::error!(
                    diagnostic = %error,
                    monitor_id,
                    "recovering a poisoned monitor state while stopping it"
                );
                error.into_inner()
            });
            if inner.status == ProcessStatus::Running {
                inner.status = ProcessStatus::Stopped;
                inner.completion_reason = Some("explicit_stop".to_string());
                if let Some(tx) = inner.abort.take() {
                    if tx.send(()).is_err() {
                        tracing::debug!(monitor_id, "monitor abort receiver had already completed");
                    }
                }
            }
        }
        state.changed.notify_all();
        if let Some(listener) = state.listener.as_ref() {
            listener.on_finished(&state.snapshot());
        }
        Ok(MonitorStopOutcome {
            summary: state.snapshot(),
        })
    }
}

impl Drop for MonitorRegistry {
    fn drop(&mut self) {
        let monitors = self
            .monitors
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for state in monitors.values() {
            let mut inner = state
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(abort) = inner.abort.take() {
                if abort.send(()).is_err() {
                    tracing::debug!(
                        monitor_id = %state.monitor_id,
                        "monitor abort receiver had already completed during registry shutdown"
                    );
                }
            }
            // Let the runner receive the abort signal and terminate its whole
            // process tree. Aborting the task here would only drop the direct
            // child handle and could leave descendants alive with inherited
            // stdout/stderr pipes.
            inner.worker.take();
        }
    }
}

fn collect_events_locked(
    state: &MonitorState,
    inner: &MonitorInner,
    since_seq: u64,
    limit: usize,
) -> MonitorRead {
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
    MonitorRead {
        monitor_id: state.monitor_id.clone(),
        status,
        events,
        last_seq: last_seq_in_batch,
        has_more,
        dropped_lines: state.dropped_lines.load(Ordering::Acquire),
        exit_code,
        completion_reason: inner.completion_reason.clone(),
    }
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

    let mut child = match agena_process::spawn(cmd) {
        Ok(child) => child,
        Err(err) => {
            mark_failed(&state, format!("failed to spawn: {err}"));
            return;
        }
    };
    let stdout = child.stdout().take();
    let stderr = if capture_stderr {
        child.stderr().take()
    } else {
        None
    };
    let (condition_tx, mut condition_rx) = tokio::sync::mpsc::channel(2);

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
            Err(error) => TerminationCause::WaitError(
                agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to wait for the monitored process",
                    &error,
                ),
            ),
        },
    };

    let (mut final_status, final_exit_code, mut completion_reason) = match outcome {
        TerminationCause::Stopped => {
            terminate_monitored_child(&state, &mut child, ProcessStatus::Stopped, "explicit_stop")
                .await
        }
        TerminationCause::TimedOut => {
            terminate_monitored_child(&state, &mut child, ProcessStatus::TimedOut, "timeout").await
        }
        TerminationCause::Condition(PatternOutcome::Success) => {
            terminate_monitored_child(&state, &mut child, ProcessStatus::Exited, "success_pattern")
                .await
        }
        TerminationCause::Condition(PatternOutcome::Failure) => {
            terminate_monitored_child(&state, &mut child, ProcessStatus::Failed, "failure_pattern")
                .await
        }
        TerminationCause::Quiet => {
            terminate_monitored_child(&state, &mut child, ProcessStatus::Exited, "quiet_period")
                .await
        }
        TerminationCause::Exited(code) => (ProcessStatus::Exited, code, "process_exit".to_string()),
        TerminationCause::WaitError(reason) => {
            push_event(
                &state,
                ProcessStream::Stderr,
                format!("wait failed: {reason}"),
            );
            terminate_monitored_child(&state, &mut child, ProcessStatus::Failed, "wait_error").await
        }
    };

    // A shell may exit while a descendant still owns inherited pipes.
    // `process-wrap` targets the complete process group or Job Object.
    if let Err(error) = child.start_kill() {
        push_event(
            &state,
            ProcessStream::Stderr,
            agena_failure::diagnostic::format_error_chain_with_context(
                "failed to terminate the remaining monitored process tree",
                &error,
            ),
        );
        final_status = ProcessStatus::Failed;
        completion_reason = "process_tree_cleanup_failed".to_string();
    }
    join_stream_tasks(stdout_task, stderr_task).await;

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
        inner.worker = None;
    }
    state.changed.notify_all();
    if let Some(listener) = state.listener.as_ref() {
        listener.on_finished(&state.snapshot());
    }
}

/// Run a WebSocket monitor: each text frame becomes an event. The connection
/// stays open until stopped (abort), the timeout elapses, or the peer closes.
async fn run_ws_monitor(
    state: Arc<MonitorState>,
    ws: MonitorWsParams,
    quiet_period_ms: Option<u64>,
    timeout_ms: u64,
    abort_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut abort_rx = abort_rx;
    let connect = async {
        let mut request =
            tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
                ws.url.clone(),
            )
            .map_err(|error| format!("invalid websocket url: {error}"))?;
        if !ws.protocols.is_empty() {
            request.headers_mut().insert(
                tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_PROTOCOL,
                ws.protocols.join(", ").parse().expect("valid header value"),
            );
        }
        tokio_tungstenite::connect_async(request)
            .await
            .map(|(stream, _)| stream)
            .map_err(|error| format!("websocket connect failed: {error}"))
    };
    tokio::pin!(connect);
    let timeout_sleep = tokio::time::sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(timeout_sleep);
    let stream = tokio::select! {
        biased;
        _ = &mut abort_rx => {
            mark_ws_finished(&state, ProcessStatus::Stopped, "explicit_stop".to_string());
            return;
        }
        result = &mut connect => match result {
            Ok(stream) => stream,
            Err(error) => {
                push_event(&state, ProcessStream::Stderr, error);
                mark_ws_finished(&state, ProcessStatus::Failed, "ws_connect_failed".to_string());
                return;
            }
        },
        _ = &mut timeout_sleep => {
            mark_ws_finished(&state, ProcessStatus::TimedOut, "timeout".to_string());
            return;
        }
    };

    let quiet_wait = async {
        let Some(quiet_period_ms) = quiet_period_ms else {
            std::future::pending::<()>().await;
            return;
        };
        wait_for_quiet(&state, quiet_period_ms).await;
    };
    tokio::pin!(quiet_wait);

    // The timeout now covers the whole feed lifetime, not just the connect.
    let lifetime = async {
        let mut stream = stream;
        loop {
            match stream.next().await {
                Some(Ok(WsMessage::Text(text))) => {
                    push_event(&state, ProcessStream::Stdout, text.to_string());
                }
                Some(Ok(WsMessage::Binary(data))) => {
                    // Text frames are the event contract; binary frames are
                    // surfaced as an opaque marker so nothing is silently lost.
                    push_event(
                        &state,
                        ProcessStream::Stderr,
                        format!("[binary frame, {} bytes]", data.len()),
                    );
                }
                Some(Ok(WsMessage::Close(frame))) => {
                    let reason = frame
                        .map(|frame| frame.reason.to_string())
                        .unwrap_or_default();
                    push_event(
                        &state,
                        ProcessStream::Stderr,
                        format!("[ws closed {reason}]"),
                    );
                    return TerminationCause::Exited(None);
                }
                Some(Ok(WsMessage::Ping(payload))) => {
                    let _ = payload;
                    continue;
                }
                Some(Ok(WsMessage::Pong(_))) | Some(Ok(WsMessage::Frame(_))) => continue,
                Some(Err(error)) => {
                    push_event(
                        &state,
                        ProcessStream::Stderr,
                        format!("[ws error: {error}]"),
                    );
                    return TerminationCause::WaitError(error.to_string());
                }
                None => {
                    push_event(&state, ProcessStream::Stderr, "[ws closed]".to_string());
                    return TerminationCause::Exited(None);
                }
            }
        }
    };
    tokio::pin!(lifetime);

    let outcome = tokio::select! {
        biased;
        _ = &mut abort_rx => TerminationCause::Stopped,
        _ = &mut quiet_wait => TerminationCause::Quiet,
        _ = &mut timeout_sleep => TerminationCause::TimedOut,
        result = &mut lifetime => result,
    };
    let (final_status, completion_reason) = match outcome {
        TerminationCause::Stopped => (ProcessStatus::Stopped, "explicit_stop".to_string()),
        TerminationCause::TimedOut => (ProcessStatus::TimedOut, "timeout".to_string()),
        TerminationCause::Quiet => (ProcessStatus::Exited, "quiet_period".to_string()),
        TerminationCause::Exited(_) => (ProcessStatus::Exited, "ws_closed".to_string()),
        TerminationCause::WaitError(reason) => (ProcessStatus::Failed, reason),
        TerminationCause::Condition(_) => {
            unreachable!("ws monitor has no success/failure patterns")
        }
    };
    mark_ws_finished(&state, final_status, completion_reason);
}

/// Terminalize a ws monitor's state and notify the listener once.
fn mark_ws_finished(state: &MonitorState, status: ProcessStatus, completion_reason: String) {
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.status == ProcessStatus::Running {
            inner.status = status;
            inner.completion_reason = Some(completion_reason);
            inner.ended_at_ms = Some(Utc::now().timestamp_millis());
            inner.abort = None;
            inner.worker = None;
        }
    }
    state.changed.notify_all();
    if let Some(listener) = state.listener.as_ref() {
        listener.on_finished(&state.snapshot());
    }
}

async fn terminate_monitored_child(
    state: &MonitorState,
    child: &mut ManagedChild,
    success_status: ProcessStatus,
    completion_reason: &str,
) -> (ProcessStatus, Option<i32>, String) {
    match child.terminate(Duration::from_millis(150)).await {
        Ok(status) => (success_status, status.code(), completion_reason.to_string()),
        Err(error) => {
            push_event(
                state,
                ProcessStream::Stderr,
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!("failed to terminate monitored process ({completion_reason})"),
                    &error,
                ),
            );
            (
                ProcessStatus::Failed,
                None,
                format!("{completion_reason}_termination_failed"),
            )
        }
    }
}

async fn join_stream_tasks(
    mut stdout_task: Option<tokio::task::JoinHandle<()>>,
    mut stderr_task: Option<tokio::task::JoinHandle<()>>,
) {
    let stdout_abort = stdout_task
        .as_ref()
        .map(tokio::task::JoinHandle::abort_handle);
    let stderr_abort = stderr_task
        .as_ref()
        .map(tokio::task::JoinHandle::abort_handle);
    let joined = async {
        let stdout = async {
            match stdout_task.as_mut() {
                Some(handle) => handle.await,
                None => Ok(()),
            }
        };
        let stderr = async {
            match stderr_task.as_mut() {
                Some(handle) => handle.await,
                None => Ok(()),
            }
        };
        tokio::join!(stdout, stderr)
    };
    match tokio::time::timeout(Duration::from_secs(2), joined).await {
        Ok((stdout_result, stderr_result)) => {
            for (stream, result) in [("stdout", stdout_result), ("stderr", stderr_result)] {
                if let Err(error) = result {
                    tracing::error!(
                        target: "agena_runtime::monitor",
                        stream,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            format!("background process {stream} reader task failed"),
                            &error,
                        ),
                        "background process reader task failed"
                    );
                }
            }
        }
        Err(timeout_error) => {
            if let Some(abort) = stdout_abort {
                abort.abort();
            }
            if let Some(abort) = stderr_abort {
                abort.abort();
            }
            for (stream, task) in [
                ("stdout", stdout_task.take()),
                ("stderr", stderr_task.take()),
            ] {
                if let Some(task) = task
                    && let Err(error) = task.await
                    && !error.is_cancelled()
                {
                    tracing::error!(
                        target: "agena_runtime::monitor",
                        stream,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            format!("background process {stream} reader did not stop cleanly after abort"),
                            &error,
                        ),
                        "background process reader abort failed"
                    );
                }
            }
            tracing::warn!(
                target: "agena_runtime::monitor",
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "background process pipes did not close after 2 seconds",
                    &timeout_error,
                ),
                "background process reader tasks were aborted"
            );
        }
    }
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
    inner.worker = None;
    drop(inner);
    state.changed.notify_all();
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
    condition_tx: tokio::sync::mpsc::Sender<PatternOutcome>,
) where
    R: tokio::io::AsyncRead + Unpin + Send,
{
    let mut reader = FramedRead::new(
        reader,
        LinesCodec::new_with_max_length(READER_LINE_BYTE_CAP),
    );
    let mut resume_after_decoder_error = false;
    loop {
        match reader.next().await {
            // FramedRead returns one transitional `None` after a decoder
            // error. LinesCodec itself remains recoverable and discards the
            // rest of the overlong line, so poll it again instead of treating
            // that transitional value as pipe EOF.
            None if resume_after_decoder_error => {
                resume_after_decoder_error = false;
                continue;
            }
            None => break,
            Some(Ok(line)) => {
                resume_after_decoder_error = false;
                state
                    .last_activity_ms
                    .store(Utc::now().timestamp_millis(), Ordering::Release);
                if failure
                    .as_ref()
                    .is_some_and(|pattern| pattern.is_match(&line))
                {
                    if let Err(error) = condition_tx.try_send(PatternOutcome::Failure) {
                        tracing::debug!(
                            diagnostic = %error,
                            "monitor failure-pattern outcome was already queued or no longer observed"
                        );
                    }
                } else if success
                    .as_ref()
                    .is_some_and(|pattern| pattern.is_match(&line))
                {
                    if let Err(error) = condition_tx.try_send(PatternOutcome::Success) {
                        tracing::debug!(
                            diagnostic = %error,
                            "monitor success-pattern outcome was already queued or no longer observed"
                        );
                    }
                }
                if let Some(re) = include.as_ref()
                    && !re.is_match(&line)
                {
                    continue;
                }
                push_event(&state, stream, line);
            }
            Some(Err(error)) => {
                resume_after_decoder_error = true;
                push_event(
                    &state,
                    ProcessStream::Stderr,
                    format!(
                        "background output line was discarded after exceeding the {READER_LINE_BYTE_CAP}-byte UTF-8 line limit: {error}"
                    ),
                );
            }
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
        inner.buffer.push_back(event.clone());
    }
    state.changed.notify_all();
    // Forward the event to the observer so the runtime can project it into the
    // transcript as a `system_notification` part (everything-is-a-part).
    if let Some(listener) = state.listener.as_ref() {
        listener.on_event(&event, &state.snapshot());
    }
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

    fn process_exists(pid: i32) -> bool {
        // SAFETY: signal 0 only checks existence/permission.
        (unsafe { libc::kill(pid, 0) } == 0)
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }

    fn start_params(command: &str) -> StartParams {
        StartParams {
            process_id: None,
            command: command.to_string(),
            ws: None,
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

    async fn wait_for_pid_file(path: &std::path::Path) -> i32 {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(path)
                    && let Ok(pid) = text.trim().parse::<i32>()
                {
                    return pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("monitor publishes a complete pid")
    }

    async fn wait_for_terminal(registry: Arc<MonitorRegistry>, id: String) -> MonitorRead {
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut since_seq = 0;
            let mut events = Vec::new();
            loop {
                let registry = Arc::clone(&registry);
                let id = id.clone();
                let read = tokio::task::spawn_blocking(move || {
                    registry
                        .read(ReadParams {
                            monitor_id: id,
                            since_seq,
                            limit: Some(32),
                            wait_ms: 250,
                        })
                        .expect("read monitor")
                })
                .await
                .expect("join read");
                let mut read = read;
                since_seq = read.last_seq;
                events.append(&mut read.events);
                if read.status != ProcessStatus::Running {
                    read.events = events;
                    return read;
                }
            }
        })
        .await
        .expect("monitor reaches terminal state")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reserved_process_identity_is_visible_before_work_and_replay_is_idempotent() {
        let registry = Arc::new(MonitorRegistry::from_handle(Handle::current()));
        let mut params = start_params("printf 'done\\n'");
        params.process_id = Some("proc_reserved_identity".to_owned());
        let first = registry
            .start(params.clone())
            .expect("start reserved process");
        assert_eq!(first.summary.process_id, "proc_reserved_identity");
        let replay = registry.start(params).expect("replay reserved process");
        assert_eq!(replay.summary.process_id, "proc_reserved_identity");
        assert_eq!(
            registry
                .list()
                .iter()
                .filter(|summary| summary.process_id == "proc_reserved_identity")
                .count(),
            1,
            "replaying a durable launch id must not spawn a duplicate process"
        );
        let terminal =
            wait_for_terminal(Arc::clone(&registry), "proc_reserved_identity".to_owned()).await;
        assert_ne!(terminal.status, ProcessStatus::Running);
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_registry_aborts_persistent_process() {
        let pid_path = std::env::temp_dir().join(format!(
            "agena-monitor-drop-{}-{}.pid",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let registry = MonitorRegistry::from_handle(Handle::current());
        let mut params = start_params(
            format!("echo $$ > {}; exec sleep 30", pid_path.to_string_lossy()).as_str(),
        );
        params.persistent = true;
        params.timeout_ms = None;
        registry.start(params).expect("start persistent monitor");

        let pid = wait_for_pid_file(&pid_path).await;

        drop(registry);
        tokio::time::timeout(Duration::from_secs(2), async {
            while process_exists(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("registry drop kills the persistent process");
        let _ = std::fs::remove_file(pid_path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stopping_monitor_kills_shell_descendants() {
        let pid_path = std::env::temp_dir().join(format!(
            "agena-monitor-descendant-{}-{}.pid",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let registry = Arc::new(MonitorRegistry::from_handle(Handle::current()));
        let mut params = start_params(
            format!("sleep 30 & echo $! > {}; wait", pid_path.to_string_lossy()).as_str(),
        );
        params.persistent = true;
        params.timeout_ms = None;
        let id = registry
            .start(params)
            .expect("start monitor")
            .summary
            .process_id;

        let pid = wait_for_pid_file(&pid_path).await;
        registry.stop(&id).expect("stop monitor");
        tokio::time::timeout(Duration::from_secs(2), async {
            while process_exists(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("monitor stop should terminate descendants");
        let _ = std::fs::remove_file(pid_path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn oversized_output_line_is_bounded_and_following_lines_are_observed() {
        let registry = Arc::new(MonitorRegistry::from_handle(Handle::current()));
        let mut params = start_params(
            "head -c 100000 /dev/zero | LC_ALL=C tr '\\000' x; printf '\\nREADY\\n'; exec sleep 30",
        );
        params.success_pattern = Some("^READY$".to_string());
        let id = registry.start(params).expect("start").summary.process_id;
        let read = wait_for_terminal(registry, id).await;

        assert_eq!(read.completion_reason.as_deref(), Some("success_pattern"));
        assert!(read.events.iter().any(|event| event.line == "READY"));
        assert!(read.events.iter().any(|event| {
            event
                .line
                .contains("discarded after exceeding the 65536-byte UTF-8 line limit")
        }));
    }
}
