//! Concrete activity service state: registry + source bridges + log readers.
//!
//! The runtime owns one [`ActivityRuntimeState`] for its whole lifetime. Source
//! bridges push shell processes, runtime tasks, and agents into the
//! unified registry; the application-facing [`RuntimeActivityService`]
//! implementation reads the same state.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use agena_domain::{
    AgentStatus, BackgroundActivity, BackgroundActivityKind, BackgroundActivityLogLine,
    BackgroundActivityLogRead, BackgroundActivityStatus, ProcessEvent, ProcessStatus,
    ProcessStream, ProcessSummary,
};
use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, RecoveryDirective,
    RetryDirective, UserPresentation,
};
use agena_plugin_sdk::activity::ActivitySourceAdapter;
use agena_runtime_contracts::part_content::SystemNotificationContent;
use agena_runtime_session::SessionManager;
use agena_storage::kernel::{
    RequestId, SessionConfiguration, SessionId, SessionSummaryView, ToolId,
};
use agena_storage::store::PartState;

use super::registry::ActivityRegistry;
use crate::{
    RuntimeBackgroundTask, RuntimeBackgroundTaskListener, RuntimeBackgroundTaskRegistry,
    RuntimeBackgroundTaskStatus,
};

/// Shared mutable state backing the runtime's activity service.
#[derive(Clone)]
pub(crate) struct ActivityRuntimeState {
    pub(crate) registry: ActivityRegistry,
    pub(crate) monitor: Option<Arc<dyn crate::MonitorService>>,
    /// Adapters registered by plugins for activity kinds they own. When an
    /// adapter exists for a kind, log reads and stop requests dispatch to it
    /// instead of the built-in per-kind behavior.
    sources: Arc<parking_lot::Mutex<Vec<(BackgroundActivityKind, Arc<dyn ActivitySourceAdapter>)>>>,
}

impl ActivityRuntimeState {
    pub(crate) fn new(
        registry: ActivityRegistry,
        monitor: Option<Arc<dyn crate::MonitorService>>,
    ) -> Self {
        Self {
            registry,
            monitor,
            sources: Arc::new(parking_lot::Mutex::new(Vec::new())),
        }
    }

    /// Register (or replace) the plugin adapter that owns one activity kind.
    pub(crate) fn register_source(
        &self,
        kind: BackgroundActivityKind,
        adapter: Arc<dyn ActivitySourceAdapter>,
    ) {
        let mut sources = self.sources.lock();
        if let Some(entry) = sources.iter_mut().find(|(existing, _)| *existing == kind) {
            entry.1 = adapter;
        } else {
            sources.push((kind, adapter));
        }
    }

    /// Look up the adapter registered for a kind, if any.
    pub(crate) fn source_for(
        &self,
        kind: BackgroundActivityKind,
    ) -> Option<Arc<dyn ActivitySourceAdapter>> {
        self.sources
            .lock()
            .iter()
            .find(|(existing, _)| *existing == kind)
            .map(|(_, adapter)| Arc::clone(adapter))
    }
}

/// Monitor listener that projects every background shell process into the
/// unified activity registry and forwards terminal summaries to the runtime's
/// background-completion bridge, which commits a reliable delivery. The
/// `on_finished` slot is populated after the runtime is assembled,
/// because the bridge needs the session manager.
pub(crate) struct MonitorActivityBridge {
    pub(crate) registry: ActivityRegistry,
    pub(crate) on_finished: SharedFinishedCallback,
    /// Publishes selected monitor events through reliable delivery. Populated
    /// after the runtime is assembled, like `on_finished`.
    pub(crate) on_event: SharedEventCallback,
    pub(crate) on_watch_changed: SharedFinishedCallback,
}

type SharedFinishedCallback =
    Arc<std::sync::Mutex<Option<Arc<dyn Fn(&ProcessSummary) + Send + Sync + 'static>>>>;
type SharedEventCallback = Arc<
    std::sync::Mutex<Option<Arc<dyn Fn(&ProcessEvent, &ProcessSummary) + Send + Sync + 'static>>>,
>;

impl std::fmt::Debug for MonitorActivityBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonitorActivityBridge")
            .field("registry", &self.registry)
            .finish_non_exhaustive()
    }
}

