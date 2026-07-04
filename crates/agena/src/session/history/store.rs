use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection,
    DbErr, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    TransactionTrait, sea_query::OnConflict,
};

use crate::db::entities::{activity_message, activity_part, activity_projection_state};
use crate::event::{
    DomainEvent, EventFilter, EventKind, EventPublisher, MessagePartUpdatedEvent, PublishContext,
    Scope, StoreRange,
};
use crate::message::{Message, MessagePart, MessageSource};
use crate::session::SessionRuntimeState;

use super::{RunAbortReason, RunAborted, RunId, RunStarted};
use crate::role::Role;

#[derive(Debug, Clone, FromQueryResult)]
struct ProjectedPartSummaryRow {
    part_id: i64,
    message_id: i64,
    part_index: i32,
    status: crate::message::ExecutionStatus,
    kind: crate::message::PartKind,
    name: Option<String>,
    summary: Option<String>,
    has_detail: bool,
    operation_id: Option<String>,
    created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ProjectedMessageHeader {
    pub id: i64,
    pub role: crate::role::Role,
    pub state: crate::message::ExecutionStatus,
    pub created_at: DateTime<Utc>,
    pub metadata: crate::message::MessageMetadata,
    pub usage: Option<crate::message::MessageUsage>,
    pub part_count: u64,
}

#[derive(Clone)]
pub(crate) struct SessionHistoryStore {
    publisher: Arc<EventPublisher>,
    db: DatabaseConnection,
}

impl SessionHistoryStore {
    pub(crate) fn new(publisher: Arc<EventPublisher>, db: DatabaseConnection) -> Self {
        Self { publisher, db }
    }

    pub(crate) async fn load_projection(
        &self,
        session_id: i64,
        base_runtime: SessionRuntimeState,
    ) -> Result<LoadedSessionProjection, DbErr> {
        self.ensure_projection_current(session_id).await?;
        let messages = self.read_projected_messages(session_id, true).await?;

        Ok(LoadedSessionProjection {
            messages,
            runtime: base_runtime,
        })
    }

    pub(crate) async fn list_projected_messages(
        &self,
        session_id: i64,
        include_full_parts: bool,
    ) -> Result<Vec<Message>, DbErr> {
        self.ensure_projection_current(session_id).await?;
        self.read_projected_messages(session_id, include_full_parts)
            .await
    }

    pub(crate) async fn list_projected_messages_page(
        &self,
        session_id: i64,
        include_full_parts: bool,
        cursor: Option<(i64, i64)>,
        limit: u64,
    ) -> Result<(Vec<Message>, bool, Option<(i64, i64)>), DbErr> {
        self.ensure_projection_current(session_id).await?;
        self.read_projected_messages_page(session_id, include_full_parts, cursor, limit)
            .await
    }

    pub(crate) async fn list_projected_message_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<ProjectedMessageHeader>, DbErr> {
        self.ensure_projection_current(session_id).await?;
        self.list_projected_message_rows(session_id)
            .await?
            .into_iter()
            .map(projected_message_header_from_row)
            .collect()
    }

    pub(crate) async fn list_projected_message_headers_page(
        &self,
        session_id: i64,
        cursor: Option<(i64, i64)>,
        limit: u64,
    ) -> Result<(Vec<ProjectedMessageHeader>, bool, Option<(i64, i64)>), DbErr> {
        self.ensure_projection_current(session_id).await?;
        let (message_rows, has_more, next_cursor) = self
            .list_projected_message_rows_page(session_id, cursor, limit)
            .await?;
        let messages = message_rows
            .into_iter()
            .map(projected_message_header_from_row)
            .collect::<Result<Vec<_>, DbErr>>()?;
        Ok((messages, has_more, next_cursor))
    }

    pub(crate) async fn find_projected_message(
        &self,
        session_id: i64,
        message_id: i64,
        include_full_parts: bool,
    ) -> Result<Option<Message>, DbErr> {
        self.ensure_projection_current(session_id).await?;
        self.read_projected_message(session_id, message_id, include_full_parts)
            .await
    }

    pub(crate) async fn find_projected_message_header(
        &self,
        session_id: i64,
        message_id: i64,
    ) -> Result<Option<ProjectedMessageHeader>, DbErr> {
        self.ensure_projection_current(session_id).await?;
        let row = activity_message::Entity::find_by_id(message_id)
            .filter(activity_message::Column::SessionId.eq(session_id))
            .filter(activity_message::Column::IsHidden.eq(false))
            .one(&self.db)
            .await?;
        row.map(projected_message_header_from_row).transpose()
    }

