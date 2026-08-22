//! Concrete activity service state: registry + source bridges + log readers.
//!
//! The runtime owns one [`ActivityRuntimeState`] for its whole lifetime. Source
//! bridges push shell processes, runtime tasks, and delegated tasks into the
//! unified registry; the application-facing [`RuntimeActivityService`]
//! implementation reads the same state.

use std::sync::{Arc, Mutex};

use agena_domain::{
    BackgroundActivity, BackgroundActivityKind, BackgroundActivityLogLine,
    BackgroundActivityLogRead, BackgroundActivityStatus, ProcessEvent, ProcessStatus,
    ProcessStream, ProcessSummary, SubtaskStatus,
};
use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, RecoveryDirective,
    RetryDirective, UserPresentation,
};
use agena_plugin_sdk::activity::ActivitySourceAdapter;
use agena_runtime_contracts::part_content::SystemNotificationContent;
use agena_runtime_session::SessionManager;
use agena_storage::store::{Part, PartState, SessionMeta};

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
/// background-completion bridge (which terminalizes the matching transcript
/// part). The `on_finished` slot is populated after the runtime is assembled,
/// because the bridge needs the session manager.
pub(crate) struct MonitorActivityBridge {
    pub(crate) registry: ActivityRegistry,
    pub(crate) on_finished: SharedFinishedCallback,
    /// Projected per-event (everything-is-a-part Monitor: every event is a
    /// `system_notification` part). Populated after the runtime is assembled,
    /// like `on_finished`.
    pub(crate) on_event: SharedEventCallback,
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

impl crate::MonitorListener for MonitorActivityBridge {
    fn on_started(&self, summary: &ProcessSummary) {
        self.registry.upsert(shell_activity(summary));
    }

    fn on_event(&self, event: &ProcessEvent, summary: &ProcessSummary) {
        self.registry.upsert(shell_activity(summary));
        if let Some(callback) = self
            .on_event
            .lock()
            .expect("monitor on_event lock")
            .as_ref()
        {
            callback(event, summary);
        }
    }

