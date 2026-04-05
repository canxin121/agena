use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait,
    QueryFilter, QueryOrder,
};

use crate::db::entities;
use crate::event::SessionEvent;
use crate::session::{SessionEventRecord, SessionEventType};

pub async fn append_session_event<C>(
    db: &C,
    session_id: i64,
    seq: i64,
    payload: SessionEvent,
    now: DateTime<Utc>,
) -> Result<entities::session_event::Model, DbErr>
where
    C: ConnectionTrait,
{
    let model = entities::session_event::ActiveModel {
        session_id: Set(session_id),
        seq: Set(seq),
        event_type: Set(SessionEventType::from(&payload)),
        payload: Set(payload),
        causation_id: Set(None),
        correlation_id: Set(None),
        created_at_ms: Set(now.timestamp_millis()),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(model)
}

pub async fn list_session_events<C>(
    db: &C,
    session_id: i64,
) -> Result<Vec<SessionEventRecord>, DbErr>
where
    C: ConnectionTrait,
{
    let rows = entities::session_event::Entity::find()
        .filter(entities::session_event::Column::SessionId.eq(session_id))
        .order_by_asc(entities::session_event::Column::Seq)
        .order_by_asc(entities::session_event::Column::Id)
        .all(db)
        .await?;

    rows.into_iter().map(to_session_event_record).collect()
}

pub async fn latest_event_seq<C>(db: &C, session_id: i64) -> Result<Option<i64>, DbErr>
where
    C: ConnectionTrait,
{
    let row = entities::session_event::Entity::find()
        .filter(entities::session_event::Column::SessionId.eq(session_id))
        .order_by_desc(entities::session_event::Column::Seq)
        .order_by_desc(entities::session_event::Column::Id)
        .one(db)
        .await?;

    Ok(row.map(|row| row.seq))
}

fn to_session_event_record(
    row: entities::session_event::Model,
) -> Result<SessionEventRecord, DbErr> {
    Ok(SessionEventRecord {
        event_id: Some(row.id),
        session_id: row.session_id,
        seq: row.seq,
        event_type: row.event_type,
        payload: row.payload,
        causation_id: row.causation_id,
        correlation_id: row.correlation_id,
        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
    })
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<DateTime<Utc>, DbErr> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| DbErr::Custom(format!("invalid timestamp millis: {timestamp_ms}")))
}
