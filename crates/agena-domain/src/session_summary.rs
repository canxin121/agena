use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{ExecutionAccess, SessionLifecycleState, SessionRelationKind, SubtaskStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Pagination and filter request for listing sessions.
pub struct SessionListRequest {
    #[serde(default)]
    pub offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(default)]
    pub include_subagents: bool,
    /// Restrict to direct children of this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<i64>,
    /// Restrict to root sessions (`parent_id IS NULL`).
    #[serde(default)]
    pub roots_only: bool,
    /// Case-insensitive title substring filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Summary of a session (identity, relation, lifecycle).
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
    pub subtask_access: Option<ExecutionAccess>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtask_status: Option<SubtaskStatus>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: u64,
    pub child_session_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
}
