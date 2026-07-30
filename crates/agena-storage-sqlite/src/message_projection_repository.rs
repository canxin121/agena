use std::sync::Arc;

use agena_storage::{
    MessageProjectionHeaderRecord, MessageProjectionMessageWrite, MessageProjectionOpenIdentity,
    MessageProjectionPartRecord, MessageProjectionPartWrite, MessageProjectionRepository,
    MessageProjectionRepositoryError, MessageProjectionTransactionWriter,
};
use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, FromJsonQueryResult,
    Statement, Value,
};
use serde::{Deserialize, Serialize};

use crate::{StoredExecutionStatus, StoredPartKind, StoredRole};

const TABLE: &str = "agena_transcript_messages";
const COLUMNS: &str =
    "message_id, turn_id, role, state, created_at_ms, metadata, provider_state, usage, part_count";
const PART_TABLE: &str = "agena_transcript_parts";
const PART_COLUMNS: &str = "part_id, message_id, part_index, status, kind, name, summary, has_detail, activity_id, segment_id, operation_id, created_at_ms, content";
const PROJECTION_STATE_TABLE: &str = "agena_transcript_projection_states";

/// SQLite's SeaORM JSON adapter for provider-owned completion accounting.
/// The transparent wrapper is deliberately storage-owned: it preserves the
/// on-disk JSON shape while keeping `CompletionUsage` free of ORM concerns.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, FromJsonQueryResult)]
#[serde(transparent)]
pub struct PersistedCompletionUsage(pub agena_provider::CompletionUsage);

impl From<agena_provider::CompletionUsage> for PersistedCompletionUsage {
    fn from(value: agena_provider::CompletionUsage) -> Self {
        Self(value)
    }
}

impl From<PersistedCompletionUsage> for agena_provider::CompletionUsage {
    fn from(value: PersistedCompletionUsage) -> Self {
        value.0
    }
}

/// SQLite reader for materialized message-projection headers.
pub struct SeaMessageProjectionRepository {
    db: Arc<DatabaseConnection>,
}

/// Transaction-scoped SQLite writer for materialized message parts.
///
/// The session-history projection owns event interpretation, but it supplies
/// the active transaction to this adapter so part rows and the projection
/// watermark can be committed atomically.
pub struct SeaMessageProjectionTransactionWriter;

impl SeaMessageProjectionRepository {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
}

impl SeaMessageProjectionTransactionWriter {
    pub async fn terminalize_open_messages_in_transaction(
        transaction: &DatabaseTransaction,
        session_id: i64,
        identity: &MessageProjectionOpenIdentity,
        status: agena_domain::ExecutionStatus,
        updated_at_ms: i64,
    ) -> Result<(), MessageProjectionRepositoryError> {
        let (identity_column, identity_value) = match identity {
            MessageProjectionOpenIdentity::RunId(value) => ("run_id", value),
            MessageProjectionOpenIdentity::ExecutionId(value) => ("execution_id", value),
        };
        let terminal_status = StoredExecutionStatus::from(status);
        let open_statuses: [Value; 2] = [
            StoredExecutionStatus::Pending.into(),
            StoredExecutionStatus::InProgress.into(),
        ];
        transaction
            .execute(statement(
                format!(
                    "UPDATE {PART_TABLE} SET status = ? WHERE message_id IN \
                     (SELECT message_id FROM {TABLE} WHERE session_id = ? AND {identity_column} = ?) \
                     AND status IN (?, ?)"
                ),
                [
                    terminal_status.into(),
                    session_id.into(),
                    identity_value.clone().into(),
                    open_statuses[0].clone(),
                    open_statuses[1].clone(),
                ],
            ))
            .await
            .map_err(map_error)?;
        transaction
            .execute(statement(
                format!(
                    "UPDATE {TABLE} SET state = ?, updated_at_ms = ? \
                     WHERE session_id = ? AND {identity_column} = ? AND state IN (?, ?)"
                ),
                [
                    terminal_status.into(),
                    updated_at_ms.into(),
                    session_id.into(),
                    identity_value.clone().into(),
                    open_statuses[0].clone(),
                    open_statuses[1].clone(),
                ],
            ))
            .await
            .map_err(map_error)?;
        Ok(())
    }

