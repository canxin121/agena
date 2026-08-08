use sea_orm::entity::prelude::*;

use super::entities::{session, workspace};

/// Single unified event-log table (`agena_events`).
///
/// Routing and observability metadata live in their own typed columns;
/// `payload_json` carries only the kind payload (i.e. `EventKind` serialised
/// as `{"kind": "...", "payload": {...}}`). The full `DomainEvent` envelope
/// is reconstructed from the columns + payload at read time.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_events")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    #[sea_orm(unique)]
    pub event_uuid: String,
    #[sea_orm(unique)]
    pub seq_global: i64,
    pub seq_session: Option<i64>,
    pub session_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub kind_tag: String,
    pub envelope_schema: i32,
    #[sea_orm(column_name = "payload_json", column_type = "JsonBinary")]
    pub payload: serde_json::Value,
    pub causation_uuid: Option<String>,
    pub correlation_uuid: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
/// SeaORM relations of the event entity.
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
        belongs_to = "workspace::Entity",
        from = "Column::WorkspaceId",
        to = "workspace::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Workspace,
}

impl Related<session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl Related<workspace::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Workspace.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
