use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::db::entities;
use crate::session::history::{HistoryItem, HistoryRecord};

pub(crate) async fn append_history_item<C>(
    db: &C,
    session_id: i64,
    seq: i64,
    item: HistoryItem,
    now: DateTime<Utc>,
) -> Result<HistoryRecord, DbErr>
where
    C: ConnectionTrait,
{
    let event_id = Uuid::new_v4();
    let event_type = item.event_type().to_string();
    let row = entities::session_history_event::ActiveModel {
        session_id: Set(session_id),
        seq: Set(seq),
        event_uuid: Set(event_id.to_string()),
        event_type: Set(event_type),
        payload: Set(item),
        causation_uuid: Set(None),
        correlation_uuid: Set(None),
        created_at_ms: Set(now.timestamp_millis()),
        ..Default::default()
    }
    .insert(db)
    .await?;

    row.try_into()
}

pub(crate) async fn append_history_items<C>(
    db: &C,
    session_id: i64,
    mut next_seq: i64,
    items: impl IntoIterator<Item = HistoryItem>,
    now: DateTime<Utc>,
) -> Result<Vec<HistoryRecord>, DbErr>
where
    C: ConnectionTrait,
{
    let mut records = Vec::new();
    for item in items {
        next_seq += 1;
        records.push(append_history_item(db, session_id, next_seq, item, now).await?);
    }
    Ok(records)
}

pub(crate) async fn list_history_records<C>(
    db: &C,
    session_id: i64,
) -> Result<Vec<HistoryRecord>, DbErr>
where
    C: ConnectionTrait,
{
    let rows = entities::session_history_event::Entity::find()
        .filter(entities::session_history_event::Column::SessionId.eq(session_id))
        .order_by_asc(entities::session_history_event::Column::Seq)
        .order_by_asc(entities::session_history_event::Column::Id)
        .all(db)
        .await?;

    rows.into_iter().map(TryInto::try_into).collect()
}

pub(crate) async fn latest_history_seq<C>(db: &C, session_id: i64) -> Result<Option<i64>, DbErr>
where
    C: ConnectionTrait,
{
    let row = entities::session_history_event::Entity::find()
        .filter(entities::session_history_event::Column::SessionId.eq(session_id))
        .order_by_desc(entities::session_history_event::Column::Seq)
        .order_by_desc(entities::session_history_event::Column::Id)
        .one(db)
        .await?;

    Ok(row.map(|row| row.seq))
}

impl TryFrom<entities::session_history_event::Model> for HistoryRecord {
    type Error = DbErr;

    fn try_from(row: entities::session_history_event::Model) -> Result<Self, Self::Error> {
        let event_id = Uuid::parse_str(row.event_uuid.as_str())
            .map_err(|err| DbErr::Custom(format!("invalid history event uuid: {err}")))?;
        let causation_id = row
            .causation_uuid
            .map(|value| Uuid::parse_str(value.as_str()))
            .transpose()
            .map_err(|err| DbErr::Custom(format!("invalid history causation uuid: {err}")))?;
        let correlation_id = row
            .correlation_uuid
            .map(|value| Uuid::parse_str(value.as_str()))
            .transpose()
            .map_err(|err| DbErr::Custom(format!("invalid history correlation uuid: {err}")))?;
        let created_at = DateTime::from_timestamp_millis(row.created_at_ms)
            .ok_or_else(|| DbErr::Custom(format!("invalid history timestamp millis: {}", row.created_at_ms)))?;

        Ok(Self {
            seq: row.seq,
            event_id,
            session_id: row.session_id,
            created_at,
            causation_id,
            correlation_id,
            item: row.payload,
        })
    }
}
