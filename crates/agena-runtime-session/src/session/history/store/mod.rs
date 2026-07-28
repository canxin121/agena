use std::sync::Arc;

use chrono::{DateTime, Utc};
#[cfg(test)]
use sea_orm::{
    ActiveModelTrait, ActiveValue, QuerySelect,
    sea_query::{Expr, OnConflict},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    PaginatorTrait, QueryFilter, TransactionTrait,
};

use crate::{
    db::entities::{activity_message, activity_part, activity_projection_state},
    event::{DomainEvent, EventKind, EventPublisher, MessagePartCheckpointedEvent, PublishContext},
    message::{ActivityKind, ActivityPart, Message, MessageMetadata, MessagePart, PartContent},
    session::SessionRuntimeState,
};
use agena_storage::{
    MessageProjectionHeaderRecord, MessageProjectionPartRecord, MessageProjectionPartWrite,
    MessageProjectionRepository, MessageProjectionTransactionWriter, StoreRange,
};
#[cfg(test)]
use agena_storage_sqlite::StoredRole;
use agena_storage_sqlite::{StoredExecutionStatus, StoredPartKind};

use super::{RunAborted, RunId, RunStarted};
use agena_domain::{
    EventFilter, EventScope, ExecutionFailureKind, ExecutionFinishedEvent, ExecutionOutcome,
    ExecutionStartedEvent, ExecutionStatus, MessageSource, PromptCompactionCompletedEvent, Role,
    RunAbortReason,
};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub struct ProjectedMessageHeader {
    pub id: i64,
    pub role: Role,
    pub state: ExecutionStatus,
    pub created_at: DateTime<Utc>,
    pub metadata: crate::message::MessageMetadata,
    pub usage: Option<agena_provider::CompletionUsage>,
    pub part_count: u64,
}

#[derive(Clone)]
pub(crate) struct SessionHistoryStore {
    publisher: Arc<EventPublisher>,
    db: DatabaseConnection,
    message_projection_repository: Arc<dyn MessageProjectionRepository>,
    message_projection_transaction_writer:
        Arc<dyn MessageProjectionTransactionWriter<DatabaseTransaction>>,
}

impl SessionHistoryStore {
    pub(crate) fn new(
        publisher: Arc<EventPublisher>,
        db: DatabaseConnection,
        message_projection_repository: Arc<dyn MessageProjectionRepository>,
        message_projection_transaction_writer: Arc<
            dyn MessageProjectionTransactionWriter<DatabaseTransaction>,
        >,
    ) -> Self {
        Self {
            publisher,
            db,
            message_projection_repository,
            message_projection_transaction_writer,
        }
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

    pub(crate) async fn reconcile_interrupted_lifecycles(
        &self,
        session_id: i64,
    ) -> Result<(), DbErr> {
        let events = self.list_session_events(session_id).await?;
        self.abort_hanging_lifecycles(session_id, events.as_slice())
            .await?;
        self.ensure_projection_current(session_id).await?;
        Ok(())
    }

    pub(crate) async fn reconcile_unmatched_runs(
        &self,
        session_id: i64,
        reason: RunAbortReason,
        message: String,
    ) -> Result<(), DbErr> {
        let events = self.list_session_events(session_id).await?;
        let (started_runs, _) = unmatched_lifecycles(events.as_slice());
        if started_runs.is_empty() {
            return Ok(());
        }
        let ctx = PublishContext::for_session(session_id);
        let mut pending = Vec::with_capacity(started_runs.len());
        for run_id in started_runs {
            pending.push(
                self.publisher
                    .build(
                        ctx.clone(),
                        EventKind::RunAborted(RunAborted {
                            run_id,
                            reason,
                            message: Some(message.clone()),
                        }),
                    )
                    .await
                    .map_err(|error| {
                        DbErr::Custom(format!("build run reconciliation event failed: {error}"))
                    })?,
            );
        }
        self.publisher
            .publish_batch(pending)
            .await
            .map_err(|error| {
                DbErr::Custom(format!("publish run reconciliation failed: {error}"))
            })?;
        Ok(())
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

    pub(crate) async fn list_projected_message_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<ProjectedMessageHeader>, DbErr> {
        self.ensure_projection_current(session_id).await?;
        self.message_projection_repository
            .list_headers(session_id)
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))?
            .into_iter()
            .map(projected_message_header_from_record)
            .collect()
    }

    async fn load_projected_parts_for_messages(
        &self,
        message_ids: &[i64],
        include_full_parts: bool,
    ) -> Result<std::collections::BTreeMap<i64, Vec<MessagePart>>, DbErr> {
        if message_ids.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }

