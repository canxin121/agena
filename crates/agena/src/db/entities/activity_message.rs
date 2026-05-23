use sea_orm::entity::prelude::*;

use crate::{
    message::{MessageMetadata, MessageProviderState},
    role::Role,
};

use super::session;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_activity_messages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub message_id: i64,
    pub session_id: i64,
    pub role: Role,
    pub state: crate::message::ExecutionStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[sea_orm(column_type = "JsonBinary")]
    pub metadata: MessageMetadata,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub provider_state: Option<MessageProviderState>,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub usage: Option<crate::message::MessageUsage>,
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
