use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait,
    QueryFilter, QueryOrder,
};

use crate::db::entities;
use crate::permission::{PermissionMode, PermissionScope, PersistedPermissionRule};

pub async fn upsert_rule<C>(
    db: &C,
    rule: &PersistedPermissionRule,
) -> Result<(entities::permission_rule::Model, bool), DbErr>
where
    C: ConnectionTrait,
{
    let now_ms = Utc::now().timestamp_millis();
    if let Some(existing) = entities::permission_rule::Entity::find()
        .filter(entities::permission_rule::Column::ActionKey.eq(rule.action_key.as_str()))
        .filter(entities::permission_rule::Column::Scope.eq(scope_to_string(rule.scope)))
        .filter(entities::permission_rule::Column::SessionId.eq(rule.session_id))
        .filter(entities::permission_rule::Column::WorkspaceId.eq(rule.workspace_id))
        .one(db)
        .await?
    {
        let mut active: entities::permission_rule::ActiveModel = existing.into();
        active.mode = Set(mode_to_string(rule.mode));
        active.source = Set(rule.source.clone());
        active.reason = Set(rule.reason.clone());
        active.operator = Set(rule.operator.clone());
        active.revoked_at_ms = Set(rule.revoked_at_ms);
        active.revoked_reason = Set(rule.revoked_reason.clone());
        active.revoked_by = Set(rule.revoked_by.clone());
        active.updated_at_ms = Set(now_ms);
        return active.update(db).await.map(|model| (model, false));
    }

    entities::permission_rule::ActiveModel {
        action_key: Set(rule.action_key.clone()),
        mode: Set(mode_to_string(rule.mode)),
        scope: Set(scope_to_string(rule.scope)),
        session_id: Set(rule.session_id),
        workspace_id: Set(rule.workspace_id),
        source: Set(rule.source.clone()),
        reason: Set(rule.reason.clone()),
        operator: Set(rule.operator.clone()),
        revoked_at_ms: Set(rule.revoked_at_ms),
        revoked_reason: Set(rule.revoked_reason.clone()),
        revoked_by: Set(rule.revoked_by.clone()),
        created_at_ms: Set(now_ms),
        updated_at_ms: Set(now_ms),
        ..Default::default()
    }
    .insert(db)
    .await
    .map(|model| (model, true))
}

pub async fn resolve_rule<C>(
    db: &C,
    action_key: &str,
    session_id: Option<i64>,
    workspace_id: Option<i64>,
) -> Result<Option<entities::permission_rule::Model>, DbErr>
where
    C: ConnectionTrait,
{
    if let Some(session_id) = session_id
        && let Some(item) = entities::permission_rule::Entity::find()
            .filter(entities::permission_rule::Column::ActionKey.eq(action_key))
            .filter(
                entities::permission_rule::Column::Scope
                    .eq(scope_to_string(PermissionScope::Session)),
            )
            .filter(entities::permission_rule::Column::SessionId.eq(session_id))
            .filter(entities::permission_rule::Column::RevokedAtMs.is_null())
            .order_by_desc(entities::permission_rule::Column::UpdatedAtMs)
            .order_by_desc(entities::permission_rule::Column::Id)
            .one(db)
            .await?
    {
        return Ok(Some(item));
    }

    if let Some(workspace_id) = workspace_id
        && let Some(item) = entities::permission_rule::Entity::find()
            .filter(entities::permission_rule::Column::ActionKey.eq(action_key))
            .filter(
                entities::permission_rule::Column::Scope
                    .eq(scope_to_string(PermissionScope::Workspace)),
            )
            .filter(entities::permission_rule::Column::WorkspaceId.eq(workspace_id))
            .filter(entities::permission_rule::Column::RevokedAtMs.is_null())
            .order_by_desc(entities::permission_rule::Column::UpdatedAtMs)
            .order_by_desc(entities::permission_rule::Column::Id)
            .one(db)
            .await?
    {
        return Ok(Some(item));
    }

    if let Some(item) = entities::permission_rule::Entity::find()
        .filter(entities::permission_rule::Column::ActionKey.eq(action_key))
        .filter(
            entities::permission_rule::Column::Scope.eq(scope_to_string(PermissionScope::Global)),
        )
        .filter(entities::permission_rule::Column::SessionId.is_null())
        .filter(entities::permission_rule::Column::WorkspaceId.is_null())
        .filter(entities::permission_rule::Column::RevokedAtMs.is_null())
        .order_by_desc(entities::permission_rule::Column::UpdatedAtMs)
        .order_by_desc(entities::permission_rule::Column::Id)
        .one(db)
        .await?
    {
        return Ok(Some(item));
    }

    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsePermissionValueError;

pub fn mode_from_string(value: &str) -> Result<PermissionMode, ParsePermissionValueError> {
    match value {
        "allow" => Ok(PermissionMode::Allow),
        "ask" => Ok(PermissionMode::Ask),
        "deny" => Ok(PermissionMode::Deny),
        _ => Err(ParsePermissionValueError),
    }
}

pub fn mode_to_string(mode: PermissionMode) -> String {
    match mode {
        PermissionMode::Allow => "allow".to_string(),
        PermissionMode::Ask => "ask".to_string(),
        PermissionMode::Deny => "deny".to_string(),
    }
}

pub fn scope_to_string(scope: PermissionScope) -> String {
    match scope {
        PermissionScope::Session => "session".to_string(),
        PermissionScope::Workspace => "workspace".to_string(),
        PermissionScope::Global => "global".to_string(),
    }
}

pub fn scope_from_string(value: &str) -> Result<PermissionScope, ParsePermissionValueError> {
    match value {
        "session" => Ok(PermissionScope::Session),
        "workspace" => Ok(PermissionScope::Workspace),
        "global" => Ok(PermissionScope::Global),
        _ => Err(ParsePermissionValueError),
    }
}

pub async fn revoke_rule<C>(
    db: &C,
    rule_id: i64,
    revoked_reason: Option<String>,
    revoked_by: Option<String>,
) -> Result<Option<entities::permission_rule::Model>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(existing) = entities::permission_rule::Entity::find_by_id(rule_id)
        .one(db)
        .await?
    else {
        return Ok(None);
    };

    let mut active: entities::permission_rule::ActiveModel = existing.into();
    active.revoked_at_ms = Set(Some(Utc::now().timestamp_millis()));
    active.revoked_reason = Set(revoked_reason);
    active.revoked_by = Set(revoked_by);
    active.updated_at_ms = Set(Utc::now().timestamp_millis());
    active.update(db).await.map(Some)
}
