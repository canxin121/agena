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

/// One physical provider request observed by the imported sampling hooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedModelAttemptInput {
    /// The Agena session that owns the provider call and hook authority.
    pub session_id: i64,
    pub root_session_id: i64,
    pub turn_id: String,
    pub request_id: String,
    pub attempt_number: u32,
    pub kind: ImportedSamplingKind,
    pub model: String,
    #[serde(default)]
    pub agent_type: Option<String>,
    /// Absent before sending the request.
    #[serde(default)]
    pub outcome: Option<ImportedModelAttemptOutcome>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportedSamplingKind {
    Sampling,
    ForegroundCompaction,
    BackgroundCompaction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedModelAttemptOutcome {
    pub status: ImportedModelAttemptStatus,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub response_id: Option<String>,
    pub token_usage: Option<ImportedModelTokenUsage>,
    pub usage: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportedModelAttemptStatus {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedModelTokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportedModelAttemptCharge {
    pub extra_codex_rollout_budget_units: f64,
}

/// About to compact session history, before any compaction effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionBeforeInput {
    pub session_id: i64,
    pub model: String,
    pub automatic: bool,
    pub estimated_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_instructions: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompactionBeforePatch {
    pub deny_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_instructions: Option<String>,
}

/// Compaction attempt settled, including failures and no-op attempts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionAfterInput {
    pub session_id: i64,
    pub model: String,
    pub automatic: bool,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_summary: Option<String>,
}

/// A child Agent is about to be accepted under its parent's authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentStartInput {
    pub session_id: i64,
    pub child_session_id: i64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubagentStartPatch {
    pub deny_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionRequestPatch {
    pub deny_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportedTaskChange {
    Create,
    Complete,
}

/// Before the built-in task bridge commits a task mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedTaskBeforeInput {
    pub change: ImportedTaskChange,
    pub session_id: i64,
    pub task_id: String,
    pub subject: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}
