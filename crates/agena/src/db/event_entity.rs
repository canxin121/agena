use sea_orm::entity::prelude::*;

/// Single unified event-log table (`agena_events`). Replaces the legacy
/// `agena_session_events` and `agena_session_history_events` tables.
///
/// `payload_json` stores the full `DomainEvent<K>` envelope. We keep `meta`
/// columns (`seq_global`, `session_id`, `workspace_id`, `kind_tag`) duplicated
/// from the JSON for indexed filtering.
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
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
