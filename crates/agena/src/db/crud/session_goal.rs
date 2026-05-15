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
    pub token_budget: Option<Option<u64>>,
    pub expected_goal_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalAccountingMode {
    ActiveStatusOnly,
    ActiveOnly,
    ActiveOrCompleted,
    ActiveOrStopped,
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
        active.status = Set(
            goal_status_label(goal_status_after_budget_limit(GoalStatus::Active, 0, token_budget))
                .to_string(),
        );
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
        status: Set(
            goal_status_label(goal_status_after_budget_limit(GoalStatus::Active, 0, token_budget))
                .to_string(),
        ),
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
    let token_budget = existing_goal_token_budget(&existing)?;
    let resumed_status =
        goal_status_after_budget_limit(GoalStatus::Active, existing.tokens_used, token_budget);
    let mut active: entities::session_goal::ActiveModel = existing.into();
    active.status = Set(goal_status_label(resumed_status).to_string());
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

    let existing_status = goal_status_from_label(existing.status.as_str()).ok_or_else(|| {
        DbErr::Custom(format!(
            "invalid goal status for session {} goal {}: {}",
            session_id, existing.id, existing.status
        ))
    })?;
    let token_budget = match update.token_budget {
        Some(token_budget) => token_budget,
        None => existing_goal_token_budget(&existing)?,
    };
    let token_budget_i64 = token_budget
        .map(i64::try_from)
        .transpose()
        .map_err(|_| DbErr::Custom("goal token budget exceeds i64".to_string()))?;
    let status = goal_status_after_update(
        existing_status,
        update.status,
        existing.tokens_used,
        token_budget,
    );
    let completed_at_ms = if status == GoalStatus::Completed {
        Some(Utc::now().timestamp_millis())
    } else {
        None
    };

    let now_ms = Utc::now().timestamp_millis();
    let mut active: entities::session_goal::ActiveModel = existing.into();
    if let Some(objective) = update.objective {
        active.objective = Set(objective);
    }
    active.status = Set(goal_status_label(status).to_string());
    active.token_budget = Set(token_budget_i64);
    active.updated_at_ms = Set(now_ms);
    active.completed_at_ms = Set(completed_at_ms);
    active.update(db).await.map(Some)
}

pub async fn account_usage<C>(
    db: &C,
    session_id: i64,
    token_delta: u64,
    time_delta_seconds: u64,
    mode: GoalAccountingMode,
    expected_goal_id: Option<i64>,
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
    if expected_goal_id.is_some_and(|goal_id| goal_id != existing.id) {
        return Ok(Some(existing));
    }
    let Some(existing_status) = goal_status_from_label(existing.status.as_str()) else {
        return Ok(Some(existing));
    };
    if !goal_status_matches_accounting_mode(existing_status, mode) {
        return Ok(Some(existing));
    }

    let now_ms = Utc::now().timestamp_millis();
    let token_delta = i64::try_from(token_delta)
        .map_err(|_| DbErr::Custom("goal token delta exceeds i64".to_string()))?;
    let time_delta_seconds = i64::try_from(time_delta_seconds)
        .map_err(|_| DbErr::Custom("goal time delta exceeds i64".to_string()))?;
    let new_tokens_used = existing.tokens_used.saturating_add(token_delta);
    let new_time_used_seconds = existing.time_used_seconds.saturating_add(time_delta_seconds);
    let token_budget = existing_goal_token_budget(&existing)?;
    let next_status =
        goal_status_after_accounting(existing_status, new_tokens_used, token_budget, mode);

    let mut active: entities::session_goal::ActiveModel = existing.into();
    active.tokens_used = Set(new_tokens_used);
    active.time_used_seconds = Set(new_time_used_seconds);
    active.status = Set(goal_status_label(next_status).to_string());
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

fn goal_status_after_budget_limit(
    status: GoalStatus,
    tokens_used: i64,
    token_budget: Option<u64>,
) -> GoalStatus {
    if status == GoalStatus::Active && goal_budget_exhausted(tokens_used, token_budget) {
        GoalStatus::BudgetLimited
    } else {
        status
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

fn goal_status_after_update(
    existing_status: GoalStatus,
    requested_status: Option<GoalStatus>,
    tokens_used: i64,
    token_budget: Option<u64>,
) -> GoalStatus {
    match requested_status {
        Some(GoalStatus::Paused) if existing_status == GoalStatus::BudgetLimited => {
            GoalStatus::BudgetLimited
        }
        Some(status) => goal_status_after_budget_limit(status, tokens_used, token_budget),
        None => goal_status_after_budget_limit(existing_status, tokens_used, token_budget),
    }
}

fn goal_status_matches_accounting_mode(status: GoalStatus, mode: GoalAccountingMode) -> bool {
    match mode {
        GoalAccountingMode::ActiveStatusOnly => status == GoalStatus::Active,
        GoalAccountingMode::ActiveOnly => {
            matches!(status, GoalStatus::Active | GoalStatus::BudgetLimited)
        }
        GoalAccountingMode::ActiveOrCompleted => matches!(
            status,
            GoalStatus::Active | GoalStatus::BudgetLimited | GoalStatus::Completed
        ),
        GoalAccountingMode::ActiveOrStopped => matches!(
            status,
            GoalStatus::Active | GoalStatus::Paused | GoalStatus::BudgetLimited
        ),
    }
}

fn goal_status_after_accounting(
    existing_status: GoalStatus,
    new_tokens_used: i64,
    token_budget: Option<u64>,
    mode: GoalAccountingMode,
) -> GoalStatus {
    let can_transition_to_budget_limited = match mode {
        GoalAccountingMode::ActiveStatusOnly
        | GoalAccountingMode::ActiveOnly
        | GoalAccountingMode::ActiveOrCompleted => existing_status == GoalStatus::Active,
        GoalAccountingMode::ActiveOrStopped => {
            matches!(
                existing_status,
                GoalStatus::Active | GoalStatus::Paused | GoalStatus::BudgetLimited
            )
        }
    };
    if can_transition_to_budget_limited && goal_budget_exhausted(new_tokens_used, token_budget) {
        GoalStatus::BudgetLimited
    } else {
        existing_status
    }
}

fn goal_budget_exhausted(tokens_used: i64, token_budget: Option<u64>) -> bool {
    token_budget
        .and_then(|budget| i64::try_from(budget).ok())
        .is_some_and(|budget| tokens_used >= budget)
}

fn existing_goal_token_budget(
    existing: &entities::session_goal::Model,
) -> Result<Option<u64>, DbErr> {
    existing
        .token_budget
        .map(u64::try_from)
        .transpose()
        .map_err(|_| DbErr::Custom("goal token budget is negative".to_string()))
}