        let part_rows = self
            .message_projection_repository
            .list_parts(message_ids, include_full_parts)
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))?
            .into_iter()
            .map(projected_part_from_record)
            .collect::<Result<Vec<_>, DbErr>>()?;

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
        let filter = EventFilter::new(EventScope::Session { session_id });
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
    async fn abort_hanging_lifecycles(
        &self,
        session_id: i64,
        events: &[DomainEvent],
    ) -> Result<Vec<DomainEvent>, DbErr> {
        let (started_runs, started_executions) = unmatched_lifecycles(events);
        if started_runs.is_empty() && started_executions.is_empty() {
            return Ok(Vec::new());
        }
        let ctx = PublishContext::for_session(session_id);
        let mut pending: Vec<DomainEvent> =
            Vec::with_capacity(started_runs.len() + started_executions.len());
        for run_id in started_runs {
            let kind = EventKind::RunAborted(RunAborted {
                run_id,
                reason: RunAbortReason::ProcessRestart,
                message: Some("process restart detected on session load".to_string()),
            });
            pending.push(
                self.publisher
                    .build(ctx.clone(), kind)
                    .await
                    .map_err(|err| DbErr::Custom(format!("build abort event failed: {err}")))?,
            );
        }
        for execution_id in started_executions {
            let kind = EventKind::ExecutionFinished(ExecutionFinishedEvent {
                session_id,
                execution_id,
                outcome: ExecutionOutcome::Failed {
                    failure_kind: ExecutionFailureKind::ProcessRestart,
                    message: "process restart interrupted execution".to_string(),
                },
                ts_ms: Utc::now().timestamp_millis(),
            });
            pending.push(
                self.publisher
                    .build(ctx.clone(), kind)
                    .await
                    .map_err(|err| {
                        DbErr::Custom(format!("build execution recovery event failed: {err}"))
                    })?,
            );
        }
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
        let mut built: Vec<DomainEvent> = Vec::with_capacity(kinds.len());
        for kind in kinds {
            built.push(
                self.publisher
                    .build(ctx.clone(), kind)
                    .await
                    .map_err(|err| DbErr::Custom(format!("build history event failed: {err}")))?,
            );
        }
        let built = self
            .publisher
            .append_batch_silent(built)
            .await
            .map_err(|err| DbErr::Custom(format!("persist history batch failed: {err}")))?;
        self.ensure_projection_current(session_id).await?;
        for event in &built {
            self.publisher
                .bus()
                .publish(event.clone())
                .await
                .map_err(|error| {
                    DbErr::Custom(format!("broadcast projected history event failed: {error}"))
                })?;
        }
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
        let mut built: Vec<DomainEvent> = Vec::with_capacity(kinds.len());
        for kind in kinds {
            built.push(
                self.publisher
                    .build(ctx.clone(), kind)
                    .await
                    .map_err(|err| DbErr::Custom(format!("build history event failed: {err}")))?,
            );
        }
        let built = self
            .publisher
            .append_batch_silent(built)
            .await
            .map_err(|err| DbErr::Custom(format!("append silent history batch failed: {err}")))?;
        self.ensure_projection_current(session_id).await?;
        Ok(built)
    }

    async fn ensure_projection_current(&self, session_id: i64) -> Result<i64, DbErr> {
        let projected_seq = self
            .load_projection_state(session_id)
            .await?
            .map(|row| row.last_seq_global)
            .unwrap_or(0);
        let pending = self
            .list_session_events_after(session_id, projected_seq)
            .await?;
        if !pending.is_empty() {
            let txn = self.db.begin().await?;
            let part_writer = TransactionProjectionPartWriter::new(Arc::clone(
                &self.message_projection_transaction_writer,
            ));
            apply_projection_events_on_connection(
                &txn,
                &part_writer,
                session_id,
                pending.as_slice(),
            )
            .await?;
            txn.commit().await?;
        }
        Ok(self
            .load_projection_state(session_id)
            .await?
            .map(|row| row.last_seq_global)
            .unwrap_or(projected_seq))
    }

    async fn rebuild_projection_from_history(&self, session_id: i64) -> Result<(), DbErr> {
        let events = self.list_session_events(session_id).await?;

        let txn = self.db.begin().await?;
        let part_writer = TransactionProjectionPartWriter::new(Arc::clone(
            &self.message_projection_transaction_writer,
        ));
        part_writer
            .clear_session_projection(&txn, session_id)
            .await?;
        apply_projection_events_on_connection(&txn, &part_writer, session_id, events.as_slice())
            .await?;
        txn.commit().await?;
        Ok(())
    }
}

