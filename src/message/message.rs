use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::message::part::{PartKind, SessionMessagePart};
use crate::message::usage::MessageUsage;
use crate::role::Role;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionMessage {
    pub id: i64,
    pub role: Role,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<SessionMessagePart>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<MessageUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
}

impl SessionMessage {
    pub fn empty_with_id(id: i64, role: Role, created_at: DateTime<Utc>) -> Self {
        Self {
            id,
            role,
            parts: Vec::new(),
            created_at,
            metadata: HashMap::new(),
            usage: None,
            finish: None,
        }
    }

    pub fn push_part(&mut self, mut part: SessionMessagePart) {
        part.message_id = self.id;
        self.parts.push(part);
    }

    pub fn text_parts(&self) -> impl Iterator<Item = &str> {
        self.parts.iter().filter_map(SessionMessagePart::text)
    }

    pub fn reasoning_parts(&self) -> impl Iterator<Item = &str> {
        self.parts
            .iter()
            .filter_map(SessionMessagePart::reasoning_text)
    }

    pub fn parts_of_kind(&self, kind: PartKind) -> impl Iterator<Item = &SessionMessagePart> {
        self.parts.iter().filter(move |part| part.kind() == kind)
    }

    pub fn get_text(&self) -> String {
        self.text_parts().collect::<String>()
    }

    pub fn get_reasoning(&self) -> String {
        self.reasoning_parts().collect::<String>()
    }
}
