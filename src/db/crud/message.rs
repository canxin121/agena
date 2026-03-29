use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, QueryOrder,
};

use crate::db::entities;
use crate::message::{Message, MessageMetadata, MessagePart, MessageSource};

#[derive(Debug, Clone)]
pub struct NewMessageRecord {
    pub session_id: i64,
    pub role: crate::role::Role,
    pub status: crate::message::MessageStatus,
    pub metadata: MessageMetadata,
    pub usage: Option<crate::message::MessageUsage>,
    pub finish: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn create_message(
    db: &DatabaseConnection,
    input: NewMessageRecord,
) -> Result<entities::message::Model, DbErr> {
    let ts_ms = input.created_at.timestamp_millis();
    entities::message::ActiveModel {
        session_id: Set(input.session_id),
        role: Set(input.role),
        status: Set(input.status),
        source: Set(input.metadata.source),
        parent_message_id: Set(input.metadata.parent_message_id),
        generated_by_call_id: Set(input.metadata.generated_by_call_id),
        model_provider_id: Set(input.metadata.model_provider_id),
        model_id: Set(input.metadata.model_id),
        usage: Set(input.usage),
        finish: Set(input.finish),
        created_at_ms: Set(ts_ms),
        updated_at_ms: Set(ts_ms),
        ..Default::default()
    }
    .insert(db)
    .await
}

pub async fn insert_message_with_parts(
    db: &DatabaseConnection,
    session_id: i64,
    message: &Message,
) -> Result<Message, DbErr> {
    let created = create_message(
        db,
        NewMessageRecord {
            session_id,
            role: message.role,
            status: message.state,
            metadata: message.metadata.clone(),
            usage: message.usage.clone(),
            finish: message.finish.clone(),
            created_at: message.created_at,
        },
    )
    .await?;

    let mut persisted_parts = Vec::with_capacity(message.parts.len());
    for (idx, part) in message.parts.iter().enumerate() {
        persisted_parts.push(insert_part(db, created.id, idx as i32, part).await?);
    }

    Ok(Message {
        id: created.id,
        role: created.role,
        state: created.status,
        parts: persisted_parts,
        created_at: timestamp_millis_to_utc(created.created_at_ms)?,
        metadata: MessageMetadata {
            source: created.source,
            parent_message_id: created.parent_message_id,
            generated_by_call_id: created.generated_by_call_id,
            model_provider_id: created.model_provider_id,
            model_id: created.model_id,
            tags: Vec::new(),
        },
        usage: created.usage,
        finish: created.finish,
    })
}

pub async fn list_messages_with_parts(
    db: &DatabaseConnection,
    session_id: i64,
) -> Result<Vec<Message>, DbErr> {
    let message_rows = entities::message::Entity::find()
        .filter(entities::message::Column::SessionId.eq(session_id))
        .order_by_asc(entities::message::Column::CreatedAtMs)
        .order_by_asc(entities::message::Column::Id)
        .all(db)
        .await?;

    let mut result = Vec::with_capacity(message_rows.len());
    for row in message_rows {
        let part_rows = entities::message_part::Entity::find()
            .filter(entities::message_part::Column::MessageId.eq(row.id))
            .order_by_asc(entities::message_part::Column::PartIndex)
            .all(db)
            .await?;

        let mut parts = Vec::with_capacity(part_rows.len());
        for part_row in part_rows {
            let detail = if part_row.has_detail {
                entities::message_part_detail::Entity::find_by_id(part_row.id)
                    .one(db)
                    .await?
                    .map(|item| item.detail)
            } else {
                None
            };

            parts.push(MessagePart::from_summary(
                crate::message::MessagePartSummary {
                    id: part_row.id,
                    message_id: part_row.message_id,
                    part_index: part_row.part_index,
                    status: part_row.status,
                    kind: part_row.kind,
                    name: part_row.name,
                    summary: part_row.summary_text,
                    has_detail: part_row.has_detail,
                    operation_id: part_row.operation_id,
                    created_at: timestamp_millis_to_utc(part_row.created_at_ms)?,
                },
                detail,
            ));
        }

        result.push(Message {
            id: row.id,
            role: row.role,
            state: row.status,
            parts,
            created_at: timestamp_millis_to_utc(row.created_at_ms)?,
            metadata: MessageMetadata {
                source: row.source,
                parent_message_id: row.parent_message_id,
                generated_by_call_id: row.generated_by_call_id,
                model_provider_id: row.model_provider_id,
                model_id: row.model_id,
                tags: Vec::new(),
            },
            usage: row.usage,
            finish: row.finish,
        });
    }

    Ok(result)
}

pub async fn delete_messages_by_session_id(
    db: &DatabaseConnection,
    session_id: i64,
) -> Result<u64, DbErr> {
    let deleted = entities::message::Entity::delete_many()
        .filter(entities::message::Column::SessionId.eq(session_id))
        .exec(db)
        .await?;
    Ok(deleted.rows_affected)
}

async fn insert_part(
    db: &DatabaseConnection,
    message_id: i64,
    part_index: i32,
    part: &MessagePart,
) -> Result<MessagePart, DbErr> {
    let created_at_ms = part.created_at.timestamp_millis();
    let part_row = entities::message_part::ActiveModel {
        message_id: Set(message_id),
        part_index: Set(part_index),
        kind: Set(part.kind),
        status: Set(part.status),
        name: Set(part.name.clone()),
        summary_text: Set(part.summary.clone()),
        has_detail: Set(part.has_detail),
        call_id: Set(extract_call_id(part)),
        operation_id: Set(part.operation_id.clone()),
        created_at_ms: Set(created_at_ms),
        updated_at_ms: Set(created_at_ms),
        ..Default::default()
    }
    .insert(db)
    .await?;

    if let Some(content) = part.content.clone()
        && part.has_detail
    {
        entities::message_part_detail::ActiveModel {
            part_id: Set(part_row.id),
            detail: Set(content.clone()),
            updated_at_ms: Set(created_at_ms),
        }
        .insert(db)
        .await?;
    }

    Ok(MessagePart {
        id: part_row.id,
        message_id,
        part_index,
        status: part_row.status,
        kind: part_row.kind,
        name: part_row.name,
        summary: part_row.summary_text,
        has_detail: part_row.has_detail,
        operation_id: part_row.operation_id,
        created_at: timestamp_millis_to_utc(part_row.created_at_ms)?,
        content: part.content.clone(),
    })
}

fn extract_call_id(part: &MessagePart) -> Option<i64> {
    part.content.as_ref().and_then(|content| match content {
        crate::message::PartContent::ToolExecution(tool) => match tool {
            crate::message::ToolExecutionPart::Pending { call_id, .. }
            | crate::message::ToolExecutionPart::InProgress { call_id, .. }
            | crate::message::ToolExecutionPart::Completed { call_id, .. }
            | crate::message::ToolExecutionPart::Failed { call_id, .. } => Some(*call_id),
        },
        _ => None,
    })
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<DateTime<Utc>, DbErr> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| DbErr::Custom(format!("invalid timestamp millis: {timestamp_ms}")))
}

#[allow(dead_code)]
fn _default_source_for_role(role: crate::role::Role) -> MessageSource {
    match role {
        crate::role::Role::User => MessageSource::User,
        crate::role::Role::Assistant => MessageSource::Assistant,
        crate::role::Role::System => MessageSource::System,
        crate::role::Role::Tool => MessageSource::Tool,
    }
}