fn unmatched_lifecycles(
    events: &[DomainEvent],
) -> (
    std::collections::BTreeSet<RunId>,
    std::collections::BTreeSet<agena_domain::ExecutionId>,
) {
    let mut started_runs = std::collections::BTreeSet::new();
    let mut started_executions = std::collections::BTreeSet::new();
    for event in events {
        match &event.kind {
            EventKind::ExecutionStarted(payload) => {
                started_executions.insert(payload.execution_id);
            }
            EventKind::ExecutionFinished(payload) => {
                started_executions.remove(&payload.execution_id);
            }
            EventKind::RunStarted(RunStarted { run_id, .. }) => {
                started_runs.insert(*run_id);
            }
            EventKind::RunCompleted(payload) => {
                started_runs.remove(&payload.run_id);
            }
            EventKind::RunAborted(payload) => {
                started_runs.remove(&payload.run_id);
            }
            _ => {}
        }
    }
    (started_runs, started_executions)
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

fn projected_message_from_record(
    record: MessageProjectionHeaderRecord,
    parts: Vec<MessagePart>,
) -> Result<Message, DbErr> {
    let metadata: crate::message::MessageMetadata = serde_json::from_value(record.metadata)
        .map_err(|error| DbErr::Custom(format!("decode projected message metadata: {error}")))?;
    if record.turn_id != metadata.turn_id {
        return Err(DbErr::Custom(format!(
            "message {} has inconsistent turn identity: column {:?}, metadata {:?}",
            record.message_id, record.turn_id, metadata.turn_id
        )));
    }
    let provider_state = record
        .provider_state
        .map(serde_json::from_value::<crate::message::MessageProviderState>)
        .transpose()
        .map_err(|error| DbErr::Custom(format!("decode projected provider state: {error}")))?;
    let usage = record
        .usage
        .map(serde_json::from_value::<agena_provider::CompletionUsage>)
        .transpose()
        .map_err(|error| DbErr::Custom(format!("decode projected message usage: {error}")))?;
    Ok(Message {
        id: record.message_id,
        role: record.role,
        state: record.state,
        parts,
        created_at: timestamp_millis_to_utc(record.created_at_ms)?,
        metadata,
        provider_state,
        usage,
    })
}

fn projected_message_header_from_record(
    record: MessageProjectionHeaderRecord,
) -> Result<ProjectedMessageHeader, DbErr> {
    let part_count = u64::try_from(record.part_count).map_err(|_| {
        DbErr::Custom(format!(
            "negative projected part count: {}",
            record.part_count
        ))
    })?;
    let metadata: crate::message::MessageMetadata = serde_json::from_value(record.metadata)
        .map_err(|error| DbErr::Custom(format!("decode projected message metadata: {error}")))?;
    if record.turn_id != metadata.turn_id {
        return Err(DbErr::Custom(format!(
            "message {} has inconsistent turn identity: column {:?}, metadata {:?}",
            record.message_id, record.turn_id, metadata.turn_id
        )));
    }
    let usage = record
        .usage
        .map(serde_json::from_value::<agena_provider::CompletionUsage>)
        .transpose()
        .map_err(|error| DbErr::Custom(format!("decode projected message usage: {error}")))?;
    Ok(ProjectedMessageHeader {
        id: record.message_id,
        role: record.role,
        state: record.state,
        created_at: timestamp_millis_to_utc(record.created_at_ms)?,
        metadata,
        usage,
        part_count,
    })
}

fn projected_part_from_record(record: MessageProjectionPartRecord) -> Result<MessagePart, DbErr> {
    let content = record
        .content
        .map(serde_json::from_value::<crate::message::PartContent>)
        .transpose()
        .map_err(|error| DbErr::Custom(format!("decode projected part content: {error}")))?;
    Ok(MessagePart {
        id: record.part_id,
        message_id: record.message_id,
        part_index: record.part_index,
        status: record.status,
        kind: record.kind,
        name: record.name,
        summary: record.summary,
        has_detail: record.has_detail,
        operation_id: record.operation_id,
        created_at: timestamp_millis_to_utc(record.created_at_ms)?,
        content,
    })
}

fn projected_message_records_needing_part_repair(
    records: &[MessageProjectionHeaderRecord],
    parts_by_message: &std::collections::BTreeMap<i64, Vec<MessagePart>>,
) -> Vec<i64> {
    records
        .iter()
        .filter(|record| {
            projected_message_record_needs_part_repair(
                record,
                parts_by_message.get(&record.message_id).map_or(0, Vec::len),
            )
        })
        .map(|record| record.message_id)
        .collect()
}

fn projected_message_record_needs_part_repair(
    record: &MessageProjectionHeaderRecord,
    loaded_part_count: usize,
) -> bool {
    let expected_part_count = usize::try_from(record.part_count).unwrap_or_default();
    loaded_part_count < expected_part_count
        || (loaded_part_count == 0 && matches!(record.role, Role::Assistant | Role::User))
}

fn source_if_missing(
    mut metadata: crate::message::MessageMetadata,
    source: MessageSource,
) -> crate::message::MessageMetadata {
    if metadata.source != MessageSource::System {
        metadata.source = source;
    }
    metadata
}

fn activity_message_row(
    message_id: i64,
    session_id: i64,
    execution_id: agena_domain::ExecutionId,
    state: ExecutionStatus,
    created_at_ms: i64,
) -> activity_message::Model {
    activity_message::Model {
        message_id,
        session_id,
        turn_id: None,
        execution_id: Some(execution_id.to_string()),
        run_id: None,
        role: Role::System.into(),
        state: state.into(),
        created_at_ms,
        updated_at_ms: created_at_ms,
        metadata: MessageMetadata {
            source: MessageSource::System,
            ..Default::default()
        },
        provider_state: None,
        usage: None,
        part_count: 1,
        is_hidden: false,
    }
}

fn activity_message_part(
    part_id: i64,
    message_id: i64,
    status: ExecutionStatus,
    created_at_ms: i64,
    activity: ActivityPart,
) -> Result<MessagePart, DbErr> {
    let created_at = timestamp_millis_to_utc(created_at_ms)?;
    let mut part = MessagePart::from_content(
        part_id,
        message_id,
        created_at,
        status,
        PartContent::Activity(activity.clone()),
    );
    part.operation_id = Some(activity.activity_id);
    Ok(part)
}

async fn project_execution_started<C, W>(
    db: &C,
    part_writer: &W,
    payload: &ExecutionStartedEvent,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
    W: ProjectionPartWriter<C> + ProjectionMessageWriter<C>,
{
    let activity = ActivityPart::execution(payload.execution_id, payload.source, payload.ts_ms);
    let mut message = activity_message_row(
        payload.activity_message_id.raw(),
        payload.session_id,
        payload.execution_id,
        ExecutionStatus::InProgress,
        payload.ts_ms,
    );
    // A submitted user message is already an immediate optimistic transcript
    // record. Keep its execution activity latent unless it fails or is
    // cancelled; this avoids placing "Generating response" before the user
    // message while still preserving a durable terminal error.
    message.is_hidden = payload.source == agena_domain::ExecutionSource::User;
    let part = activity_message_part(
        payload.activity_part_id.raw(),
        payload.activity_message_id.raw(),
        ExecutionStatus::InProgress,
        payload.ts_ms,
        activity,
    )?;
    part_writer.upsert_message(db, message).await?;
    part_writer.upsert_part(db, payload.session_id, &part).await
}

async fn load_execution_activity<C>(
    db: &C,
    session_id: i64,
    execution_id: agena_domain::ExecutionId,
) -> Result<Option<(activity_message::Model, activity_part::Model, ActivityPart)>, DbErr>
where
    C: ConnectionTrait,
{
    let parts = activity_part::Entity::find()
        .filter(activity_part::Column::Kind.eq(StoredPartKind::Activity))
        .filter(activity_part::Column::OperationId.eq(execution_id.to_string()))
        .all(db)
        .await?;
    let mut matches = Vec::new();
    for part in parts {
        let Some(message) = activity_message::Entity::find_by_id(part.message_id)
            .one(db)
            .await?
        else {
            return Err(DbErr::Custom(format!(
                "activity part {} has no owning message {}",
                part.part_id, part.message_id
            )));
        };
        if message.session_id != session_id {
            continue;
        }
        let content = part.content.clone().ok_or_else(|| {
            DbErr::Custom(format!("activity part {} has no content", part.part_id))
        })?;
        let PartContent::Activity(activity) = content else {
            return Err(DbErr::Custom(format!(
                "activity part {} has non-activity content",
                part.part_id
            )));
        };
        matches.push((message, part, activity));
    }
    if matches.len() > 1 {
        return Err(DbErr::Custom(format!(
            "execution {execution_id} resolves to {} activities in session {session_id}",
            matches.len()
        )));
    }
    Ok(matches.pop())
}

async fn project_execution_finished<C, W>(
    db: &C,
    part_writer: &W,
    payload: &ExecutionFinishedEvent,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
    W: ProjectionPartWriter<C> + ProjectionMessageWriter<C>,
{
    let Some((mut message, stored_part, mut activity)) =
        load_execution_activity(db, payload.session_id, payload.execution_id).await?
    else {
        return Err(DbErr::Custom(format!(
            "execution {} finished without a projected activity",
            payload.execution_id
        )));
    };
    let source = activity.execution_source();
    let status = match &payload.outcome {
        ExecutionOutcome::Completed => {
            activity.complete_execution(payload.ts_ms);
            ExecutionStatus::Completed
        }
        ExecutionOutcome::Cancelled => {
            activity.cancel_execution(payload.ts_ms);
            message.is_hidden = false;
            ExecutionStatus::Cancelled
        }
        ExecutionOutcome::Failed {
            failure_kind,
            message: failure_message,
        } => {
            activity.fail_execution(payload.ts_ms, *failure_kind, failure_message.clone());
            message.is_hidden = false;
            ExecutionStatus::Failed
        }
    };
    if matches!(&payload.outcome, ExecutionOutcome::Completed)
        && matches!(
            source,
            Some(agena_domain::ExecutionSource::User | agena_domain::ExecutionSource::Continue)
        )
    {
        let assistant_exists = activity_message::Entity::find()
            .filter(activity_message::Column::SessionId.eq(payload.session_id))
            .filter(activity_message::Column::ExecutionId.eq(payload.execution_id.to_string()))
            .filter(activity_message::Column::Role.eq(agena_storage_sqlite::StoredRole::Assistant))
            .count(db)
            .await?
            > 0;
        message.is_hidden = assistant_exists;
    }
    message.state = status.into();
    message.updated_at_ms = payload.ts_ms;
    let part = activity_message_part(
        stored_part.part_id,
        stored_part.message_id,
        status,
        stored_part.created_at_ms,
        activity,
    )?;
    part_writer.upsert_message(db, message).await?;
    part_writer.upsert_part(db, payload.session_id, &part).await
}

async fn project_compaction_completed<C, W>(
    db: &C,
    part_writer: &W,
    payload: &PromptCompactionCompletedEvent,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
    W: ProjectionPartWriter<C> + ProjectionMessageWriter<C>,
{
    if let Some((mut message, stored_part, mut current)) =
        load_execution_activity(db, payload.session_id, payload.execution_id).await?
        && matches!(
            current.kind,
            ActivityKind::Execution {
                source: agena_domain::ExecutionSource::Compaction,
                ..
            }
        )
    {
        current.apply_compaction(payload.execution_id, payload.activity.clone());
        message.updated_at_ms = payload.ts_ms;
        let part = activity_message_part(
            stored_part.part_id,
            stored_part.message_id,
            ExecutionStatus::InProgress,
            stored_part.created_at_ms,
            current,
        )?;
        part_writer.upsert_message(db, message).await?;
        return part_writer.upsert_part(db, payload.session_id, &part).await;
    }

    let message_id = payload.standalone_message_id.ok_or_else(|| {
        DbErr::Custom("non-manual compaction activity is missing a message id".to_owned())
    })?;
    let part_id = payload.standalone_part_id.ok_or_else(|| {
        DbErr::Custom("non-manual compaction activity is missing a part id".to_owned())
    })?;
    let mut activity = ActivityPart::execution(
        payload.execution_id,
        agena_domain::ExecutionSource::Compaction,
        payload.ts_ms,
    );
    activity.activity_id = format!("compaction:{}", payload.activity.checkpoint_id);
    activity.apply_compaction(payload.execution_id, payload.activity.clone());
    activity.complete_execution(payload.ts_ms);
    let message = activity_message_row(
        message_id.raw(),
        payload.session_id,
        payload.execution_id,
        ExecutionStatus::Completed,
        payload.ts_ms,
    );
    let part = activity_message_part(
        part_id.raw(),
        message_id.raw(),
        ExecutionStatus::Completed,
        payload.ts_ms,
        activity,
    )?;
    part_writer.upsert_message(db, message).await?;
    part_writer.upsert_part(db, payload.session_id, &part).await
}

/// Local bridge from Runtime's private `MessagePart` aggregate to the
/// storage write contract. Production projection rebuilds use the
/// transaction-scoped implementation below; the direct SeaORM helper remains
/// available to focused regression tests without widening the public
/// transcript contract.
#[async_trait::async_trait]
trait ProjectionPartWriter<C>: Send + Sync
where
    C: ConnectionTrait,
{
    async fn upsert_part(
        &self,
        connection: &C,
        session_id: i64,
        part: &MessagePart,
    ) -> Result<(), DbErr>;
}

#[async_trait::async_trait]
trait ProjectionMessageWriter<C>: Send + Sync
where
    C: ConnectionTrait,
{
    async fn upsert_message(
        &self,
        connection: &C,
        message: activity_message::Model,
    ) -> Result<(), DbErr>;
}

#[async_trait::async_trait]
trait ProjectionLifecycleWriter<C>: Send + Sync
where
    C: ConnectionTrait,
{
    async fn terminalize_open_messages(
        &self,
        connection: &C,
        session_id: i64,
        identity: agena_storage::MessageProjectionOpenIdentity,
        status: ExecutionStatus,
        updated_at_ms: i64,
    ) -> Result<(), DbErr>;

    async fn clear_session_projection(&self, connection: &C, session_id: i64) -> Result<(), DbErr>;

    async fn upsert_projection_watermark(
        &self,
        connection: &C,
        session_id: i64,
        last_seq_global: i64,
        updated_at_ms: i64,
    ) -> Result<(), DbErr>;
}

#[cfg(test)]
struct RuntimeProjectionPartWriter;

#[cfg(test)]
#[async_trait::async_trait]
impl<C> ProjectionPartWriter<C> for RuntimeProjectionPartWriter
where
    C: ConnectionTrait + Sync,
{
    async fn upsert_part(
        &self,
        connection: &C,
        session_id: i64,
        part: &MessagePart,
    ) -> Result<(), DbErr> {
        upsert_part_projection(connection, session_id, part).await
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl<C> ProjectionMessageWriter<C> for RuntimeProjectionPartWriter
where
    C: ConnectionTrait + Sync,
{
    async fn upsert_message(
        &self,
        connection: &C,
        message: activity_message::Model,
    ) -> Result<(), DbErr> {
        upsert_message_projection(connection, message).await
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl<C> ProjectionLifecycleWriter<C> for RuntimeProjectionPartWriter
where
    C: ConnectionTrait + Sync,
{
    async fn terminalize_open_messages(
        &self,
        connection: &C,
        session_id: i64,
        identity: agena_storage::MessageProjectionOpenIdentity,
        status: ExecutionStatus,
        _updated_at_ms: i64,
    ) -> Result<(), DbErr> {
        let (column, value) = match identity {
            agena_storage::MessageProjectionOpenIdentity::RunId(value) => ("run_id", value),
            agena_storage::MessageProjectionOpenIdentity::ExecutionId(value) => {
                ("execution_id", value)
            }
        };
        terminalize_open_messages(connection, session_id, column, &value, status).await
    }

    async fn clear_session_projection(&self, connection: &C, session_id: i64) -> Result<(), DbErr> {
        clear_projection_for_session(connection, session_id).await
    }

    async fn upsert_projection_watermark(
        &self,
        connection: &C,
        session_id: i64,
        last_seq_global: i64,
        _updated_at_ms: i64,
    ) -> Result<(), DbErr> {
        upsert_projection_state(connection, session_id, last_seq_global).await
    }
}

struct TransactionProjectionPartWriter {
    writer: Arc<dyn MessageProjectionTransactionWriter<DatabaseTransaction>>,
}

impl TransactionProjectionPartWriter {
    fn new(writer: Arc<dyn MessageProjectionTransactionWriter<DatabaseTransaction>>) -> Self {
        Self { writer }
    }
}

#[async_trait::async_trait]
impl ProjectionPartWriter<DatabaseTransaction> for TransactionProjectionPartWriter {
    async fn upsert_part(
        &self,
        transaction: &DatabaseTransaction,
        session_id: i64,
        part: &MessagePart,
    ) -> Result<(), DbErr> {
        self.writer
            .upsert_part_in_transaction(
                transaction,
                &MessageProjectionPartWrite {
                    session_id,
                    part_id: part.id,
                    message_id: part.message_id,
                    part_index: part.part_index,
                    status: part.status,
                    kind: part.kind,
                    name: part.name.clone(),
                    summary: part.summary.clone(),
                    has_detail: part.has_detail,
                    operation_id: part.operation_id.clone(),
                    created_at_ms: part.created_at.timestamp_millis(),
                    content: part
                        .content
                        .as_ref()
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|error| {
                            DbErr::Custom(format!("serialize projection part: {error}"))
                        })?,
                },
            )
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))
    }
}

#[async_trait::async_trait]
impl ProjectionMessageWriter<DatabaseTransaction> for TransactionProjectionPartWriter {
    async fn upsert_message(
        &self,
        transaction: &DatabaseTransaction,
        message: activity_message::Model,
    ) -> Result<(), DbErr> {
        if message.turn_id != message.metadata.turn_id {
            return Err(DbErr::Custom(format!(
                "message {} has inconsistent turn identity: column {:?}, metadata {:?}",
                message.message_id, message.turn_id, message.metadata.turn_id
            )));
        }
        self.writer
            .upsert_message_in_transaction(
                transaction,
                &agena_storage::MessageProjectionMessageWrite {
                    message_id: message.message_id,
                    session_id: message.session_id,
                    turn_id: message.turn_id,
                    execution_id: message.execution_id,
                    run_id: message.run_id,
                    role: message.role.into(),
                    state: message.state.into(),
                    created_at_ms: message.created_at_ms,
                    updated_at_ms: message.updated_at_ms,
                    metadata: serde_json::to_value(message.metadata).map_err(|error| {
                        DbErr::Custom(format!("serialize projection message metadata: {error}"))
                    })?,
                    provider_state: message
                        .provider_state
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|error| {
                            DbErr::Custom(format!(
                                "serialize projection message provider state: {error}"
                            ))
                        })?,
                    usage: message
                        .usage
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|error| {
                            DbErr::Custom(format!("serialize projection message usage: {error}"))
                        })?,
                    part_count: message.part_count,
                    is_hidden: message.is_hidden,
                },
            )
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))
    }
}

#[async_trait::async_trait]
impl ProjectionLifecycleWriter<DatabaseTransaction> for TransactionProjectionPartWriter {
    async fn terminalize_open_messages(
        &self,
        transaction: &DatabaseTransaction,
        session_id: i64,
        identity: agena_storage::MessageProjectionOpenIdentity,
        status: ExecutionStatus,
        updated_at_ms: i64,
    ) -> Result<(), DbErr> {
        self.writer
            .terminalize_open_messages_in_transaction(
                transaction,
                session_id,
                &identity,
                status,
                updated_at_ms,
            )
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))
    }

    async fn clear_session_projection(
        &self,
        transaction: &DatabaseTransaction,
        session_id: i64,
    ) -> Result<(), DbErr> {
        self.writer
            .clear_session_projection_in_transaction(transaction, session_id)
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))
    }

    async fn upsert_projection_watermark(
        &self,
        transaction: &DatabaseTransaction,
        session_id: i64,
        last_seq_global: i64,
        updated_at_ms: i64,
    ) -> Result<(), DbErr> {
        self.writer
            .upsert_projection_watermark_in_transaction(
                transaction,
                session_id,
                last_seq_global,
                updated_at_ms,
            )
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))
    }
}

