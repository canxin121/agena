use sea_orm::entity::prelude::*;

use super::{message, message_part_detail};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_message_parts")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub message_id: i64,
    pub part_index: i32,
    pub kind: crate::message::PartKind,
    pub status: crate::message::ExecutionStatus,
    pub name: Option<String>,
    pub summary_text: Option<String>,
    pub has_detail: bool,
    pub call_id: Option<i64>,
    pub operation_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "message::Entity",
        from = "Column::MessageId",
        to = "message::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Message,
    #[sea_orm(has_one = "message_part_detail::Entity")]
    MessagePartDetail,
}

impl Related<message::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Message.def()
    }
}

impl Related<message_part_detail::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MessagePartDetail.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