    pub(crate) async fn list_projected_parts(
        &self,
        message_id: i64,
        include_full_parts: bool,
    ) -> Result<Vec<MessagePart>, DbErr> {
        if include_full_parts {
            let rows = activity_part::Entity::find()
                .filter(activity_part::Column::MessageId.eq(message_id))
                .order_by_asc(activity_part::Column::PartIndex)
                .all(&self.db)
                .await?;
            rows.into_iter()
                .map(|row| {
                    Ok(MessagePart {
                        id: row.part_id,
                        message_id: row.message_id,
                        part_index: row.part_index,
                        status: row.status,
                        kind: row.kind,
                        name: row.name,
                        summary: row.summary,
                        has_detail: row.has_detail,
                        operation_id: row.operation_id,
                        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
                        content: row.content,
                    })
                })
                .collect()
        } else {
            let rows = activity_part::Entity::find()
                .select_only()
                .column(activity_part::Column::PartId)
                .column(activity_part::Column::MessageId)
                .column(activity_part::Column::PartIndex)
                .column(activity_part::Column::Status)
                .column(activity_part::Column::Kind)
                .column(activity_part::Column::Name)
                .column(activity_part::Column::Summary)
                .column(activity_part::Column::HasDetail)
                .column(activity_part::Column::OperationId)
                .column(activity_part::Column::CreatedAtMs)
                .filter(activity_part::Column::MessageId.eq(message_id))
                .order_by_asc(activity_part::Column::PartIndex)
                .into_model::<ProjectedPartSummaryRow>()
                .all(&self.db)
                .await?;
            rows.into_iter()
                .map(|row| {
                    Ok(MessagePart {
                        id: row.part_id,
                        message_id: row.message_id,
                        part_index: row.part_index,
                        status: row.status,
                        kind: row.kind,
                        name: row.name,
                        summary: row.summary,
                        has_detail: row.has_detail,
                        operation_id: row.operation_id,
                        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
                        content: None,
                    })
                })
                .collect()
        }
    }

    pub(crate) async fn find_projected_part(
        &self,
        part_id: i64,
    ) -> Result<Option<MessagePart>, DbErr> {
        let row = activity_part::Entity::find_by_id(part_id)
            .one(&self.db)
            .await?;
        row.map(|row| {
            Ok(MessagePart {
                id: row.part_id,
                message_id: row.message_id,
                part_index: row.part_index,
                status: row.status,
                kind: row.kind,
                name: row.name,
                summary: row.summary,
                has_detail: row.has_detail,
                operation_id: row.operation_id,
                created_at: timestamp_millis_to_utc(row.created_at_ms)?,
                content: row.content,
            })
        })
        .transpose()
    }

    pub(crate) async fn find_session_id_for_message(
        &self,
        message_id: i64,
    ) -> Result<Option<i64>, DbErr> {
        let row = activity_message::Entity::find_by_id(message_id)
            .one(&self.db)
            .await?;
        Ok(row.map(|row| row.session_id))
    }

    pub(crate) async fn find_session_id_for_part(
        &self,
        part_id: i64,
    ) -> Result<Option<i64>, DbErr> {
        let row = activity_part::Entity::find_by_id(part_id)
            .one(&self.db)
            .await?;
        Ok(row.map(|row| row.session_id))
    }

    async fn list_projected_message_rows(
        &self,
        session_id: i64,
    ) -> Result<Vec<activity_message::Model>, DbErr> {
        activity_message::Entity::find()
            .filter(activity_message::Column::SessionId.eq(session_id))
            .filter(activity_message::Column::IsHidden.eq(false))
            .order_by_asc(activity_message::Column::CreatedAtMs)
            .order_by_asc(activity_message::Column::MessageId)
            .all(&self.db)
            .await
    }

    async fn list_projected_message_rows_page(
        &self,
        session_id: i64,
        cursor: Option<(i64, i64)>,
        limit: u64,
    ) -> Result<(Vec<activity_message::Model>, bool, Option<(i64, i64)>), DbErr> {
        let limit = usize::try_from(limit)
            .map_err(|_| DbErr::Custom(format!("page limit too large: {limit}")))?;
        let fetch_limit = limit
            .checked_add(1)
            .ok_or_else(|| DbErr::Custom(format!("page limit overflow: {limit}")))?;

        let mut statement = activity_message::Entity::find()
            .filter(activity_message::Column::SessionId.eq(session_id))
            .filter(activity_message::Column::IsHidden.eq(false))
            .order_by_desc(activity_message::Column::CreatedAtMs)
            .order_by_desc(activity_message::Column::MessageId);

        if let Some((created_at_ms, message_id)) = cursor {
            statement = statement.filter(
                Condition::any()
                    .add(activity_message::Column::CreatedAtMs.lt(created_at_ms))
                    .add(
                        Condition::all()
                            .add(activity_message::Column::CreatedAtMs.eq(created_at_ms))
                            .add(activity_message::Column::MessageId.lt(message_id)),
                    ),
            );
        }

        let mut rows = statement
            .limit(
                u64::try_from(fetch_limit)
                    .map_err(|_| DbErr::Custom(format!("page limit too large: {fetch_limit}")))?,
            )
            .all(&self.db)
            .await?;
        let has_more = rows.len() > limit;
        if has_more {
            rows.truncate(limit);
        }
        let next_cursor = rows.last().map(|row| (row.created_at_ms, row.message_id));
        rows.reverse();
        Ok((rows, has_more, next_cursor))
    }

    async fn load_projected_parts_for_messages(
        &self,
        message_ids: &[i64],
        include_full_parts: bool,
    ) -> Result<std::collections::BTreeMap<i64, Vec<MessagePart>>, DbErr> {
        if message_ids.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }

        let part_rows = if include_full_parts {
            activity_part::Entity::find()
                .filter(activity_part::Column::MessageId.is_in(message_ids.iter().copied()))
                .order_by_asc(activity_part::Column::MessageId)
                .order_by_asc(activity_part::Column::PartIndex)
                .all(&self.db)
                .await?
                .into_iter()
                .map(|row| {
                    Ok(MessagePart {
                        id: row.part_id,
                        message_id: row.message_id,
                        part_index: row.part_index,
                        status: row.status,
                        kind: row.kind,
                        name: row.name,
                        summary: row.summary,
                        has_detail: row.has_detail,
                        operation_id: row.operation_id,
                        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
                        content: row.content,
                    })
                })
                .collect::<Result<Vec<_>, DbErr>>()?
        } else {
            activity_part::Entity::find()
                .select_only()
                .column(activity_part::Column::PartId)
                .column(activity_part::Column::MessageId)
                .column(activity_part::Column::PartIndex)
                .column(activity_part::Column::Status)
                .column(activity_part::Column::Kind)
                .column(activity_part::Column::Name)
                .column(activity_part::Column::Summary)
                .column(activity_part::Column::HasDetail)
                .column(activity_part::Column::OperationId)
                .column(activity_part::Column::CreatedAtMs)
                .filter(activity_part::Column::MessageId.is_in(message_ids.iter().copied()))
                .order_by_asc(activity_part::Column::MessageId)
                .order_by_asc(activity_part::Column::PartIndex)
                .into_model::<ProjectedPartSummaryRow>()
                .all(&self.db)
                .await?
                .into_iter()
                .map(|row| {
                    Ok(MessagePart {
                        id: row.part_id,
                        message_id: row.message_id,
                        part_index: row.part_index,
                        status: row.status,
                        kind: row.kind,
                        name: row.name,
                        summary: row.summary,
                        has_detail: row.has_detail,
                        operation_id: row.operation_id,
                        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
                        content: None,
                    })
                })
                .collect::<Result<Vec<_>, DbErr>>()?
        };

        let mut parts_by_message = std::collections::BTreeMap::<i64, Vec<MessagePart>>::new();
        for part in part_rows {
            parts_by_message
                .entry(part.message_id)
                .or_default()
                .push(part);
        }
        Ok(parts_by_message)
    }

    pub(crate) async fn list_session_events(
        &self,
        session_id: i64,
    ) -> Result<Vec<DomainEvent>, DbErr> {
        self.list_session_events_after(session_id, 0).await
    }

    async fn list_session_events_after(
        &self,
        session_id: i64,
        after_seq: i64,
    ) -> Result<Vec<DomainEvent>, DbErr> {
        let filter = EventFilter::new(Scope::Session { session_id });
        let mut all = Vec::new();
        let mut cursor = after_seq;
        loop {
            let chunk = self
                .publisher
                .store()
                .range(
                    &filter,
                    StoreRange {
                        after_seq_global: cursor,
                        limit: 1024,
                    },
                )
                .await
                .map_err(|err| DbErr::Custom(format!("event store range failed: {err}")))?;
            if chunk.is_empty() {
                break;
            }
            cursor = chunk.last().map(|e| e.meta.seq_global).unwrap_or(cursor);
            all.extend(chunk);
        }
        Ok(all)
    }

    /// Persist a synthetic `RunAborted { ProcessRestart }` for any
    /// `RunStarted` that lacks a matching `RunCompleted` / `RunAborted` in
    /// `events`. Returns the freshly published events so the caller can fold
    /// them into the view in one pass.
    async fn abort_hanging_runs(
        &self,
        session_id: i64,
        events: &[DomainEvent],
    ) -> Result<Vec<DomainEvent>, DbErr> {
        use std::collections::HashSet;
        let mut started: HashSet<RunId> = HashSet::new();
        for event in events {
            match &event.kind {
                EventKind::RunStarted(RunStarted { run_id, .. }) => {
                    started.insert(*run_id);
                }
                EventKind::RunCompleted(payload) => {
                    started.remove(&payload.run_id);
                }
                EventKind::RunAborted(payload) => {
                    started.remove(&payload.run_id);
                }
                _ => {}
            }
        }
        if started.is_empty() {
            return Ok(Vec::new());
        }
        let ctx = PublishContext::for_session(session_id);
        let pending: Vec<DomainEvent> = started
            .into_iter()
            .map(|run_id| {
                let kind = EventKind::RunAborted(RunAborted {
                    run_id,
                    reason: RunAbortReason::ProcessRestart,
                    message: Some("process restart detected on session load".to_string()),
                });
                self.publisher.build(ctx.clone(), kind)
            })
            .collect();
        self.publisher
            .publish_batch(pending)
            .await
            .map_err(|err| DbErr::Custom(format!("publish abort batch failed: {err}")))
    }

    /// Append a batch of events for a session through `publish_batch` so the
    /// store sees a single transactional append.
    pub(crate) async fn append_items(
        &self,
        session_id: i64,
        kinds: Vec<EventKind>,
        _now: DateTime<Utc>,
    ) -> Result<Vec<DomainEvent>, DbErr> {
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        let ctx = PublishContext::for_session(session_id);
        let built: Vec<DomainEvent> = kinds
            .into_iter()
            .map(|kind| self.publisher.build(ctx.clone(), kind))
            .collect();
        let built = self
            .publisher
            .publish_batch(built)
            .await
            .map_err(|err| DbErr::Custom(format!("publish history batch failed: {err}")))?;
        self.apply_projection_events(session_id, built.as_slice())
            .await?;
        Ok(built)
    }

    /// Persist a batch of events for a session **without** broadcasting them
    /// on the in-process bus. Used by replay-only flows (fork copy, import
    /// replay) so subscribers don't observe historical reconstructions as
    /// fresh activity.
    pub(crate) async fn append_items_silent(
        &self,
        session_id: i64,
        kinds: Vec<EventKind>,
    ) -> Result<Vec<DomainEvent>, DbErr> {
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        let ctx = PublishContext::for_session(session_id);
        let built: Vec<DomainEvent> = kinds
            .into_iter()
            .map(|kind| self.publisher.build(ctx.clone(), kind))
            .collect();
        let built = self
            .publisher
            .append_batch_silent(built)
            .await
            .map_err(|err| DbErr::Custom(format!("append silent history batch failed: {err}")))?;
        self.apply_projection_events(session_id, built.as_slice())
            .await?;
        Ok(built)
    }