#[cfg(test)]
async fn upsert_message_projection<C>(db: &C, row: activity_message::Model) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    if row.turn_id != row.metadata.turn_id {
        return Err(DbErr::Custom(format!(
            "message {} has inconsistent turn identity: column {:?}, metadata {:?}",
            row.message_id, row.turn_id, row.metadata.turn_id
        )));
    }
    if let Some(existing) = activity_message::Entity::find_by_id(row.message_id)
        .one(db)
        .await?
    {
        if existing.session_id != row.session_id {
            return Err(DbErr::Custom(format!(
                "message {} belongs to session {}, cannot reassign it to session {}",
                row.message_id, existing.session_id, row.session_id
            )));
        }
        if existing.turn_id != row.turn_id {
            return Err(DbErr::Custom(format!(
                "message {} turn identity is immutable: stored {:?}, received {:?}",
                row.message_id, existing.turn_id, row.turn_id
            )));
        }
        if existing.role != row.role || existing.created_at_ms != row.created_at_ms {
            return Err(DbErr::Custom(format!(
                "message {} immutable identity fields changed",
                row.message_id
            )));
        }
    }

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
         (message_id, session_id, turn_id, execution_id, run_id, role, state, created_at_ms, updated_at_ms, metadata, provider_state, usage, part_count, is_hidden) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(message_id) DO UPDATE SET \
         execution_id = excluded.execution_id, run_id = excluded.run_id, \
         state = excluded.state, updated_at_ms = excluded.updated_at_ms, \
         metadata = excluded.metadata, provider_state = excluded.provider_state, usage = excluded.usage, \
         part_count = excluded.part_count, is_hidden = excluded.is_hidden \
         WHERE agena_activity_messages.session_id = excluded.session_id \
           AND agena_activity_messages.turn_id IS excluded.turn_id \
           AND agena_activity_messages.role = excluded.role \
           AND agena_activity_messages.created_at_ms = excluded.created_at_ms",
        [
            row.message_id.into(),
            row.session_id.into(),
            row.turn_id.into(),
            row.execution_id.into(),
            row.run_id.into(),
            role_db_value(row.role.into()).into(),
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
    if db.execute(stmt).await?.rows_affected() == 0 {
        return Err(DbErr::Custom(format!(
            "message {} projection identity changed concurrently",
            row.message_id
        )));
    }
    Ok(())
}

