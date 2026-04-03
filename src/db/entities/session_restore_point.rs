use sea_orm::entity::prelude::*;

use super::session;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_session_restore_points")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    pub session_id: i64,
    pub upto_seq: i64,
    pub call_id: Option<i64>,
    pub message_id: Option<i64>,
    pub operation_id: Option<String>,
    #[sea_orm(column_name = "snapshot_json", column_type = "JsonBinary")]
    pub snapshot: crate::checkpoint::SessionRestorePointSnapshot,
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
