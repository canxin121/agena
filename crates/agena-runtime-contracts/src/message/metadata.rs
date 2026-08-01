use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use agena_domain::{AssistantReasoningField, MessageSource};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
pub struct MessageProviderState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<AssistantReasoningField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub gemini_thought_signatures: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anthropic_thinking_blocks: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub openai_reasoning_items: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_chat_reasoning_details: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_reasoning_opaque: Option<String>,
}

impl MessageProviderState {
    pub fn is_empty(&self) -> bool {
        self.assistant_reasoning_field.is_none()
            && self.response_id.is_none()
            && self.gemini_thought_signatures.is_empty()
            && self.anthropic_thinking_blocks.is_empty()
            && self.openai_reasoning_items.is_empty()
            && self.openai_chat_reasoning_details.is_none()
            && self.copilot_reasoning_opaque.is_none()
    }
}

impl From<MessageProviderState> for agena_provider::CompletionInputProviderState {
    fn from(value: MessageProviderState) -> Self {
        Self {
            assistant_reasoning_field: value.assistant_reasoning_field,
            response_id: value.response_id,
            gemini_thought_signatures: value.gemini_thought_signatures,
            anthropic_thinking_blocks: value.anthropic_thinking_blocks,
            openai_reasoning_items: value.openai_reasoning_items,
            openai_chat_reasoning_details: value.openai_chat_reasoning_details,
            copilot_reasoning_opaque: value.copilot_reasoning_opaque,
        }
    }
}

impl From<agena_provider::CompletionInputProviderState> for MessageProviderState {
    fn from(value: agena_provider::CompletionInputProviderState) -> Self {
        Self {
            assistant_reasoning_field: value.assistant_reasoning_field,
            response_id: value.response_id,
            gemini_thought_signatures: value.gemini_thought_signatures,
            anthropic_thinking_blocks: value.anthropic_thinking_blocks,
            openai_reasoning_items: value.openai_reasoning_items,
            openai_chat_reasoning_details: value.openai_chat_reasoning_details,
            copilot_reasoning_opaque: value.copilot_reasoning_opaque,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
pub struct MessageMetadata {
    pub source: MessageSource,
    /// Optional caller-provided key used to make an externally scheduled or
    /// retried user message idempotent across process restarts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_turn_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_by_call_id: Option<i64>,
    /// True for a tool operation initiated directly by a trusted UI/API
    /// surface rather than emitted by a model turn. Permission replies execute
    /// the tool but must not start an unrelated model continuation.
    #[serde(default)]
    pub externally_initiated_tool: bool,
    pub model_provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_adapter_id: Option<String>,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_speed_mode: Option<String>,
}

impl Default for MessageMetadata {
    fn default() -> Self {
        Self {
            source: MessageSource::Assistant,
            idempotency_key: None,
            model_turn_id: None,
            parent_message_id: None,
            generated_by_call_id: None,
            externally_initiated_tool: false,
            model_provider_id: String::new(),
            model_adapter_id: None,
            model_id: String::new(),
            model_thinking_mode: None,
            model_speed_mode: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MessageProviderState;
    use std::collections::BTreeMap;

    #[test]
    fn provider_replay_state_round_trips_through_contract_value() {
        let state = MessageProviderState {
            response_id: Some("response-1".to_owned()),
            gemini_thought_signatures: BTreeMap::from([("part".to_owned(), "sig".to_owned())]),
            ..Default::default()
        };
        let contract: agena_provider::CompletionInputProviderState = state.clone().into();
        let restored = MessageProviderState::from(contract);
        assert_eq!(restored, state);
    }
}