#[cfg(test)]
fn role_db_value(role: Role) -> i8 {
    match role {
        Role::User => 1,
        Role::Assistant => 2,
        Role::System => 3,
        Role::Tool => 4,
    }
}

#[cfg(test)]
fn execution_status_db_value(status: StoredExecutionStatus) -> i8 {
    match status {
        StoredExecutionStatus::Pending => 1,
        StoredExecutionStatus::InProgress => 2,
        StoredExecutionStatus::Completed => 3,
        StoredExecutionStatus::Failed => 4,
        StoredExecutionStatus::Cancelled => 5,
    }
}

#[cfg(test)]
async fn upsert_part_projection<C>(db: &C, session_id: i64, part: &MessagePart) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let owner = activity_message::Entity::find_by_id(part.message_id)
        .one(db)
        .await?
        .ok_or_else(|| {
            DbErr::Custom(format!(
                "part {} references missing message {}",
                part.id, part.message_id
            ))
        })?;
    if owner.session_id != session_id {
        return Err(DbErr::Custom(format!(
            "message {} belongs to session {}, cannot attach part {} from session {}",
            part.message_id, owner.session_id, part.id, session_id
        )));
    }
    if let Some(existing) = activity_part::Entity::find_by_id(part.id).one(db).await? {
        if existing.message_id != part.message_id {
            return Err(DbErr::Custom(format!(
                "part {} belongs to message {}, cannot reassign it to message {}",
                part.id, existing.message_id, part.message_id
            )));
        }
        if existing.part_index != part.part_index
            || existing.kind != part.kind.into()
            || existing.operation_id != part.operation_id
            || existing.created_at_ms != part.created_at.timestamp_millis()
        {
            return Err(DbErr::Custom(format!(
                "part {} immutable identity fields changed",
                part.id
            )));
        }
    }
    if let Some(operation_id) = part.operation_id.as_deref() {
        let existing = activity_part::Entity::find()
            .select_only()
            .column(activity_part::Column::PartId)
            .filter(activity_part::Column::MessageId.eq(part.message_id))
            .filter(activity_part::Column::OperationId.eq(operation_id))
            .filter(activity_part::Column::Kind.eq(StoredPartKind::from(part.kind)))
            .into_tuple::<i64>()
            .all(db)
            .await?;
        for part_id in existing {
            if part_id != part.id {
                return Err(DbErr::Custom(format!(
                    "operation identity {} for message {} kind {:?} is already bound to part {}, cannot rebind it to part {}",
                    operation_id, part.message_id, part.kind, part_id, part.id
                )));
            }
        }
    }

    activity_part::Entity::insert(activity_part::ActiveModel {
        part_id: ActiveValue::Set(part.id),
        message_id: ActiveValue::Set(part.message_id),
        part_index: ActiveValue::Set(part.part_index),
        status: ActiveValue::Set(part.status.into()),
        kind: ActiveValue::Set(part.kind.into()),
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
                activity_part::Column::Status,
                activity_part::Column::Name,
                activity_part::Column::Summary,
                activity_part::Column::HasDetail,
                activity_part::Column::Content,
            ])
            .action_and_where(Expr::cust(
                "agena_activity_parts.message_id = excluded.message_id \
                 AND agena_activity_parts.part_index = excluded.part_index \
                 AND agena_activity_parts.kind = excluded.kind \
                 AND agena_activity_parts.operation_id IS excluded.operation_id \
                 AND agena_activity_parts.created_at_ms = excluded.created_at_ms",
            ))
            .to_owned(),
    )
    .exec(db)
    .await?;
    let persisted = activity_part::Entity::find_by_id(part.id)
        .one(db)
        .await?
        .ok_or_else(|| DbErr::Custom(format!("part {} disappeared after upsert", part.id)))?;
    if persisted.message_id != part.message_id
        || persisted.part_index != part.part_index
        || persisted.kind != part.kind.into()
        || persisted.operation_id != part.operation_id
        || persisted.created_at_ms != part.created_at.timestamp_millis()
    {
        return Err(DbErr::Custom(format!(
            "part {} projection identity changed concurrently",
            part.id
        )));
    }
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

