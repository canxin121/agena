use serde::{Deserialize, Serialize};

use crate::hooks::ChatMessage;

// ── session.compacting ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCompactingInput {
    pub session_id: i64,
    pub messages: Vec<ChatMessage>,
    pub strategy: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionCompactingPatch {
    /// Replace the message list that will be handed to the compaction
    /// strategy. If `None`, the original list is used unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<ChatMessage>>,
}

// ── session.compacted ──────────────────────────────────────────────────────

/// Fired after compaction completes. Notification — no patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCompactedInput {
    pub session_id: i64,
    pub strategy: String,
    pub summary: String,
    pub messages_before: usize,
    pub messages_after: usize,
}

// ── session.start ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStartSource {
    Startup,
    Resume,
    Clear,
    Compact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Synthetic user message injected as the first turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_user_message: Option<String>,
}

// ── session.end ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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
