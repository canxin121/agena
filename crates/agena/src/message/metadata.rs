use sea_orm::{
    FromJsonQueryResult,
    entity::prelude::{DeriveActiveEnum, EnumIter},
};
use serde::{Deserialize, Serialize};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
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
            model_verbosity: None,
            provider_metadata: None,
            tags: Vec::new(),
        }
    }
}

impl MessageMetadata {
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|existing| existing == tag)
    }

    pub fn add_tag(&mut self, tag: impl Into<String>) {
        let tag = tag.into();
        if !tag.trim().is_empty() && !self.has_tag(tag.as_str()) {
            self.tags.push(tag);
        }
    }
}