async fn touch_message_projection<C, W>(
    db: &C,
    message_writer: &W,
    message_id: i64,
    updated_at_ms: i64,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
    W: ProjectionMessageWriter<C>,
{
    if let Some(message) = activity_message::Entity::find_by_id(message_id)
        .one(db)
        .await?
    {
        let part_count = count_parts_for_message(db, message_id).await? as i64;
        let mut updated = message;
        updated.updated_at_ms = updated_at_ms;
        updated.part_count = part_count;
        message_writer.upsert_message(db, updated).await?;
    }
    Ok(())
}

async fn project_tool_call_issued<C, W>(
    db: &C,
    message_writer: &W,
    _session_id: i64,
    payload: &super::ToolCallIssued,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
    W: ProjectionMessageWriter<C>,
{
    if activity_message::Entity::find_by_id(payload.message_id.raw())
        .one(db)
        .await?
        .is_none()
    {
        return Ok(());
    }

    let operation_parts = activity_part::Entity::find()
        .filter(activity_part::Column::MessageId.eq(payload.message_id.raw()))
        .filter(activity_part::Column::Kind.eq(StoredPartKind::Operation))
        .filter(activity_part::Column::OperationId.eq(payload.call_id.as_ref()))
        .all(db)
        .await?;
    match operation_parts.as_slice() {
        [_] => {}
        [] => {
            return Err(DbErr::Custom(format!(
                "tool call {} for message {} has no persisted assistant operation part",
                payload.call_id,
                payload.message_id.raw()
            )));
        }
        parts => {
            return Err(DbErr::Custom(format!(
                "tool call {} for message {} resolves to {} operation parts",
                payload.call_id,
                payload.message_id.raw(),
                parts.len()
            )));
        }
    }
    touch_message_projection(
        db,
        message_writer,
        payload.message_id.raw(),
        payload.created_at.timestamp_millis(),
    )
    .await
}

async fn update_tool_result_projection<C, W>(
    db: &C,
    part_writer: &W,
    session_id: i64,
    payload: &super::ToolCallCompleted,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
    W: ProjectionPartWriter<C> + ProjectionMessageWriter<C>,
{
    let operation_parts = activity_part::Entity::find()
        .filter(activity_part::Column::MessageId.eq(payload.message_id.raw()))
        .filter(activity_part::Column::Kind.eq(StoredPartKind::Operation))
        .filter(activity_part::Column::OperationId.eq(payload.call_id.as_ref()))
        .all(db)
        .await?;
    let existing = match operation_parts.as_slice() {
        [part] => part,
        [] => {
            return Err(DbErr::Custom(format!(
                "tool result {} for message {} has no persisted assistant operation part",
                payload.call_id,
                payload.message_id.raw()
            )));
        }
        parts => {
            return Err(DbErr::Custom(format!(
                "tool result {} for message {} resolves to {} operation parts",
                payload.call_id,
                payload.message_id.raw(),
                parts.len()
            )));
        }
    };
    if existing.part_id != payload.part.id {
        return Err(DbErr::Custom(format!(
            "tool result {} for message {} targets part {}, but the operation is bound to part {}",
            payload.call_id,
            payload.message_id.raw(),
            payload.part.id,
            existing.part_id
        )));
    }

    let mut authoritative_part = payload.part.clone();
    authoritative_part.message_id = payload.message_id.raw();
    part_writer
        .upsert_part(db, session_id, &authoritative_part)
        .await?;

    touch_message_projection(
        db,
        part_writer,
        payload.message_id.raw(),
        payload.completed_at.timestamp_millis(),
    )
    .await
}

impl SessionHistoryStore {
    async fn read_projected_messages(
        &self,
        session_id: i64,
        include_full_parts: bool,
    ) -> Result<Vec<Message>, DbErr> {
        let mut message_rows = self
            .message_projection_repository
            .list_headers(session_id)
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))?;

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
        if projected_message_records_needing_part_repair(message_rows.as_slice(), &parts_by_message)
            .is_empty()
        {
            return message_rows
                .into_iter()
                .map(|row| {
                    let message_id = row.message_id;
                    projected_message_from_record(
                        row,
                        parts_by_message.remove(&message_id).unwrap_or_default(),
                    )
                })
                .collect();
        }

        self.rebuild_projection_from_history(session_id).await?;
        message_rows = self
            .message_projection_repository
            .list_headers(session_id)
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))?;
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
                projected_message_from_record(
                    row,
                    parts_by_message.remove(&message_id).unwrap_or_default(),
                )
            })
            .collect()
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

