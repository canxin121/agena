//! Agent lifecycle interception contracts. Each event owns a typed decision;
//! no universal JSON decision or protocol-specific compatibility layer.

use serde::{Deserialize, Serialize};

/// Checked immediately before a prepared provider request starts network I/O.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRequestBeforeInput {
    pub session_id: i64,
    pub provider: String,
    pub model: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRequestBeforePatch {
    /// Additional policy denial. It cannot grant permissions or rewrite the
    /// committed provider request.
    pub deny_reason: Option<String>,
}

/// Emitted after a model round settles, including errors and cancellation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRequestAfterInput {
    pub session_id: i64,
    pub provider: String,
    pub model: String,
    pub request_id: String,
    pub status: String,
    pub duration_ms: u64,
    pub usage: Option<serde_json::Value>,
}

/// A recoverable model failure at the boundary between completed rounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRequestErrorInput {
    pub session_id: i64,
    pub provider: String,
    pub model: String,
    pub request_id: String,
    pub message: String,
    pub attempt: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRequestErrorPatch {
    /// Retry by creating a NEW round, never by replaying an already admitted
    /// provider effect. The session controls retry budgets.
    pub retry: bool,
}

/// About to compact session history, before any compaction effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionBeforeInput {
    pub session_id: i64,
    pub model: String,
    pub automatic: bool,
    pub estimated_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompactionBeforePatch {
    pub deny_reason: Option<String>,
}

/// Compaction attempt settled, including failures and no-op attempts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionAfterInput {
    pub session_id: i64,
    pub model: String,
    pub automatic: bool,
    pub status: String,
}

/// A child Agent is about to be accepted under its parent's authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentStartInput {
    pub session_id: i64,
    pub child_session_id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubagentStartPatch {
    pub deny_reason: Option<String>,
}

/// A child Agent settled and its terminal state is known.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentStopInput {
    pub session_id: i64,
    pub child_session_id: i64,
    pub name: String,
    pub status: String,
}

/// A tool is about to require interactive permission. This hook can add a
/// denial but cannot grant permission or replace the host's approval decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequestInput {
    pub session_id: i64,
    pub tool_name: String,
    pub action: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionRequestPatch {
    pub deny_reason: Option<String>,
}
