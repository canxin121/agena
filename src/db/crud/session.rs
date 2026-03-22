use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};

use crate::db::{crud::workspace, entities};

pub async fn list_child_session_ids(
    db: &DatabaseConnection,
    parent_session_id: i64,
) -> Result<Vec<i64>, DbErr> {
    entities::session::Entity::find()
        .select_only()
        .column(entities::session::Column::Id)
        .filter(entities::session::Column::ParentId.eq(parent_session_id))
        .order_by_asc(entities::session::Column::Id)
        .into_tuple::<i64>()
        .all(db)
        .await
}

pub async fn list_session_ids_by_workspace_id(
    db: &DatabaseConnection,
    workspace_id: i64,
) -> Result<Vec<i64>, DbErr> {
    entities::session::Entity::find()
        .select_only()
        .column(entities::session::Column::Id)
        .filter(entities::session::Column::WorkspaceId.eq(workspace_id))
        .order_by_desc(entities::session::Column::UpdatedAtMs)
        .order_by_desc(entities::session::Column::Id)
        .into_tuple::<i64>()
        .all(db)
        .await
}

pub async fn list_sessions_by_workspace_id(
    db: &DatabaseConnection,
    workspace_id: i64,
) -> Result<Vec<entities::session::Model>, DbErr> {
    entities::session::Entity::find()
        .filter(entities::session::Column::WorkspaceId.eq(workspace_id))
        .order_by_desc(entities::session::Column::UpdatedAtMs)
        .order_by_desc(entities::session::Column::Id)
        .all(db)
        .await
}

pub async fn list_session_ids_by_workspace(
    db: &DatabaseConnection,
    workspace_path: &str,
) -> Result<Vec<i64>, DbErr> {
    let Some(workspace_id) = workspace::get_workspace_id_by_path(db, workspace_path).await? else {
        return Ok(Vec::new());
    };

    list_session_ids_by_workspace_id(db, workspace_id).await
}

pub async fn list_sessions_by_workspace(
    db: &DatabaseConnection,
    workspace_path: &str,
) -> Result<Vec<entities::session::Model>, DbErr> {
    let Some(workspace_id) = workspace::get_workspace_id_by_path(db, workspace_path).await? else {
        return Ok(Vec::new());
    };

    list_sessions_by_workspace_id(db, workspace_id).await
}
