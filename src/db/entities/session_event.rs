use sea_orm::entity::prelude::*;

use super::session;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_session_events")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    pub session_id: i64,
    pub seq: i64,
    pub event_type: crate::session::SessionEventType,
    #[sea_orm(column_name = "payload_json", column_type = "JsonBinary")]
    pub payload: crate::event::SessionEvent,
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
}

impl Related<session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
