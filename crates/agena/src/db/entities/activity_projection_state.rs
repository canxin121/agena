use sea_orm::entity::prelude::*;

use super::session;

/// Watermark for the session message projection stored in
/// `agena_activity_messages` / `agena_activity_parts`.
///
/// The row records the highest `seq_global` from the durable event log that
/// has been applied to the message read model. Readers can compare this
/// watermark with the event store's session watermark to detect stale or
/// missing projections and trigger an explicit rebuild.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_activity_projection_states")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub session_id: i64,
    pub last_seq_global: i64,
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
}

impl Related<session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