impl MonitorActivityBridge {
    /// `upsert` replaces the whole record, so a record synthesized from the
    /// live process carries forward the provenance stored by the operation
    /// that owns the same id.
    fn carry_forward(&self, next: BackgroundActivity) -> BackgroundActivity {
        let previous = self.registry.get(&next.id);
        carry_provenance_forward(next, previous.as_ref())
    }
}

/// Carry the durable provenance of `previous` into a freshly synthesized
/// record: the owning session, the source part and the background-operation id
/// describe the work, not the live process reading.
fn carry_provenance_forward(
    mut next: BackgroundActivity,
    previous: Option<&BackgroundActivity>,
) -> BackgroundActivity {
    let Some(previous) = previous else {
        return next;
    };
    if next.session_id.is_none() {
        next.session_id = previous.session_id;
    }
    if next.parent_session_id.is_none() {
        next.parent_session_id = previous.parent_session_id;
    }
    if next.operation_id.is_none() {
        next.operation_id = previous.operation_id.clone();
    }
    if next.source_part_id.is_none() {
        next.source_part_id = previous.source_part_id;
    }
    next
}

impl crate::MonitorListener for MonitorActivityBridge {
    fn on_started(&self, summary: &ProcessSummary) {
        self.registry
            .upsert(self.carry_forward(shell_activity(summary)));
    }

    fn on_event(&self, event: &ProcessEvent, summary: &ProcessSummary) {
        self.registry
            .upsert(self.carry_forward(shell_activity(summary)));
        if let Some(callback) = self
            .on_event
            .lock()
            .expect("monitor on_event lock")
            .as_ref()
        {
            callback(event, summary);
        }
    }

    fn on_watch_changed(&self, summary: &ProcessSummary) {
        self.on_started(summary);
        let callback = self
            .on_watch_changed
            .lock()
            .expect("watch callback lock")
            .clone();
        if let Some(callback) = callback {
            callback(summary);
        }
    }

    fn on_finished(&self, summary: &ProcessSummary) {
        self.registry
            .upsert(self.carry_forward(shell_activity(summary)));
        if let Some(callback) = self
            .on_finished
            .lock()
            .expect("monitor on_finished lock")
            .as_ref()
        {
            callback(summary);
        }
    }
}

fn shell_activity(summary: &ProcessSummary) -> BackgroundActivity {
    let terminal = summary.status != ProcessStatus::Running;
    BackgroundActivity {
        id: summary.process_id.clone(),
        kind: BackgroundActivityKind::Shell,
        status: match summary.status {
            ProcessStatus::Running => BackgroundActivityStatus::Running,
            ProcessStatus::Exited => BackgroundActivityStatus::Succeeded,
            ProcessStatus::TimedOut => BackgroundActivityStatus::Failed,
            ProcessStatus::Stopped => BackgroundActivityStatus::Stopped,
            ProcessStatus::Failed => BackgroundActivityStatus::Failed,
        },
        title: {
            let label = if summary.description.trim().is_empty() {
                summary.command.as_str()
            } else {
                summary.description.as_str()
            };
            // An interactive terminal reads as a live terminal, not as one
            // more background process.
            format!(
                "{} · {label}",
                if summary.tty {
                    "Terminal"
                } else if summary.websocket {
                    "WebSocket"
                } else if summary.monitored {
                    "Watch command"
                } else {
                    "Run process"
                }
            )
        },
        description: summary.command.clone(),
        command: Some(summary.command.clone()),
        workdir: None,
        // Present when the runtime knows the owner: the activity panel is
        // session-scoped, and a terminal session has no background operation
        // record to supply the session otherwise.
        session_id: summary.owner_session_id,
        parent_session_id: None,
        operation_id: None,
        source_part_id: None,
        created_at_ms: summary.started_at_ms,
        started_at_ms: summary.started_at_ms,
        finished_at_ms: summary.ended_at_ms,
        next_event_at_ms: None,
        exit_code: summary.exit_code,
        message: summary.completion_reason.clone(),
        failure: None,
        last_seq: summary.last_seq,
        has_more: summary.buffered_lines as u64 > 0,
        dropped_lines: summary.dropped_lines,
        cancellable: !terminal,
        dismissible: terminal,
    }
}