    fn on_finished(&self, summary: &ProcessSummary) {
        self.registry.upsert(shell_activity(summary));
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
        title: if summary.description.trim().is_empty() {
            format!("Run process · {}", summary.command)
        } else {
            format!("Run process · {}", summary.description)
        },
        description: summary.command.clone(),
        command: Some(summary.command.clone()),
        workdir: None,
        session_id: None,
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

fn subtask_status_to_background(status: SubtaskStatus) -> BackgroundActivityStatus {
    match status {
        SubtaskStatus::Created => BackgroundActivityStatus::Pending,
        SubtaskStatus::Running => BackgroundActivityStatus::Running,
        SubtaskStatus::Completed => BackgroundActivityStatus::Succeeded,
        SubtaskStatus::Failed => BackgroundActivityStatus::Failed,
        SubtaskStatus::Cancelled | SubtaskStatus::Interrupted => {
            BackgroundActivityStatus::Cancelled
        }
        SubtaskStatus::TimedOut => BackgroundActivityStatus::Failed,
    }
}

fn subtask_activity(
    task_id: &str,
    session_id: i64,
    parent_session_id: Option<i64>,
    status: BackgroundActivityStatus,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
    message: Option<String>,
    failure: Option<agena_failure::UserProblem>,
) -> BackgroundActivity {
    BackgroundActivity {
        id: format!("task_{task_id}"),
        kind: BackgroundActivityKind::Task,
        status,
        title: format!("Delegated task · {task_id}"),
        description: String::new(),
        command: None,
        workdir: None,
        session_id: Some(session_id),
        parent_session_id,
        operation_id: None,
        source_part_id: None,
        created_at_ms: started_at_ms,
        started_at_ms,
        finished_at_ms,
        next_event_at_ms: None,
        exit_code: None,
        message,
        failure,
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        cancellable: status.is_active(),
        dismissible: status.is_terminal(),
    }
}

fn persisted_subtask_failure(meta: &SessionMeta) -> Option<Failure> {
    let value = meta.subtask_failure.as_ref()?;
    match serde_json::from_value::<Failure>(value.clone()) {
        Ok(failure) => Some(failure),
        Err(error) => {
            tracing::error!(
                session_id = meta.id,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode a persisted subtask failure",
                    &error,
                ),
                "persisted subtask failure is malformed"
            );
            None
        }
    }
}

/// Project a session's persisted subtask state ([`SessionMeta`] columns) into
/// the unified registry. v2 keeps subtask state in the `sessions` row; the
/// facade's [`SessionChange::SessionMetaUpdated`] notifications drive this.
pub(crate) fn upsert_task_activity_from_meta(registry: &ActivityRegistry, meta: &SessionMeta) {
    let Some(task_id) = meta.task_id.as_deref() else {
        return;
    };
    let Some(raw_status) = meta.subtask_status.as_deref() else {
        return;
    };
    let Some(status) = SubtaskStatus::parse(raw_status) else {
        return;
    };
    let started_at = meta.subtask_started_at_ms.unwrap_or(meta.updated_at_ms);
    let persisted_failure = persisted_subtask_failure(meta);
    registry.upsert(subtask_activity(
        task_id,
        meta.id,
        meta.parent_id,
        subtask_status_to_background(status),
        started_at,
        meta.subtask_finished_at_ms,
        persisted_failure
            .as_ref()
            .map(|failure| failure.user.fallback.clone()),
        persisted_failure.as_ref().map(Into::into),
    ));
}

/// Correlates launched-in-background operations (a monitored shell process or
/// a delegated task) with their owning session so the session layer can
/// route completion into the durable operation coordinator.
///
/// Two completion signals arrive here:
/// - the facade's `SessionMetaUpdated` carries the child session id and its
///   terminal subtask status → terminalize the parent's `task` part;
/// - [`crate::MonitorListener::on_finished`] carries a `ProcessSummary`; its
///   stable id resolves through the database's unique `(kind, external_id)`
///   index. No observer event or process-local correlation map is required.
#[derive(Clone)]
pub(crate) struct BackgroundCompletionBridge {
    /// Late-bound: the session manager is assembled after the initial
    /// snapshot, so the slot starts empty and is set once the runtime exists.
    manager: Arc<Mutex<Option<Arc<SessionManager>>>>,
}

impl BackgroundCompletionBridge {
    pub(crate) fn new(manager: Option<Arc<SessionManager>>) -> Self {
        Self {
            manager: Arc::new(Mutex::new(manager)),
        }
    }

    /// Late-bind the session manager (assembled after the initial snapshot).
    pub(crate) fn set_manager(&self, manager: Option<Arc<SessionManager>>) {
        *self.manager.lock().expect("background manager lock") = manager;
    }

    /// Facade observer subscription for low-latency task completion. Durable
    /// maintenance reconciliation remains authoritative when this event is
    /// dropped; part events carry no operational control state.
    pub(crate) fn observer(&self) -> agena_storage::store::SessionObserver {
        let bridge = self.clone();
        Arc::new(move |change| match change {
            agena_storage::store::SessionChange::SessionMetaUpdated { meta, .. } => {
                terminalize_task_part(&bridge, meta);
            }
            agena_storage::store::SessionChange::PartAdded { .. }
            | agena_storage::store::SessionChange::PartUpdated { .. }
            | agena_storage::store::SessionChange::PartRemoved { .. } => {}
        })
    }

