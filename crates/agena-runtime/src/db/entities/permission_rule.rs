use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_permission_rules")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    pub action_key: String,
    pub mode: String,
    pub scope: String,
    pub session_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub source: String,
    pub reason: Option<String>,
    pub operator: Option<String>,
    pub revoked_at_ms: Option<i64>,
    pub revoked_reason: Option<String>,
    pub revoked_by: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