    pub async fn clear_session_projection_in_transaction(
        transaction: &DatabaseTransaction,
        session_id: i64,
    ) -> Result<(), MessageProjectionRepositoryError> {
        transaction
            .execute(statement(
                "DELETE FROM agena_text_segments WHERE \
                 (owner_kind = 'turn_input' AND owner_id IN (SELECT turn_id FROM agena_turns WHERE session_id = ?)) \
                 OR (owner_kind = 'response' AND owner_id IN (SELECT response_id FROM agena_responses WHERE turn_id IN (SELECT turn_id FROM agena_turns WHERE session_id = ?)))"
                    .to_owned(),
                [session_id.into(), session_id.into()],
            ))
            .await
            .map_err(map_error)?;
        transaction
            .execute(statement(
                "DELETE FROM agena_activities WHERE \
                 (owner_kind = 'turn_input' AND owner_id IN (SELECT turn_id FROM agena_turns WHERE session_id = ?)) \
                 OR (owner_kind = 'response' AND owner_id IN (SELECT response_id FROM agena_responses WHERE turn_id IN (SELECT turn_id FROM agena_turns WHERE session_id = ?)))"
                    .to_owned(),
                [session_id.into(), session_id.into()],
            ))
            .await
            .map_err(map_error)?;
        transaction
            .execute(statement(
                "DELETE FROM agena_turns WHERE session_id = ?".to_owned(),
                [session_id.into()],
            ))
            .await
            .map_err(map_error)?;
        transaction
            .execute(statement(
                format!("DELETE FROM {TABLE} WHERE session_id = ?"),
                [session_id.into()],
            ))
            .await
            .map_err(map_error)?;
        transaction
            .execute(statement(
                format!("DELETE FROM {PROJECTION_STATE_TABLE} WHERE session_id = ?"),
                [session_id.into()],
            ))
            .await
            .map_err(map_error)?;
        Ok(())
    }

    pub async fn upsert_projection_watermark_in_transaction(
        transaction: &DatabaseTransaction,
        session_id: i64,
        last_seq_global: i64,
        updated_at_ms: i64,
    ) -> Result<(), MessageProjectionRepositoryError> {
        transaction
            .execute(statement(
                format!(
                    "INSERT INTO {PROJECTION_STATE_TABLE} (session_id, last_seq_global, updated_at_ms) \
                     VALUES (?, ?, ?) ON CONFLICT(session_id) DO UPDATE SET \
                     last_seq_global = excluded.last_seq_global, updated_at_ms = excluded.updated_at_ms"
                ),
                [session_id.into(), last_seq_global.into(), updated_at_ms.into()],
            ))
            .await
            .map_err(map_error)?;
        Ok(())
    }

