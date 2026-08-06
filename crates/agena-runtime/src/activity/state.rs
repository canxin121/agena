//! Concrete activity service state: registry + source bridges + log readers.
//!
//! The runtime owns one [`ActivityRuntimeState`] for its whole lifetime. Source
//! bridges push shell processes, runtime tasks, and delegated tasks into the
//! unified registry; the application-facing [`RuntimeActivityService`]
//! implementation reads the same state.

use std::sync::Arc;

use agena_domain::{
    BackgroundActivity, BackgroundActivityKind, BackgroundActivityLogLine,
    BackgroundActivityLogRead, BackgroundActivityStatus, ProcessStatus, ProcessSummary,
    SubtaskStatusChangedEvent,
};
use agena_plugin_sdk::activity::ActivitySourceAdapter;
use agena_runtime_session::SessionManager;

use super::registry::ActivityRegistry;
use crate::{
    RuntimeBackgroundTask, RuntimeBackgroundTaskListener, RuntimeBackgroundTaskStatus,
    RuntimeBackgroundTaskRegistry,
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
/// unified activity registry.
#[derive(Debug)]
pub(crate) struct MonitorActivityBridge {
    pub(crate) registry: ActivityRegistry,
}

impl crate::MonitorListener for MonitorActivityBridge {
    fn on_started(&self, summary: &ProcessSummary) {
        self.registry.upsert(shell_activity(summary));
    }

    fn on_finished(&self, summary: &ProcessSummary) {
        self.registry.upsert(shell_activity(summary));
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
        created_at_ms: summary.started_at_ms,
        started_at_ms: summary.started_at_ms,
        finished_at_ms: summary.ended_at_ms,
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
        created_at_ms: task.created_at.timestamp_millis(),
        started_at_ms: task.started_at.timestamp_millis(),
        finished_at_ms: task.finished_at.map(|at| at.timestamp_millis()),
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

/// Project a delegated-task status event into the unified registry.
pub(crate) fn upsert_task_activity(
    registry: &ActivityRegistry,
    event: &SubtaskStatusChangedEvent,
) {
    let status = match event.status {
        agena_domain::SubtaskStatus::Created => BackgroundActivityStatus::Pending,
        agena_domain::SubtaskStatus::Running => BackgroundActivityStatus::Running,
        agena_domain::SubtaskStatus::Completed => BackgroundActivityStatus::Succeeded,
        agena_domain::SubtaskStatus::Failed => BackgroundActivityStatus::Failed,
        agena_domain::SubtaskStatus::Cancelled
        | agena_domain::SubtaskStatus::Interrupted => BackgroundActivityStatus::Cancelled,
        agena_domain::SubtaskStatus::TimedOut => BackgroundActivityStatus::Failed,
    };
    let started_at = event.started_at_ms.unwrap_or(event.ts_ms);
    registry.upsert(BackgroundActivity {
        id: format!("task_{}", event.task_id),
        kind: BackgroundActivityKind::Task,
        status,
        title: format!("Delegated task · {}", event.task_id),
        description: String::new(),
        command: None,
        workdir: None,
        session_id: Some(event.session_id),
        parent_session_id: Some(event.parent_session_id),
        created_at_ms: started_at,
        started_at_ms: started_at,
        finished_at_ms: event.finished_at_ms,
        exit_code: None,
                message: event
            .failure
            .as_ref()
            .map(|failure| failure.user.fallback.clone()),
        failure: event.failure.clone(),
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        cancellable: status.is_active(),
        dismissible: status.is_terminal(),
    });
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
        Err(_) => empty_task_logs(task_id, after_cursor),
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

