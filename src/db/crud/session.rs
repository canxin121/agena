use std::collections::HashMap;

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait,
    FromQueryResult, QueryFilter, QueryOrder, QuerySelect,
};

use crate::db::entities;
use crate::session::SessionListRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionMessageStats {
    pub message_count: i64,
    pub last_message_at_ms: Option<i64>,
}

#[derive(Debug, Clone, FromQueryResult)]
struct SessionMessageStatsRow {
    session_id: i64,
    message_count: i64,
    last_message_at_ms: Option<i64>,
}

#[derive(Debug, Clone, FromQueryResult)]
struct SessionChildCountRow {
    session_id: i64,
    child_session_count: i64,
}

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
        version: Set(1),
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
    let next_version = existing.version + 1;
    let mut active: entities::session::ActiveModel = existing.into();
    active.version = Set(next_version);
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

pub async fn list_sessions_by_workspace_id_with_request<C>(
    db: &C,
    workspace_id: i64,
    request: SessionListRequest,
) -> Result<Vec<entities::session::Model>, DbErr>
where
    C: ConnectionTrait,
{
    let mut query = entities::session::Entity::find()
        .filter(entities::session::Column::WorkspaceId.eq(workspace_id))
        .order_by_desc(entities::session::Column::UpdatedAtMs)
        .order_by_desc(entities::session::Column::Id);

    if let Some(limit) = request.limit {
        if request.offset > 0 {
            query = query.offset(request.offset);
        }
        query = query.limit(limit);
        return query.all(db).await;
    }

    let sessions = query.all(db).await?;
    let offset = usize::try_from(request.offset)
        .map_err(|_| DbErr::Custom(format!("session list offset too large: {}", request.offset)))?;
    Ok(sessions.into_iter().skip(offset).collect())
}

pub async fn session_message_stats_by_session_ids<C>(
    db: &C,
    session_ids: &[i64],
) -> Result<HashMap<i64, SessionMessageStats>, DbErr>
where
    C: ConnectionTrait,
{
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }

    entities::message::Entity::find()
        .select_only()
        .column(entities::message::Column::SessionId)
        .column_as(entities::message::Column::Id.count(), "message_count")
        .column_as(
            entities::message::Column::CreatedAtMs.max(),
            "last_message_at_ms",
        )
        .filter(entities::message::Column::SessionId.is_in(session_ids.iter().copied()))
        .group_by(entities::message::Column::SessionId)
        .into_model::<SessionMessageStatsRow>()
        .all(db)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| {
                    (
                        row.session_id,
                        SessionMessageStats {
                            message_count: row.message_count,
                            last_message_at_ms: row.last_message_at_ms,
                        },
                    )
                })
                .collect()
        })
}

pub async fn child_session_counts_by_parent_ids<C>(
    db: &C,
    parent_ids: &[i64],
) -> Result<HashMap<i64, i64>, DbErr>
where
    C: ConnectionTrait,
{
    if parent_ids.is_empty() {
        return Ok(HashMap::new());
    }

    entities::session::Entity::find()
        .select_only()
        .column_as(entities::session::Column::ParentId, "session_id")
        .column_as(entities::session::Column::Id.count(), "child_session_count")
        .filter(entities::session::Column::ParentId.is_in(parent_ids.iter().copied()))
        .group_by(entities::session::Column::ParentId)
        .into_model::<SessionChildCountRow>()
        .all(db)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| (row.session_id, row.child_session_count))
                .collect()
        })
}
