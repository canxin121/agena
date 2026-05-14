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
    let token_budget = token_budget
        .map(i64::try_from)
        .transpose()
        .map_err(|_| DbErr::Custom("goal token budget exceeds i64".to_string()))?;

    if let Some(existing) = get_by_session_id(db, session_id).await? {
        let mut active: entities::session_goal::ActiveModel = existing.into();
        active.objective = Set(objective);
        active.status = Set(goal_status_label(GoalStatus::Active).to_string());
        active.token_budget = Set(token_budget);
        active.updated_at_ms = Set(now_ms);
        active.completed_at_ms = Set(None);
        return active.update(db).await;
    }

    entities::session_goal::ActiveModel {
        session_id: Set(session_id),
        objective: Set(objective),
        status: Set(goal_status_label(GoalStatus::Active).to_string()),
        token_budget: Set(token_budget),
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
        GoalStatus::Completed => "completed",
    }
}
