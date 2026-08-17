use serde::{Deserialize, Serialize};

// ── agent.stop ─────────────────────────────────────────────────────────────

/// Fired when the agent is about to stop after completing a run. Plugins
/// can inspect the last assistant message and optionally block the stop
/// (causing the agent to continue with an injected follow-up message).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStopInput {
    pub session_id: i64,
    /// True when a `Stop` hook is already active (prevents infinite loops).
    #[serde(default)]
    pub stop_hook_active: bool,
    /// The final assistant message text, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_assistant_message: Option<String>,
    /// When present, the run just failed with this error before stopping.
    /// Hooks can inspect it to decide whether to continue (for example the
    /// workflow plan autorun keeps retrying after a failed run instead of
    /// aborting the plan).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch describing how the agent run should stop.
pub struct AgentStopPatch {
    /// If set, the stop is blocked and this message is injected as the next
    /// user message, causing the next run to continue automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continue_with_message: Option<String>,
    /// Human-readable reason recorded in the session log when blocking stop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ── agent.cancel ───────────────────────────────────────────────────────────

/// Fired after a user cancellation has been accepted for an active
/// execution. Plugins use this lifecycle hook to clear execution-local
/// automation, such as an active workflow plan's autorun flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCancelInput {
    pub session_id: i64,
    pub execution_id: String,
}
