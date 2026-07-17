use crate::db::entities;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait,
    QueryFilter, QuerySelect,
};

pub async fn get_workspace_id_by_path<C>(db: &C, workspace_path: &str) -> Result<Option<i64>, DbErr>
where
    C: ConnectionTrait,
{
    let normalized =
        crate::project_paths::normalize_workspace_path(workspace_path).map_err(DbErr::Custom)?;

    entities::workspace::Entity::find()
        .select_only()
        .column(entities::workspace::Column::Id)
        .filter(entities::workspace::Column::Path.eq(normalized))
        .into_tuple::<i64>()
        .one(db)
        .await
}

pub async fn ensure_workspace_id<C>(db: &C, workspace_path: &str) -> Result<i64, DbErr>
where
    C: ConnectionTrait,
{
    if let Some(existing_id) = get_workspace_id_by_path(db, workspace_path).await? {
        return Ok(existing_id);
    }

    let now_ms = Utc::now().timestamp_millis();
    let model = entities::workspace::ActiveModel {
        path: Set(
            crate::project_paths::normalize_workspace_path(workspace_path)
                .map_err(DbErr::Custom)?,
        ),
        created_at_ms: Set(now_ms),
        updated_at_ms: Set(now_ms),
        ..Default::default()
    }
    .insert(db)
    .await?;

    Ok(model.id)
}
