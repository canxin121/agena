use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};
use thiserror::Error;

use crate::message::metadata::MessageMetadata;
use crate::message::part::SessionMessagePart;
use crate::message::usage::MessageUsage;
use crate::role::Role;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionMessage {
    pub id: i64,
    pub role: Role,
    #[serde(default)]
    pub state: MessageStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<SessionMessagePart>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: MessageMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<MessageUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    Default,
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
pub enum MessageStatus {
    #[default]
    #[sea_orm(num_value = 1)]
    Pending,
    #[sea_orm(num_value = 2)]
    InProgress,
    #[sea_orm(num_value = 3)]
    Completed,
    #[sea_orm(num_value = 4)]
    Failed,
}

impl MessageStatus {
    pub fn can_transition(self, next: Self) -> bool {
        if self == next {
            return true;
        }

        match (self, next) {
            (Self::Pending, Self::InProgress | Self::Failed) => true,
            (Self::InProgress, Self::Completed | Self::Failed) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid message state transition: {from:?} -> {to:?}")]
pub struct MessageStateTransitionError {
    pub from: MessageStatus,
    pub to: MessageStatus,
}

impl SessionMessage {
    pub fn push_part(&mut self, mut part: SessionMessagePart) {
        part.message_id = self.id;
        part.part_index = self.parts.len() as i32;
        self.parts.push(part);
    }

    pub fn transition_state(
        &mut self,
        next: MessageStatus,
    ) -> Result<(), MessageStateTransitionError> {
        let from = self.state;
        if !from.can_transition(next) {
            return Err(MessageStateTransitionError { from, to: next });
        }
        self.state = next;
        Ok(())
    }
}