/// Listener that projects runtime maintenance tasks into the unified registry.
#[derive(Debug)]
pub(crate) struct RuntimeTaskActivityBridge {
    pub(crate) registry: ActivityRegistry,
}

impl RuntimeBackgroundTaskListener for RuntimeTaskActivityBridge {
    fn on_started(&self, task: &RuntimeBackgroundTask) {
        self.registry.upsert(runtime_task_activity(task));
    }

    fn on_finished(&self, task: &RuntimeBackgroundTask) {
        self.registry.upsert(runtime_task_activity(task));
    }
}

fn runtime_task_activity(task: &RuntimeBackgroundTask) -> BackgroundActivity {
    let terminal = !matches!(task.status, RuntimeBackgroundTaskStatus::Running);
    BackgroundActivity {
        id: task.id.clone(),
        kind: BackgroundActivityKind::Runtime,
        status: match task.status {
            RuntimeBackgroundTaskStatus::Running => BackgroundActivityStatus::Running,
            RuntimeBackgroundTaskStatus::Succeeded => BackgroundActivityStatus::Succeeded,
            RuntimeBackgroundTaskStatus::Failed => BackgroundActivityStatus::Failed,
            RuntimeBackgroundTaskStatus::Cancelled => BackgroundActivityStatus::Cancelled,
        },
        title: task.title.clone(),
        description: String::new(),
        command: None,
        workdir: None,
        session_id: None,
        parent_session_id: None,
        operation_id: None,
        source_part_id: None,
        created_at_ms: task.created_at.timestamp_millis(),
        started_at_ms: task.started_at.timestamp_millis(),
        finished_at_ms: task.finished_at.map(|at| at.timestamp_millis()),
        next_event_at_ms: None,
        exit_code: None,
        message: task.message.clone(),
        failure: task.failure.as_ref().map(Into::into),
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        cancellable: task.cancellable && !terminal,
        dismissible: terminal,
    }
}

fn agent_status_to_background(status: AgentStatus) -> BackgroundActivityStatus {
    match status {
        AgentStatus::Created => BackgroundActivityStatus::Pending,
        AgentStatus::Running => BackgroundActivityStatus::Running,
        AgentStatus::Completed => BackgroundActivityStatus::Succeeded,
        AgentStatus::Failed => BackgroundActivityStatus::Failed,
        AgentStatus::Cancelled | AgentStatus::Interrupted => BackgroundActivityStatus::Cancelled,
        AgentStatus::TimedOut => BackgroundActivityStatus::Failed,
    }
}

/// Project agent configuration into the bounded process-local activity registry.
pub(crate) fn upsert_agent_activity_from_meta(
    registry: &ActivityRegistry,
    meta: &SessionSummaryView,
    configuration: &SessionConfiguration,
) {
    let Some(agent) = &configuration.agent else {
        return;
    };
    let agent_id = meta.id.get();
    let status = agent_status_to_background(agent.status);
    let started_at_ms = agent.started_at_ms.unwrap_or(meta.created_at_ms);
    registry.upsert(BackgroundActivity {
        id: format!("agent_{agent_id}"),
        kind: BackgroundActivityKind::Agent,
        status,
        title: format!("Agent · {}", agent.name),
        description: String::new(),
        command: None,
        workdir: None,
        session_id: Some(agent_id),
        parent_session_id: configuration.parent_session_id.map(SessionId::get),
        operation_id: None,
        source_part_id: None,
        created_at_ms: started_at_ms,
        started_at_ms,
        finished_at_ms: agent.finished_at_ms,
        next_event_at_ms: None,
        exit_code: None,
        message: agent
            .failure
            .as_ref()
            .map(|failure| failure.user.fallback.clone()),
        failure: agent.failure.as_ref().map(Into::into),
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        cancellable: status.is_active(),
        dismissible: status.is_terminal(),
    });
}

/// Publishes bounded process event batches and terminal observations as
/// reliable kernel deliveries. Activity/source lifecycle is owned by the
/// process registries; this bridge never relaunches or overwrites tool facts.
#[derive(Clone)]
pub(crate) struct BackgroundCompletionBridge {
    /// Late-bound: the session manager is assembled after the initial
    /// snapshot, so the slot starts empty and is set once the runtime exists.
    manager: Arc<Mutex<Option<Arc<SessionManager>>>>,
    monitor_events: Arc<Mutex<HashMap<String, MonitorEventRoute>>>,
}

