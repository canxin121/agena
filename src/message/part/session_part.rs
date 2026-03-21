use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{PartContent, PartKind};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionMessagePart {
    pub id: i64,
    pub message_id: i64,
    pub created_at: DateTime<Utc>,
    #[serde(flatten)]
    pub content: PartContent,
}

impl SessionMessagePart {
    pub const fn kind(&self) -> PartKind {
        self.content.kind()
    }

    pub fn text(&self) -> Option<&str> {
        self.content.text_value()
    }

    pub fn reasoning_text(&self) -> Option<&str> {
        self.content.reasoning_value()
    }
}