    async fn ensure_projection_current(&self, session_id: i64) -> Result<i64, DbErr> {
        let filter = EventFilter::new(Scope::Session { session_id });
        let projected_seq = self
            .load_projection_state(session_id)
            .await?
            .map(|row| row.last_seq_global)
            .unwrap_or(0);
        let stale = !self
            .publisher
            .store()
            .range(
                &filter,
                StoreRange {
                    after_seq_global: projected_seq,
                    limit: 1,
                },
            )
            .await
            .map_err(|err| DbErr::Custom(format!("event store range failed: {err}")))?
            .is_empty();
        if stale {
            self.rebuild_projection_from_history(session_id).await?;
        }
        Ok(self
            .load_projection_state(session_id)
            .await?
            .map(|row| row.last_seq_global)
            .unwrap_or(projected_seq))
    }

    async fn rebuild_projection_from_history(&self, session_id: i64) -> Result<(), DbErr> {
        let mut events = self.list_session_events(session_id).await?;
        let aborted = self.abort_hanging_runs(session_id, &events).await?;
        if !aborted.is_empty() {
            events.extend(aborted);
        }

        let txn = self.db.begin().await?;
        clear_projection_for_session(&txn, session_id).await?;
        apply_projection_events_on_connection(&txn, session_id, events.as_slice()).await?;
        txn.commit().await?;
        Ok(())
    }

    async fn apply_projection_events(
        &self,
        session_id: i64,
        events: &[DomainEvent],
    ) -> Result<(), DbErr> {
        apply_projection_events_on_connection(&self.db, session_id, events).await
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LoadedSessionProjection {
    pub messages: Vec<Message>,
    pub runtime: SessionRuntimeState,
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<chrono::DateTime<Utc>, DbErr> {
    chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| DbErr::Custom(format!("timestamp out of range: {timestamp_ms}")))
}

fn projected_message_from_row(
    row: activity_message::Model,
    parts: Vec<MessagePart>,
) -> Result<Message, DbErr> {
    Ok(Message {
        id: row.message_id,
        role: row.role,
        state: row.state,
        parts,
        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
        metadata: row.metadata,
        provider_state: row.provider_state,
        usage: row.usage,
    })
}

fn projected_message_header_from_row(
    row: activity_message::Model,
) -> Result<ProjectedMessageHeader, DbErr> {
    let part_count = u64::try_from(row.part_count)
        .map_err(|_| DbErr::Custom(format!("negative projected part count: {}", row.part_count)))?;
    Ok(ProjectedMessageHeader {
        id: row.message_id,
        role: row.role,
        state: row.state,
        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
        metadata: row.metadata,
        usage: row.usage,
        part_count,
    })
}

fn projected_messages_needing_part_repair(
    rows: &[activity_message::Model],
    parts_by_message: &std::collections::BTreeMap<i64, Vec<MessagePart>>,
) -> Vec<i64> {
    rows.iter()
        .filter(|row| {
            projected_message_needs_part_repair(
                row,
                parts_by_message
                    .get(&row.message_id)
                    .map_or(0, std::vec::Vec::len),
            )
        })
        .map(|row| row.message_id)
        .collect()
}

fn projected_message_needs_part_repair(
    row: &activity_message::Model,
    loaded_part_count: usize,
) -> bool {
    let expected_part_count = usize::try_from(row.part_count).unwrap_or_default();
    loaded_part_count < expected_part_count
        || (loaded_part_count == 0 && matches!(row.role, Role::Assistant | Role::User))
}

fn with_source_if_missing(
    mut metadata: crate::message::MessageMetadata,
    source: MessageSource,
) -> crate::message::MessageMetadata {
    if metadata.source != MessageSource::System {
        metadata.source = source;
    }
    metadata
}

async fn upsert_message_projection<C>(db: &C, row: activity_message::Model) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let metadata = serde_json::to_value(&row.metadata)
        .map_err(|err| DbErr::Custom(format!("serialize message metadata: {err}")))?;
    let metadata = sea_orm::Value::Json(Some(Box::new(metadata)));
    let usage = row
        .usage
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|err| DbErr::Custom(format!("serialize message usage: {err}")))?
        .map(Box::new);
    let usage = sea_orm::Value::Json(usage);
    let stmt = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO agena_activity_messages \
         (message_id, session_id, role, state, created_at_ms, updated_at_ms, metadata, provider_state, usage, part_count, is_hidden) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(message_id) DO UPDATE SET \
         session_id = excluded.session_id, role = excluded.role, state = excluded.state, \
         created_at_ms = excluded.created_at_ms, updated_at_ms = excluded.updated_at_ms, \
         metadata = excluded.metadata, provider_state = excluded.provider_state, usage = excluded.usage, \
         part_count = excluded.part_count, is_hidden = excluded.is_hidden",
        [
            row.message_id.into(),
            row.session_id.into(),
            role_db_value(row.role).into(),
            execution_status_db_value(row.state).into(),
            row.created_at_ms.into(),
            row.updated_at_ms.into(),
            metadata,
            sea_orm::Value::Json(
                row.provider_state
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(|err| DbErr::Custom(format!("serialize provider state: {err}")))?
                    .map(Box::new),
            ),
            usage,
            row.part_count.into(),
            row.is_hidden.into(),
        ],
    );
    db.execute(stmt).await?;
    Ok(())
}

