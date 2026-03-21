use sea_orm::entity::prelude::*;

use super::message;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "parts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    pub message_id: i64,
    pub part_type_json: String,
    pub created_at_ms: i64,
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
}

impl Related<message::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Message.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
