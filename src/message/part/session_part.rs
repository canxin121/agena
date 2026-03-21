use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{ExecutionStatus, PartContent, PartKind};

#[derive(Debug, Error)]
#[error("invalid part state transition: {from:?} -> {to:?}")]
pub struct PartStateTransitionError {
    pub from: ExecutionStatus,
    pub to: ExecutionStatus,
}

fn can_transition(from: ExecutionStatus, to: ExecutionStatus) -> bool {
    if from == to {
        return true;
    }

    match (from, to) {
        (ExecutionStatus::Pending, ExecutionStatus::InProgress | ExecutionStatus::Failed) => true,
        (ExecutionStatus::InProgress, ExecutionStatus::Completed | ExecutionStatus::Failed) => true,
        _ => false,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionMessagePart {
    pub id: i64,
    pub message_id: i64,
    #[serde(default)]
    pub status: ExecutionStatus,
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

    pub fn reasoning_summary(&self) -> Option<&[String]> {
        self.content.reasoning_summary_value()
    }

    pub fn transition_status(
        &mut self,
        to: ExecutionStatus,
    ) -> Result<(), PartStateTransitionError> {
        let from = self.status;
        if !can_transition(from, to) {
            return Err(PartStateTransitionError { from, to });
        }
        self.status = to;
        Ok(())
    }
}
