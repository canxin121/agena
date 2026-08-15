//! Wire resources for the unified background-activity service.

use serde::{Deserialize, Serialize};

use agena_domain::{BackgroundActivity, BackgroundActivityLogRead};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// A background activity (process, task) with its log state.
pub struct BackgroundActivityResource {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_part_id: Option<i64>,
    pub created_at_ms: i64,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_event_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::UserProblem>,
    pub last_seq: u64,
    pub has_more: bool,
    pub dropped_lines: u64,
    pub cancellable: bool,
    pub dismissible: bool,
    /// Controls valid for the member in its current state. Clients render
    /// this list instead of reimplementing per-kind lifecycle rules.
    #[serde(default)]
    pub controls: Vec<String>,
}

impl From<&BackgroundActivity> for BackgroundActivityResource {
    fn from(activity: &BackgroundActivity) -> Self {
        Self {
            id: activity.id.clone(),
            kind: activity.kind.as_str().to_string(),
            status: activity.status.as_str().to_string(),
            title: activity.title.clone(),
            description: activity.description.clone(),
            command: activity.command.clone(),
            workdir: activity.workdir.clone(),
            session_id: activity.session_id,
            parent_session_id: activity.parent_session_id,
            operation_id: activity.operation_id.clone(),
            source_part_id: activity.source_part_id,
            created_at_ms: activity.created_at_ms,
            started_at_ms: activity.started_at_ms,
            finished_at_ms: activity.finished_at_ms,
            next_event_at_ms: activity.next_event_at_ms,
            exit_code: activity.exit_code,
            message: activity.message.clone(),
            failure: activity.failure.clone(),
            last_seq: activity.last_seq,
            has_more: activity.has_more,
            dropped_lines: activity.dropped_lines,
            cancellable: activity.cancellable,
            dismissible: activity.dismissible,
            controls: activity_controls(activity),
        }
    }
}

fn activity_controls(activity: &BackgroundActivity) -> Vec<String> {
    use agena_domain::{BackgroundActivityKind, BackgroundActivityStatus};
    match activity.kind {
        BackgroundActivityKind::Cron => match activity.status {
            BackgroundActivityStatus::Paused => vec!["resume".to_owned(), "delete".to_owned()],
            BackgroundActivityStatus::Stopped if activity.next_event_at_ms.is_none() => {
                vec!["dismiss".to_owned()]
            }
            status if status.is_active() => vec!["pause".to_owned(), "delete".to_owned()],
            _ => vec!["delete".to_owned()],
        },
        _ if activity.is_active() && activity.cancellable => vec!["stop".to_owned()],
        _ if activity.dismissible => vec!["dismiss".to_owned()],
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// One line of a background activity log.
pub struct BackgroundActivityLogLineResource {
    pub seq: u64,
    pub stream: String,
    pub ts_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// A page of background activity log lines.
pub struct BackgroundActivityLogResource {
    pub activity_id: String,
    pub status: String,
    pub lines: Vec<BackgroundActivityLogLineResource>,
    pub last_seq: u64,
    pub has_more: bool,
    pub dropped_lines: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_reason: Option<String>,
}

impl From<BackgroundActivityLogRead> for BackgroundActivityLogResource {
    fn from(read: BackgroundActivityLogRead) -> Self {
        Self {
            activity_id: read.activity_id,
            status: read.status.as_str().to_string(),
            lines: read
                .lines
                .into_iter()
                .map(|line| BackgroundActivityLogLineResource {
                    seq: line.seq,
                    stream: line.stream,
                    ts_ms: line.ts_ms,
                    text: line.text,
                })
                .collect(),
            last_seq: read.last_seq,
            has_more: read.has_more,
            dropped_lines: read.dropped_lines,
            exit_code: read.exit_code,
            completion_reason: read.completion_reason,
        }
    }
}
