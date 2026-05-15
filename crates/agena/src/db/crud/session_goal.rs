use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait,
    QueryFilter,
};

use crate::db::entities;
use crate::session::GoalStatus;

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
    token_budget: Option<u64>,
) -> Result<entities::session_goal::Model, DbErr>
where
    C: ConnectionTrait,
{
    let now_ms = Utc::now().timestamp_millis();
    let token_budget_i64 = token_budget
        .map(i64::try_from)
        .transpose()
        .map_err(|_| DbErr::Custom("goal token budget exceeds i64".to_string()))?;

    if let Some(existing) = get_by_session_id(db, session_id).await? {
        let mut active: entities::session_goal::ActiveModel = existing.into();
        active.objective = Set(objective);
        active.status = Set(goal_status_after_budget_limit(0, token_budget).to_string());
        active.token_budget = Set(token_budget_i64);
        active.tokens_used = Set(0);
        active.time_used_seconds = Set(0);
        active.updated_at_ms = Set(now_ms);
        active.completed_at_ms = Set(None);
        return active.update(db).await;
    }

    entities::session_goal::ActiveModel {
        session_id: Set(session_id),
        objective: Set(objective),
        status: Set(goal_status_after_budget_limit(0, token_budget).to_string()),
        token_budget: Set(token_budget_i64),
        tokens_used: Set(0),
        time_used_seconds: Set(0),
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
    let token_budget = existing
        .token_budget
        .map(u64::try_from)
        .transpose()
        .map_err(|_| DbErr::Custom("goal token budget is negative".to_string()))?;
    let resumed_status = goal_status_after_budget_limit(existing.tokens_used, token_budget);
    let mut active: entities::session_goal::ActiveModel = existing.into();
    active.status = Set(resumed_status.to_string());
    active.updated_at_ms = Set(now_ms);
    active.update(db).await.map(Some)
}

pub async fn account_usage<C>(
    db: &C,
    session_id: i64,
    token_delta: u64,
    time_delta_seconds: u64,
) -> Result<Option<entities::session_goal::Model>, DbErr>
where
    C: ConnectionTrait,
{
    if token_delta == 0 && time_delta_seconds == 0 {
        return get_by_session_id(db, session_id).await;
    }

    let Some(existing) = get_by_session_id(db, session_id).await? else {
        return Ok(None);
    };
    if !matches!(
        goal_status_from_label(existing.status.as_str()),
        Some(GoalStatus::Active | GoalStatus::BudgetLimited)
    ) {
        return Ok(Some(existing));
    }

    let now_ms = Utc::now().timestamp_millis();
    let token_delta = i64::try_from(token_delta)
        .map_err(|_| DbErr::Custom("goal token delta exceeds i64".to_string()))?;
    let time_delta_seconds = i64::try_from(time_delta_seconds)
        .map_err(|_| DbErr::Custom("goal time delta exceeds i64".to_string()))?;
    let new_tokens_used = existing.tokens_used.saturating_add(token_delta);
    let new_time_used_seconds = existing
        .time_used_seconds
        .saturating_add(time_delta_seconds);
    let token_budget = existing
        .token_budget
        .map(u64::try_from)
        .transpose()
        .map_err(|_| DbErr::Custom("goal token budget is negative".to_string()))?;

    let mut active: entities::session_goal::ActiveModel = existing.into();
    active.tokens_used = Set(new_tokens_used);
    active.time_used_seconds = Set(new_time_used_seconds);
    active.status = Set(goal_status_after_budget_limit(new_tokens_used, token_budget).to_string());
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
        GoalStatus::BudgetLimited => "budget_limited",
        GoalStatus::Completed => "completed",
    }
}

fn goal_status_after_budget_limit(tokens_used: i64, token_budget: Option<u64>) -> &'static str {
    if token_budget
        .and_then(|budget| i64::try_from(budget).ok())
        .is_some_and(|budget| tokens_used >= budget)
    {
        goal_status_label(GoalStatus::BudgetLimited)
    } else {
        goal_status_label(GoalStatus::Active)
    }
}

fn goal_status_from_label(value: &str) -> Option<GoalStatus> {
    match value {
        "active" => Some(GoalStatus::Active),
        "paused" => Some(GoalStatus::Paused),
        "budget_limited" => Some(GoalStatus::BudgetLimited),
        "completed" => Some(GoalStatus::Completed),
        _ => None,
    }
}
