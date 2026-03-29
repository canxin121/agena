use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, QueryOrder,
};

use crate::db::entities;
use crate::permission::PermissionMode;

pub async fn upsert_rule(
    db: &DatabaseConnection,
    action_key: &str,
    mode: PermissionMode,
) -> Result<entities::permission_rule::Model, DbErr> {
    let now_ms = Utc::now().timestamp_millis();
    if let Some(existing) = entities::permission_rule::Entity::find()
        .filter(entities::permission_rule::Column::ActionKey.eq(action_key))
        .one(db)
        .await?
    {
        let mut active: entities::permission_rule::ActiveModel = existing.into();
        active.mode = Set(mode_to_string(mode));
        active.updated_at_ms = Set(now_ms);
        return active.update(db).await;
    }

    entities::permission_rule::ActiveModel {
        action_key: Set(action_key.to_string()),
        mode: Set(mode_to_string(mode)),
        created_at_ms: Set(now_ms),
        updated_at_ms: Set(now_ms),
        ..Default::default()
    }
    .insert(db)
    .await
}

pub async fn resolve_rule(
    db: &DatabaseConnection,
    action_key: &str,
) -> Result<Option<PermissionMode>, DbErr> {
    let item = entities::permission_rule::Entity::find()
        .filter(entities::permission_rule::Column::ActionKey.eq(action_key))
        .order_by_desc(entities::permission_rule::Column::UpdatedAtMs)
        .one(db)
        .await?;
    Ok(item
        .and_then(|value| mode_from_string(value.mode.as_str()).ok()))
}

fn mode_from_string(value: &str) -> Result<PermissionMode, ()> {
    match value {
        "allow" => Ok(PermissionMode::Allow),
        "ask" => Ok(PermissionMode::Ask),
        "deny" => Ok(PermissionMode::Deny),
        _ => Err(()),
    }
}

fn mode_to_string(mode: PermissionMode) -> String {
    match mode {
        PermissionMode::Allow => "allow".to_string(),
        PermissionMode::Ask => "ask".to_string(),
        PermissionMode::Deny => "deny".to_string(),
    }
}
