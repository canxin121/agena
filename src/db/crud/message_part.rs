use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder};

use crate::db::entities;
use crate::message::{PartContent, SessionMessagePart, SessionMessagePartSummary};

pub async fn list_message_part_summaries(
    db: &DatabaseConnection,
    message_id: i64,
) -> Result<Vec<SessionMessagePartSummary>, DbErr> {
    let models = entities::message_part::Entity::find()
        .filter(entities::message_part::Column::MessageId.eq(message_id))
        .order_by_asc(entities::message_part::Column::PartIndex)
        .all(db)
        .await?;

    models.into_iter().map(map_message_part_summary).collect()
}

pub async fn get_message_part_detail(
    db: &DatabaseConnection,
    part_id: i64,
) -> Result<Option<PartContent>, DbErr> {
    let model = entities::message_part_detail::Entity::find_by_id(part_id)
        .one(db)
        .await?;

    Ok(model.map(|record| record.detail))
}

pub async fn get_message_part_with_detail(
    db: &DatabaseConnection,
    part_id: i64,
) -> Result<Option<SessionMessagePart>, DbErr> {
    let Some(model) = entities::message_part::Entity::find_by_id(part_id)
        .one(db)
        .await?
    else {
        return Ok(None);
    };

    let summary = map_message_part_summary(model)?;
    let detail = if summary.has_detail {
        get_message_part_detail(db, part_id).await?
    } else {
        None
    };

    Ok(Some(SessionMessagePart::from_summary(summary, detail)))
}

fn map_message_part_summary(
    model: entities::message_part::Model,
) -> Result<SessionMessagePartSummary, DbErr> {
    Ok(SessionMessagePartSummary {
        id: model.id,
        message_id: model.message_id,
        part_index: model.part_index,
        status: model.status,
        kind: model.kind,
        name: model.name,
        summary: model.summary_text,
        has_detail: model.has_detail,
        operation_id: model.operation_id,
        created_at: timestamp_millis_to_utc(model.created_at_ms)?,
    })
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<DateTime<Utc>, DbErr> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| DbErr::Custom(format!("invalid timestamp millis: {timestamp_ms}")))
}
