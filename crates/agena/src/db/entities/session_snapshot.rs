use sea_orm::entity::prelude::*;

/// Materialised snapshot of a session's projected `Vec<Message>` view.
///
/// One row per session. The snapshot stores the result of folding all events
/// up to and including `last_seq`; subsequent loads only need to fold events
/// strictly after `last_seq` against the saved view, turning a session load
/// from O(events) into O(events_since_snapshot).
///
/// The snapshot is a pure derived projection — losing it never costs data,
/// the next load just re-folds from the event log. Writes are best-effort
/// fire-and-forget after a successful load; failures are logged and
/// otherwise ignored.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_session_snapshots")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub session_id: i64,
    pub last_seq: i64,
    #[sea_orm(column_name = "view_json", column_type = "JsonBinary")]
    pub view: serde_json::Value,
    pub updated_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
