//! SQLite-specific active-enum encodings for stable domain values.

use agena_domain::{ExecutionStatus, PartKind, Role};
use sea_orm::entity::prelude::{DeriveActiveEnum, EnumIter};

/// Persisted SQLite representation of [`Role`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "i8", db_type = "TinyInteger")]
pub enum StoredRole {
    #[sea_orm(num_value = 1)]
    User,
    #[sea_orm(num_value = 2)]
    Assistant,
    #[sea_orm(num_value = 3)]
    System,
    #[sea_orm(num_value = 4)]
    Tool,
}
impl From<Role> for StoredRole {
    fn from(value: Role) -> Self {
        match value {
            Role::User => Self::User,
            Role::Assistant => Self::Assistant,
            Role::System => Self::System,
            Role::Tool => Self::Tool,
        }
    }
}
impl From<StoredRole> for Role {
    fn from(value: StoredRole) -> Self {
        match value {
            StoredRole::User => Self::User,
            StoredRole::Assistant => Self::Assistant,
            StoredRole::System => Self::System,
            StoredRole::Tool => Self::Tool,
        }
    }
}

/// Persisted SQLite representation of [`ExecutionStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "i8", db_type = "TinyInteger")]
pub enum StoredExecutionStatus {
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
impl From<ExecutionStatus> for StoredExecutionStatus {
    fn from(value: ExecutionStatus) -> Self {
        match value {
            ExecutionStatus::Pending => Self::Pending,
            ExecutionStatus::InProgress => Self::InProgress,
            ExecutionStatus::Completed => Self::Completed,
            ExecutionStatus::Failed => Self::Failed,
            ExecutionStatus::Cancelled => Self::Cancelled,
        }
    }
}
impl From<StoredExecutionStatus> for ExecutionStatus {
    fn from(value: StoredExecutionStatus) -> Self {
        match value {
            StoredExecutionStatus::Pending => Self::Pending,
            StoredExecutionStatus::InProgress => Self::InProgress,
            StoredExecutionStatus::Completed => Self::Completed,
            StoredExecutionStatus::Failed => Self::Failed,
            StoredExecutionStatus::Cancelled => Self::Cancelled,
        }
    }
}

/// Persisted SQLite representation of [`PartKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "i8", db_type = "TinyInteger")]
pub enum StoredPartKind {
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
    #[sea_orm(num_value = 7)]
    Activity,
    #[sea_orm(num_value = 8)]
    SkillReference,
}
impl From<PartKind> for StoredPartKind {
    fn from(value: PartKind) -> Self {
        match value {
            PartKind::Text => Self::Text,
            PartKind::Reasoning => Self::Reasoning,
            PartKind::Operation => Self::Operation,
            PartKind::Activity => Self::Activity,
            PartKind::Attachment => Self::Attachment,
            PartKind::SkillReference => Self::SkillReference,
            PartKind::Request => Self::Request,
            PartKind::Error => Self::Error,
        }
    }
}
impl From<StoredPartKind> for PartKind {
    fn from(value: StoredPartKind) -> Self {
        match value {
            StoredPartKind::Text => Self::Text,
            StoredPartKind::Reasoning => Self::Reasoning,
            StoredPartKind::Operation => Self::Operation,
            StoredPartKind::Activity => Self::Activity,
            StoredPartKind::Attachment => Self::Attachment,
            StoredPartKind::SkillReference => Self::SkillReference,
            StoredPartKind::Request => Self::Request,
            StoredPartKind::Error => Self::Error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{StoredExecutionStatus, StoredPartKind, StoredRole};
    use sea_orm::ActiveEnum;
    #[test]
    fn preserves_existing_codes_and_assigns_activity_a_new_code() {
        assert_eq!(StoredRole::Assistant.to_value(), 2);
        assert_eq!(StoredExecutionStatus::Cancelled.to_value(), 5);
        assert_eq!(StoredPartKind::Error.to_value(), 6);
        assert_eq!(StoredPartKind::Activity.to_value(), 7);
        assert_eq!(StoredPartKind::SkillReference.to_value(), 8);
    }
}