    pub async fn upsert_message_in_transaction(
        transaction: &DatabaseTransaction,
        message: &MessageProjectionMessageWrite,
    ) -> Result<(), MessageProjectionRepositoryError> {
        let role = StoredRole::from(message.role);
        let state = StoredExecutionStatus::from(message.state);
        if let Some(existing) = transaction
            .query_one(statement(
                format!("SELECT session_id, turn_id, role, created_at_ms FROM {TABLE} WHERE message_id = ?"),
                [message.message_id.into()],
            ))
            .await
            .map_err(map_error)?
        {
            let existing_session_id: i64 = existing.try_get("", "session_id").map_err(map_error)?;
            let existing_turn_id: Option<i64> = existing.try_get("", "turn_id").map_err(map_error)?;
            let existing_role: StoredRole = existing.try_get("", "role").map_err(map_error)?;
            let existing_created_at_ms: i64 = existing.try_get("", "created_at_ms").map_err(map_error)?;
            if existing_session_id != message.session_id {
                return Err(MessageProjectionRepositoryError::Backend(format!(
                    "message {} belongs to session {}, cannot reassign it to session {}",
                    message.message_id, existing_session_id, message.session_id
                )));
            }
            if existing_turn_id != message.turn_id {
                return Err(MessageProjectionRepositoryError::Backend(format!(
                    "message {} turn identity is immutable: stored {:?}, received {:?}",
                    message.message_id, existing_turn_id, message.turn_id
                )));
            }
            if existing_role != role || existing_created_at_ms != message.created_at_ms {
                return Err(MessageProjectionRepositoryError::Backend(format!(
                    "message {} immutable identity fields changed",
                    message.message_id
                )));
            }
        }
        transaction
            .execute(statement(
                format!(
                    "INSERT INTO {TABLE} (message_id, session_id, turn_id, execution_id, run_id, role, state, created_at_ms, updated_at_ms, metadata, provider_state, usage, part_count) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                     ON CONFLICT(message_id) DO UPDATE SET \
                     execution_id = excluded.execution_id, run_id = excluded.run_id, state = excluded.state, \
                     updated_at_ms = excluded.updated_at_ms, metadata = excluded.metadata, \
                     provider_state = excluded.provider_state, usage = excluded.usage, \
                     part_count = excluded.part_count \
                     WHERE {TABLE}.session_id = excluded.session_id \
                     AND {TABLE}.turn_id IS excluded.turn_id \
                     AND {TABLE}.role = excluded.role \
                     AND {TABLE}.created_at_ms = excluded.created_at_ms"
                ),
                [
                    message.message_id.into(), message.session_id.into(), message.turn_id.into(),
                    message.execution_id.clone().into(), message.run_id.clone().into(), role.into(),
                    state.into(), message.created_at_ms.into(), message.updated_at_ms.into(),
                    message.metadata.clone().into(), message.provider_state.clone().into(),
                    message.usage.clone().into(), message.part_count.into(),
                ],
            ))
            .await
            .map_err(map_error)?;
        let persisted = transaction
            .query_one(statement(
                format!("SELECT session_id, turn_id, role, created_at_ms FROM {TABLE} WHERE message_id = ?"),
                [message.message_id.into()],
            ))
            .await
            .map_err(map_error)?
            .ok_or_else(|| MessageProjectionRepositoryError::Backend(format!("message {} disappeared after upsert", message.message_id)))?;
        let persisted_session_id: i64 = persisted.try_get("", "session_id").map_err(map_error)?;
        let persisted_turn_id: Option<i64> = persisted.try_get("", "turn_id").map_err(map_error)?;
        let persisted_role: StoredRole = persisted.try_get("", "role").map_err(map_error)?;
        let persisted_created_at_ms: i64 =
            persisted.try_get("", "created_at_ms").map_err(map_error)?;
        if persisted_session_id != message.session_id
            || persisted_turn_id != message.turn_id
            || persisted_role != role
            || persisted_created_at_ms != message.created_at_ms
        {
            return Err(MessageProjectionRepositoryError::Backend(format!(
                "message {} projection identity changed concurrently",
                message.message_id
            )));
        }
        Ok(())
    }