fn role_db_value(role: Role) -> i8 {
    match role {
        Role::User => 1,
        Role::Assistant => 2,
        Role::System => 3,
        Role::Tool => 4,
    }
}

fn execution_status_db_value(status: crate::message::ExecutionStatus) -> i8 {
    match status {
        crate::message::ExecutionStatus::Pending => 1,
        crate::message::ExecutionStatus::InProgress => 2,
        crate::message::ExecutionStatus::Completed => 3,
        crate::message::ExecutionStatus::Failed => 4,
        crate::message::ExecutionStatus::Cancelled => 5,
    }
}

async fn upsert_part_projection<C>(db: &C, session_id: i64, part: &MessagePart) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    if let Some(operation_id) = part.operation_id.as_deref() {
        let existing = activity_part::Entity::find()
            .select_only()
            .column(activity_part::Column::PartId)
            .filter(activity_part::Column::MessageId.eq(part.message_id))
            .filter(activity_part::Column::OperationId.eq(operation_id))
            .filter(activity_part::Column::Kind.eq(part.kind))
            .into_tuple::<i64>()
            .all(db)
            .await?;
        for part_id in existing {
            if part_id != part.id {
                activity_part::Entity::delete_by_id(part_id)
                    .exec(db)
                    .await?;
            }
        }
    }

    activity_part::Entity::insert(activity_part::ActiveModel {
        part_id: ActiveValue::Set(part.id),
        message_id: ActiveValue::Set(part.message_id),
        session_id: ActiveValue::Set(session_id),
        part_index: ActiveValue::Set(part.part_index),
        status: ActiveValue::Set(part.status),
        kind: ActiveValue::Set(part.kind),
        name: ActiveValue::Set(part.name.clone()),
        summary: ActiveValue::Set(part.summary.clone()),
        has_detail: ActiveValue::Set(part.has_detail),
        operation_id: ActiveValue::Set(part.operation_id.clone()),
        created_at_ms: ActiveValue::Set(part.created_at.timestamp_millis()),
        content: ActiveValue::Set(part.content.clone()),
    })
    .on_conflict(
        OnConflict::column(activity_part::Column::PartId)
            .update_columns([
                activity_part::Column::MessageId,
                activity_part::Column::SessionId,
                activity_part::Column::PartIndex,
                activity_part::Column::Status,
                activity_part::Column::Kind,
                activity_part::Column::Name,
                activity_part::Column::Summary,
                activity_part::Column::HasDetail,
                activity_part::Column::OperationId,
                activity_part::Column::CreatedAtMs,
                activity_part::Column::Content,
            ])
            .to_owned(),
    )
    .exec(db)
    .await?;
    Ok(())
}

async fn count_parts_for_message<C>(db: &C, message_id: i64) -> Result<u64, DbErr>
where
    C: ConnectionTrait,
{
    activity_part::Entity::find()
        .filter(activity_part::Column::MessageId.eq(message_id))
        .count(db)
        .await
}

async fn touch_message_projection<C>(
    db: &C,
    message_id: i64,
    updated_at_ms: i64,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    if let Some(message) = activity_message::Entity::find_by_id(message_id)
        .one(db)
        .await?
    {
        let part_count = count_parts_for_message(db, message_id).await? as i64;
        let mut active: activity_message::ActiveModel = message.into();
        active.updated_at_ms = ActiveValue::Set(updated_at_ms);
        active.part_count = ActiveValue::Set(part_count);
        active.update(db).await?;
    }
    Ok(())
}

fn project_system_notice_part(payload: &super::SystemNoticeAppended) -> MessagePart {
    let mut part = MessagePart::with_content(
        payload.message_id.raw(),
        payload.message_id.raw(),
        payload.created_at,
        crate::message::ExecutionStatus::Completed,
        crate::message::PartContent::text(payload.text.clone()),
    );
    part.part_index = 0;
    part
}

async fn project_tool_call_issued<C>(
    db: &C,
    session_id: i64,
    payload: &super::ToolCallIssued,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    if activity_message::Entity::find_by_id(payload.message_id.raw())
        .one(db)
        .await?
        .is_none()
    {
        return Ok(());
    }

    let part_id = synthetic_tool_call_part_id(payload.message_id.raw(), payload.call_id.as_str());
    let part_index = count_parts_for_message(db, payload.message_id.raw()).await? as i32;
    let invocation = crate::message::ToolInvocation {
        name: payload.name.to_string(),
        plugin_name: None,
        input: crate::message::StructuredObject::try_from(payload.arguments.clone())
            .unwrap_or_default(),
    };
    let mut part = MessagePart::with_content(
        part_id,
        payload.message_id.raw(),
        payload.created_at,
        crate::message::ExecutionStatus::Pending,
        crate::message::PartContent::Operation(crate::message::OperationPart::pending(
            0,
            invocation,
            payload.name.to_string(),
            crate::message::TimeRange::default(),
        )),
    );
    part.part_index = part_index;
    part.operation_id = Some(payload.call_id.as_str().to_owned());

    upsert_part_projection(db, session_id, &part).await?;
    touch_message_projection(
        db,
        payload.message_id.raw(),
        payload.created_at.timestamp_millis(),
    )
    .await
}

