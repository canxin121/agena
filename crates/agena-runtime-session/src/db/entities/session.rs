use sea_orm::entity::prelude::*;

use super::workspace;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agena_sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    pub parent_id: Option<i64>,
    /// Distance to the tree root in `parent_id` chain. Root sessions have `0`.
    /// Filled by the store on creation/fork; never recomputed lazily.
    pub depth: i64,
    /// Id of the topmost session in this `parent_id` chain (or `id` itself if
    /// the session has no parent). Lets `WHERE root_id = ?` pull the entire
    /// session tree in one query.
    pub root_id: i64,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    /// Creation is a real lifecycle: copied history is not visible as a
    /// usable session until its event stream and read model are complete.
    pub lifecycle_state: String,
    pub creation_error: Option<String>,
    #[sea_orm(column_name = "runtime_state_json", column_type = "JsonBinary")]
    pub runtime_state: Option<crate::session::SessionRuntimeState>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "workspace::Entity",
        from = "Column::WorkspaceId",
        to = "workspace::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Workspace,
    /// Self-referencing parent → child link. `ON DELETE CASCADE` ensures
    /// removing a session also removes the entire descendant subtree, so
    /// child rows never end up dangling against a missing `parent_id`.
    #[sea_orm(
        belongs_to = "Entity",
        from = "Column::ParentId",
        to = "Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    SelfParent,
}

impl Related<workspace::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Workspace.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
