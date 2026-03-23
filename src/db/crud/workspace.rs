use chrono::Utc;
use path_clean::PathClean;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, QuerySelect,
};
use std::path::Path;

use crate::db::entities;

pub async fn get_workspace_id_by_path(
    db: &DatabaseConnection,
    workspace_path: &str,
) -> Result<Option<i64>, DbErr> {
    let normalized = normalized_workspace_path(workspace_path)?;

    entities::workspace::Entity::find()
        .select_only()
        .column(entities::workspace::Column::Id)
        .filter(entities::workspace::Column::Path.eq(normalized))
        .into_tuple::<i64>()
        .one(db)
        .await
}

pub async fn ensure_workspace_id(
    db: &DatabaseConnection,
    workspace_path: &str,
) -> Result<i64, DbErr> {
    if let Some(existing_id) = get_workspace_id_by_path(db, workspace_path).await? {
        return Ok(existing_id);
    }

    let now_ms = Utc::now().timestamp_millis();
    let model = entities::workspace::ActiveModel {
        path: Set(normalized_workspace_path(workspace_path)?),
        created_at_ms: Set(now_ms),
        updated_at_ms: Set(now_ms),
        ..Default::default()
    }
    .insert(db)
    .await?;

    Ok(model.id)
}

fn normalized_workspace_path(workspace_path: &str) -> Result<String, DbErr> {
    let raw = workspace_path.trim();
    if raw.is_empty() {
        return Err(DbErr::Custom("workspace path cannot be empty".to_string()));
    }

    let cleaned = Path::new(raw).clean();
    let mut normalized = cleaned.to_string_lossy().replace('\\', "/");
    while normalized.ends_with('/') && normalized.len() > 1 && !is_windows_drive_root(&normalized) {
        normalized.pop();
    }
    if cfg!(windows) {
        normalized.make_ascii_lowercase();
    }
    Ok(normalized)
}

fn is_windows_drive_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}
