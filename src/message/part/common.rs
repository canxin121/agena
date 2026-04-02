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
    ToolExecution,
    #[sea_orm(num_value = 4)]
    CommandExecution,
    #[sea_orm(num_value = 5)]
    FileChange,
    #[sea_orm(num_value = 6)]
    WebSearch,
    #[sea_orm(num_value = 7)]
    TodoList,
    #[sea_orm(num_value = 8)]
    Error,
    #[sea_orm(num_value = 9)]
    Attachment,
    #[sea_orm(num_value = 10)]
    PermissionRequest,
}
