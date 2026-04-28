use serde::{Deserialize, Serialize};

// ── agent.stop ─────────────────────────────────────────────────────────────

/// Fired when the agent is about to stop after completing a turn. Plugins
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentStopPatch {
    /// If set, the stop is blocked and this message is injected as the next
    /// user turn, causing the agent to continue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continue_with_message: Option<String>,
    /// Human-readable reason recorded in the session log when blocking stop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
