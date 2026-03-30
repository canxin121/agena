use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect,
};

use crate::db::{crud::workspace, entities};

pub async fn create_session<C>(
    db: &C,
    workspace_id: i64,
    parent_id: Option<i64>,
    title: impl Into<String>,
) -> Result<entities::session::Model, DbErr>
where
    C: ConnectionTrait,
{
    let now_ms = Utc::now().timestamp_millis();
    entities::session::ActiveModel {
        parent_id: Set(parent_id),
        workspace_id: Set(workspace_id),
        title: Set(title.into()),
        created_at_ms: Set(now_ms),
        updated_at_ms: Set(now_ms),
        ..Default::default()
    }
    .insert(db)
    .await
}

pub async fn get_session_by_id<C>(
    db: &C,
    session_id: i64,
) -> Result<Option<entities::session::Model>, DbErr>
where
    C: ConnectionTrait,
{
    entities::session::Entity::find_by_id(session_id)
        .one(db)
        .await
}

pub async fn touch_session_updated_at<C>(
    db: &C,
    session_id: i64,
) -> Result<Option<entities::session::Model>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(existing) = get_session_by_id(db, session_id).await? else {
        return Ok(None);
    };
    let mut active: entities::session::ActiveModel = existing.into();
    active.updated_at_ms = Set(Utc::now().timestamp_millis());
    active.update(db).await.map(Some)
}

pub async fn delete_session_by_id<C>(db: &C, session_id: i64) -> Result<u64, DbErr>
where
    C: ConnectionTrait,
{
    let deleted = entities::session::Entity::delete_by_id(session_id)
        .exec(db)
        .await?;
    Ok(deleted.rows_affected)
}

pub async fn list_child_session_ids<C>(db: &C, parent_session_id: i64) -> Result<Vec<i64>, DbErr>
where
    C: ConnectionTrait,
{
    entities::session::Entity::find()
        .select_only()
        .column(entities::session::Column::Id)
        .filter(entities::session::Column::ParentId.eq(parent_session_id))
        .order_by_asc(entities::session::Column::Id)
        .into_tuple::<i64>()
        .all(db)
        .await
}

pub async fn list_session_ids_by_workspace_id<C>(
    db: &C,
    workspace_id: i64,
) -> Result<Vec<i64>, DbErr>
where
    C: ConnectionTrait,
{
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

pub async fn list_sessions_by_workspace_id<C>(
    db: &C,
    workspace_id: i64,
) -> Result<Vec<entities::session::Model>, DbErr>
where
    C: ConnectionTrait,
{
    entities::session::Entity::find()
        .filter(entities::session::Column::WorkspaceId.eq(workspace_id))
        .order_by_desc(entities::session::Column::UpdatedAtMs)
        .order_by_desc(entities::session::Column::Id)
        .all(db)
        .await
}

pub async fn list_session_ids_by_workspace<C>(
    db: &C,
    workspace_path: &str,
) -> Result<Vec<i64>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(workspace_id) = workspace::get_workspace_id_by_path(db, workspace_path).await? else {
        return Ok(Vec::new());
    };

    list_session_ids_by_workspace_id(db, workspace_id).await
}

pub async fn list_sessions_by_workspace<C>(
    db: &C,
    workspace_path: &str,
) -> Result<Vec<entities::session::Model>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(workspace_id) = workspace::get_workspace_id_by_path(db, workspace_path).await? else {
        return Ok(Vec::new());
    };

    list_sessions_by_workspace_id(db, workspace_id).await
}
