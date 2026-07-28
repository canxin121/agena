use serde::{Deserialize, Serialize};

use crate::{ExecutionId, MessageId, PartId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptCompactionStrategy {
    #[default]
    LocalSummary,
    OpenAiResponses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptCompactionTrigger {
    #[default]
    Manual,
    Auto,
    Reactive,
}

/// Safe, provider-agnostic metadata for a visible compaction activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCompactionActivity {
    pub checkpoint_id: String,
    pub generation: u64,
    pub compacted_through_message_id: i64,
    pub trigger: PromptCompactionTrigger,
    pub strategy: PromptCompactionStrategy,
    pub before_tokens: u64,
    pub after_tokens: u64,
}

/// Durable, user-visible compaction lifecycle event. This is application
/// activity, not conversation content, and must never enter a model request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCompactionCompletedEvent {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standalone_message_id: Option<MessageId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standalone_part_id: Option<PartId>,
    pub activity: PromptCompactionActivity,
    pub ts_ms: i64,
}

impl PromptCompactionActivity {
    pub fn reduced_tokens(&self) -> u64 {
        self.before_tokens.saturating_sub(self.after_tokens)
    }

    pub fn reduction_percent(&self) -> f64 {
        if self.before_tokens == 0 {
            return 0.0;
        }
        self.reduced_tokens() as f64 * 100.0 / self.before_tokens as f64
    }
}

#[cfg(test)]
mod tests {
    use super::{PromptCompactionStrategy, PromptCompactionTrigger};

    #[test]
    fn compaction_preferences_have_stable_defaults_and_wire_values() {
        assert_eq!(
            PromptCompactionStrategy::default(),
            PromptCompactionStrategy::LocalSummary
        );
        assert_eq!(
            PromptCompactionTrigger::default(),
            PromptCompactionTrigger::Manual
        );
        assert_eq!(
            serde_json::to_string(&PromptCompactionStrategy::OpenAiResponses).unwrap(),
            "\"open_ai_responses\""
        );
        assert_eq!(
            serde_json::from_str::<PromptCompactionTrigger>("\"reactive\"").unwrap(),
            PromptCompactionTrigger::Reactive
        );
    }
}
