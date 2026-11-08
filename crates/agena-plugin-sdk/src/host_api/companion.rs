use agena_superagent::{AssistantPatch, ReminderSpec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompanionAction {
    Profile,
    Configure,
    Memories,
    Recall,
    Remember,
    Forget,
    Remind,
    Reminders,
    PauseReminder,
    ResumeReminder,
    CancelReminder,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, agena_macros::ToolInput)]
#[serde(deny_unknown_fields)]
pub struct CompanionRequest {
    pub action: CompanionAction,
    /// Exact quote from the CURRENT direct user message. Required for writes.
    #[serde(default)]
    pub user_quote: Option<String>,
    /// Current profile/reminder revision from live context; never guess it.
    #[serde(default)]
    pub expected_revision: Option<i64>,
    #[serde(default)]
    pub patch: Option<AssistantPatch>,
    /// Stable descriptive memory key, not a path. Use recall before corrections.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// user / feedback / project / reference / other. Personal preferences use user.
    #[serde(default)]
    pub memory_type: Option<String>,
    /// Revision returned by memory recall. Required for correction/deletion.
    #[serde(default)]
    pub expected_sha256: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub reminder: Option<ReminderSpec>,
    #[serde(default)]
    pub reminder_id: Option<String>,
}
