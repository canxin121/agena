use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Active,
    Archived,
    Deleted,
}

impl SessionStatus {
    pub fn can_transition(self, next: Self) -> bool {
        if self == next {
            return true;
        }

        matches!(
            (self, next),
            (Self::Active, Self::Archived | Self::Deleted) | (Self::Archived, Self::Deleted)
        )
    }
}

#[derive(Debug, Error)]
#[error("invalid session status transition: {from:?} -> {to:?}")]
pub struct SessionStatusTransitionError {
    pub from: SessionStatus,
    pub to: SessionStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Running,
    Completed,
    Failed,
    Aborted,
}

impl TurnStatus {
    pub fn can_transition(self, next: Self) -> bool {
        if self == next {
            return true;
        }

        matches!(self, Self::Running) && !matches!(next, Self::Running)
    }
}

#[derive(Debug, Error)]
#[error("invalid turn status transition: {from:?} -> {to:?}")]
pub struct TurnStatusTransitionError {
    pub from: TurnStatus,
    pub to: TurnStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Started,
    Updated,
    Completed,
    Failed,
}

impl ItemStatus {
    pub fn can_transition(self, next: Self) -> bool {
        if self == next {
            return true;
        }

        matches!(
            (self, next),
            (
                Self::Started,
                Self::Updated | Self::Completed | Self::Failed
            ) | (
                Self::Updated,
                Self::Updated | Self::Completed | Self::Failed
            )
        )
    }
}

#[derive(Debug, Error)]
#[error("invalid item status transition: {from:?} -> {to:?}")]
pub struct ItemStatusTransitionError {
    pub from: ItemStatus,
    pub to: ItemStatus,
}
