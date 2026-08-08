use serde::{Deserialize, Serialize};

// ── user.prompt.submit ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of the user prompt submit hook.
pub struct UserPromptSubmitInput {
    pub session_id: i64,
    /// The raw text the user submitted.
    pub prompt: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied to a submitted user prompt.
pub struct UserPromptSubmitPatch {
    /// Replace the prompt with a different text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Inject extra context appended to the prompt (not visible to the user).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
    /// If set, the prompt is blocked and this reason is shown to the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_reason: Option<String>,
}
