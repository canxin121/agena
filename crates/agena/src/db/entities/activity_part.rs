use sea_orm::entity::prelude::*;

use crate::message::{ExecutionStatus, PartContent, PartKind};

use super::activity_message;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_activity_parts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub part_id: i64,
    pub message_id: i64,
    pub part_index: i32,
    pub status: ExecutionStatus,
    pub kind: PartKind,
    pub name: Option<String>,
    pub summary: Option<String>,
    #[sea_orm(default_value = false)]
    pub has_detail: bool,
    pub operation_id: Option<String>,
    pub created_at_ms: i64,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub content: Option<PartContent>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "activity_message::Entity",
        from = "Column::MessageId",
        to = "activity_message::Column::MessageId",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Message,
}

impl Related<activity_message::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Message.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