struct MonitorEventRoute {
    generation: uuid::Uuid,
    // None caches an unavailable delivery route until the process finishes.
    sender: Option<tokio::sync::watch::Sender<MonitorEventTail>>,
    flushed: tokio::sync::oneshot::Receiver<()>,
    cancel: tokio_util::sync::CancellationToken,
}

#[derive(Clone, Default)]
struct MonitorEventTail {
    lines: VecDeque<(u64, String)>,
    bytes: usize,
    last_seq: u64,
    ready_seq: Option<u64>,
    output_resource: Option<agena_domain::ContentRef>,
}

impl MonitorEventTail {
    fn push(&mut self, event: &ProcessEvent, summary: &ProcessSummary) {
        const MAX_BYTES: usize = 6 * 1024;
        const MAX_LINES: usize = 64;
        let stream = match event.stream {
            ProcessStream::Stdout => "out",
            ProcessStream::Stderr => "err",
        };
        // Bound before copying: a single long line cannot balloon the bridge.
        let mut start = event.line.len().saturating_sub(MAX_BYTES - 256);
        while !event.line.is_char_boundary(start) {
            start += 1;
        }
        let line = format!(
            "#{:>5} {stream} {}{}",
            event.seq,
            if start > 0 {
                format!(
                    "[notification omits {start} leading bytes; inspect shell.read/output_resource] "
                )
            } else {
                String::new()
            },
            &event.line[start..]
        );
        let delivery_seq = event.notification_seq.unwrap_or(event.seq);
        if event.notification.as_deref() == Some("ready") {
            self.ready_seq = Some(delivery_seq);
        }
        self.output_resource = summary.output_resource.clone();
        self.bytes += line.len();
        let index = self.lines.partition_point(|(seq, _)| *seq < delivery_seq);
        self.lines.insert(index, (delivery_seq, line));
        self.last_seq = self.last_seq.max(delivery_seq);
        while self.bytes > MAX_BYTES || self.lines.len() > MAX_LINES {
            if let Some((_, line)) = self.lines.pop_front() {
                self.bytes -= line.len();
            }
        }
    }
}

