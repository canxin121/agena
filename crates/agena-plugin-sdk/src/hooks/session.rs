use serde::{Deserialize, Serialize};

// ── run lifecycle ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of the pre-run hook.
pub struct PreRunInput {
    pub session_id: i64,
    pub model: String,
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of the post-run hook.
pub struct PostRunInput {
    pub session_id: i64,
    pub model: String,
    pub status: String,
    pub message_count: usize,
}

// ── session.start ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Source that started a session.
pub enum SessionStartSource {
    Startup,
    Resume,
    Clear,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of the session start hook.
pub struct SessionStartInput {
    pub session_id: i64,
    pub source: SessionStartSource,
    pub workspace_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Patch returned by a plugin in response to `session.start`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionStartPatch {
    /// Extra context injected into the system prompt for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
    /// Synthetic user message injected before the first run starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_user_message: Option<String>,
}

// ── session.end ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Reason a session ended.
pub enum SessionEndReason {
    Clear,
    Resume,
    Logout,
    UserExit,
    Other,
}

/// Fired when a session ends. Notification — no patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEndInput {
    pub session_id: i64,
    pub reason: SessionEndReason,
}