    /// Settle the durable shell/monitor aggregate for a process that just
    /// reached a terminal state.
    pub(crate) fn complete_shell(&self, summary: &ProcessSummary) {
        let terminal = match summary.status {
            ProcessStatus::Exited => PartState::Completed,
            ProcessStatus::TimedOut | ProcessStatus::Stopped | ProcessStatus::Failed => {
                PartState::Failed
            }
            // A completion signal must be terminal; a spurious running report
            // has nothing to do here.
            ProcessStatus::Running => return,
        };
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
        let summary_line = outcome
            .as_ref()
            .map(String::as_str)
            .unwrap_or_else(|failure| failure.user.fallback.as_str())
            .to_string();
        tokio::spawn(async move {
            let Some((session_id, kind)) = bridge
                .resolve_background_session_either(&["shell", "monitor"], &process_id)
                .await
            else {
                return;
            };
            let Some(manager) = bridge
                .manager
                .lock()
                .expect("background manager lock")
                .clone()
            else {
                return;
            };
            // The operation settle atomically appends an Assistant-owned
            // notification to the launch run and its durable wake delivery.
            let result = manager
                .settle_background_operation(
                    session_id,
                    &kind,
                    &process_id,
                    terminal,
                    outcome,
                    SystemNotificationContent {
                        operation_id: process_id.clone(),
                        operation_kind: kind.clone(),
                        status: status.to_string(),
                        summary: summary_line.clone(),
                        body: summary_line,
                        ..Default::default()
                    },
                )
                .await;
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
    /// Only `kind:"monitor"` operations (the Monitor tool) project
    /// events; plain/monitored shells keep their logs in the streaming buffer
    /// (queryable via `shell.logs`).
    pub(crate) fn settle_monitor_event(&self, event: &ProcessEvent, summary: &ProcessSummary) {
        let bridge = self.clone();
        let process_id = summary.process_id.clone();
        let event_seq = event.seq;
        let stream = match event.stream {
            ProcessStream::Stdout => "out",
            ProcessStream::Stderr => "err",
        };
        let summary_line = format!("#{:>5} {} {}", event_seq, stream, event.line);
        tokio::spawn(async move {
            let Some(session_id) = bridge
                .resolve_background_session("monitor", &process_id)
                .await
            else {
                return;
            };
            let Some(manager) = bridge
                .manager
                .lock()
                .expect("background manager lock")
                .clone()
            else {
                return;
            };
            let result = manager
                .settle_background_event(
                    session_id,
                    "monitor",
                    &process_id,
                    event_seq,
                    SystemNotificationContent {
                        operation_id: process_id.clone(),
                        operation_kind: "monitor".to_string(),
                        status: "event".to_string(),
                        summary: summary_line.clone(),
                        body: summary_line,
                        event_seq: Some(event_seq),
                        ..Default::default()
                    },
                )
                .await;
            if let Err(error) = &result {
                tracing::warn!(
                    target: "agena_background",
                    %session_id, %process_id, event_seq, %error,
                    "failed to settle monitor event"
                );
            }
        });
    }

    /// Resolve exclusively through the durable operation aggregate.
    async fn resolve_background_session(&self, kind: &str, id: &str) -> Option<i64> {
        let manager = self
            .manager
            .lock()
            .expect("background manager lock")
            .clone()?;
        let operation_kind = agena_storage::store::BackgroundOperationKind::parse(kind)?;
        match manager
            .background_operation_owner_for_external(&[operation_kind], id)
            .await
        {
            Ok(Some((session_id, _))) => Some(session_id),
            Ok(None) => {
                tracing::error!(
                    target: "agena_background",
                    %kind, %id,
                    "background signal has no durable operation owner"
                );
                None
            }
            Err(error) => {
                tracing::error!(
                    target: "agena_background",
                    %kind, %id, %error,
                    "failed to query durable background operation owner"
                );
                None
            }
        }
    }

    /// Resolve the session for a background marker whose launch kind is
    /// ambiguous. The process registry indexes every process under the kind of
    /// its launch tool: `shell.run` (with or without a `monitor` sub-object)
    /// indexes under `"shell"` while the `monitor.start` tool indexes under
    /// `"monitor"`, and a terminal `on_finished` event does not say which.
    /// Try each candidate kind in order and return the first hit with the kind
    /// that matched, so the settle uses the kind the index actually holds.
    async fn resolve_background_session_either(
        &self,
        kinds: &[&str],
        id: &str,
    ) -> Option<(i64, String)> {
        let durable_kinds = kinds
            .iter()
            .filter_map(|kind| agena_storage::store::BackgroundOperationKind::parse(kind))
            .collect::<Vec<_>>();
        let manager = self
            .manager
            .lock()
            .expect("background manager lock")
            .clone()?;
        match manager
            .background_operation_owner_for_external(&durable_kinds, id)
            .await
        {
            Ok(Some((session_id, kind))) => Some((session_id, kind.as_str().to_owned())),
            Ok(None) => {
                tracing::error!(
                    target: "agena_background",
                    ?kinds, %id,
                    "background completion has no durable operation owner"
                );
                None
            }
            Err(error) => {
                tracing::error!(
                    target: "agena_background",
                    ?kinds, %id, %error,
                    "failed to query durable background completion owner"
                );
                None
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

/// Terminalize the parent's `task` part once the child session settles. The
/// child's terminal status and failure come from the persisted subtask columns
/// on [`SessionMeta`]; the completed task's final text is loaded from the
/// child session's own parts.
fn terminalize_task_part(bridge: &BackgroundCompletionBridge, meta: SessionMeta) {
    let Some(task_id) = meta.task_id.clone() else {
        return;
    };
    let Some(parent_id) = meta.parent_id else {
        return;
    };
    let Some(status) = meta
        .subtask_status
        .as_deref()
        .and_then(SubtaskStatus::parse)
    else {
        return;
    };
    if !status.is_terminal() {
        return;
    }
    let persisted_failure = persisted_subtask_failure(&meta);
    let (terminal, failure) = match status {
        SubtaskStatus::Completed => (PartState::Completed, None),
        SubtaskStatus::Failed => (
            PartState::Failed,
            Some(persisted_failure.clone().unwrap_or_else(|| {
                background_failure(
                    ProcessStatus::Failed,
                    "The delegated task failed.".to_string(),
                )
            })),
        ),
        SubtaskStatus::Cancelled => (
            PartState::Cancelled,
            Some(background_failure(
                ProcessStatus::Stopped,
                "The delegated task was cancelled.".to_string(),
            )),
        ),
        SubtaskStatus::TimedOut => (
            PartState::Failed,
            Some(persisted_failure.clone().unwrap_or_else(|| {
                background_failure(
                    ProcessStatus::TimedOut,
                    "The delegated task timed out.".to_string(),
                )
            })),
        ),
        SubtaskStatus::Interrupted => (
            PartState::Cancelled,
            Some(persisted_failure.unwrap_or_else(|| {
                background_failure(
                    ProcessStatus::Stopped,
                    "The delegated task was interrupted.".to_string(),
                )
            })),
        ),
        SubtaskStatus::Created | SubtaskStatus::Running => return,
    };
    let task_label = if meta.title.trim().is_empty() {
        task_id.clone()
    } else {
        format!("\"{}\" ({task_id})", meta.title.trim())
    };
    let Some(manager) = bridge
        .manager
        .lock()
        .unwrap_or_else(|error| {
            tracing::error!(
                diagnostic = %error,
                "background completion manager lock is poisoned; recovering state"
            );
            error.into_inner()
        })
        .clone()
    else {
        return;
    };
    let child_session_id = meta.id;
    let notification_status = match status {
        SubtaskStatus::Completed => "completed",
        SubtaskStatus::Failed => "failed",
        SubtaskStatus::Cancelled => "cancelled",
        SubtaskStatus::Interrupted => "interrupted",
        SubtaskStatus::TimedOut => "timed_out",
        SubtaskStatus::Created | SubtaskStatus::Running => unreachable!("terminal status"),
    }
    .to_string();
    let summary_base = match status {
        SubtaskStatus::Completed => format!("Task {task_label} finished"),
        SubtaskStatus::Failed => format!("Task {task_label} failed"),
        SubtaskStatus::Cancelled => format!("Task {task_label} cancelled"),
        SubtaskStatus::Interrupted => format!("Task {task_label} interrupted"),
        SubtaskStatus::TimedOut => format!("Task {task_label} timed out"),
        SubtaskStatus::Created | SubtaskStatus::Running => unreachable!("terminal status"),
    };
    tokio::spawn(async move {
        let outcome = match terminal {
            PartState::Completed => {
                let final_text = match manager.session_store().load(child_session_id).await {
                    Ok(view) => final_child_text_from_parts(&view.parts)
                        .unwrap_or_else(|| "Task completed.".to_string()),
                    Err(error) => {
                        tracing::error!(
                            session_id = child_session_id,
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "load a completed child session for its final task result",
                                &error,
                            ),
                            "task completion notification is using a fallback result"
                        );
                        "Task completed, but its final result could not be loaded.".to_string()
                    }
                };
                Ok(final_text)
            }
            _ => Err(failure.unwrap_or_else(|| {
                background_failure(
                    ProcessStatus::Failed,
                    "The delegated task ended without a persisted failure detail.".to_owned(),
                )
            })),
        };
        // Wake the model with a completion notification (the agena analog of
        // Claude Code's `<task-notification>`); the task's final text is
        // wrapped in `<result>` so the model can distinguish it from its own
        // turn output. The summary is a one-line `Task "…" finished` (mirroring
        // Claude's notification headline); a failure appends the surfaced
        // reason. The settle terminalizes the tool part, appends the
        // notification onto the launching run, and wakes the model in one
        // atomic step.
        let summary = match &outcome {
            Ok(_) => summary_base,
            Err(failure) => {
                let reason = failure.user.fallback.trim().trim_end_matches('.');
                if reason.is_empty() {
                    summary_base
                } else {
                    format!("{summary_base}: {reason}")
                }
            }
        };
        let notification = SystemNotificationContent {
            operation_id: task_id.clone(),
            operation_kind: "task".to_string(),
            status: notification_status,
            summary,
            body: match &outcome {
                Ok(text) => format!("<result>{text}</result>"),
                Err(failure) => failure.user.fallback.clone(),
            },
            ..Default::default()
        };
        if let Err(error) = manager
            .settle_background_operation(
                parent_id,
                "task",
                &task_id,
                terminal,
                outcome,
                notification,
            )
            .await
        {
            tracing::warn!(
                target: "agena_background",
                %parent_id, %task_id, %error,
                "failed to settle background task operation"
            );
        }
    });
}

/// The child session's freshest assistant text, used as the completed task
/// part's terminal summary. Mirrors the subtask runner's final-text scan.
fn final_child_text_from_parts(parts: &[Part]) -> Option<String> {
    parts
        .iter()
        .rev()
        .find_map(|part| {
            part.content
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| part.summary.clone())
        })
        .filter(|text| !text.trim().is_empty())
}

/// Shell log reader: translates [`crate::MonitorRead`] into the unified
/// [`BackgroundActivityLogRead`] contract.
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
            })
            .collect(),
        last_seq: read.last_seq,
        has_more: read.has_more,
        dropped_lines: read.dropped_lines,
        exit_code: read.exit_code,
        completion_reason: read.completion_reason,
    })
}

/// Read a delegated task's transcript as unified log lines.
/// `session_manager` may be missing during early bootstrap or after reload;
/// callers treat that as an empty result.
pub(crate) async fn read_task_logs(
    session_manager: Option<&Arc<SessionManager>>,
    parent_session_id: i64,
    task_id: &str,
    after_cursor: i64,
) -> BackgroundActivityLogRead {
    let Some(manager) = session_manager else {
        return empty_task_logs(task_id, after_cursor);
    };
    match manager
        .read_subtask_output(parent_session_id, task_id, after_cursor, 200)
        .await
    {
        Ok(output) => {
            let mut lines = Vec::new();
            let mut last_seq = after_cursor;
            for chunk in output.chunks {
                last_seq = chunk.cursor;
                let text = format!(
                    "{}: {}",
                    format!("{:?}", chunk.role).to_lowercase(),
                    chunk.text
                );
                lines.push(BackgroundActivityLogLine {
                    seq: chunk.cursor as u64,
                    stream: "message".to_string(),
                    ts_ms: chunk.created_at_ms,
                    text,
                });
            }
            BackgroundActivityLogRead {
                activity_id: format!("task_{task_id}"),
                status: BackgroundActivityStatus::Running,
                lines,
                last_seq: last_seq as u64,
                has_more: output.has_more,
                dropped_lines: 0,
                exit_code: None,
                completion_reason: None,
            }
        }
        Err(error) => {
            tracing::warn!(
                parent_session_id,
                task_id,
                after_cursor,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to read delegated-task logs; returning an empty activity-log page",
                    &error,
                ),
                "delegated-task activity log is temporarily unavailable"
            );
            empty_task_logs(task_id, after_cursor)
        }
    }
}

fn empty_task_logs(task_id: &str, cursor: i64) -> BackgroundActivityLogRead {
    BackgroundActivityLogRead {
        activity_id: format!("task_{task_id}"),
        status: BackgroundActivityStatus::Running,
        lines: Vec::new(),
        last_seq: cursor as u64,
        has_more: false,
        dropped_lines: 0,
        exit_code: None,
        completion_reason: None,
    }
}
