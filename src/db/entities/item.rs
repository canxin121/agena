use sea_orm::entity::prelude::*;

use super::{event_log, session, turn};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_items")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    pub session_id: i64,
    pub turn_id: i64,
    pub item_index: i32,
    pub role: String,
    pub kind: String,
    pub status: String,
    pub payload_json: String,
    pub version: i64,
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
    #[sea_orm(
        belongs_to = "turn::Entity",
        from = "Column::TurnId",
        to = "turn::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Turn,
    #[sea_orm(has_many = "event_log::Entity")]
    EventLog,
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

impl Related<event_log::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::EventLog.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
