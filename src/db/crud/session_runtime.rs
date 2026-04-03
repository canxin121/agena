use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait,
    QueryFilter, QueryOrder,
};

use crate::db::entities;
use crate::event::SessionEvent;
use crate::session::{Session, SessionCheckpoint, SessionEventRecord, SessionEventType};

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

pub async fn save_checkpoint<C>(
    db: &C,
    session_id: i64,
    upto_seq: i64,
    session: Session,
    state_hash: Option<String>,
    now: DateTime<Utc>,
) -> Result<entities::session_checkpoint::Model, DbErr>
where
    C: ConnectionTrait,
{
    let snapshot = serde_json::to_value(session).map_err(|err| DbErr::Custom(err.to_string()))?;

    entities::session_checkpoint::ActiveModel {
        session_id: Set(session_id),
        upto_seq: Set(upto_seq),
        snapshot: Set(snapshot),
        state_hash: Set(state_hash),
        created_at_ms: Set(now.timestamp_millis()),
        ..Default::default()
    }
    .insert(db)
    .await
}

pub async fn latest_checkpoint<C>(
    db: &C,
    session_id: i64,
) -> Result<Option<SessionCheckpoint>, DbErr>
where
    C: ConnectionTrait,
{
    let row = entities::session_checkpoint::Entity::find()
        .filter(entities::session_checkpoint::Column::SessionId.eq(session_id))
        .order_by_desc(entities::session_checkpoint::Column::UptoSeq)
        .order_by_desc(entities::session_checkpoint::Column::Id)
        .one(db)
        .await?;

    row.map(to_session_checkpoint).transpose()
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

fn to_session_checkpoint(
    row: entities::session_checkpoint::Model,
) -> Result<SessionCheckpoint, DbErr> {
    Ok(SessionCheckpoint {
        id: row.id,
        session_id: row.session_id,
        upto_seq: row.upto_seq,
        session: deserialize_checkpoint_session(row.snapshot)?,
        state_hash: row.state_hash,
        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
    })
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<DateTime<Utc>, DbErr> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| DbErr::Custom(format!("invalid timestamp millis: {timestamp_ms}")))
}

fn deserialize_checkpoint_session(value: serde_json::Value) -> Result<Session, DbErr> {
    let mut session = try_deserialize_session(&value)
        .or_else(|| value.get("session").and_then(try_deserialize_session))
        .or_else(|| value.get("Current").and_then(try_deserialize_session))
        .or_else(|| value.as_str().and_then(deserialize_session_from_str))
        .ok_or_else(|| {
            DbErr::Custom(
                "data did not match the current or legacy checkpoint session shape".into(),
            )
        })?;
    session.refresh_derived();
    Ok(session)
}

fn try_deserialize_session(value: &serde_json::Value) -> Option<Session> {
    serde_json::from_value::<Session>(value.clone()).ok()
}

fn deserialize_session_from_str(raw: &str) -> Option<Session> {
    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    try_deserialize_session(&value)
        .or_else(|| value.get("session").and_then(try_deserialize_session))
        .or_else(|| value.get("Current").and_then(try_deserialize_session))
}
