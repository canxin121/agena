//! Unified domain model for long-running background activities.
//!
//! Agena tools create background work of several kinds — background shell
//! processes, delegated subagent tasks, runtime maintenance tasks, managed
//! browser sessions, scheduled jobs — and this module is the single wire
//! contract they all share. A [`BackgroundActivity`] is a stable descriptor
//! plus a log cursor; transports (REST, WS, SSE) and UIs (TUI, web) can list,
//! inspect, follow, and control every kind without knowing its source.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

/// Source category of a background activity.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum BackgroundActivityKind {
    /// A long-lived shell process or monitor spawned by `shell.run`.
    Shell,
    /// A delegated subagent task spawned by `tasks.create` / `tasks.run`.
    Task,
    /// A runtime-maintained maintenance task (marketplace sync, catalog
    /// refresh, runtime reload).
    Runtime,
    /// A managed interactive browser session (`web.browser_*`).
    Browser,
}

impl BackgroundActivityKind {
    pub const ALL: [Self; 4] = [Self::Shell, Self::Task, Self::Runtime, Self::Browser];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Task => "task",
            Self::Runtime => "runtime",
            Self::Browser => "browser",
        }
    }
}

/// Lifecycle status shared by every activity source.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum BackgroundActivityStatus {
    /// Created but not yet executing (delegated tasks queue behind admission
    /// limits).
    Pending,
    Running,
    Succeeded,
    Failed,
    /// Cancelled by an explicit operator action.
    Cancelled,
    /// Terminated by an explicit operator stop.
    Stopped,
}

impl BackgroundActivityStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }

    pub fn is_terminal(self) -> bool {
        !self.is_active()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Stopped => "stopped",
        }
    }
}

/// Stable descriptor for one unit of background work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundActivity {
    /// Stable id unique across sources; ids are prefixed by source
    /// (`proc_…` shell, `task_…` delegated, `rtask_…` runtime, `browser_…`).
    pub id: String,
    pub kind: BackgroundActivityKind,
    pub status: BackgroundActivityStatus,
    /// Human-facing headline (e.g. “Run process · cargo build”).
    pub title: String,
    /// Secondary human-facing detail (command summary or prompt excerpt).
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    /// Owning session when the activity is tied to a session run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    /// Parent session for delegated child-session work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<i64>,
    pub created_at_ms: i64,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Free-form status/progress message (e.g. “Installed 3 plugins”).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::UserProblem>,
    /// Highest contiguous log sequence available for this activity.
    pub last_seq: u64,
    /// Whether more log lines exist before the cursor than the source retains.
    pub has_more: bool,
    /// Cumulative dropped/evicted log lines.
    pub dropped_lines: u64,
    /// Whether an operator may stop/cancel the activity while running.
    pub cancellable: bool,
    /// Whether an operator may dismiss the activity from the list.
    pub dismissible: bool,
}

impl BackgroundActivity {
    pub fn is_active(&self) -> bool {
        self.status.is_active()
    }

    pub fn running_seconds(&self, now_ms: i64) -> Option<i64> {
        let start = self.started_at_ms;
        let end = self.finished_at_ms.unwrap_or(now_ms);
        (end >= start).then_some((end - start) / 1000)
    }
}

/// One line of activity output in the unified log cursor protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackgroundActivityLogLine {
    pub seq: u64,
    /// `stdout` / `stderr` for shell, `message` for agent transcripts.
    pub stream: String,
    pub ts_ms: i64,
    pub text: String,
}

/// Unified log read result. Mirrors the shell monitor read contract so every
/// source can offer incremental `since_seq` tails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackgroundActivityLogRead {
    pub activity_id: String,
    pub status: BackgroundActivityStatus,
    pub lines: Vec<BackgroundActivityLogLine>,
    pub last_seq: u64,
    pub has_more: bool,
    pub dropped_lines: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_reason: Option<String>,
}

/// Why a [`BackgroundActivityChangedEvent`] was published.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum BackgroundActivityEventReason {
    Started,
    Updated,
    Finished,
    Dismissed,
}

/// Bus event published for every activity mutation so TUI/Web can react live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundActivityChangedEvent {
    pub activity_id: String,
    pub reason: BackgroundActivityEventReason,
    pub activity: BackgroundActivity,
    pub ts_ms: i64,
}

/// Query filter shared by list endpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BackgroundActivityFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<BackgroundActivityKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<BackgroundActivityStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    /// If set, only active (pending/running) activities are returned.
    pub active_only: bool,
}

impl BackgroundActivityFilter {
    pub fn matches(&self, activity: &BackgroundActivity) -> bool {
        if !self.kinds.is_empty() && !self.kinds.contains(&activity.kind) {
            return false;
        }
        if !self.statuses.is_empty() && !self.statuses.contains(&activity.status) {
            return false;
        }
        if let Some(session_id) = self.session_id {
            let matches = activity.session_id == Some(session_id)
                || activity.parent_session_id == Some(session_id);
            if !matches {
                return false;
            }
        }
        if self.active_only && !activity.is_active() {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BackgroundActivity {
        BackgroundActivity {
            id: "proc_1".into(),
            kind: BackgroundActivityKind::Shell,
            status: BackgroundActivityStatus::Running,
            title: "Run process · cargo build".into(),
            description: "cargo build".into(),
            command: Some("cargo build".into()),
            workdir: Some("/repo".into()),
            session_id: Some(7),
            parent_session_id: None,
            created_at_ms: 1,
            started_at_ms: 1,
            finished_at_ms: None,
            exit_code: None,
            message: None,
            failure: None,
            last_seq: 0,
            has_more: false,
            dropped_lines: 0,
            cancellable: true,
            dismissible: true,
        }
    }

    #[test]
    fn status_lifecycle_helpers() {
        assert!(BackgroundActivityStatus::Running.is_active());
        assert!(BackgroundActivityStatus::Failed.is_terminal());
        assert_eq!(BackgroundActivityStatus::Stopped.as_str(), "stopped");
    }

    #[test]
    fn kind_round_trips_wire_names() {
        for kind in BackgroundActivityKind::ALL {
            let json = serde_json::to_string(&kind).unwrap();
            let back: BackgroundActivityKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, back);
            assert_eq!(kind.as_str(), json.trim_matches('"'));
        }
    }

    #[test]
    fn filter_scopes_by_session_and_active() {
        let activity = sample();
        let filter = BackgroundActivityFilter {
            kinds: vec![BackgroundActivityKind::Shell],
            statuses: vec![BackgroundActivityStatus::Running],
            session_id: Some(7),
            active_only: true,
        };
        assert!(filter.matches(&activity));
        assert!(
            !BackgroundActivityFilter {
                session_id: Some(8),
                ..Default::default()
            }
            .matches(&activity)
        );
        assert!(
            !BackgroundActivityFilter {
                statuses: vec![BackgroundActivityStatus::Failed],
                ..Default::default()
            }
            .matches(&activity)
        );
    }

    #[test]
    fn running_seconds_uses_finish_when_available() {
        let activity = sample();
        assert_eq!(activity.running_seconds(1_000), Some(0));
        let mut finished = sample();
        finished.finished_at_ms = Some(61_000);
        assert_eq!(finished.running_seconds(1_000), Some(60));
    }
}