    pub async fn upsert_part_in_transaction(
        transaction: &DatabaseTransaction,
        part: &MessageProjectionPartWrite,
    ) -> Result<(), MessageProjectionRepositoryError> {
        let owner = transaction
            .query_one(statement(
                format!("SELECT session_id FROM {TABLE} WHERE message_id = ?"),
                [part.message_id.into()],
            ))
            .await
            .map_err(map_error)?
            .ok_or_else(|| {
                MessageProjectionRepositoryError::Backend(format!(
                    "part {} references missing message {}",
                    part.part_id, part.message_id
                ))
            })?;
        let owner_session_id: i64 = owner.try_get("", "session_id").map_err(map_error)?;
        if owner_session_id != part.session_id {
            return Err(MessageProjectionRepositoryError::Backend(format!(
                "message {} belongs to session {}, cannot attach part {} from session {}",
                part.message_id, owner_session_id, part.part_id, part.session_id
            )));
        }

        let kind = StoredPartKind::from(part.kind);
        if let Some(existing) = transaction
            .query_one(statement(
                format!("SELECT message_id, part_index, kind, activity_id, segment_id, operation_id, created_at_ms FROM {PART_TABLE} WHERE part_id = ?"),
                [part.part_id.into()],
            ))
            .await
            .map_err(map_error)?
        {
            let existing_message_id: i64 = existing.try_get("", "message_id").map_err(map_error)?;
            let existing_part_index: i32 = existing.try_get("", "part_index").map_err(map_error)?;
            let existing_kind: StoredPartKind = existing.try_get("", "kind").map_err(map_error)?;
            let existing_activity_id: Option<String> =
                existing.try_get("", "activity_id").map_err(map_error)?;
            let existing_segment_id: Option<String> =
                existing.try_get("", "segment_id").map_err(map_error)?;
            let existing_operation_id: Option<String> = existing.try_get("", "operation_id").map_err(map_error)?;
            let existing_created_at_ms: i64 = existing.try_get("", "created_at_ms").map_err(map_error)?;
            if existing_message_id != part.message_id
                || existing_part_index != part.part_index
                || existing_kind != kind
                || existing_activity_id != part.activity_id.map(|id| id.to_string())
                || existing_segment_id != part.segment_id.map(|id| id.to_string())
                || existing_operation_id != part.operation_id
                || existing_created_at_ms != part.created_at_ms
            {
                return Err(MessageProjectionRepositoryError::Backend(format!(
                    "part {} immutable identity fields changed",
                    part.part_id
                )));
            }
        }

        transaction
            .execute(statement(
                format!(
                    "INSERT INTO {PART_TABLE} ({PART_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                     ON CONFLICT(part_id) DO UPDATE SET \
                     status = excluded.status, name = excluded.name, summary = excluded.summary, \
                     has_detail = excluded.has_detail, content = excluded.content \
                     WHERE {PART_TABLE}.message_id = excluded.message_id \
                    AND {PART_TABLE}.part_index = excluded.part_index \
                    AND {PART_TABLE}.kind = excluded.kind \
                    AND {PART_TABLE}.activity_id IS excluded.activity_id \
                    AND {PART_TABLE}.segment_id IS excluded.segment_id \
                    AND {PART_TABLE}.operation_id IS excluded.operation_id \
                     AND {PART_TABLE}.created_at_ms = excluded.created_at_ms"
                ),
                [
                    part.part_id.into(),
                    part.message_id.into(),
                    part.part_index.into(),
                    StoredExecutionStatus::from(part.status).into(),
                    kind.into(),
                    part.name.clone().into(),
                    part.summary.clone().into(),
                    part.has_detail.into(),
                    part.activity_id.map(|id| id.to_string()).into(),
                    part.segment_id.map(|id| id.to_string()).into(),
                    part.operation_id.clone().into(),
                    part.created_at_ms.into(),
                    part.content.clone().into(),
                ],
            ))
            .await
            .map_err(map_error)?;

        let persisted = transaction
            .query_one(statement(
                format!("SELECT message_id, part_index, kind, activity_id, segment_id, operation_id, created_at_ms FROM {PART_TABLE} WHERE part_id = ?"),
                [part.part_id.into()],
            ))
            .await
            .map_err(map_error)?
            .ok_or_else(|| {
                MessageProjectionRepositoryError::Backend(format!(
                    "part {} disappeared after upsert",
                    part.part_id
                ))
            })?;
        let persisted_message_id: i64 = persisted.try_get("", "message_id").map_err(map_error)?;
        let persisted_part_index: i32 = persisted.try_get("", "part_index").map_err(map_error)?;
        let persisted_kind: StoredPartKind = persisted.try_get("", "kind").map_err(map_error)?;
        let persisted_activity_id: Option<String> =
            persisted.try_get("", "activity_id").map_err(map_error)?;
        let persisted_segment_id: Option<String> =
            persisted.try_get("", "segment_id").map_err(map_error)?;
        let persisted_operation_id: Option<String> =
            persisted.try_get("", "operation_id").map_err(map_error)?;
        let persisted_created_at_ms: i64 =
            persisted.try_get("", "created_at_ms").map_err(map_error)?;
        if persisted_message_id != part.message_id
            || persisted_part_index != part.part_index
            || persisted_kind != kind
            || persisted_activity_id != part.activity_id.map(|id| id.to_string())
            || persisted_segment_id != part.segment_id.map(|id| id.to_string())
            || persisted_operation_id != part.operation_id
            || persisted_created_at_ms != part.created_at_ms
        {
            return Err(MessageProjectionRepositoryError::Backend(format!(
                "part {} projection identity changed concurrently",
                part.part_id
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl MessageProjectionTransactionWriter<DatabaseTransaction>
    for SeaMessageProjectionTransactionWriter
{
    async fn terminalize_open_messages_in_transaction(
        &self,
        transaction: &DatabaseTransaction,
        session_id: i64,
        identity: &MessageProjectionOpenIdentity,
        status: agena_domain::ExecutionStatus,
        updated_at_ms: i64,
    ) -> Result<(), MessageProjectionRepositoryError> {
        Self::terminalize_open_messages_in_transaction(
            transaction,
            session_id,
            identity,
            status,
            updated_at_ms,
        )
        .await
    }

    async fn clear_session_projection_in_transaction(
        &self,
        transaction: &DatabaseTransaction,
        session_id: i64,
    ) -> Result<(), MessageProjectionRepositoryError> {
        Self::clear_session_projection_in_transaction(transaction, session_id).await
    }

    async fn upsert_projection_watermark_in_transaction(
        &self,
        transaction: &DatabaseTransaction,
        session_id: i64,
        last_seq_global: i64,
        updated_at_ms: i64,
    ) -> Result<(), MessageProjectionRepositoryError> {
        Self::upsert_projection_watermark_in_transaction(
            transaction,
            session_id,
            last_seq_global,
            updated_at_ms,
        )
        .await
    }

    async fn upsert_message_in_transaction(
        &self,
        transaction: &DatabaseTransaction,
        message: &MessageProjectionMessageWrite,
    ) -> Result<(), MessageProjectionRepositoryError> {
        Self::upsert_message_in_transaction(transaction, message).await
    }

    async fn upsert_part_in_transaction(
        &self,
        transaction: &DatabaseTransaction,
        part: &MessageProjectionPartWrite,
    ) -> Result<(), MessageProjectionRepositoryError> {
        Self::upsert_part_in_transaction(transaction, part).await
    }
}

#[async_trait]
impl MessageProjectionRepository for SeaMessageProjectionRepository {
    async fn list_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<MessageProjectionHeaderRecord>, MessageProjectionRepositoryError> {
        self.db
            .query_all(statement(
                format!(
                    "SELECT {COLUMNS} FROM {TABLE} WHERE session_id = ? ORDER BY created_at_ms ASC, message_id ASC"
                ),
                [session_id.into()],
            ))
            .await
            .map_err(map_error)?
            .into_iter()
            .map(header_from_row)
            .collect()
    }

    async fn list_headers_page(
        &self,
        session_id: i64,
        cursor: Option<(i64, i64)>,
        limit: u64,
    ) -> Result<
        (Vec<MessageProjectionHeaderRecord>, bool, Option<(i64, i64)>),
        MessageProjectionRepositoryError,
    > {
        let limit = i64::try_from(limit).map_err(|_| {
            MessageProjectionRepositoryError::Backend("message page limit exceeds i64".to_owned())
        })?;
        let fetch_limit = limit.checked_add(1).ok_or_else(|| {
            MessageProjectionRepositoryError::Backend("message page limit overflow".to_owned())
        })?;
        let mut sql = format!("SELECT {COLUMNS} FROM {TABLE} WHERE session_id = ?");
        let mut values = vec![session_id.into()];
        if let Some((created_at_ms, message_id)) = cursor {
            sql.push_str(" AND (created_at_ms < ? OR (created_at_ms = ? AND message_id < ?))");
            values.extend([
                created_at_ms.into(),
                created_at_ms.into(),
                message_id.into(),
            ]);
        }
        sql.push_str(" ORDER BY created_at_ms DESC, message_id DESC LIMIT ?");
        values.push(fetch_limit.into());
        let mut records = self
            .db
            .query_all(statement(sql, values))
            .await
            .map_err(map_error)?
            .into_iter()
            .map(header_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = records.len() > usize::try_from(limit).unwrap_or(usize::MAX);
        if has_more {
            records.pop();
        }
        let next_cursor = records
            .last()
            .map(|record| (record.created_at_ms, record.message_id));
        records.reverse();
        Ok((records, has_more, next_cursor))
    }

    async fn get_header(
        &self,
        session_id: i64,
        message_id: i64,
    ) -> Result<Option<MessageProjectionHeaderRecord>, MessageProjectionRepositoryError> {
        self.db
            .query_one(statement(
                format!(
                    "SELECT {COLUMNS} FROM {TABLE} WHERE session_id = ? AND message_id = ? LIMIT 1"
                ),
                [session_id.into(), message_id.into()],
            ))
            .await
            .map_err(map_error)?
            .map(header_from_row)
            .transpose()
    }

    async fn list_parts(
        &self,
        message_ids: &[i64],
        include_content: bool,
    ) -> Result<Vec<MessageProjectionPartRecord>, MessageProjectionRepositoryError> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", message_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        self.db.query_all(statement(
            format!("SELECT {PART_COLUMNS} FROM {PART_TABLE} WHERE message_id IN ({placeholders}) ORDER BY message_id ASC, part_index ASC"),
            message_ids.iter().copied().map(Into::into),
        )).await.map_err(map_error)?.into_iter().map(|row| part_from_row(row, include_content)).collect()
    }

    async fn get_part(
        &self,
        part_id: i64,
    ) -> Result<Option<MessageProjectionPartRecord>, MessageProjectionRepositoryError> {
        self.db
            .query_one(statement(
                format!("SELECT {PART_COLUMNS} FROM {PART_TABLE} WHERE part_id = ? LIMIT 1"),
                [part_id.into()],
            ))
            .await
            .map_err(map_error)?
            .map(|row| part_from_row(row, true))
            .transpose()
    }
}

fn statement(sql: String, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}

fn header_from_row(
    row: sea_orm::QueryResult,
) -> Result<MessageProjectionHeaderRecord, MessageProjectionRepositoryError> {
    let role: StoredRole = row.try_get("", "role").map_err(map_error)?;
    let state: StoredExecutionStatus = row.try_get("", "state").map_err(map_error)?;
    Ok(MessageProjectionHeaderRecord {
        message_id: row.try_get("", "message_id").map_err(map_error)?,
        turn_id: row.try_get("", "turn_id").map_err(map_error)?,
        role: role.into(),
        state: state.into(),
        created_at_ms: row.try_get("", "created_at_ms").map_err(map_error)?,
        metadata: row.try_get("", "metadata").map_err(map_error)?,
        provider_state: row.try_get("", "provider_state").map_err(map_error)?,
        usage: row.try_get("", "usage").map_err(map_error)?,
        part_count: row.try_get("", "part_count").map_err(map_error)?,
    })
}

fn part_from_row(
    row: sea_orm::QueryResult,
    include_content: bool,
) -> Result<MessageProjectionPartRecord, MessageProjectionRepositoryError> {
    let status: StoredExecutionStatus = row.try_get("", "status").map_err(map_error)?;
    let kind: StoredPartKind = row.try_get("", "kind").map_err(map_error)?;
    Ok(MessageProjectionPartRecord {
        part_id: row.try_get("", "part_id").map_err(map_error)?,
        message_id: row.try_get("", "message_id").map_err(map_error)?,
        part_index: row.try_get("", "part_index").map_err(map_error)?,
        status: status.into(),
        kind: kind.into(),
        name: row.try_get("", "name").map_err(map_error)?,
        summary: row.try_get("", "summary").map_err(map_error)?,
        has_detail: row.try_get("", "has_detail").map_err(map_error)?,
        activity_id: row
            .try_get::<Option<String>>("", "activity_id")
            .map_err(map_error)?
            .map(|value| {
                uuid::Uuid::parse_str(&value)
                    .map(agena_domain::ActivityId)
                    .map_err(map_error)
            })
            .transpose()?,
        segment_id: row
            .try_get::<Option<String>>("", "segment_id")
            .map_err(map_error)?
            .map(|value| {
                uuid::Uuid::parse_str(&value)
                    .map(agena_domain::ResponseSegmentId)
                    .map_err(map_error)
            })
            .transpose()?,
        operation_id: row.try_get("", "operation_id").map_err(map_error)?,
        created_at_ms: row.try_get("", "created_at_ms").map_err(map_error)?,
        content: if include_content {
            row.try_get("", "content").map_err(map_error)?
        } else {
            None
        },
    })
}

fn map_error(error: impl std::fmt::Display) -> MessageProjectionRepositoryError {
    MessageProjectionRepositoryError::Backend(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{ExecutionStatus, PartKind, Role};
    use sea_orm::{ConnectionTrait, Database, TransactionTrait};

    async fn repository() -> SeaMessageProjectionRepository {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "CREATE TABLE {TABLE} (message_id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL, turn_id INTEGER NULL, role INTEGER NOT NULL, state INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, metadata JSON NOT NULL, provider_state JSON NULL, usage JSON NULL, part_count INTEGER NOT NULL)"
            ),
        ))
        .await
        .expect("create projection fixture");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "CREATE TABLE {PART_TABLE} (part_id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL, part_index INTEGER NOT NULL, status INTEGER NOT NULL, kind INTEGER NOT NULL, name TEXT NULL, summary TEXT NULL, has_detail BOOLEAN NOT NULL, activity_id TEXT NULL, segment_id TEXT NULL, operation_id TEXT NULL, created_at_ms INTEGER NOT NULL, content JSON NULL)"
            ),
        ))
        .await
        .expect("create part fixture");
        for (id, created_at_ms) in [(11, 100), (12, 200)] {
            db.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("INSERT INTO {TABLE} (message_id, session_id, turn_id, role, state, created_at_ms, metadata, provider_state, usage, part_count) VALUES (?, 7, ?, 2, 3, ?, ?, ?, ?, ?)") ,
                [id.into(), id.into(), created_at_ms.into(), serde_json::json!({"turn_id": id}).into(), serde_json::json!({"response_id": id.to_string()}).into(), serde_json::json!({"output_tokens": id}).into(), (id - 10).into()],
            )).await.expect("insert projection header");
        }
        db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!("INSERT INTO {PART_TABLE} (part_id, message_id, part_index, status, kind, name, summary, has_detail, activity_id, segment_id, operation_id, created_at_ms, content) VALUES (51, 12, 0, 3, 1, 'text', 'summary', 1, NULL, NULL, NULL, 201, ?)") ,
            [serde_json::json!({"type":"text","text":"detail","synthetic":false}).into()],
        )).await.expect("insert part");
        SeaMessageProjectionRepository::new(Arc::new(db))
    }

    #[tokio::test]
    async fn reads_visible_headers_with_stable_order_and_cursor() {
        let repository = repository().await;
        let headers = repository.list_headers(7).await.expect("headers");
        assert_eq!(
            headers
                .iter()
                .map(|header| header.message_id)
                .collect::<Vec<_>>(),
            vec![11, 12]
        );
        assert_eq!(headers[0].role, Role::Assistant);
        assert_eq!(headers[0].state, ExecutionStatus::Completed);
        assert_eq!(
            headers[1].usage,
            Some(serde_json::json!({"output_tokens": 12}))
        );

        let (page, has_more, cursor) = repository
            .list_headers_page(7, None, 1)
            .await
            .expect("first page");
        assert_eq!(page[0].message_id, 12);
        assert!(has_more);
        assert_eq!(cursor, Some((200, 12)));
        let (page, has_more, _) = repository
            .list_headers_page(7, cursor, 1)
            .await
            .expect("second page");
        assert_eq!(page[0].message_id, 11);
        assert!(!has_more);
        assert_eq!(
            repository.get_header(7, 13).await.expect("hidden header"),
            None
        );
        let summary_parts = repository
            .list_parts(&[12], false)
            .await
            .expect("summary parts");
        assert_eq!(summary_parts[0].content, None);
        let full_part = repository
            .get_part(51)
            .await
            .expect("part")
            .expect("present");
        assert_eq!(full_part.kind, agena_domain::PartKind::Text);
        assert!(full_part.content.is_some());
    }

    #[tokio::test]
    async fn transaction_writer_keeps_part_write_inside_caller_transaction() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "CREATE TABLE {TABLE} (message_id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL)"
            ),
        ))
        .await
        .expect("create message fixture");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "CREATE TABLE {PART_TABLE} (part_id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL, part_index INTEGER NOT NULL, status INTEGER NOT NULL, kind INTEGER NOT NULL, name TEXT NULL, summary TEXT NULL, has_detail BOOLEAN NOT NULL, activity_id TEXT NULL, segment_id TEXT NULL, operation_id TEXT NULL, created_at_ms INTEGER NOT NULL, content JSON NULL)"
            ),
        ))
        .await
        .expect("create part fixture");
        db.execute(statement(
            format!("INSERT INTO {TABLE} (message_id, session_id) VALUES (?, ?)"),
            [41.into(), 7.into()],
        ))
        .await
        .expect("insert message fixture");

        let txn = db.begin().await.expect("begin transaction");
        SeaMessageProjectionTransactionWriter::upsert_part_in_transaction(
            &txn,
            &MessageProjectionPartWrite {
                session_id: 7,
                part_id: 51,
                message_id: 41,
                part_index: 0,
                status: ExecutionStatus::InProgress,
                kind: PartKind::Text,
                name: Some("text".to_owned()),
                summary: None,
                has_detail: true,
                activity_id: None,
                segment_id: Some(agena_domain::ResponseSegmentId::new()),
                operation_id: None,
                created_at_ms: 100,
                content: Some(serde_json::json!({"type": "text", "text": "pending"})),
            },
        )
        .await
        .expect("write part in transaction");
        txn.rollback().await.expect("rollback transaction");

        let count: i64 = db
            .query_one(statement(
                format!("SELECT COUNT(*) AS count FROM {PART_TABLE}"),
                [],
            ))
            .await
            .expect("count parts")
            .expect("count row")
            .try_get("", "count")
            .expect("count value");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn transaction_writer_keeps_message_write_inside_caller_transaction() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "CREATE TABLE {TABLE} (message_id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL, turn_id INTEGER NULL, execution_id TEXT NULL, run_id TEXT NULL, role INTEGER NOT NULL, state INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, metadata JSON NOT NULL, provider_state JSON NULL, usage JSON NULL, part_count INTEGER NOT NULL)"
            ),
        ))
        .await
        .expect("create message fixture");

        let txn = db.begin().await.expect("begin transaction");
        SeaMessageProjectionTransactionWriter::upsert_message_in_transaction(
            &txn,
            &MessageProjectionMessageWrite {
                message_id: 41,
                session_id: 7,
                turn_id: Some(9),
                execution_id: Some("execution-1".to_owned()),
                run_id: Some("run-1".to_owned()),
                role: Role::Assistant,
                state: ExecutionStatus::InProgress,
                created_at_ms: 100,
                updated_at_ms: 101,
                metadata: serde_json::json!({"turn_id": 9}),
                provider_state: Some(serde_json::json!({"response_id": "response-1"})),
                usage: None,
                part_count: 1,
            },
        )
        .await
        .expect("write message in transaction");
        txn.rollback().await.expect("rollback transaction");

        let count: i64 = db
            .query_one(statement(
                format!("SELECT COUNT(*) AS count FROM {TABLE}"),
                [],
            ))
            .await
            .expect("count messages")
            .expect("count row")
            .try_get("", "count")
            .expect("count value");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn transaction_writer_terminalizes_projection_and_updates_watermark_atomically() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "CREATE TABLE {TABLE} (message_id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL, run_id TEXT NULL, execution_id TEXT NULL, state INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)"
            ),
        ))
        .await
        .expect("create message fixture");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "CREATE TABLE {PART_TABLE} (part_id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL, status INTEGER NOT NULL)"
            ),
        ))
        .await
        .expect("create part fixture");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "CREATE TABLE {PROJECTION_STATE_TABLE} (session_id INTEGER PRIMARY KEY, last_seq_global INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)"
            ),
        ))
        .await
        .expect("create watermark fixture");
        db.execute(statement(
            format!("INSERT INTO {TABLE} (message_id, session_id, run_id, state, updated_at_ms) VALUES (41, 7, 'run-1', ?, 1)"),
            [StoredExecutionStatus::InProgress.into()],
        ))
        .await
        .expect("insert message");
        db.execute(statement(
            format!("INSERT INTO {PART_TABLE} (part_id, message_id, status) VALUES (51, 41, ?)"),
            [StoredExecutionStatus::InProgress.into()],
        ))
        .await
        .expect("insert part");

        let txn = db.begin().await.expect("begin transaction");
        SeaMessageProjectionTransactionWriter::terminalize_open_messages_in_transaction(
            &txn,
            7,
            &MessageProjectionOpenIdentity::RunId("run-1".to_owned()),
            ExecutionStatus::Cancelled,
            100,
        )
        .await
        .expect("terminalize projection");
        SeaMessageProjectionTransactionWriter::upsert_projection_watermark_in_transaction(
            &txn, 7, 99, 100,
        )
        .await
        .expect("upsert watermark");
        txn.commit().await.expect("commit transaction");

        let message_state: StoredExecutionStatus = db
            .query_one(statement(
                format!("SELECT state FROM {TABLE} WHERE message_id = 41"),
                [],
            ))
            .await
            .expect("query message")
            .expect("message row")
            .try_get("", "state")
            .expect("message state");
        let part_state: StoredExecutionStatus = db
            .query_one(statement(
                format!("SELECT status FROM {PART_TABLE} WHERE part_id = 51"),
                [],
            ))
            .await
            .expect("query part")
            .expect("part row")
            .try_get("", "status")
            .expect("part status");
        let watermark: i64 = db
            .query_one(statement(
                format!(
                    "SELECT last_seq_global FROM {PROJECTION_STATE_TABLE} WHERE session_id = 7"
                ),
                [],
            ))
            .await
            .expect("query watermark")
            .expect("watermark row")
            .try_get("", "last_seq_global")
            .expect("watermark value");
        assert_eq!(message_state, StoredExecutionStatus::Cancelled);
        assert_eq!(part_state, StoredExecutionStatus::Cancelled);
        assert_eq!(watermark, 99);
    }
}