impl BackgroundCompletionBridge {
    pub(crate) fn new(manager: Option<Arc<SessionManager>>) -> Self {
        Self {
            manager: Arc::new(Mutex::new(manager)),
            monitor_events: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Late-bind the session manager (assembled after the initial snapshot).
    pub(crate) fn set_manager(&self, manager: Option<Arc<SessionManager>>) {
        let stopping = manager.is_none();
        *self.manager.lock().expect("background manager lock") = manager;
        if stopping {
            let mut routes = self
                .monitor_events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for route in routes.values() {
                route.cancel.cancel();
            }
            routes.clear();
        }
    }

    pub(crate) fn update_shell_watch(&self, summary: &ProcessSummary) {
        if let Some(route) = self
            .monitor_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&summary.process_id)
        {
            route.cancel.cancel();
        }
    }

    /// Settle the durable shell/monitor aggregate for a process that just
    /// reached a terminal state.
    pub(crate) fn complete_shell(&self, summary: &ProcessSummary) {
        // Persistent interactive terminals are explicitly read/written by the
        // caller. They have no durable background launch or AI wake to settle.
        if summary.tty || summary.owner_session_id.is_none() {
            return;
        }
        let terminal = match summary.status {
            ProcessStatus::Exited => PartState::Completed,
            ProcessStatus::TimedOut | ProcessStatus::Stopped | ProcessStatus::Failed => {
                PartState::Failed
            }
            // A completion signal must be terminal; a spurious running report
            // has nothing to do here.
            ProcessStatus::Running => return,
        };
        // Closing the publisher lets the one event worker flush its last
        // bounded tail and releases the per-process route cache.
        let event_flush = self
            .monitor_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&summary.process_id)
            .map(|route| {
                drop(route.sender);
                route.flushed
            });
        let command_label = format!("\"{}\"", summary.command.trim());
        let outcome = if terminal == PartState::Completed {
            Ok(match summary.exit_code {
                Some(code) => format!("Command {command_label} completed (exit code {code})"),
                None => format!("Command {command_label} completed"),
            })
        } else {
            let fallback = match summary.status {
                ProcessStatus::TimedOut => format!("Command {command_label} timed out."),
                ProcessStatus::Stopped => format!("Command {command_label} was stopped."),
                ProcessStatus::Failed => format!(
                    "Command {command_label} failed (exit {}).",
                    summary
                        .exit_code
                        .map_or_else(|| "unknown".to_string(), |code| code.to_string())
                ),
                _ => format!("Command {command_label} failed."),
            };
            Err(background_failure(summary.status, fallback))
        };
        let bridge = self.clone();
        let process_id = summary.process_id.clone();
        let status = match summary.status {
            ProcessStatus::Exited => "completed",
            ProcessStatus::TimedOut => "timed_out",
            ProcessStatus::Stopped => "stopped",
            ProcessStatus::Failed => "failed",
            ProcessStatus::Running => unreachable!("complete_shell filters Running above"),
        };
        let mut summary_line = outcome
            .as_ref()
            .map(String::as_str)
            .unwrap_or_else(|failure| failure.user.fallback.as_str())
            .to_string();
        if let Some(resource) = &summary.output_resource {
            summary_line.push_str(&format!("\nOutput resource: {}", resource.resource_id));
        }
        tokio::spawn(async move {
            // Preserve event-before-completion order without letting a stalled
            // event/database worker indefinitely delay terminal notification.
            if let Some(flushed) = event_flush {
                let _ = tokio::time::timeout(Duration::from_secs(2), flushed).await;
            }
            let Some((session_id, kind)) = bridge
                .resolve_background_session_either(&["shell", "monitor"], &process_id)
                .await
            else {
                return;
            };
            // The kernel retains a business delivery; consuming it and
            // queueing an idle continuation is one durable transaction.
            let result=bridge.deliver_background(session_id,&process_id,format!("process:{process_id}:completion"),
                serde_json::json!({"text":summary_line,"kind":kind,"process_id":process_id,"status":status})).await;
            if let Err(error) = &result {
                tracing::warn!(
                    target: "agena_background",
                    %session_id, %process_id, %error,
                    "failed to settle background shell operation"
                );
            }
        });
    }

