use super::*;

pub use agena_api::resource::{MessageResource, PartLoadMode};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MessageListQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub parts: PartLoadMode,
}
