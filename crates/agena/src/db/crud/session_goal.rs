use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait,
    QueryFilter,
};

use crate::db::entities;
use crate::session::GoalStatus;

#[derive(Debug, Clone, Default)]
pub struct GoalUpdate {
    pub objective: Option<String>,
    pub status: Option<GoalStatus>,
    pub expected_goal_id: Option<i64>,
}

pub async fn get_by_session_id<C>(
    db: &C,
    session_id: i64,
) -> Result<Option<entities::session_goal::Model>, DbErr>
where
    C: ConnectionTrait,
{
    entities::session_goal::Entity::find()
        .filter(entities::session_goal::Column::SessionId.eq(session_id))
        .one(db)
        .await
}

pub async fn upsert_goal<C>(
    db: &C,
    session_id: i64,
    objective: String,
) -> Result<entities::session_goal::Model, DbErr>
where
    C: ConnectionTrait,
{
    let now_ms = Utc::now().timestamp_millis();

    if let Some(existing) = get_by_session_id(db, session_id).await? {
        let mut active: entities::session_goal::ActiveModel = existing.into();
        active.objective = Set(objective);
        active.status = Set(goal_status_label(GoalStatus::Active).to_string());
        active.updated_at_ms = Set(now_ms);
        active.completed_at_ms = Set(None);
        return active.update(db).await;
    }

    entities::session_goal::ActiveModel {
        session_id: Set(session_id),
        objective: Set(objective),
        status: Set(goal_status_label(GoalStatus::Active).to_string()),
        created_at_ms: Set(now_ms),
        updated_at_ms: Set(now_ms),
        completed_at_ms: Set(None),
        ..Default::default()
    }
    .insert(db)
    .await
}

pub async fn mark_completed<C>(
    db: &C,
    session_id: i64,
) -> Result<Option<entities::session_goal::Model>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(existing) = get_by_session_id(db, session_id).await? else {
        return Ok(None);
    };
    let now_ms = Utc::now().timestamp_millis();
    let mut active: entities::session_goal::ActiveModel = existing.into();
    active.status = Set(goal_status_label(GoalStatus::Completed).to_string());
    active.updated_at_ms = Set(now_ms);
    active.completed_at_ms = Set(Some(now_ms));
    active.update(db).await.map(Some)
}

pub async fn pause_active<C>(
    db: &C,
    session_id: i64,
) -> Result<Option<entities::session_goal::Model>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(existing) = get_by_session_id(db, session_id).await? else {
        return Ok(None);
    };
    if goal_status_from_label(existing.status.as_str()) != Some(GoalStatus::Active) {
        return Ok(None);
    }

    let now_ms = Utc::now().timestamp_millis();
    let mut active: entities::session_goal::ActiveModel = existing.into();
    active.status = Set(goal_status_label(GoalStatus::Paused).to_string());
    active.updated_at_ms = Set(now_ms);
    active.update(db).await.map(Some)
}

pub async fn resume_paused<C>(
    db: &C,
    session_id: i64,
) -> Result<Option<entities::session_goal::Model>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(existing) = get_by_session_id(db, session_id).await? else {
        return Ok(None);
    };
    if goal_status_from_label(existing.status.as_str()) != Some(GoalStatus::Paused) {
        return Ok(None);
    }

    let now_ms = Utc::now().timestamp_millis();
    let mut active: entities::session_goal::ActiveModel = existing.into();
    active.status = Set(goal_status_label(GoalStatus::Active).to_string());
    active.updated_at_ms = Set(now_ms);
    active.update(db).await.map(Some)
}

pub async fn update_goal<C>(
    db: &C,
    session_id: i64,
    update: GoalUpdate,
) -> Result<Option<entities::session_goal::Model>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(existing) = get_by_session_id(db, session_id).await? else {
        return Ok(None);
    };
    if update
        .expected_goal_id
        .is_some_and(|expected_goal_id| expected_goal_id != existing.id)
    {
        return Ok(None);
    }

    let now_ms = Utc::now().timestamp_millis();
    let mut active: entities::session_goal::ActiveModel = existing.into();
    if let Some(objective) = update.objective {
        active.objective = Set(objective);
    }
    if let Some(status) = update.status {
        active.status = Set(goal_status_label(status).to_string());
        active.completed_at_ms = Set(if status == GoalStatus::Completed {
            Some(now_ms)
        } else {
            None
        });
    }
    active.updated_at_ms = Set(now_ms);
    active.update(db).await.map(Some)
}

pub async fn clear_by_session_id<C>(db: &C, session_id: i64) -> Result<bool, DbErr>
where
    C: ConnectionTrait,
{
    let result = entities::session_goal::Entity::delete_many()
        .filter(entities::session_goal::Column::SessionId.eq(session_id))
        .exec(db)
        .await?;
    Ok(result.rows_affected > 0)
}

fn goal_status_label(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Active => "active",
        GoalStatus::Paused => "paused",
        GoalStatus::Completed => "completed",
    }
}

fn goal_status_from_label(value: &str) -> Option<GoalStatus> {
    match value {
        "active" => Some(GoalStatus::Active),
        "paused" => Some(GoalStatus::Paused),
        "completed" => Some(GoalStatus::Completed),
        _ => None,
    }
}
