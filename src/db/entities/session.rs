use sea_orm::entity::prelude::*;

use super::{checkpoint, event_log, item, turn};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    pub parent_id: Option<i64>,
    pub project_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub title: String,
    pub status: String,
    pub last_event_seq: i64,
    pub version: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "turn::Entity")]
    Turn,
    #[sea_orm(has_many = "item::Entity")]
    Item,
    #[sea_orm(has_many = "event_log::Entity")]
    EventLog,
    #[sea_orm(has_many = "checkpoint::Entity")]
    Checkpoint,
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

impl Related<event_log::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::EventLog.def()
    }
}

impl Related<checkpoint::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Checkpoint.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