async fn apply_projection_events_on_connection<C, W>(
    db: &C,
    part_writer: &W,
    session_id: i64,
    events: &[DomainEvent],
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
    W: ProjectionPartWriter<C> + ProjectionMessageWriter<C> + ProjectionLifecycleWriter<C>,
{
    for event in events {
        match &event.kind {
            EventKind::ExecutionStarted(payload) => {
                ensure_projection_session(session_id, payload.session_id, "execution_started")?;
                project_execution_started(db, part_writer, payload).await?;
            }
            EventKind::CompactionCompleted(payload) => {
                ensure_projection_session(session_id, payload.session_id, "compaction_completed")?;
                project_compaction_completed(db, part_writer, payload).await?;
            }
            EventKind::UserMessageAppended(payload) => {
                let metadata = source_if_missing(payload.metadata.clone(), MessageSource::User);
                part_writer
                    .upsert_message(
                        db,
                        activity_message::Model {
                            message_id: payload.message_id.raw(),
                            session_id,
                            turn_id: metadata.turn_id,
                            execution_id: Some(payload.execution_id.to_string()),
                            run_id: Some(payload.run_id.to_string()),
                            role: Role::User.into(),
                            state: StoredExecutionStatus::Completed,
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
                    part_writer
                        .upsert_part(db, session_id, part)
                        .await
                        .map_err(|err| {
                            DbErr::Custom(format!(
                                "project user part {} for message {}: {err}",
                                part.id, part.message_id
                            ))
                        })?;
                }
            }
            EventKind::AssistantMessageFinished(payload) => {
                if matches!(
                    payload.status,
                    ExecutionStatus::Pending | ExecutionStatus::InProgress
                ) {
                    return Err(DbErr::Custom(format!(
                        "assistant terminal event {} has nonterminal status {:?}",
                        payload.message_id, payload.status
                    )));
                }
                let metadata =
                    source_if_missing(payload.metadata.clone(), MessageSource::Assistant);
                // Streaming checkpoints are the first durable observation of
                // an assistant message and therefore establish its immutable
                // creation time. Older producers sampled the run-buffer time
                // separately from the live Message time; when those samples
                // straddled a millisecond boundary, the terminal event could
                // differ by 1 ms and make an otherwise valid history
                // impossible to replay. Preserve the already-projected
                // identity while applying the terminal state. New producers
                // reuse one timestamp, so this is also the compatibility path
                // for affected existing databases.
                let created_at_ms = activity_message::Entity::find_by_id(payload.message_id.raw())
                    .one(db)
                    .await?
                    .map(|message| message.created_at_ms)
                    .unwrap_or_else(|| payload.created_at.timestamp_millis());
                part_writer
                    .upsert_message(
                        db,
                        activity_message::Model {
                            message_id: payload.message_id.raw(),
                            session_id,
                            turn_id: metadata.turn_id,
                            execution_id: Some(payload.execution_id.to_string()),
                            run_id: Some(payload.run_id.to_string()),
                            role: Role::Assistant.into(),
                            state: payload.status.into(),
                            created_at_ms,
                            updated_at_ms: payload.created_at.timestamp_millis(),
                            metadata,
                            provider_state: payload.provider_state.clone(),
                            usage: payload.usage.clone().map(Into::into),
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
                    part_writer
                        .upsert_part(db, session_id, part)
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
                project_tool_call_issued(db, part_writer, session_id, payload)
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
                update_tool_result_projection(db, part_writer, session_id, payload)
                    .await
                    .map_err(|err| {
                        DbErr::Custom(format!(
                            "project tool result for message {} call {}: {err}",
                            payload.message_id.raw(),
                            payload.call_id
                        ))
                    })?;
            }
            EventKind::MessagePartCheckpointed(update) => {
                apply_message_part_update_on_connection(db, part_writer, update)
                    .await
                    .map_err(|err| {
                        DbErr::Custom(format!(
                            "project part update {} for message {}: {err}",
                            update.part.id, update.message_id
                        ))
                    })?;
            }
            EventKind::RunAborted(payload) => {
                let status = match payload.reason {
                    RunAbortReason::UserCancelled => ExecutionStatus::Cancelled,
                    RunAbortReason::ProcessRestart
                    | RunAbortReason::ProviderError
                    | RunAbortReason::Internal => ExecutionStatus::Failed,
                };
                part_writer
                    .terminalize_open_messages(
                        db,
                        session_id,
                        agena_storage::MessageProjectionOpenIdentity::RunId(
                            payload.run_id.to_string(),
                        ),
                        status,
                        Utc::now().timestamp_millis(),
                    )
                    .await?;
            }
            EventKind::ExecutionFinished(payload) => {
                ensure_projection_session(session_id, payload.session_id, "execution_finished")?;
                project_execution_finished(db, part_writer, payload).await?;
                let status = match &payload.outcome {
                    // A successfully finished execution must not own an open
                    // message. Fail closed if an upstream bug violated that
                    // invariant: an inactive UI must never render a spinner.
                    ExecutionOutcome::Completed | ExecutionOutcome::Failed { .. } => {
                        ExecutionStatus::Failed
                    }
                    ExecutionOutcome::Cancelled => ExecutionStatus::Cancelled,
                };
                part_writer
                    .terminalize_open_messages(
                        db,
                        session_id,
                        agena_storage::MessageProjectionOpenIdentity::ExecutionId(
                            payload.execution_id.to_string(),
                        ),
                        status,
                        Utc::now().timestamp_millis(),
                    )
                    .await?;
            }
            _ => {}
        }
    }

    if let Some(last_seq_global) = events.iter().map(|event| event.meta.seq_global).max() {
        part_writer
            .upsert_projection_watermark(
                db,
                session_id,
                last_seq_global,
                Utc::now().timestamp_millis(),
            )
            .await?;
    }

    Ok(())
}

fn ensure_projection_session(
    envelope_session_id: i64,
    payload_session_id: i64,
    event_kind: &str,
) -> Result<(), DbErr> {
    if envelope_session_id == payload_session_id {
        Ok(())
    } else {
        Err(DbErr::Custom(format!(
            "{event_kind} payload targets session {payload_session_id}, but its event envelope targets session {envelope_session_id}"
        )))
    }
}

async fn apply_message_part_update_on_connection<C, W>(
    db: &C,
    part_writer: &W,
    update: &MessagePartCheckpointedEvent,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
    W: ProjectionPartWriter<C> + ProjectionMessageWriter<C>,
{
    let message_row = match activity_message::Entity::find_by_id(update.message_id)
        .one(db)
        .await?
    {
        Some(row) => row,
        None => {
            let metadata = source_if_missing(
                update.message_metadata.clone(),
                role_default_source(update.message_role),
            );
            let row = activity_message::Model {
                message_id: update.message_id,
                session_id: update.session_id,
                turn_id: metadata.turn_id,
                execution_id: update.execution_id.map(|id| id.to_string()),
                run_id: update.run_id.map(|id| id.to_string()),
                role: update.message_role.into(),
                state: update.message_state.into(),
                created_at_ms: update.message_created_at.timestamp_millis(),
                updated_at_ms: update.ts_ms,
                metadata,
                provider_state: None,
                usage: None,
                part_count: 0,
                is_hidden: false,
            };
            part_writer.upsert_message(db, row.clone()).await?;
            row
        }
    };

    if message_row.turn_id != update.message_metadata.turn_id {
        return Err(DbErr::Custom(format!(
            "message {} checkpoint changed turn identity from {:?} to {:?}",
            update.message_id, message_row.turn_id, update.message_metadata.turn_id
        )));
    }

    // Checkpoints are observations of mutable streaming state, never commands
    // that may reopen terminal history. This also makes a delayed checkpoint
    // harmless if it is delivered after RunAborted/ExecutionFinished.
    if !ExecutionStatus::from(message_row.state).can_transition(update.message_state) {
        return Ok(());
    }
    if let Some(existing_part) = activity_part::Entity::find_by_id(update.part.id)
        .one(db)
        .await?
        && !ExecutionStatus::from(existing_part.status).can_transition(update.part.status)
    {
        return Ok(());
    }

    part_writer
        .upsert_part(db, update.session_id, &update.part)
        .await?;

    let mut updated = message_row;
    if let Some(execution_id) = update.execution_id {
        updated.execution_id = Some(execution_id.to_string());
    }
    if let Some(run_id) = update.run_id {
        updated.run_id = Some(run_id.to_string());
    }
    updated.state = update.message_state.into();
    updated.updated_at_ms = update.ts_ms;
    updated.part_count = count_parts_for_message(db, update.message_id).await? as i64;
    part_writer.upsert_message(db, updated).await?;
    Ok(())
}

#[cfg(test)]
async fn terminalize_open_messages<C>(
    db: &C,
    session_id: i64,
    identity: &str,
    value: &str,
    status: ExecutionStatus,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let mut query =
        activity_message::Entity::find().filter(activity_message::Column::SessionId.eq(session_id));
    query = match identity {
        "run_id" => query.filter(activity_message::Column::RunId.eq(value)),
        "execution_id" => query.filter(activity_message::Column::ExecutionId.eq(value)),
        _ => {
            return Err(DbErr::Custom(format!(
                "unknown message identity: {identity}"
            )));
        }
    };
    let messages = query.all(db).await?;
    for message in messages {
        let message_id = message.message_id;
        if matches!(
            ExecutionStatus::from(message.state),
            ExecutionStatus::Pending | ExecutionStatus::InProgress
        ) {
            let mut active: activity_message::ActiveModel = message.into();
            active.state = ActiveValue::Set(status.into());
            active.updated_at_ms = ActiveValue::Set(Utc::now().timestamp_millis());
            active.update(db).await?;
        }

        // Parts have their own lifecycle. A completed assistant message can
        // still own an in-flight tool part, so close parts independently of
        // whether the parent message itself is open.
        activity_part::Entity::update_many()
            .col_expr(
                activity_part::Column::Status,
                Expr::value(StoredExecutionStatus::from(status)),
            )
            .filter(activity_part::Column::MessageId.eq(message_id))
            .filter(activity_part::Column::Status.is_in([
                StoredExecutionStatus::Pending,
                StoredExecutionStatus::InProgress,
            ]))
            .exec(db)
            .await?;
    }
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

#[cfg(test)]
async fn clear_projection_for_session<C>(db: &C, session_id: i64) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    // Parts are owned exclusively by messages and cascade with them. There is
    // intentionally no session_id column on parts to clean independently.
    activity_message::Entity::delete_many()
        .filter(activity_message::Column::SessionId.eq(session_id))
        .exec(db)
        .await?;
    activity_projection_state::Entity::delete_by_id(session_id)
        .exec(db)
        .await?;
    Ok(())
}

#[cfg(test)]
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
