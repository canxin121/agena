use sea_orm::entity::prelude::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};

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
pub enum ExecutionStatus {
    #[default]
    #[sea_orm(num_value = 1)]
    Pending,
    #[sea_orm(num_value = 2)]
    InProgress,
    #[sea_orm(num_value = 3)]
    Completed,
    #[sea_orm(num_value = 4)]
    Failed,
    #[sea_orm(num_value = 5)]
    Cancelled,
}

impl ExecutionStatus {
    /// True if a status may legally transition to `next`. Identity transitions
    /// are allowed (idempotent updates).
    pub fn can_transition(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        matches!(
            (self, next),
            (Self::Pending, Self::InProgress | Self::Failed)
                | (Self::Pending, Self::Cancelled)
                | (
                    Self::InProgress,
                    Self::Completed | Self::Failed | Self::Cancelled
                )
        )
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid execution status transition: {from:?} -> {to:?}")]
pub struct ExecutionStatusTransitionError {
    pub from: ExecutionStatus,
    pub to: ExecutionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TimeRange {
    pub start_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<i64>,
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
    DeriveActiveEnum,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[sea_orm(rs_type = "i8", db_type = "TinyInteger")]
pub enum PartKind {
    #[sea_orm(num_value = 1)]
    Text,
    #[sea_orm(num_value = 2)]
    Reasoning,
    #[sea_orm(num_value = 3)]
    Operation,
    #[sea_orm(num_value = 4)]
    Attachment,
    #[sea_orm(num_value = 5)]
    Request,
    #[sea_orm(num_value = 6)]
    Error,
}
