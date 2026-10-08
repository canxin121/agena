//! Runtime background-process / monitor registry.
//!
//! A background process runs a long-lived shell command and captures every
//! stdout/stderr line as a numbered event; a monitor may alternatively watch a
//! WebSocket feed whose text frames become events. The public model-visible
//! tool surface is shell.spawn / shell.watch / shell.logs / shell.stop plus
//! WebSocket monitor.start / monitor.stop, backed by this registry.
//!
//! Captured events live in a ring buffer (default 1000 lines) so the model can
//! walk forward through history without losing recent activity. Lines that get
//! evicted are counted in `dropped_lines` so the model knows it missed
//! something. A [`MonitorListener`] receives start/finish transitions and
//! selected events as they arrive — the runtime coalesces watched-command and
//! WebSocket events into bounded system_notification batches. Ordinary jobs
//! notify only on completion. The ring buffer retains diagnostic output even
//! when an event is excluded from notifications.
//!
//! # Concurrency model
//!
//! The process runner, pipe readers, WebSocket reader, timeout, cancellation,
//! and child wait all run on the caller's Tokio runtime. The registry also has
//! synchronous readers for non-async consumers; those reads sleep on a condition
//! variable and async callers isolate them with `spawn_blocking`.

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
use uuid::Uuid;

use crate::part::{ShellMonitorPatternKind, ShellWatchNotifications, ShellWatchPolicy};
use crate::process_output::{OutputCursor, select_events, text_budget};
use agena_domain::{ProcessEvent, ProcessStatus, ProcessStream, ProcessSummary};
use agena_process::ManagedChild;
#[path = "monitor/watch.rs"]
mod watch;
use watch::{OutputMatcher, WatchState};

