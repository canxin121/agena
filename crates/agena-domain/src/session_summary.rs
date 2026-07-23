use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{SessionLifecycleState, SessionRelationKind, SubtaskStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionListRequest {
    #[serde(default)]
    pub offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(default)]
    pub include_subagents: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub depth: i64,
    pub root_id: i64,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    pub relation_kind: SessionRelationKind,
    pub lifecycle_state: SessionLifecycleState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_cutoff_seq_global: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_message_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtask_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtask_status: Option<SubtaskStatus>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: u64,
    pub child_session_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
}