async fn update_tool_result_projection<C>(
    db: &C,
    session_id: i64,
    payload: &super::ToolCallCompleted,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    if let Some(part) = payload.part.as_ref() {
        let mut authoritative_part = part.clone();
        authoritative_part.message_id = payload.message_id.raw();
        upsert_part_projection(db, session_id, &authoritative_part).await?;
        delete_duplicate_operation_parts(
            db,
            payload.message_id.raw(),
            payload.call_id.as_str(),
            authoritative_part.id,
        )
        .await?;
        touch_message_projection(
            db,
            payload.message_id.raw(),
            payload.completed_at.timestamp_millis(),
        )
        .await?;
        return Ok(());
    }

    let existing = activity_part::Entity::find()
        .filter(activity_part::Column::MessageId.eq(payload.message_id.raw()))
        .filter(activity_part::Column::OperationId.eq(payload.call_id.as_str()))
        .one(db)
        .await?;

    let output_text = match &payload.output {
        super::transcript::TranscriptToolOutput::Text { text } => text.clone(),
        super::transcript::TranscriptToolOutput::Error { message } => message.clone(),
    };
    let (part_id, part_index, call_id, invocation, mut lifecycle) = match existing {
        Some(existing) => {
            let (call_id, invocation, lifecycle) = match existing.content.as_ref() {
                Some(crate::message::PartContent::Operation(operation)) => (
                    operation.call_id,
                    operation.invocation.clone(),
                    operation.lifecycle.clone(),
                ),
                _ => (
                    0,
                    crate::message::ToolInvocation::new(
                        payload.tool_name.as_str().to_owned(),
                        crate::message::StructuredObject::default(),
                    ),
                    crate::message::TimeRange::default(),
                ),
            };
            (
                existing.part_id,
                existing.part_index,
                call_id,
                invocation,
                lifecycle,
            )
        }
        None => (
            synthetic_tool_call_part_id(payload.message_id.raw(), payload.call_id.as_str()),
            count_parts_for_message(db, payload.message_id.raw()).await? as i32,
            0,
            crate::message::ToolInvocation::new(
                payload.tool_name.as_str().to_owned(),
                crate::message::StructuredObject::default(),
            ),
            crate::message::TimeRange::default(),
        ),
    };
    if lifecycle.end_ms.is_none() {
        lifecycle.end_ms = Some(payload.completed_at.timestamp_millis());
    }

    let status = match &payload.output {
        super::transcript::TranscriptToolOutput::Error { .. } => {
            crate::message::ExecutionStatus::Failed
        }
        super::transcript::TranscriptToolOutput::Text { .. } => {
            crate::message::ExecutionStatus::Completed
        }
    };
    let content = match &payload.output {
        super::transcript::TranscriptToolOutput::Error { message } => {
            crate::message::PartContent::Operation(crate::message::OperationPart::failed(
                call_id,
                invocation,
                message.clone(),
                output_text,
                Vec::new(),
                Vec::new(),
                crate::message::ToolOutput::default(),
                lifecycle,
            ))
        }
        super::transcript::TranscriptToolOutput::Text { .. } => {
            crate::message::PartContent::Operation(crate::message::OperationPart::completed(
                call_id,
                invocation,
                output_text,
                Vec::new(),
                Vec::new(),
                crate::message::ToolOutput::default(),
                lifecycle,
            ))
        }
    };

    let mut part = MessagePart::with_content(
        part_id,
        payload.message_id.raw(),
        payload.completed_at,
        status,
        content,
    );
    part.part_index = part_index;
    part.operation_id = Some(payload.call_id.as_str().to_owned());
    upsert_part_projection(db, session_id, &part).await?;

    touch_message_projection(
        db,
        payload.message_id.raw(),
        payload.completed_at.timestamp_millis(),
    )
    .await
}

async fn delete_duplicate_operation_parts<C>(
    db: &C,
    message_id: i64,
    operation_id: &str,
    keep_part_id: i64,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let duplicates = activity_part::Entity::find()
        .filter(activity_part::Column::MessageId.eq(message_id))
        .filter(activity_part::Column::OperationId.eq(operation_id))
        .all(db)
        .await?;

    for duplicate in duplicates {
        if duplicate.part_id == keep_part_id
            || duplicate.kind != crate::message::PartKind::Operation
        {
            continue;
        }
        activity_part::Entity::delete_by_id(duplicate.part_id)
            .exec(db)
            .await?;
    }

    Ok(())
}

fn synthetic_tool_call_part_id(message_id: i64, call_id: &str) -> i64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in message_id.to_le_bytes().iter().chain(call_id.as_bytes()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    -((hash & 0x0000_3fff_ffff_ffff) as i64) - 1_000_000
}

impl SessionHistoryStore {
    async fn read_projected_messages(
        &self,
        session_id: i64,
        include_full_parts: bool,
    ) -> Result<Vec<Message>, DbErr> {
        let mut message_rows = self.list_projected_message_rows(session_id).await?;

        if message_rows.is_empty() {
            return Ok(Vec::new());
        }

        let message_ids = message_rows
            .iter()
            .map(|row| row.message_id)
            .collect::<Vec<_>>();
        let mut parts_by_message = self
            .load_projected_parts_for_messages(message_ids.as_slice(), include_full_parts)
            .await?;
        if projected_messages_needing_part_repair(message_rows.as_slice(), &parts_by_message)
            .is_empty()
        {
            return message_rows
                .into_iter()
                .map(|row| {
                    let message_id = row.message_id;
                    projected_message_from_row(
                        row,
                        parts_by_message.remove(&message_id).unwrap_or_default(),
                    )
                })
                .collect();
        }

        self.rebuild_projection_from_history(session_id).await?;
        message_rows = self.list_projected_message_rows(session_id).await?;
        let message_ids = message_rows
            .iter()
            .map(|row| row.message_id)
            .collect::<Vec<_>>();
        parts_by_message = self
            .load_projected_parts_for_messages(message_ids.as_slice(), include_full_parts)
            .await?;
        message_rows
            .into_iter()
            .map(|row| {
                let message_id = row.message_id;
                projected_message_from_row(
                    row,
                    parts_by_message.remove(&message_id).unwrap_or_default(),
                )
            })
            .collect()
    }

