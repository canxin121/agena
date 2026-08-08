use sea_orm::entity::prelude::*;

use crate::message::PartContent;
use agena_storage_sqlite::{StoredExecutionStatus, StoredPartKind};

use super::model_message;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_model_message_parts")]
/// SeaORM entity for a stored model message part.
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub part_id: i64,
    pub message_id: i64,
    pub part_index: i32,
    pub status: StoredExecutionStatus,
    pub kind: StoredPartKind,
    pub name: Option<String>,
    pub summary: Option<String>,
    #[sea_orm(default_value = false)]
    pub has_detail: bool,
    #[sea_orm(default_value = false)]
    pub awaits_user_reply: bool,
    pub activity_id: Option<String>,
    pub segment_id: Option<String>,
    pub operation_id: Option<String>,
    pub created_at_ms: i64,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub content: Option<PartContent>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
/// SeaORM relations of the model message part entity.
pub enum Relation {
    #[sea_orm(
        belongs_to = "model_message::Entity",
        from = "Column::MessageId",
        to = "model_message::Column::MessageId",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Message,
}

impl Related<model_message::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Message.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
