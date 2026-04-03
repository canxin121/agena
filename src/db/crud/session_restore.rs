use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait,
    QueryFilter, QueryOrder,
};

use crate::{
    checkpoint::{CheckpointBlob, SessionRestorePoint, SessionRestorePointSnapshot},
    db::entities,
};

pub async fn upsert_blob<C>(db: &C, blob: &CheckpointBlob, now: DateTime<Utc>) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    if entities::checkpoint_blob::Entity::find_by_id(blob.hash.clone())
        .one(db)
        .await?
        .is_some()
    {
        return Ok(());
    }

    entities::checkpoint_blob::ActiveModel {
        hash: Set(blob.hash.clone()),
        bytes: Set(blob.bytes.clone()),
        size_bytes: Set(blob.bytes.len() as i64),
        created_at_ms: Set(now.timestamp_millis()),
    }
    .insert(db)
    .await?;
    Ok(())
}

pub async fn load_blob<C>(db: &C, hash: &str) -> Result<Option<Vec<u8>>, DbErr>
where
    C: ConnectionTrait,
{
    Ok(
        entities::checkpoint_blob::Entity::find_by_id(hash.to_string())
            .one(db)
            .await?
            .map(|row| row.bytes),
    )
}

pub async fn create_restore_point<C>(
    db: &C,
    session_id: i64,
    upto_seq: i64,
    call_id: Option<i64>,
    message_id: Option<i64>,
    operation_id: Option<&str>,
    snapshot: SessionRestorePointSnapshot,
    now: DateTime<Utc>,
) -> Result<entities::session_restore_point::Model, DbErr>
where
    C: ConnectionTrait,
{
    entities::session_restore_point::ActiveModel {
        session_id: Set(session_id),
        upto_seq: Set(upto_seq),
        call_id: Set(call_id),
        message_id: Set(message_id),
        operation_id: Set(operation_id.map(ToOwned::to_owned)),
        snapshot: Set(snapshot),
        created_at_ms: Set(now.timestamp_millis()),
        ..Default::default()
    }
    .insert(db)
    .await
}

pub async fn latest_restore_point<C>(
    db: &C,
    session_id: i64,
) -> Result<Option<SessionRestorePoint>, DbErr>
where
    C: ConnectionTrait,
{
    let row = entities::session_restore_point::Entity::find()
        .filter(entities::session_restore_point::Column::SessionId.eq(session_id))
        .order_by_desc(entities::session_restore_point::Column::Id)
        .one(db)
        .await?;

    row.map(to_restore_point).transpose()
}

pub async fn find_restore_point<C>(
    db: &C,
    session_id: i64,
    restore_point_id: i64,
) -> Result<Option<SessionRestorePoint>, DbErr>
where
    C: ConnectionTrait,
{
    let row = entities::session_restore_point::Entity::find_by_id(restore_point_id)
        .filter(entities::session_restore_point::Column::SessionId.eq(session_id))
        .one(db)
        .await?;

    row.map(to_restore_point).transpose()
}

fn to_restore_point(
    row: entities::session_restore_point::Model,
) -> Result<SessionRestorePoint, DbErr> {
    Ok(SessionRestorePoint {
        id: row.id,
        session_id: row.session_id,
        upto_seq: row.upto_seq,
        call_id: row.call_id,
        message_id: row.message_id,
        operation_id: row.operation_id,
        snapshot: row.snapshot,
        created_at: DateTime::from_timestamp_millis(row.created_at_ms).ok_or_else(|| {
            DbErr::Custom(format!("invalid timestamp millis: {}", row.created_at_ms))
        })?,
    })
}