    async fn read_projected_messages_page(
        &self,
        session_id: i64,
        include_full_parts: bool,
        cursor: Option<(i64, i64)>,
        limit: u64,
    ) -> Result<(Vec<Message>, bool, Option<(i64, i64)>), DbErr> {
        let (mut message_rows, mut has_more, mut next_cursor) = self
            .list_projected_message_rows_page(session_id, cursor, limit)
            .await?;

        if message_rows.is_empty() {
            return Ok((Vec::new(), has_more, next_cursor));
        }

        let message_ids = message_rows
            .iter()
            .map(|row| row.message_id)
            .collect::<Vec<_>>();
        let mut parts_by_message = self
            .load_projected_parts_for_messages(message_ids.as_slice(), include_full_parts)
            .await?;
        if !projected_messages_needing_part_repair(message_rows.as_slice(), &parts_by_message)
            .is_empty()
        {
            self.rebuild_projection_from_history(session_id).await?;
            (message_rows, has_more, next_cursor) = self
                .list_projected_message_rows_page(session_id, cursor, limit)
                .await?;
            if message_rows.is_empty() {
                return Ok((Vec::new(), has_more, next_cursor));
            }
            let message_ids = message_rows
                .iter()
                .map(|row| row.message_id)
                .collect::<Vec<_>>();
            parts_by_message = self
                .load_projected_parts_for_messages(message_ids.as_slice(), include_full_parts)
                .await?;
        }

        let messages = message_rows
            .into_iter()
            .map(|row| {
                let message_id = row.message_id;
                projected_message_from_row(
                    row,
                    parts_by_message.remove(&message_id).unwrap_or_default(),
                )
            })
            .collect::<Result<Vec<_>, DbErr>>()?;

        Ok((messages, has_more, next_cursor))
    }

    async fn read_projected_message(
        &self,
        session_id: i64,
        message_id: i64,
        include_full_parts: bool,
    ) -> Result<Option<Message>, DbErr> {
        let row = activity_message::Entity::find_by_id(message_id)
            .filter(activity_message::Column::SessionId.eq(session_id))
            .filter(activity_message::Column::IsHidden.eq(false))
            .one(&self.db)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let mut parts = self
            .list_projected_parts(message_id, include_full_parts)
            .await?;
        if projected_message_needs_part_repair(&row, parts.len()) {
            self.rebuild_projection_from_history(session_id).await?;
            let row = activity_message::Entity::find_by_id(message_id)
                .filter(activity_message::Column::SessionId.eq(session_id))
                .filter(activity_message::Column::IsHidden.eq(false))
                .one(&self.db)
                .await?;
            let Some(row) = row else {
                return Ok(None);
            };
            parts = self
                .list_projected_parts(message_id, include_full_parts)
                .await?;
            return Ok(Some(projected_message_from_row(row, parts)?));
        }
        Ok(Some(projected_message_from_row(row, parts)?))
    }

    async fn load_projection_state(
        &self,
        session_id: i64,
    ) -> Result<Option<activity_projection_state::Model>, DbErr> {
        activity_projection_state::Entity::find_by_id(session_id)
            .one(&self.db)
            .await
    }
}

