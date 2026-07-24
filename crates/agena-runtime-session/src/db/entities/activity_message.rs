use sea_orm::entity::prelude::*;

use crate::message::{MessageMetadata, MessageProviderState};
use agena_storage_sqlite::{StoredExecutionStatus, StoredRole};

use super::session;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_activity_messages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub message_id: i64,
    pub session_id: i64,
    /// Stable user-visible conversation turn. Multiple provider messages may
    /// belong to one turn while remaining separate protocol records.
    pub turn_id: Option<i64>,
    pub execution_id: Option<String>,
    pub run_id: Option<String>,
    pub role: StoredRole,
    pub state: StoredExecutionStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[sea_orm(column_type = "JsonBinary")]
    pub metadata: MessageMetadata,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub provider_state: Option<MessageProviderState>,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub usage: Option<agena_storage_sqlite::PersistedCompletionUsage>,
    pub part_count: i64,
    #[sea_orm(default_value = false)]
    pub is_hidden: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "session::Entity",
        from = "Column::SessionId",
        to = "session::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Session,
}

impl Related<session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
