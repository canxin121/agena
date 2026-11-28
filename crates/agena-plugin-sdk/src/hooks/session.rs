use serde::{Deserialize, Serialize};

// ── run lifecycle ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Conversation,
    Compaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Emitted once after an execution is claimed and before its first effect.
pub struct PreRunInput {
    pub session_id: i64,
    pub model: String,
    pub execution_id: String,
    pub kind: RunKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Emitted once when an execution attempt settles, including failure/cancel.
pub struct PostRunInput {
    pub session_id: i64,
    pub model: String,
    pub execution_id: String,
    pub kind: RunKind,
    pub status: RunStatus,
}

/// A user turn whose final response has been committed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedTurnCompletedInput {
    pub session_id: i64,
    pub user_text: String,
    pub assistant_text: String,
}

// ── session.start ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Source that started a session.
pub enum SessionStartSource {
    Startup,
    Resume,
    Clear,
    Compact,
    Fork,
}

impl SessionStartSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Resume => "resume",
            Self::Clear => "clear",
            Self::Compact => "compact",
            Self::Fork => "fork",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of the session start hook.
pub struct SessionStartInput {
    pub session_id: i64,
    pub source: SessionStartSource,
    pub workspace_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_cache: Option<SessionStartCacheContext>,
}

/// Cache evidence for a resume or fork with a completed provider response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartCacheContext {
    pub seconds_since_last_response: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    pub prompt_cache_likely_expired: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cache_write_usd: Option<f64>,
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
    /// A custom title requested at session startup, resume, or fork.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_title: Option<String>,
    /// Stop the next conversation turn or end the current turn after a
    /// committed automatic compaction. Used by imported Codex hooks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// Refresh filesystem skill and command catalogs after the hooks settle.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reload_skills: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ImportedSessionSetupInput {
    pub session_id: i64,
    pub trigger: agena_session_api::commands::SessionSetupTrigger,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionFileChange {
    Add,
    Change,
    Unlink,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFileChangedInput {
    pub session_id: i64,
    pub path: std::path::PathBuf,
    pub change: SessionFileChange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFileChangedPatch {
    pub watch_paths: Vec<std::path::PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCwdChangedInput {
    pub session_id: i64,
    pub old_cwd: std::path::PathBuf,
    pub new_cwd: std::path::PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedInstructionsLoadedInput {
    pub session_id: i64,
    pub compacted: bool,
    pub files: Vec<ImportedInstructionFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedInstructionFile {
    pub path: std::path::PathBuf,
    pub content_hash: String,
    pub trigger_file_path: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_path: Option<std::path::PathBuf>,
    #[serde(default)]
    pub scope: ImportedInstructionScope,
    #[serde(default)]
    pub origin: ImportedInstructionOrigin,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub enum ImportedInstructionScope {
    #[default]
    Project,
    Local,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportedInstructionOrigin {
    #[default]
    Conventional,
    PathGlobMatch {
        globs: Vec<String>,
    },
    Include {
        parent_file_path: std::path::PathBuf,
    },
}
