use sea_orm::entity::prelude::*;

use super::{message_part, session};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_messages")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub session_id: i64,
    pub role: crate::role::Role,
    pub status: crate::message::MessageStatus,
    pub source: crate::message::MessageSource,
    pub parent_message_id: Option<i64>,
    pub generated_by_call_id: Option<i64>,
    pub model_provider_id: String,
    pub model_id: String,
    #[sea_orm(column_name = "usage_json", column_type = "JsonBinary")]
    pub usage: Option<crate::message::MessageUsage>,
    pub finish: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
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
    #[sea_orm(has_many = "message_part::Entity")]
    MessagePart,
}

impl Related<session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl Related<message_part::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MessagePart.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
