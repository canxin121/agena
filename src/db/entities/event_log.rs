use sea_orm::entity::prelude::*;

use super::{item, session, turn};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_event_log")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub event_id: i64,
    pub session_id: i64,
    pub turn_id: Option<i64>,
    pub item_id: Option<i64>,
    pub seq: i64,
    pub event_type: String,
    pub payload_json: String,
    pub causation_id: Option<i64>,
    pub correlation_id: Option<i64>,
    pub created_at_ms: i64,
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
    #[sea_orm(
        belongs_to = "turn::Entity",
        from = "Column::TurnId",
        to = "turn::Column::Id",
        on_update = "Cascade",
        on_delete = "SetNull"
    )]
    Turn,
    #[sea_orm(
        belongs_to = "item::Entity",
        from = "Column::ItemId",
        to = "item::Column::Id",
        on_update = "Cascade",
        on_delete = "SetNull"
    )]
    Item,
}

impl Related<session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl Related<turn::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Turn.def()
    }
}

impl Related<item::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Item.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