async fn apply_projection_events_on_connection<C>(
    db: &C,
    session_id: i64,
    events: &[DomainEvent],
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    for event in events {
        match &event.kind {
            EventKind::UserMessageAppended(payload) => {
                let metadata =
                    with_source_if_missing(payload.metadata.clone(), MessageSource::User);
                upsert_message_projection(
                    db,
                    activity_message::Model {
                        message_id: payload.message_id.raw(),
                        session_id,
                        role: Role::User,
                        state: crate::message::ExecutionStatus::Completed,
                        created_at_ms: payload.created_at.timestamp_millis(),
                        updated_at_ms: payload.created_at.timestamp_millis(),
                        metadata,
                        provider_state: payload.provider_state.clone(),
                        usage: None,
                        part_count: payload.parts.len() as i64,
                        is_hidden: false,
                    },
                )
                .await
                .map_err(|err| {
                    DbErr::Custom(format!(
                        "project user message {}: {err}",
                        payload.message_id.raw()
                    ))
                })?;
                for part in &payload.parts {
                    upsert_part_projection(db, session_id, part)
                        .await
                        .map_err(|err| {
                            DbErr::Custom(format!(
                                "project user part {} for message {}: {err}",
                                part.id, part.message_id
                            ))
                        })?;
                }
            }
            EventKind::AssistantMessageCompleted(payload) => {
                let metadata =
                    with_source_if_missing(payload.metadata.clone(), MessageSource::Assistant);
                upsert_message_projection(
                    db,
                    activity_message::Model {
                        message_id: payload.message_id.raw(),
                        session_id,
                        role: Role::Assistant,
                        state: crate::message::ExecutionStatus::Completed,
                        created_at_ms: payload.created_at.timestamp_millis(),
                        updated_at_ms: payload.created_at.timestamp_millis(),
                        metadata,
                        provider_state: payload.provider_state.clone(),
                        usage: payload.usage.clone(),
                        part_count: payload.parts.len() as i64,
                        is_hidden: false,
                    },
                )
                .await
                .map_err(|err| {
                    DbErr::Custom(format!(
                        "project assistant message {}: {err}",
                        payload.message_id.raw()
                    ))
                })?;
                for part in &payload.parts {
                    upsert_part_projection(db, session_id, part)
                        .await
                        .map_err(|err| {
                            DbErr::Custom(format!(
                                "project assistant part {} for message {}: {err}",
                                part.id, part.message_id
                            ))
                        })?;
                }
            }
            EventKind::ToolCallIssued(payload) => {
                project_tool_call_issued(db, session_id, payload)
                    .await
                    .map_err(|err| {
                        DbErr::Custom(format!(
                            "project tool call for message {} call {}: {err}",
                            payload.message_id.raw(),
                            payload.call_id
                        ))
                    })?;
            }
            EventKind::ToolCallCompleted(payload) => {
                update_tool_result_projection(db, session_id, payload)
                    .await
                    .map_err(|err| {
                        DbErr::Custom(format!(
                            "project tool result for message {} call {}: {err}",
                            payload.message_id.raw(),
                            payload.call_id
                        ))
                    })?;
            }
            EventKind::SystemNoticeAppended(payload) => {
                let synthetic_part = project_system_notice_part(payload);
                upsert_message_projection(
                    db,
                    activity_message::Model {
                        message_id: payload.message_id.raw(),
                        session_id,
                        role: Role::System,
                        state: crate::message::ExecutionStatus::Completed,
                        created_at_ms: payload.created_at.timestamp_millis(),
                        updated_at_ms: payload.created_at.timestamp_millis(),
                        metadata: Default::default(),
                        provider_state: None,
                        usage: None,
                        part_count: 1,
                        is_hidden: false,
                    },
                )
                .await
                .map_err(|err| {
                    DbErr::Custom(format!(
                        "project system notice {}: {err}",
                        payload.message_id.raw()
                    ))
                })?;
                upsert_part_projection(db, session_id, &synthetic_part)
                    .await
                    .map_err(|err| {
                        DbErr::Custom(format!(
                            "project system notice part {} for message {}: {err}",
                            synthetic_part.id, synthetic_part.message_id
                        ))
                    })?;
            }
            EventKind::MessagePartUpdated(update) => {
                apply_message_part_update_on_connection(db, update)
                    .await
                    .map_err(|err| {
                        DbErr::Custom(format!(
                            "project part update {} for message {}: {err}",
                            update.part.id, update.message_id
                        ))
                    })?;
            }
            _ => {}
        }
    }

    if let Some(last_seq_global) = events.iter().map(|event| event.meta.seq_global).max() {
        upsert_projection_state(db, session_id, last_seq_global).await?;
    }

    Ok(())
}

async fn apply_message_part_update_on_connection<C>(
    db: &C,
    update: &MessagePartUpdatedEvent,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let message_row = match activity_message::Entity::find_by_id(update.message_id)
        .one(db)
        .await?
    {
        Some(row) => row,
        None => {
            let row = activity_message::Model {
                message_id: update.message_id,
                session_id: update.session_id,
                role: update.message_role,
                state: update.message_state,
                created_at_ms: update.message_created_at.timestamp_millis(),
                updated_at_ms: update.ts_ms,
                metadata: with_source_if_missing(
                    crate::message::MessageMetadata::default(),
                    role_default_source(update.message_role),
                ),
                provider_state: None,
                usage: None,
                part_count: 0,
                is_hidden: false,
            };
            upsert_message_projection(db, row.clone()).await?;
            row
        }
    };

    upsert_part_projection(db, update.session_id, &update.part).await?;

    let mut active: activity_message::ActiveModel = message_row.into();
    active.state = ActiveValue::Set(update.message_state);
    active.updated_at_ms = ActiveValue::Set(update.ts_ms);
    active.part_count =
        ActiveValue::Set(count_parts_for_message(db, update.message_id).await? as i64);
    active.update(db).await?;
    Ok(())
}

fn role_default_source(role: Role) -> MessageSource {
    match role {
        Role::User => MessageSource::User,
        Role::Assistant => MessageSource::Assistant,
        Role::System => MessageSource::System,
        Role::Tool => MessageSource::Assistant,
    }
}

async fn clear_projection_for_session<C>(db: &C, session_id: i64) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    activity_part::Entity::delete_many()
        .filter(activity_part::Column::SessionId.eq(session_id))
        .exec(db)
        .await?;
    activity_message::Entity::delete_many()
        .filter(activity_message::Column::SessionId.eq(session_id))
        .exec(db)
        .await?;
    activity_projection_state::Entity::delete_by_id(session_id)
        .exec(db)
        .await?;
    Ok(())
}

async fn upsert_projection_state<C>(
    db: &C,
    session_id: i64,
    last_seq_global: i64,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let updated_at_ms = Utc::now().timestamp_millis();
    let stmt = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO agena_activity_projection_states \
         (session_id, last_seq_global, updated_at_ms) \
         VALUES (?, ?, ?) \
         ON CONFLICT(session_id) DO UPDATE SET \
         last_seq_global = excluded.last_seq_global, \
         updated_at_ms = excluded.updated_at_ms",
        [
            session_id.into(),
            last_seq_global.into(),
            updated_at_ms.into(),
        ],
    );
    db.execute(stmt).await?;
    Ok(())
}
