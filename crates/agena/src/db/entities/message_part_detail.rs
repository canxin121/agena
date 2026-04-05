use sea_orm::entity::prelude::*;

use super::message_part;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_message_part_details")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub part_id: i64,
    #[sea_orm(column_name = "detail_json", column_type = "JsonBinary")]
    pub detail: crate::message::PartContent,
    pub updated_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "message_part::Entity",
        from = "Column::PartId",
        to = "message_part::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    MessagePart,
}

impl Related<message_part::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MessagePart.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