const DEFAULT_BUFFER_LINES: usize = 1_000;
const MAX_BUFFER_LINES: usize = 10_000;
const MAX_BUFFER_BYTES: usize = 1024 * 1024;
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// Parameters for starting a monitored process.
pub struct StartParams {
    pub output: Option<agena_storage::content::ContentWriter>,
    /// Trusted argv overrides the display command without another outer shell.
    pub argv: Option<Vec<String>>,
    /// Trusted workspace/session identity. None is host-only and invisible to AI tools.
    pub owner: Option<crate::TerminalOwner>,
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
    pub watch: Option<ShellWatchPolicy>,
    pub include_pattern: Option<String>,
    pub success_pattern: Option<String>,
    pub failure_pattern: Option<String>,
    pub quiet_period_ms: Option<u64>,
    pub max_buffered_lines: Option<u32>,
    pub capture_stderr: bool,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Model-facing bounded read; explicit cursors replay, omitted cursors consume.
#[derive(Debug, Clone, Default)]
pub struct ProcessReadOptions {
    pub since_seq: Option<u64>,
    pub event_offset: u32,
    pub limit: Option<u32>,
    pub wait_ms: u64,
    pub max_output_bytes: Option<u32>,
}

/// Outcome of a `read` action.
#[derive(Debug, Clone)]
pub struct MonitorRead {
    pub monitor_id: String,
    pub output_resource: Option<agena_domain::ContentRef>,
    pub status: ProcessStatus,
    pub ready: bool,
    pub events: Vec<ProcessEvent>,
    pub last_seq: u64,
    pub next_event_offset: u32,
    pub has_more: bool,
    pub dropped_lines: u64,
    pub exit_code: Option<i32>,
    pub completion_reason: Option<String>,
}

/// Outcome of a `start` action.
#[derive(Debug, Clone)]
pub struct MonitorStart {
    pub summary: ProcessSummary,
    /// The launch identity already existed. Cancelling a replay only cancels
    /// its wait; it must not terminate the original caller's background job.
    pub reused: bool,
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
    /// Replacing/removing a watch invalidates queued notifications from the
    /// old policy, while leaving the process's completion route intact.
    fn on_watch_changed(&self, summary: &ProcessSummary) {
        self.on_started(summary);
    }
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
    /// Native terminal sessions share this service's lifetime and observer.
    fn terminals(&self) -> Option<&crate::TerminalRegistry> {
        None
    }
    /// Unscoped methods are for trusted runtime administration, never model input.
    fn is_owned(&self, _id: &str, _owner: &crate::TerminalOwner) -> bool {
        false
    }
    fn stop_session(&self, _session: i64) {}
    fn shutdown(&self) {}
    fn start(&self, params: StartParams) -> Result<MonitorStart, MonitorError>;
    /// Called only on a blocking worker. Return after actual process creation,
    /// not merely after scheduling its Tokio runner.
    fn start_confirmed(
        &self,
        _params: StartParams,
        _cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<MonitorStart, MonitorError> {
        Err(MonitorError::Invalid(
            "confirmed background launches are unavailable".into(),
        ))
    }
    fn list(&self) -> Vec<ProcessSummary>;
    fn configure_watch(
        &self,
        _id: &str,
        _policy: Option<ShellWatchPolicy>,
        _since_seq: Option<u64>,
    ) -> Result<ProcessSummary, MonitorError> {
        Err(MonitorError::Invalid(
            "watch configuration is unavailable for this process".into(),
        ))
    }
    fn read_output_cancellable(
        &self,
        id: &str,
        options: ProcessReadOptions,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<MonitorRead, MonitorError> {
        crate::process_output::validate_cursor(options.since_seq, options.event_offset)?;
        let mut read = self.read_cancellable(
            ReadParams {
                monitor_id: id.into(),
                since_seq: options.since_seq.unwrap_or(0),
                limit: options.limit,
                wait_ms: options.wait_ms,
            },
            cancel,
        )?;
        let slice = select_events(
            read.events.iter(),
            OutputCursor {
                seq: options.since_seq.unwrap_or(0),
                offset: options.event_offset,
            },
            read.last_seq,
            MAX_READ_LIMIT,
            text_budget(options.max_output_bytes, false),
            true,
            options.since_seq.is_some(),
            crate::process_output::OutputSelection::Resumable,
        )?;
        read.events = slice.events;
        read.last_seq = slice.cursor.seq;
        read.next_event_offset = slice.cursor.offset;
        read.has_more |= slice.has_more;
        Ok(read)
    }
    fn read(&self, params: ReadParams) -> Result<MonitorRead, MonitorError>;
    fn read_cancellable(
        &self,
        mut params: ReadParams,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<MonitorRead, MonitorError> {
        let deadline = Instant::now() + Duration::from_millis(params.wait_ms.min(30_000));
        loop {
            if cancel.is_cancelled() {
                return Err(MonitorError::Invalid("shell log read cancelled".into()));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            params.wait_ms = remaining.as_millis().min(50) as u64;
            let read = self.read(params.clone())?;
            if !read.events.is_empty()
                || read.status != ProcessStatus::Running
                || Instant::now() >= deadline
            {
                return Ok(read);
            }
        }
    }
    fn stop(&self, monitor_id: &str) -> Result<MonitorStopOutcome, MonitorError>;
}

#[derive(Debug)]
struct MonitorState {
    owner: Option<crate::TerminalOwner>,
    launch: StartParams,
    monitor_id: String,
    /// The source line shown in the transcript / activity panel: the command,
    /// or `ws <url>` for a WebSocket monitor.
    command: String,
    description: String,
    started_at_ms: i64,
    watch: Mutex<WatchState>,
    watch_changed: tokio::sync::Notify,
    interaction: Mutex<()>,
    last_activity: Mutex<Instant>,
    last_activity_ms: AtomicI64,
    capacity: usize,
    /// Latest assigned seq (0 means no events yet).
    last_seq: AtomicU64,
    notification_seq: AtomicU64,
    notification_delivery: Mutex<()>,
    /// Cumulative count of evicted lines.
    dropped_lines: AtomicU64,
    inner: Mutex<MonitorInner>,
    changed: Condvar,
    /// Optional observer notified on start/finish transitions.
    listener: Option<Arc<dyn MonitorListener>>,
}

#[derive(Debug)]
struct MonitorInner {
    spawned: bool,
    ending: bool,
    cursor: OutputCursor,
    buffer: VecDeque<ProcessEvent>,
    buffered_bytes: usize,
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
            output_resource: self
                .launch
                .output
                .as_ref()
                .map(|writer| writer.resource().reference()),
            tty: false,
            websocket: self.launch.ws.is_some(),
            command: self.command.clone(),
            description: self.description.clone(),
            status: inner.status,
            background: true,
            monitored: self.watch.lock().unwrap().policy.is_some() || self.launch.ws.is_some(),
            ready: self.watch.lock().unwrap().ready,
            started_at_ms: self.started_at_ms,
            ended_at_ms: inner.ended_at_ms,
            buffered_lines: inner.buffer.len() as u32,
            last_seq: self.last_seq.load(Ordering::Acquire),
            dropped_lines: self.dropped_lines.load(Ordering::Acquire),
            exit_code: inner.exit_code,
            completion_reason: inner.completion_reason.clone(),
            owner_session_id: self.owner.as_ref().and_then(|owner| owner.session_id),
        }
    }
}

#[derive(Debug)]
/// Registry of monitored processes.
pub struct MonitorRegistry {
    terminals: crate::TerminalRegistry,
    handle: Option<Handle>,
    monitors: Mutex<HashMap<String, Arc<MonitorState>>>,
    closed: std::sync::atomic::AtomicBool,
    listener: Option<Arc<dyn MonitorListener>>,
}

impl Default for MonitorRegistry {
    fn default() -> Self {
        Self {
            handle: Handle::try_current().ok(),
            terminals: crate::TerminalRegistry::default(),
            monitors: Mutex::new(HashMap::new()),
            closed: std::sync::atomic::AtomicBool::new(false),
            listener: None,
        }
    }
}

impl MonitorRegistry {
    pub fn from_handle(handle: Handle) -> Self {
        Self {
            terminals: crate::TerminalRegistry::from_handle(handle.clone()),
            handle: Some(handle),
            monitors: Mutex::new(HashMap::new()),
            closed: std::sync::atomic::AtomicBool::new(false),
            listener: None,
        }
    }

    /// Attach an observer notified when processes start or finish. Only one
    /// listener is supported per registry; later calls replace the previous
    /// one.
    pub fn with_monitor_listener(mut self, listener: Arc<dyn MonitorListener>) -> Self {
        self.terminals.set_listener(Arc::clone(&listener));
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
    fn terminals(&self) -> Option<&crate::TerminalRegistry> {
        Some(&self.terminals)
    }
    fn is_owned(&self, id: &str, owner: &crate::TerminalOwner) -> bool {
        if self.terminals.contains(id) {
            return self.terminals.is_owned(id, owner);
        }
        self.lookup(id)
            .is_some_and(|state| state.owner.as_ref() == Some(owner))
    }
    fn stop_session(&self, session: i64) {
        self.terminals.stop_session(session);
        let states: Vec<_> = self
            .monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .filter(|state| {
                state
                    .owner
                    .as_ref()
                    .is_some_and(|owner| owner.session_id == Some(session))
            })
            .cloned()
            .collect();
        for state in states {
            request_abort(&state);
        }
    }
    fn shutdown(&self) {
        self.terminals.shutdown();
        let states = {
            let monitors = self
                .monitors
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.closed.store(true, Ordering::Release);
            monitors.values().cloned().collect::<Vec<_>>()
        };
        for state in states {
            request_abort(&state);
        }
    }
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
        let timeout_ms =
            params
                .timeout_ms
                .unwrap_or(DEFAULT_TIMEOUT_MS)
                .min(if params.ws.is_some() {
                    MAX_TIMEOUT_MS
                } else {
                    86_400_000
                });
        if !params.persistent && timeout_ms == 0 {
            return Err(MonitorError::Invalid(
                "non-persistent monitors must have timeout_ms > 0".into(),
            ));
        }
        let capacity = params
            .max_buffered_lines
            .map(|n| (n as usize).clamp(1, MAX_BUFFER_LINES))
            .unwrap_or(DEFAULT_BUFFER_LINES);
        let policy = params.watch.clone().or_else(|| {
            params.monitored.then(|| ShellWatchPolicy {
                include_pattern: params.include_pattern.clone(),
                success_pattern: params.success_pattern.clone(),
                failure_pattern: params.failure_pattern.clone(),
                quiet_period_ms: params.quiet_period_ms,
                ..Default::default()
            })
        });
        let watch = WatchState::compile(policy, 0)?;

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
            owner: params.owner.clone(),
            launch: StartParams {
                output: params
                    .output
                    .as_ref()
                    .map(agena_storage::content::ContentWriter::delegate_lifecycle),
                ..params.clone()
            },
            monitor_id: process_id,
            command: source,
            description: params.description.clone(),
            started_at_ms,
            watch: Mutex::new(watch),
            watch_changed: tokio::sync::Notify::new(),
            interaction: Mutex::new(()),
            last_activity: Mutex::new(Instant::now()),
            last_activity_ms: AtomicI64::new(started_at_ms),
            capacity,
            last_seq: AtomicU64::new(0),
            notification_seq: AtomicU64::new(0),
            notification_delivery: Mutex::new(()),
            dropped_lines: AtomicU64::new(0),
            inner: Mutex::new(MonitorInner {
                spawned: false,
                ending: false,
                cursor: OutputCursor::default(),
                buffer: VecDeque::with_capacity(capacity.min(256)),
                buffered_bytes: 0,
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
            if self.closed.load(Ordering::Acquire) {
                return Err(MonitorError::Invalid(
                    "process registry is shut down".into(),
                ));
            }
            if let Some(existing) = monitors.get(&state.monitor_id) {
                if existing.owner != params.owner {
                    return Err(MonitorError::NotFound(state.monitor_id.clone()));
                }
                if existing.launch != params {
                    return Err(MonitorError::Invalid(
                        "process id already belongs to a different launch; do not reuse it".into(),
                    ));
                }
                return Ok(MonitorStart {
                    summary: existing.snapshot(),
                    reused: true,
                });
            }
            if monitors
                .values()
                .filter(|state| state.snapshot().status == ProcessStatus::Running)
                .count()
                >= 64
            {
                return Err(MonitorError::Invalid(
                    "at most 64 ordinary background processes may run concurrently".into(),
                ));
            }
            if monitors.len() >= 256 {
                let candidate = monitors
                    .iter()
                    .filter(|(_, state)| state.snapshot().status != ProcessStatus::Running)
                    .min_by_key(|(_, state)| state.started_at_ms)
                    .map(|(id, _)| id.clone());
                if let Some(id) = candidate {
                    monitors.remove(&id);
                }
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
            handle.spawn(async move {
                run_monitor(
                    runner_state,
                    params.argv,
                    runner_command,
                    runner_workdir,
                    runner_env,
                    runner_capture_stderr,
                    runner_persistent,
                    timeout_ms,
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

        Ok(MonitorStart {
            summary,
            reused: false,
        })
    }

    fn list(&self) -> Vec<ProcessSummary> {
        let guard = self.monitors.lock().unwrap();
        let mut out: Vec<ProcessSummary> = guard.values().map(|s| s.snapshot()).collect();
        out.extend(self.terminals.list());
        out.sort_by_key(|summary| summary.started_at_ms);
        out
    }

    fn configure_watch(
        &self,
        id: &str,
        policy: Option<ShellWatchPolicy>,
        since_seq: Option<u64>,
    ) -> Result<ProcessSummary, MonitorError> {
        let state = self
            .lookup(id)
            .ok_or_else(|| MonitorError::NotFound(id.into()))?;
        if state.launch.ws.is_some() {
            return Err(MonitorError::Invalid("shell.watch configures noninteractive shell processes; use monitor tools for WebSocket subscriptions".into()));
        }
        // Compile before touching live state: invalid updates leave the old
        // policy intact. No global registry lock spans compilation or replay.
        let mut next = WatchState::compile(policy, 0)?;
        let delivery = state.notification_delivery.lock().unwrap();
        let retained = {
            let inner = state.inner.lock().unwrap();
            if inner.status != ProcessStatus::Running || inner.ending {
                return Err(MonitorError::Invalid(
                    "process has ended or begun termination; read its output instead of configuring a watch".into(),
                ));
            }
            let latest = state.last_seq.load(Ordering::Acquire);
            if since_seq.is_some_and(|seq| seq > latest) {
                return Err(MonitorError::Invalid(
                    "watch replay cursor is ahead of this process".into(),
                ));
            }
            let mut watch = state.watch.lock().unwrap();
            // Repeated attachment with the same configuration is a no-op;
            // it cannot reset once-only notifications or readiness.
            if watch.policy == next.policy {
                drop(watch);
                drop(inner);
                return Ok(state.snapshot());
            }
            next.revision = watch.revision.wrapping_add(1);
            next.since_seq = since_seq.unwrap_or(latest);
            next.configured_at = Instant::now();
            *watch = next;
            since_seq
                .map(|since| {
                    inner
                        .buffer
                        .iter()
                        .filter(|event| event.seq > since)
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        state.watch_changed.notify_waiters();
        if let Some(listener) = &state.listener {
            listener.on_watch_changed(&state.snapshot());
        }
        drop(delivery);
        let mut stdout = OutputMatcher::default();
        let mut stderr = OutputMatcher::default();
        for event in &retained {
            match event.stream {
                ProcessStream::Stdout => stdout.push(&state, event, false),
                ProcessStream::Stderr => stderr.push(&state, event, false),
            }
        }
        Ok(state.snapshot())
    }

    fn read_output_cancellable(
        &self,
        id: &str,
        options: ProcessReadOptions,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<MonitorRead, MonitorError> {
        crate::process_output::validate_cursor(options.since_seq, options.event_offset)?;
        let state = self
            .lookup(id)
            .ok_or_else(|| MonitorError::NotFound(id.into()))?;
        // Only one consuming read per process; explicit replay uses the same
        // lock so cursor validation and consumption are a single operation.
        let _interaction = crate::terminal::call::interaction(&state.interaction, cancel)?;
        let mut inner = state.inner.lock().unwrap();
        let cursor = options
            .since_seq
            .map(|seq| OutputCursor {
                seq,
                offset: options.event_offset,
            })
            .unwrap_or(inner.cursor);
        if cursor.seq > state.last_seq.load(Ordering::Acquire) {
            return Err(MonitorError::Invalid(
                "output cursor is ahead of this process".into(),
            ));
        }
        let deadline = Instant::now() + Duration::from_millis(options.wait_ms.min(30_000));
        loop {
            if cancel.is_cancelled() {
                return Err(MonitorError::Invalid("shell output read cancelled".into()));
            }
            if state.last_seq.load(Ordering::Acquire) > cursor.seq
                || inner.status != ProcessStatus::Running
                || Instant::now() >= deadline
            {
                break;
            }
            let remaining = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50));
            inner = state
                .changed
                .wait_timeout(inner, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        let slice = select_events(
            inner.buffer.iter(),
            cursor,
            state.last_seq.load(Ordering::Acquire),
            options
                .limit
                .unwrap_or(DEFAULT_READ_LIMIT as u32)
                .clamp(1, MAX_READ_LIMIT as u32) as usize,
            text_budget(options.max_output_bytes, false),
            true,
            options.since_seq.is_some(),
            crate::process_output::OutputSelection::Resumable,
        )?;
        if options.since_seq.is_none() {
            inner.cursor = slice.cursor;
        }
        Ok(MonitorRead {
            monitor_id: id.into(),
            output_resource: state
                .launch
                .output
                .as_ref()
                .map(|writer| writer.resource().reference()),
            status: inner.status,
            ready: state.watch.lock().unwrap().ready,
            events: slice.events,
            last_seq: slice.cursor.seq,
            next_event_offset: slice.cursor.offset,
            has_more: slice.has_more,
            dropped_lines: state.dropped_lines.load(Ordering::Acquire),
            exit_code: inner.exit_code,
            completion_reason: inner.completion_reason.clone(),
        })
    }

    fn start_confirmed(
        &self,
        params: StartParams,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<MonitorStart, MonitorError> {
        if cancel.is_cancelled() {
            return Err(MonitorError::Invalid("background launch cancelled".into()));
        }
        let started = self.start(params)?;
        let state = self
            .lookup(&started.summary.process_id)
            .ok_or_else(|| MonitorError::NotFound(started.summary.process_id.clone()))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut inner = state
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if cancel.is_cancelled() || Instant::now() >= deadline {
                drop(inner);
                if !started.reused {
                    self.stop(&state.monitor_id)?;
                }
                return Err(MonitorError::Invalid(
                    "background launch cancelled or startup confirmation timed out".into(),
                ));
            }
            if inner.spawned {
                drop(inner);
                return Ok(MonitorStart {
                    summary: state.snapshot(),
                    reused: started.reused,
                });
            }
            if inner.status != ProcessStatus::Running {
                return Err(MonitorError::Invalid(
                    inner
                        .completion_reason
                        .clone()
                        .unwrap_or_else(|| "background process failed to start".into()),
                ));
            }
            inner = state
                .changed
                .wait_timeout(inner, Duration::from_millis(50))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }

    fn read(&self, params: ReadParams) -> Result<MonitorRead, MonitorError> {
        if self.terminals.contains(&params.monitor_id) {
            let read = self.terminals.read_unscoped(
                &params.monitor_id,
                params.since_seq,
                params.wait_ms.min(30_000),
                params.limit,
            )?;
            return Ok(MonitorRead {
                output_resource: read.summary.output_resource,
                monitor_id: read.summary.process_id,
                status: read.summary.status,
                ready: read.summary.ready,
                events: read.events,
                last_seq: read.last_seq,
                next_event_offset: read.next_event_offset,
                has_more: read.has_more,
                dropped_lines: read.summary.dropped_lines,
                exit_code: read.summary.exit_code,
                completion_reason: read.summary.completion_reason,
            });
        }

        let state = self
            .lookup(&params.monitor_id)
            .ok_or_else(|| MonitorError::NotFound(params.monitor_id.clone()))?;
        if params.since_seq > state.last_seq.load(Ordering::Acquire) {
            return Err(MonitorError::Invalid(
                "output cursor is ahead of this background job".into(),
            ));
        }
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
        if self.terminals.contains(monitor_id) {
            return self
                .terminals
                .stop_unscoped(monitor_id)
                .map(|summary| MonitorStopOutcome { summary });
        }
        let state = self
            .lookup(monitor_id)
            .ok_or_else(|| MonitorError::NotFound(monitor_id.into()))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut inner = state
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.status == ProcessStatus::Running
            && let Some(sender) = inner.abort.take()
        {
            let _ = sender.send(());
        }
        while inner.status == ProcessStatus::Running {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(MonitorError::Invalid("stop requested but termination is not confirmed; inspect shell.logs/list before retrying".into()));
            }
            let (updated, _) = state
                .changed
                .wait_timeout(inner, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            inner = updated;
        }
        drop(inner);
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
            if let Some(abort) = inner.abort.take()
                && abort.send(()).is_err()
            {
                tracing::debug!(
                    monitor_id = %state.monitor_id,
                    "monitor abort receiver had already completed during registry shutdown"
                );
            }
            // Let the runner receive the abort signal and terminate its whole
            // process tree. Aborting the task here would only drop the direct
            // child handle and could leave descendants alive with inherited
            // stdout/stderr pipes.
            inner.worker.take();
        }
    }
}

fn request_abort(state: &MonitorState) {
    let mut inner = state
        .inner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(sender) = inner.abort.take() {
        let _ = sender.send(());
    }
    state.changed.notify_all();
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
    let global_last = state.last_seq.load(Ordering::Acquire);
    let last_seq_in_batch = events
        .last()
        .map(|e| e.seq)
        .unwrap_or(global_last.max(since_seq));
    let has_more = global_last > last_seq_in_batch;
    MonitorRead {
        monitor_id: state.monitor_id.clone(),
        output_resource: state
            .launch
            .output
            .as_ref()
            .map(|writer| writer.resource().reference()),
        status,
        ready: state.watch.lock().unwrap().ready,
        events,
        last_seq: last_seq_in_batch,
        next_event_offset: 0,
        has_more,
        dropped_lines: state.dropped_lines.load(Ordering::Acquire),
        exit_code,
        completion_reason: inner.completion_reason.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_monitor(
    state: Arc<MonitorState>,
    argv: Option<Vec<String>>,
    command: String,
    workdir: std::path::PathBuf,
    env: HashMap<String, String>,
    capture_stderr: bool,
    persistent: bool,
    timeout_ms: u64,
    abort_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut cmd = match argv {
        Some(argv) => {
            let Some((program, args)) = argv
                .split_first()
                .filter(|(program, _)| !program.is_empty())
            else {
                mark_failed(&state, "empty explicit process command".into());
                return;
            };
            let mut cmd = Command::new(program);
            cmd.args(args);
            cmd
        }
        None => build_command(&command),
    };
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
    {
        let mut inner = state
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.spawned = true;
    }
    *state.last_activity.lock().unwrap() = Instant::now();
    state.changed.notify_all();
    let stdout = child.stdout().take();
    let stderr = if capture_stderr {
        child.stderr().take()
    } else {
        None
    };
    let stdout_task = stdout.map(|reader| {
        tokio::spawn(stream_lines(
            Arc::clone(&state),
            reader,
            ProcessStream::Stdout,
        ))
    });
    let stderr_task = stderr.map(|reader| {
        tokio::spawn(stream_lines(
            Arc::clone(&state),
            reader,
            ProcessStream::Stderr,
        ))
    });

    // The registry can abort this runner during shutdown. Scope its readers
    // as well, even if a descendant retains a pipe after the child is killed.
    struct ReaderGuard(Vec<tokio::task::AbortHandle>);
    impl Drop for ReaderGuard {
        fn drop(&mut self) {
            for reader in &self.0 {
                reader.abort();
            }
        }
    }
    let notification_task = tokio::spawn(watch::deliver_notifications(Arc::clone(&state)));
    let _readers = ReaderGuard(
        stdout_task
            .iter()
            .chain(stderr_task.iter())
            .map(tokio::task::JoinHandle::abort_handle)
            .chain(std::iter::once(notification_task.abort_handle()))
            .collect(),
    );

    let mut abort_rx = abort_rx;

    let timeout_sleep = if persistent {
        None
    } else {
        Some(tokio::time::sleep(Duration::from_millis(timeout_ms)))
    };
    tokio::pin!(timeout_sleep);
    let outcome = loop {
        let quiet_wait = watch::wait_for_quiet(&state);
        tokio::pin!(quiet_wait);
        let outcome = tokio::select! {
        biased;
        _ = &mut abort_rx => TerminationCause::Stopped,
        (condition, revision) = watch::wait_for_condition(&state) => TerminationCause::Condition(condition, revision),
        revision = &mut quiet_wait => TerminationCause::WatchQuiet(revision),
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
        // Commit termination under the same state/configuration lock order
        // used by watch replacement. A removed/changed policy cannot kill the
        // process through an already-ready old quiet/condition future.
        let mut inner = state.inner.lock().unwrap();
        let watch = state.watch.lock().unwrap();
        match &outcome {
            TerminationCause::Condition(_, revision)
                if *revision != watch.revision || watch.outcome == 0 =>
            {
                continue;
            }
            TerminationCause::WatchQuiet(revision) => {
                let quiet = watch
                    .policy
                    .as_ref()
                    .and_then(|policy| policy.quiet_period_ms);
                let last = (*state.last_activity.lock().unwrap()).max(watch.configured_at);
                if *revision != watch.revision
                    || quiet.is_none_or(|ms| last.elapsed() < Duration::from_millis(ms))
                {
                    continue;
                }
            }
            _ => {}
        }
        inner.ending = true;
        break outcome;
    };

    let (mut final_status, final_exit_code, mut completion_reason) = match outcome {
        TerminationCause::Stopped => {
            terminate_monitored_child(&state, &mut child, ProcessStatus::Stopped, "explicit_stop")
                .await
        }
        TerminationCause::TimedOut => {
            terminate_monitored_child(&state, &mut child, ProcessStatus::TimedOut, "timeout").await
        }
        TerminationCause::Condition(PatternOutcome::Success, _) => {
            terminate_monitored_child(&state, &mut child, ProcessStatus::Exited, "success_pattern")
                .await
        }
        TerminationCause::Condition(PatternOutcome::Failure, _) => {
            terminate_monitored_child(&state, &mut child, ProcessStatus::Failed, "failure_pattern")
                .await
        }
        TerminationCause::WatchQuiet(_) | TerminationCause::Quiet => {
            terminate_monitored_child(&state, &mut child, ProcessStatus::Exited, "quiet_period")
                .await
        }
        TerminationCause::Exited(code) => (
            if code == Some(0) {
                ProcessStatus::Exited
            } else {
                ProcessStatus::Failed
            },
            code,
            "process_exit".to_string(),
        ),
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
    notification_task.abort();
    watch::flush_pending(&state);
    // Fast commands may exit before their final output is framed. Reconcile
    // observed conditions after draining, without masking stop/timeout/cleanup
    // failures or a nonzero natural exit with a late success pattern.
    if matches!(
        completion_reason.as_str(),
        "process_exit" | "success_pattern" | "quiet_period"
    ) {
        match state.watch.lock().unwrap().outcome {
            2 => {
                final_status = ProcessStatus::Failed;
                completion_reason = "failure_pattern".to_owned();
            }
            1 if final_status == ProcessStatus::Exited => {
                completion_reason = "success_pattern".to_owned();
            }
            _ => {}
        }
    }
    if let Some(writer) = &state.launch.output
        && let Err(error) = writer.finalize(agena_domain::ContentState::Complete).await
    {
        tracing::error!(%error, "background content seal failed");
    }

    {
        let mut inner = state.inner.lock().unwrap();
        inner.status = final_status;
        inner.exit_code = final_exit_code;
        inner.completion_reason = Some(completion_reason);
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
            tokio::task::consume_budget().await;
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
        TerminationCause::Condition(..) | TerminationCause::WatchQuiet(_) => {
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
    Condition(PatternOutcome, u64),
    WatchQuiet(u64),
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
    if let Some(writer) = state.launch.output.clone() {
        tokio::spawn(async move {
            let _ = writer
                .finalize(agena_domain::ContentState::Interrupted)
                .await;
        });
    }
    state.changed.notify_all();
    if let Some(listener) = state.listener.as_ref() {
        listener.on_finished(&state.snapshot());
    }
}

// Observe pipe activity before archival backpressure or newline framing.
// Progress without a newline, filtered lines and overlong records must also
// reset a watched command's quiet-period deadline.
struct ProcessActivityReader<R> {
    reader: R,
    state: Arc<MonitorState>,
}

impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for ProcessActivityReader<R> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        let result = std::pin::Pin::new(&mut this.reader).poll_read(cx, buffer);
        if buffer.filled().len() > before {
            this.state
                .last_activity_ms
                .store(Utc::now().timestamp_millis(), Ordering::Release);
            *this.state.last_activity.lock().unwrap() = Instant::now();
        }
        result
    }
}

async fn stream_lines<R>(state: Arc<MonitorState>, reader: R, stream: ProcessStream)
where
    R: tokio::io::AsyncRead + Unpin + Send,
{
    use tokio::io::AsyncReadExt;
    let mut reader = ProcessActivityReader {
        reader,
        state: Arc::clone(&state),
    };
    let mut bytes = [0_u8; 8 * 1024];
    let mut utf8 = Vec::new();
    let mut matcher = OutputMatcher::default();
    loop {
        tokio::task::consume_budget().await;
        match reader.read(&mut bytes).await {
            Ok(count) => {
                if count != 0 {
                    *state.last_activity.lock().unwrap() = Instant::now();
                }
                let text =
                    crate::tool::shell::decode_output(&mut utf8, &bytes[..count], count == 0);
                if !text.is_empty() {
                    if let Some(writer) = &state.launch.output {
                        let channel = match stream {
                            ProcessStream::Stdout => agena_domain::CommandOutputStream::Stdout,
                            ProcessStream::Stderr => agena_domain::CommandOutputStream::Stderr,
                        };
                        let byte_count = text.len();
                        if let Err(error) = writer.capture(agena_domain::ContentInput::Log {
                            stream: channel,
                            text: text.clone(),
                        }) {
                            writer.record_loss(byte_count, &error);
                        }
                    }
                    let event = push_output_event_filtered(&state, stream, text, true, false);
                    matcher.push(&state, &event, count == 0);
                }
                if count == 0 {
                    matcher.finish(&state);
                    break;
                }
            }
            Err(error) => {
                push_event(
                    &state,
                    ProcessStream::Stderr,
                    format!("background output pipe read failed: {error}"),
                );
                break;
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
    push_output_event(state, stream, line, false);
}

fn push_output_event(state: &MonitorState, stream: ProcessStream, line: String, chunk: bool) {
    let _ = push_output_event_filtered(state, stream, line, chunk, state.launch.ws.is_some());
}

fn push_output_event_filtered(
    state: &MonitorState,
    stream: ProcessStream,
    line: String,
    chunk: bool,
    notify: bool,
) -> ProcessEvent {
    let mut inner = state.inner.lock().unwrap();
    // Assign and append under one lock: concurrent stdout/stderr must not
    // insert an older sequence after a newer cursor has already been read.
    let seq = state.last_seq.fetch_add(1, Ordering::AcqRel) + 1;
    let event = ProcessEvent {
        seq,
        stream,
        ts_ms: Utc::now().timestamp_millis(),
        line,
        chunk,
        notification: None,
        notification_seq: None,
    };
    inner.buffered_bytes += event.line.len();
    inner.buffer.push_back(event.clone());
    while inner.buffer.len() > state.capacity || inner.buffered_bytes > MAX_BUFFER_BYTES {
        if let Some(evicted) = inner.buffer.pop_front() {
            inner.buffered_bytes -= evicted.line.len();
            state.dropped_lines.fetch_add(1, Ordering::AcqRel);
        }
    }
    drop(inner);
    state.changed.notify_all();
    // Forward the event to the observer so the runtime can project it into the
    // transcript as a `system_notification` part (everything-is-a-part).
    if notify && let Some(listener) = state.listener.as_ref() {
        listener.on_event(&event, &state.snapshot());
    }
    event
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
            output: None,
            argv: None,
            owner: None,
            process_id: None,
            command: command.to_string(),
            ws: None,
            description: "monitor test".to_string(),
            workdir: std::env::current_dir().expect("current dir"),
            timeout_ms: Some(2_000),
            persistent: false,
            monitored: true,
            watch: None,
            include_pattern: None,
            success_pattern: None,
            failure_pattern: None,
            quiet_period_ms: None,
            max_buffered_lines: Some(32),
            capture_stderr: true,
            env: std::env::vars().collect(),
        }
    }

    #[tokio::test]
    async fn plain_background_output_without_a_newline_arrives_before_exit() {
        let registry = MonitorRegistry::from_handle(tokio::runtime::Handle::current());
        let mut params = start_params("printf 'first'; sleep 0.6; printf 'second'");
        params.monitored = false;
        let started = registry.start(params).unwrap();
        let first = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let read = registry
                    .read(ReadParams {
                        monitor_id: started.summary.process_id.clone(),
                        since_seq: 0,
                        limit: Some(200),
                        wait_ms: 0,
                    })
                    .unwrap();
                if !read.events.is_empty() {
                    break read;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(first.status, ProcessStatus::Running);
        assert_eq!(first.events[0].line, "first");
        assert!(first.events[0].chunk);
        let final_read = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let read = registry
                    .read(ReadParams {
                        monitor_id: started.summary.process_id.clone(),
                        since_seq: 0,
                        limit: Some(200),
                        wait_ms: 0,
                    })
                    .unwrap();
                if read.status != ProcessStatus::Running {
                    break read;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            final_read
                .events
                .iter()
                .map(|event| event.line.as_str())
                .collect::<String>(),
            "firstsecond"
        );
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
        // The fixture itself may run for two seconds, followed by process
        // cleanup and a queued blocking read. Do not race that same deadline.
        tokio::time::timeout(Duration::from_secs(5), async {
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
        assert!(read.events.iter().any(|event| event.line.contains("READY")));
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
        assert_eq!(read.status, ProcessStatus::Exited, "{read:?}");
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
        // Raw capture and bounded matching must keep observing later records.
        params.timeout_ms = Some(10_000);
        let id = registry.start(params).expect("start").summary.process_id;
        let read = wait_for_terminal(registry, id).await;

        assert_eq!(read.completion_reason.as_deref(), Some("success_pattern"));
        assert!(read.events.iter().any(|event| event.line.contains("READY")));
        assert!(
            read.events
                .iter()
                .all(|event| event.chunk && event.line.len() <= 8192)
        );
        assert!(
            read.events
                .iter()
                .map(|event| event.line.len())
                .sum::<usize>()
                >= 100000
        );
    }
}
