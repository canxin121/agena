use sea_orm::entity::prelude::{DeriveActiveEnum, EnumIter};
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
    #[sea_orm(num_value = 4)]
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageMetadata {
    pub source: MessageSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_by_call_id: Option<i64>,
    pub model_provider_id: String,
    pub model_id: String,
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
            model_id: String::new(),
            tags: Vec::new(),
        }
    }
}