    /// Project one monitor event as an Assistant-owned `system_notification`
    /// on the monitor's launch run.
    /// Watched shell commands and WebSocket subscriptions share the bounded
    /// event bridge while retaining their own durable operation kind.
    pub(crate) fn settle_monitor_event(&self, event: &ProcessEvent, summary: &ProcessSummary) {
        // Plain shell/PTY logs are read from their process buffer. Only a
        // monitored process can need transcript event notifications.
        if summary.tty
            || summary.owner_session_id.is_none()
            || !summary.monitored
            || summary.status != ProcessStatus::Running
        {
            return;
        }
        let process_id = summary.process_id.clone();
        let mut routes = self
            .monitor_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(route) = routes.get(&process_id) {
            if let Some(sender) = &route.sender {
                sender.send_modify(|tail| tail.push(event, summary));
            }
            return;
        }
        let generation = uuid::Uuid::new_v4();
        let mut tail = MonitorEventTail::default();
        tail.push(event, summary);
        let (sender, mut events) = tokio::sync::watch::channel(tail);
        let (flushed_tx, flushed) = tokio::sync::oneshot::channel();
        let cancel = tokio_util::sync::CancellationToken::new();
        routes.insert(
            process_id.clone(),
            MonitorEventRoute {
                generation,
                sender: Some(sender),
                flushed,
                cancel: cancel.clone(),
            },
        );
        drop(routes);
        let bridge = self.clone();
        tokio::spawn(async move {
            // Resolve once through the durable index for either launch kind;
            // never query the database or start a new worker for every line.
            let owner = bridge
                .resolve_background_session_either(&["shell", "monitor"], &process_id)
                .await;
            let manager = bridge
                .manager
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let (Some((session_id, kind)), Some(_manager)) = (owner, manager) else {
                bridge.ignore_monitor_events(&process_id, generation);
                return;
            };
            let mut last_seq = 0;
            while !cancel.is_cancelled() {
                let tail = events.borrow_and_update().clone();
                let lines = tail
                    .lines
                    .iter()
                    .filter(|(seq, _)| *seq > last_seq)
                    .collect::<Vec<_>>();
                if !lines.is_empty() {
                    let missed = lines[0].0.saturating_sub(last_seq.saturating_add(1));
                    let mut body = String::new();
                    if missed > 0 {
                        body.push_str(&format!(
                            "[{missed} notification sequence positions were filtered or omitted from this bounded notification; inspect shell.read for retained details.]\n"
                        ));
                    }
                    for (_, line) in &lines {
                        if !body.is_empty() {
                            body.push('\n');
                        }
                        body.push_str(line);
                    }
                    let ready = tail.ready_seq.is_some_and(|seq| seq > last_seq);
                    if ready {
                        body.insert_str(0, "[service ready; process continues running]\n");
                    }
                    body.push_str(&format!("\nProcess {process_id}. Diagnostic replay: shell.read(process_id, since_seq=0); do not poll merely to wait."));
                    if let Some(resource) = &tail.output_resource {
                        body.push_str(&format!("\nOutput resource: {}", resource.resource_id));
                    }
                    let summary = if ready {
                        format!("Process {process_id} is ready and continues running")
                    } else if lines.len() == 1 && missed == 0 {
                        lines[0].1.clone()
                    } else {
                        format!(
                            "Watched output through notification #{} ({} retained events)",
                            tail.last_seq,
                            lines.len()
                        )
                    };
                    let notification = SystemNotificationContent {
                        operation_id: process_id.clone(),
                        operation_kind: kind.clone(),
                        status: if ready { "ready" } else { "event" }.to_owned(),
                        summary,
                        body,
                        event_seq: Some(tail.last_seq),
                        ..Default::default()
                    };
                    let mut settled = false;
                    // A transient database/delivery failure must not silently
                    // consume readiness. Retry the same durable event key;
                    // storage/outbox deduplication prevents duplicate wakes.
                    for attempt in 0..3 {
                        if cancel.is_cancelled() {
                            break;
                        }
                        match bridge.deliver_background(session_id,&process_id,format!("process:{process_id}:event:{}",tail.last_seq),
                            serde_json::json!({"text":notification.body,"kind":kind,"process_id":process_id,"status":notification.status,"event_seq":tail.last_seq})).await
                        {
                            Ok(()) => {
                                settled = true;
                                break;
                            }
                            Err(error) => {
                                tracing::warn!(target: "agena_background", %session_id, %process_id, %error, attempt, "failed to settle monitor event batch");
                                if attempt < 2 {
                                    tokio::select! {
                                        _ = cancel.cancelled() => break,
                                        _ = tokio::time::sleep(Duration::from_millis(100 << attempt)) => {},
                                    }
                                }
                            }
                        }
                    }
                    if !settled {
                        if !cancel.is_cancelled() {
                            bridge.ignore_monitor_events(&process_id, generation);
                        }
                        break;
                    }
                    last_seq = tail.last_seq;
                }
                let changed = tokio::select! {
                    _ = cancel.cancelled() => break,
                    result = events.changed() => result,
                };
                if changed.is_err() {
                    break;
                }
                // One worker and one bounded latest tail per process; neither
                // output bursts nor slow model acknowledgement spawn more work.
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                }
            }
            let _ = flushed_tx.send(());
        });
    }

    fn ignore_monitor_events(&self, process_id: &str, generation: uuid::Uuid) {
        let mut routes = self
            .monitor_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(route) = routes.get_mut(process_id)
            && route.generation == generation
        {
            route.sender = None;
        }
    }

    async fn resolve_background_session_either(
        &self,
        _kinds: &[&str],
        id: &str,
    ) -> Option<(i64, String)> {
        let (session, tool) = managed_process_scope(id)?;
        let manager = self
            .manager
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()?;
        let view = manager
            .session_kernel()
            .reader
            .tool(session, tool)
            .await
            .ok()?;
        let kind = if view.call.name.contains("monitor") {
            "monitor"
        } else {
            "shell"
        };
        Some((session.get(), kind.into()))
    }

    async fn deliver_background(
        &self,
        session: i64,
        process: &str,
        key: String,
        mut payload: serde_json::Value,
    ) -> Result<(), crate::AppError> {
        let manager = self
            .manager
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or(crate::AppError::Cancelled)?;
        let (owner, tool) = managed_process_scope(process)
            .ok_or_else(|| crate::AppError::Config("invalid managed process identity".into()))?;
        if owner.get() != session {
            return Err(crate::AppError::Config("background owner mismatch".into()));
        }
        let kernel = manager.session_kernel();
        let tool = kernel
            .reader
            .tool(owner, tool)
            .await
            .map_err(|e| crate::AppError::Internal(e.to_string()))?;
        let entry = kernel
            .reader
            .entry(owner, tool.entry_id)
            .await
            .map_err(|e| crate::AppError::Internal(e.to_string()))?;
        payload["message_id"] = entry.message_id.get().into();
        let request = RequestId::new();
        let mut retry = 0u32;
        loop {
            if self
                .manager
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
            {
                return Err(crate::AppError::Cancelled);
            }
            match kernel
                .executor
                .deliver(request, owner, key.clone(), payload.clone())
                .await
            {
                Ok(_) => {
                    let _ = manager.recover_background_deliveries(64).await;
                    return Ok(());
                }
                Err(error)
                    if matches!(
                        error,
                        agena_storage::kernel::KernelError::Busy
                            | agena_storage::kernel::KernelError::Storage(_)
                            | agena_storage::kernel::KernelError::CommitUnknown { .. }
                    ) =>
                {
                    tracing::warn!(%error,%key,"background delivery persistence will retry");
                    tokio::time::sleep(Duration::from_millis(
                        100u64.saturating_mul(1u64 << retry.min(6)),
                    ))
                    .await;
                    retry = retry.saturating_add(1);
                }
                Err(error) => return Err(crate::AppError::Internal(error.to_string())),
            }
        }
    }
}

