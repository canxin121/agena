use agena_assistant::{AssistantPatch, ReminderSpec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompanionAction {
    Profile,
    Configure,
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
    #[serde(default)]
    pub reminder: Option<ReminderSpec>,
    #[serde(default)]
    pub reminder_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companion_rejects_memory_actions_and_fields() {
        for action in ["memories", "recall", "remember", "forget"] {
            assert!(
                serde_json::from_value::<CompanionRequest>(serde_json::json!({"action": action}))
                    .is_err()
            );
        }
        for field in [
            "name",
            "content",
            "description",
            "memory_type",
            "expected_sha256",
            "query",
        ] {
            let mut request = serde_json::json!({"action": "profile"});
            request[field] = serde_json::json!("old-memory-input");
            assert!(serde_json::from_value::<CompanionRequest>(request).is_err());
        }
        let request: CompanionRequest =
            serde_json::from_value(serde_json::json!({"action": "profile"})).unwrap();
        assert_eq!(request.action, CompanionAction::Profile);
    }
}
