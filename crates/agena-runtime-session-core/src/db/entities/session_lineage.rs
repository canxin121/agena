use sea_orm::entity::prelude::*;

use super::session;

/// Immutable provenance plus the independently-owned delegated-task
/// lifecycle for every non-root session.
///
/// `agena_sessions.parent_id/root_id/depth` are the query-optimized hierarchy
/// projection. This row gives that edge its domain meaning, so callers never
/// infer fork/rewind/subagent semantics from titles or booleans.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_session_lineage")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub session_id: i64,
    pub relation_kind: String,
    pub source_cutoff_seq_global: Option<i64>,
    pub source_message_id: Option<i64>,
    pub task_id: Option<String>,
    pub subtask_status: Option<String>,
    pub subtask_started_at_ms: Option<i64>,
    pub subtask_finished_at_ms: Option<i64>,
    pub subtask_error: Option<String>,
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