fn background_failure(status: ProcessStatus, fallback: String) -> Failure {
    Failure::new(
        FailureCode::new(match status {
            ProcessStatus::TimedOut => "background.timed_out",
            ProcessStatus::Stopped => "background.stopped",
            _ => "background.failed",
        }),
        FailureCategory::DependencyUnavailable,
        FailureResponsibility::System,
        RetryDirective::AfterUserAction,
        RecoveryDirective::AskUser,
        FailureImpact::BackgroundTaskFailed,
        UserPresentation {
            key: "background-failed".to_string(),
            fallback,
            detail_key: None,
        },
    )
}

/// Resolve the deterministic launch identity without enumerating processes.
fn managed_process_scope(id: &str) -> Option<(SessionId, ToolId)> {
    let mut pieces = id.strip_prefix("proc_")?.split('_');
    let session = SessionId::try_from(pieces.next()?.parse::<i64>().ok()?).ok()?;
    let tool = ToolId::try_from(pieces.next()?.parse::<i64>().ok()?).ok()?;
    pieces.next().is_none().then_some((session, tool))
}

pub(crate) fn read_shell_logs(
    monitor: &dyn crate::MonitorService,
    activity_id: &str,
    since_seq: u64,
    limit: Option<u32>,
    wait_ms: u64,
) -> Result<BackgroundActivityLogRead, crate::MonitorError> {
    let read = monitor.read(crate::MonitorReadParams {
        monitor_id: activity_id.to_string(),
        since_seq,
        limit,
        wait_ms,
    })?;
    Ok(BackgroundActivityLogRead {
        activity_id: read.monitor_id.clone(),
        status: match read.status {
            ProcessStatus::Running => BackgroundActivityStatus::Running,
            ProcessStatus::Exited => BackgroundActivityStatus::Succeeded,
            ProcessStatus::TimedOut => BackgroundActivityStatus::Failed,
            ProcessStatus::Stopped => BackgroundActivityStatus::Stopped,
            ProcessStatus::Failed => BackgroundActivityStatus::Failed,
        },
        lines: read
            .events
            .into_iter()
            .map(|event| BackgroundActivityLogLine {
                seq: event.seq,
                stream: match event.stream {
                    agena_domain::ProcessStream::Stdout => "stdout".to_string(),
                    agena_domain::ProcessStream::Stderr => "stderr".to_string(),
                },
                ts_ms: event.ts_ms,
                text: event.line,
                chunk: event.chunk,
            })
            .collect(),
        last_seq: read.last_seq,
        has_more: read.has_more,
        dropped_lines: read.dropped_lines,
        exit_code: read.exit_code,
        completion_reason: read.completion_reason,
    })
}
