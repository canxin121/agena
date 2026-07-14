use sea_orm::{
    FromJsonQueryResult,
    entity::prelude::{DeriveActiveEnum, EnumIter},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use strum::{AsRefStr, Display, EnumString};

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    AsRefStr,
    Display,
    EnumString,
    EnumIter,
    DeriveActiveEnum,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[sea_orm(rs_type = "i8", db_type = "TinyInteger")]
pub enum MessageSource {
    #[sea_orm(num_value = 1)]
    User,
    #[sea_orm(num_value = 2)]
    Assistant,
    #[sea_orm(num_value = 3)]
    System,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    AsRefStr,
    Display,
    EnumString,
    EnumIter,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AssistantReasoningField {
    ReasoningContent,
    ReasoningDetails,
}

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
}

impl MessageProviderState {
    pub fn is_empty(&self) -> bool {
        self.assistant_reasoning_field.is_none()
            && self.response_id.is_none()
            && self.gemini_thought_signatures.is_empty()
            && self.anthropic_thinking_blocks.is_empty()
            && self.openai_reasoning_items.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
pub struct MessageMetadata {
    pub source: MessageSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_by_call_id: Option<i64>,
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
            parent_message_id: None,
            generated_by_call_id: None,
            model_provider_id: String::new(),
            model_adapter_id: None,
            model_id: String::new(),
            model_thinking_mode: None,
            model_speed_mode: None,
        }
    }
}
