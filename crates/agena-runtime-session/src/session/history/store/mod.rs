use std::sync::Arc;

use chrono::{DateTime, Utc};
#[cfg(test)]
use sea_orm::{
    ActiveModelTrait, ActiveValue,
    sea_query::{Expr, OnConflict},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    PaginatorTrait, QueryFilter, Statement, TransactionTrait,
};

use crate::{
    db::entities::{model_message, model_message_part, model_projection_state},
    event::{DomainEvent, EventKind, EventPublisher, MessagePartCheckpointedEvent, PublishContext},
    message::{Message, MessagePart, PartContent},
    session::SessionRuntimeState,
};
use agena_storage::{
    ModelMessageHeaderRecord, ModelMessagePartRecord, ModelMessagePartWrite,
    ModelMessageRepository, ModelMessageTransactionWriter, StoreRange,
};
#[cfg(test)]
use agena_storage_sqlite::StoredRole;
use agena_storage_sqlite::{StoredExecutionStatus, StoredPartKind};

use super::{RunAborted, RunId, RunStarted};
use agena_domain::{
    ActivityId, ActivityPayload, ErrorActivity, EventFilter, EventScope, ExecutionFinishedEvent,
    ExecutionOutcome, ExecutionStartedEvent, ExecutionStatus, MessageSource,
    PromptCompactionCompletedEvent, Role, RunAbortReason,
};

#[cfg(test)]
mod tests;

fn run_abort_problem(reason: RunAbortReason) -> Option<agena_failure::UserProblem> {
    use agena_failure::{
        Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility,
        RecoveryDirective, RetryDirective, UserPresentation,
    };

    let (code, category, responsibility, retry, recovery, fallback) = match reason {
        RunAbortReason::UserCancelled => return None,
        // Deliberate early stops (superseded by newer input, or a configured
        // budget exhausted) are not failures, so surface no user problem.
        RunAbortReason::Replaced | RunAbortReason::BudgetLimited => return None,
        RunAbortReason::ProviderError => (
            "provider.response_failed",
            FailureCategory::DependencyUnavailable,
            FailureResponsibility::Dependency,
            RetryDirective::Backoff,
            RecoveryDirective::Retry,
            "The provider could not complete the reply. Try again or choose another model.",
        ),
        RunAbortReason::ProcessRestart => (
            "execution.process_restarted",
            FailureCategory::Internal,
            FailureResponsibility::System,
            RetryDirective::ImmediateOnce,
            RecoveryDirective::Retry,
            "The reply was interrupted because the runtime restarted. Try again.",
        ),
        RunAbortReason::Internal => (
            "execution.internal",
            FailureCategory::Internal,
            FailureResponsibility::System,
            RetryDirective::Unknown,
            RecoveryDirective::Retry,
            "The reply stopped unexpectedly. Try again.",
        ),
    };
    let failure = Failure::new(
        FailureCode::new(code),
        category,
        responsibility,
        retry,
        recovery,
        FailureImpact::OperationFailed,
        UserPresentation::new(code, fallback),
    );
    tracing::warn!(
        failure_id = %failure.id,
        abort_reason = ?reason,
        "reconciled a run without its authoritative terminal event"
    );
    Some(failure.into())
}

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
    /// Projection catch-up is a per-session single-writer operation. Without
    /// this fence, a read-side catch-up can race an append-side barrier: both
    /// observe the same watermark and apply the same terminal event.
    projection_locks:
        Arc<std::sync::Mutex<std::collections::HashMap<i64, Arc<tokio::sync::Mutex<()>>>>>,
    message_projection_repository: Arc<dyn ModelMessageRepository>,
    message_projection_transaction_writer:
        Arc<dyn ModelMessageTransactionWriter<DatabaseTransaction>>,
}

impl SessionHistoryStore {
    pub(crate) fn new(
        publisher: Arc<EventPublisher>,
        db: DatabaseConnection,
        message_projection_repository: Arc<dyn ModelMessageRepository>,
        message_projection_transaction_writer: Arc<
            dyn ModelMessageTransactionWriter<DatabaseTransaction>,
        >,
    ) -> Self {
        Self {
            publisher,
            db,
            projection_locks: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
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
                            failure: run_abort_problem(reason),
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
                failure: run_abort_problem(RunAbortReason::ProcessRestart),
            });
            pending.push(
                self.publisher
                    .build(ctx.clone(), kind)
                    .await
                    .map_err(|err| DbErr::Custom(format!("build abort event failed: {err}")))?,
            );
        }
        for (execution_id, reply_id) in started_executions {
            let kind = EventKind::ExecutionFinished(ExecutionFinishedEvent {
                session_id,
                execution_id,
                reply_id,
                outcome: ExecutionOutcome::Failed {
                    failure: interrupted_execution_problem(),
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
        let session_lock = {
            let mut locks = self.projection_locks.lock().map_err(|_| {
                DbErr::Custom("projection coordinator lock was poisoned".to_owned())
            })?;
            Arc::clone(
                locks
                    .entry(session_id)
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        let _projection_guard = session_lock.lock().await;
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
            acquire_projection_fence(&txn, session_id).await?;
            let transactional_watermark = model_projection_state::Entity::find_by_id(session_id)
                .one(&txn)
                .await?
                .map(|row| row.last_seq_global)
                .unwrap_or(0);
            let pending = pending
                .into_iter()
                .filter(|event| event.meta.seq_global > transactional_watermark)
                .collect::<Vec<_>>();
            let part_writer = TransactionProjectionPartWriter::new(Arc::clone(
                &self.message_projection_transaction_writer,
            ));
            if !pending.is_empty() {
                apply_projection_events_on_connection(
                    &txn,
                    &part_writer,
                    session_id,
                    pending.as_slice(),
                )
                .await?;
            }
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
        acquire_projection_fence(&txn, session_id).await?;
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

fn interrupted_execution_problem() -> agena_failure::UserProblem {
    agena_failure::Failure::new(
        agena_failure::FailureCode::new("execution.process_restart"),
        agena_failure::FailureCategory::DependencyUnavailable,
        agena_failure::FailureResponsibility::System,
        agena_failure::RetryDirective::AfterUserAction,
        agena_failure::RecoveryDirective::Retry,
        agena_failure::FailureImpact::OperationFailed,
        agena_failure::UserPresentation::new(
            "execution-process-restart",
            "The reply was interrupted because the runtime restarted.",
        ),
    )
    .into()
}

/// Acquire a database-level per-session projection fence before reading the
/// transactional watermark. The no-op upsert is deliberately the first write
/// in the transaction: SQLite serializes concurrent writers here, including
/// independent `SessionHistoryStore` instances whose process-local locks are
/// not shared. After the fence is held, a stale caller re-reads the committed
/// watermark and skips events already applied by the winner.
async fn acquire_projection_fence(
    transaction: &DatabaseTransaction,
    session_id: i64,
) -> Result<(), DbErr> {
    transaction
        .execute(Statement::from_sql_and_values(
            transaction.get_database_backend(),
            "INSERT INTO agena_model_projection_states \
             (session_id, last_seq_global, updated_at_ms) VALUES (?, 0, ?) \
             ON CONFLICT(session_id) DO UPDATE SET \
             updated_at_ms = agena_model_projection_states.updated_at_ms"
                .to_owned(),
            [session_id.into(), Utc::now().timestamp_millis().into()],
        ))
        .await?;
    Ok(())
}

fn unmatched_lifecycles(
    events: &[DomainEvent],
) -> (
    std::collections::BTreeSet<RunId>,
    std::collections::BTreeMap<agena_domain::ExecutionId, agena_domain::AssistantReplyId>,
) {
    let mut started_runs = std::collections::BTreeSet::new();
    let mut started_executions = std::collections::BTreeMap::new();
    for event in events {
        match &event.kind {
            EventKind::ExecutionStarted(payload) => {
                started_executions.insert(payload.execution_id, payload.reply_id);
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
    record: ModelMessageHeaderRecord,
    parts: Vec<MessagePart>,
) -> Result<Message, DbErr> {
    let metadata: crate::message::MessageMetadata = serde_json::from_value(record.metadata)
        .map_err(|error| DbErr::Custom(format!("decode projected message metadata: {error}")))?;
    if record.model_turn_id != metadata.model_turn_id {
        return Err(DbErr::Custom(format!(
            "message {} has inconsistent turn identity: column {:?}, metadata {:?}",
            record.message_id, record.model_turn_id, metadata.model_turn_id
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
    record: ModelMessageHeaderRecord,
) -> Result<ProjectedMessageHeader, DbErr> {
    let part_count = u64::try_from(record.part_count).map_err(|_| {
        DbErr::Custom(format!(
            "negative projected part count: {}",
            record.part_count
        ))
    })?;
    let metadata: crate::message::MessageMetadata = serde_json::from_value(record.metadata)
        .map_err(|error| DbErr::Custom(format!("decode projected message metadata: {error}")))?;
    if record.model_turn_id != metadata.model_turn_id {
        return Err(DbErr::Custom(format!(
            "message {} has inconsistent turn identity: column {:?}, metadata {:?}",
            record.message_id, record.model_turn_id, metadata.model_turn_id
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

fn projected_part_from_record(record: ModelMessagePartRecord) -> Result<MessagePart, DbErr> {
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
        activity_id: record.activity_id,
        segment_id: record.segment_id,
        operation_id: record.operation_id,
        created_at: timestamp_millis_to_utc(record.created_at_ms)?,
        content,
    })
}

fn projected_message_records_needing_part_repair(
    records: &[ModelMessageHeaderRecord],
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
    record: &ModelMessageHeaderRecord,
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

async fn project_execution_started<C, W>(
    db: &C,
    _part_writer: &W,
    payload: &ExecutionStartedEvent,
    revision_seq: i64,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
    W: ProjectionPartWriter<C> + ProjectionMessageWriter<C>,
{
    let backend = db.get_database_backend();
    if payload.source == agena_domain::ExecutionSource::User {
        db.execute(Statement::from_sql_and_values(
            backend,
            "INSERT INTO agena_turns (turn_id, session_id, turn_seq, created_at_ms) \
             VALUES (?, ?, (SELECT COALESCE(MAX(turn_seq), 0) + 1 FROM agena_turns WHERE session_id = ?), ?) \
             ON CONFLICT(turn_id) DO NOTHING",
            [
                payload.turn_id.to_string().into(),
                payload.session_id.into(),
                payload.session_id.into(),
                payload.ts_ms.into(),
            ],
        ))
        .await?;
    }
    db.execute(Statement::from_sql_and_values(
        backend,
        "INSERT INTO agena_assistant_replies \
         (reply_id, turn_id, status, revision_seq, created_at_ms, finished_at_ms) \
         VALUES (?, ?, 'in_progress', ?, ?, NULL) \
         ON CONFLICT(reply_id) DO UPDATE SET \
           status = 'in_progress', revision_seq = excluded.revision_seq, finished_at_ms = NULL \
         WHERE agena_assistant_replies.turn_id = excluded.turn_id \
           AND agena_assistant_replies.revision_seq < excluded.revision_seq",
        [
            payload.reply_id.to_string().into(),
            payload.turn_id.to_string().into(),
            revision_seq.into(),
            payload.ts_ms.into(),
        ],
    ))
    .await?;
    let source = match payload.source {
        agena_domain::ExecutionSource::User => "user",
        agena_domain::ExecutionSource::Continue => "continue",
        agena_domain::ExecutionSource::Compaction => "compaction",
        agena_domain::ExecutionSource::PermissionReply => "permission_reply",
        agena_domain::ExecutionSource::UserInputReply => "user_input_reply",
    };
    db.execute(Statement::from_sql_and_values(
        backend,
        "INSERT INTO agena_reply_executions \
         (execution_id, reply_id, source, status, revision_seq, started_at_ms, finished_at_ms) \
         VALUES (?, ?, ?, 'in_progress', ?, ?, NULL) \
         ON CONFLICT(execution_id) DO NOTHING",
        [
            payload.execution_id.to_string().into(),
            payload.reply_id.to_string().into(),
            source.into(),
            revision_seq.into(),
            payload.ts_ms.into(),
        ],
    ))
    .await?;
    Ok(())
}

/// Serialize the structured failure carried by a failed execution for the
/// `agena_assistant_replies.failure_json` projection column. Non-failed
/// outcomes project `NULL` so a reply that recovers (for example a
/// `reply_waits_for_user` continuation) clears its previous failure.
fn failure_json_for_outcome(outcome: &ExecutionOutcome) -> sea_orm::Value {
    match outcome {
        ExecutionOutcome::Failed { failure } => serde_json::to_value(failure)
            .map(Box::new)
            .map(Some)
            .map(sea_orm::Value::Json)
            .unwrap_or(sea_orm::Value::Json(None)),
        _ => sea_orm::Value::Json(None),
    }
}

/// Persist a reply-level failure as a durable Error Activity owned by the
/// assistant reply, exactly like a failed tool call. The node lives only in
/// `agena_content_nodes` (the transcript history) and never in the
/// model-message parts that form the provider prompt, so it is user-facing
/// only and is never sent to the AI server. A later successful continuation
/// keeps the node (the reply recovers, the historical error stays visible),
/// and a repeated failure replaces the same stable node with the latest
/// problem instead of appending duplicates.
async fn project_reply_error_activity<C: ConnectionTrait>(
    db: &C,
    payload: &ExecutionFinishedEvent,
    revision_seq: i64,
) -> Result<(), DbErr> {
    let agena_domain::ExecutionOutcome::Failed { failure } = &payload.outcome else {
        return Ok(());
    };
    let reply_id = payload.reply_id.to_string();
    // Deterministic stable id derived from the reply id: repeated failures
    // upsert the same node instead of appending duplicates. Shared with the
    // live retry-progress node so a later refresh replaces it in place.
    let activity_id = ActivityId::for_reply_error(payload.reply_id);
    let position = next_content_position(db, "assistant_reply", reply_id.as_str()).await?;
    let payload_json = serde_json::to_value(ActivityPayload::Error(ErrorActivity {
        problem: failure.clone(),
    }))
    .map_err(|error| DbErr::Custom(format!("encode reply error activity: {error}")))?;
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO agena_content_nodes          (node_id, owner_kind, owner_id, node_type, actor, title, payload_json, text, state,           position, revision_seq, started_at_ms, finished_at_ms, created_at_ms, updated_at_ms)          VALUES (?, 'assistant_reply', ?, 'activity', 'runtime', '', ?, NULL, 'failed', ?, ?, ?, ?, ?, ?)          ON CONFLICT(node_id) DO UPDATE SET          payload_json = excluded.payload_json, state = excluded.state,          revision_seq = excluded.revision_seq, finished_at_ms = excluded.finished_at_ms,          updated_at_ms = excluded.updated_at_ms          WHERE agena_content_nodes.owner_kind = excluded.owner_kind            AND agena_content_nodes.owner_id = excluded.owner_id            AND excluded.revision_seq >= agena_content_nodes.revision_seq",
        [
            activity_id.to_string().into(),
            reply_id.clone().into(),
            payload_json.into(),
            position.into(),
            revision_seq.into(),
            payload.ts_ms.into(),
            payload.ts_ms.into(),
            payload.ts_ms.into(),
            chrono::Utc::now().timestamp_millis().into(),
        ],
    ))
    .await?;
    Ok(())
}

async fn project_execution_finished<C, W>(
    db: &C,
    _part_writer: &W,
    payload: &ExecutionFinishedEvent,
    revision_seq: i64,
) -> Result<bool, DbErr>
where
    C: ConnectionTrait,
    W: ProjectionPartWriter<C> + ProjectionMessageWriter<C>,
{
    let reply_waits_for_user = matches!(payload.outcome, ExecutionOutcome::Completed)
        && reply_has_pending_interaction(db, payload.reply_id).await?;
    let status = match payload.outcome {
        ExecutionOutcome::Completed => "completed",
        ExecutionOutcome::Cancelled => "cancelled",
        ExecutionOutcome::Failed { .. } => "failed",
    };
    let failure_json = failure_json_for_outcome(&payload.outcome);
    let result = db
        .execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "UPDATE agena_reply_executions \
             SET status = ?, revision_seq = ?, finished_at_ms = ? \
             WHERE execution_id = ? AND reply_id = ? AND finished_at_ms IS NULL",
            [
                status.into(),
                revision_seq.into(),
                payload.ts_ms.into(),
                payload.execution_id.to_string().into(),
                payload.reply_id.to_string().into(),
            ],
        ))
        .await?;
    if result.rows_affected() == 1 {
        if reply_waits_for_user {
            db.execute(Statement::from_sql_and_values(
                db.get_database_backend(),
                "UPDATE agena_assistant_replies \
                 SET status = 'in_progress', revision_seq = ?, finished_at_ms = NULL, failure_json = NULL \
                 WHERE reply_id = ? AND revision_seq < ?",
                [
                    revision_seq.into(),
                    payload.reply_id.to_string().into(),
                    revision_seq.into(),
                ],
            ))
            .await?;
        } else {
            db.execute(Statement::from_sql_and_values(
                db.get_database_backend(),
                "UPDATE agena_assistant_replies \
                 SET status = ?, revision_seq = ?, finished_at_ms = ?, failure_json = ? \
                 WHERE reply_id = ? AND revision_seq < ?",
                [
                    status.into(),
                    revision_seq.into(),
                    payload.ts_ms.into(),
                    failure_json.clone(),
                    payload.reply_id.to_string().into(),
                    revision_seq.into(),
                ],
            ))
            .await?;
            terminalize_reply_operations(db, payload, revision_seq).await?;
            if !matches!(payload.outcome, ExecutionOutcome::Completed) {
                cancel_reply_interactions(db, payload, revision_seq).await?;
            }
            if matches!(payload.outcome, ExecutionOutcome::Failed { .. }) {
                project_reply_error_activity(db, payload, revision_seq).await?;
            }
        }
        return Ok(reply_waits_for_user);
    }

    // Applying the same event again is harmless. This can happen after an
    // ambiguous transaction outcome or while recovering a projection cursor.
    let existing = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT reply_id, status, revision_seq, finished_at_ms \
             FROM agena_reply_executions WHERE execution_id = ? LIMIT 1",
            [payload.execution_id.to_string().into()],
        ))
        .await?;
    let Some(existing) = existing else {
        return Err(DbErr::Custom(format!(
            "execution-finished projection references missing reply {}",
            payload.reply_id
        )));
    };
    let existing_reply = existing.try_get::<String>("", "reply_id")?;
    if existing_reply != payload.reply_id.to_string() {
        return Err(DbErr::Custom(format!(
            "execution-finished projection identity mismatch for reply {}",
            payload.reply_id
        )));
    }
    let existing_status = existing.try_get::<String>("", "status")?;
    let existing_revision = existing.try_get::<i64>("", "revision_seq")?;
    let finished_at = existing.try_get::<Option<i64>>("", "finished_at_ms")?;
    if finished_at.is_some() && existing_status == status && existing_revision == revision_seq {
        if !reply_waits_for_user {
            terminalize_reply_operations(db, payload, revision_seq).await?;
            if !matches!(payload.outcome, ExecutionOutcome::Completed) {
                cancel_reply_interactions(db, payload, revision_seq).await?;
            }
            if matches!(payload.outcome, ExecutionOutcome::Failed { .. }) {
                project_reply_error_activity(db, payload, revision_seq).await?;
            }
        }
        return Ok(reply_waits_for_user);
    }
    // A later terminal event for an already-terminal execution is not an
    // invariant violation but the recovery cursor emitting an out-of-order
    // duplicate: the bootstrap reconcile pass (`abort_hanging_lifecycles`)
    // synthesizes `ExecutionFinished { process_restart }` events
    // transactionally, and the same execution that was running before the
    // process exit can legitimately be terminalized again by its own
    // `drive_registered` cleanup once the new process takes over. Both
    // events report a failure. Keep the earlier projection (and its
    // revision) authoritative and merely re-run the terminal side effects
    // so a late event cannot leave an open part or activity behind.
    if matches!(payload.outcome, ExecutionOutcome::Failed { .. })
        && existing_status == status
        && existing_revision <= revision_seq
    {
        terminalize_reply_operations(db, payload, revision_seq).await?;
        cancel_reply_interactions(db, payload, revision_seq).await?;
        project_reply_error_activity(db, payload, revision_seq).await?;
        // The earlier projection is authoritative, but a synthetic reconcile
        // finish may carry the first structured failure details: backfill the
        // failure projection when it is still missing.
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "UPDATE agena_assistant_replies \
             SET failure_json = COALESCE(failure_json, ?) \
             WHERE reply_id = ?",
            [failure_json, payload.reply_id.to_string().into()],
        ))
        .await?;
        return Ok(reply_waits_for_user);
    }
    // A failed execution can be completed again by the owning continuation
    // after the bootstrap reconcile synthesized the earlier failure: the
    // runtime was killed between `reconcile_interrupted_lifecycles` (which
    // terminalized the reply as failed at the aborted revision) and the
    // owner task's own `drive_registered` cleanup, so the surviving process
    // re-ran the turn to completion and emitted `ExecutionFinished
    // { completed }` at a later revision. The success is the authoritative
    // outcome; promote the reply and keep the synthetic failure only as a
    // historical record on the execution row.
    if matches!(payload.outcome, ExecutionOutcome::Completed)
        && existing_status == "failed"
        && existing_revision < revision_seq
    {
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "UPDATE agena_assistant_replies \
             SET status = 'completed', revision_seq = ?, finished_at_ms = ?, failure_json = NULL \
             WHERE reply_id = ? AND revision_seq < ?",
            [
                revision_seq.into(),
                payload.ts_ms.into(),
                payload.reply_id.to_string().into(),
                revision_seq.into(),
            ],
        ))
        .await?;
        // The `agena_reply_executions` update trigger only permits terminal
        // transitions from `in_progress`; a row already terminalized as
        // `failed` by the reconcile pass is immutable from the trigger's
        // perspective. The reply projection carries the completed outcome;
        // the historical failed record remains the execution's terminal row.
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "UPDATE agena_reply_executions \
             SET status = 'completed', revision_seq = ?, finished_at_ms = ? \
             WHERE execution_id = ? AND reply_id = ? \
               AND status = 'in_progress'",
            [
                revision_seq.into(),
                payload.ts_ms.into(),
                payload.execution_id.to_string().into(),
                payload.reply_id.to_string().into(),
            ],
        ))
        .await?;
        terminalize_reply_operations(db, payload, revision_seq).await?;
        return Ok(reply_waits_for_user);
    }
    Err(DbErr::Custom(format!(
        "conflicting terminal projection for reply {}: existing status {} at revision {}, incoming status {} at revision {}",
        payload.reply_id, existing_status, existing_revision, status, revision_seq
    )))
}

async fn reply_has_pending_interaction<C>(
    db: &C,
    reply_id: agena_domain::AssistantReplyId,
) -> Result<bool, DbErr>
where
    C: ConnectionTrait,
{
    let rows = db
        .query_all(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT payload_json FROM agena_content_nodes \
             WHERE owner_kind = 'assistant_reply' \
               AND owner_id = ? \
               AND state IN ('pending', 'in_progress')",
            [reply_id.to_string().into()],
        ))
        .await?;
    for row in rows {
        let payload = row.try_get::<serde_json::Value>("", "payload_json")?;
        let payload = serde_json::from_value::<agena_domain::ActivityPayload>(payload)
            .map_err(|error| DbErr::Custom(format!("decode pending Activity payload: {error}")))?;
        let awaits_reply = match payload {
            agena_domain::ActivityPayload::Interaction(
                agena_domain::InteractionActivity::UserInput { reply, .. },
            ) => reply.is_none(),
            agena_domain::ActivityPayload::Operation(operation) => {
                operation.authorization.awaiting().next().is_some()
            }
            _ => false,
        };
        if awaits_reply {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn cancel_reply_interactions<C>(
    db: &C,
    payload: &ExecutionFinishedEvent,
    revision_seq: i64,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let backend = db.get_database_backend();
    let reply_id = payload.reply_id.to_string();
    db.execute(Statement::from_sql_and_values(
        backend,
        "UPDATE agena_model_message_parts \
         SET status = ? \
         WHERE status IN (?, ?) \
           AND awaits_user_reply = 1 \
           AND activity_id IN ( \
             SELECT node_id FROM agena_content_nodes \
             WHERE owner_kind = 'assistant_reply' \
               AND owner_id = ? \
               AND state IN ('pending', 'in_progress') \
               AND json_extract(payload_json, '$.activity_type') = 'interaction' \
        )",
        [
            StoredExecutionStatus::Cancelled.into(),
            StoredExecutionStatus::Pending.into(),
            StoredExecutionStatus::InProgress.into(),
            reply_id.clone().into(),
        ],
    ))
    .await?;
    db.execute(Statement::from_sql_and_values(
        backend,
        "UPDATE agena_content_nodes \
         SET state = 'cancelled', revision_seq = ?, finished_at_ms = ? \
         WHERE owner_kind = 'assistant_reply' \
           AND owner_id = ? \
           AND state IN ('pending', 'in_progress') \
           AND revision_seq < ? \
           AND json_extract(payload_json, '$.activity_type') = 'interaction'",
        [
            revision_seq.into(),
            payload.ts_ms.into(),
            reply_id.into(),
            revision_seq.into(),
        ],
    ))
    .await?;
    Ok(())
}

/// Close every open non-interactive Activity owned by a reply that has truly
/// reached a terminal outcome. Permission continuations can execute work from
/// earlier model runs, so execution-id ownership is too narrow here: the
/// canonical reply is the lifecycle boundary. A suspended reply never enters
/// this function and retains all queued operations for the next continuation.
async fn terminalize_reply_operations<C>(
    db: &C,
    payload: &ExecutionFinishedEvent,
    revision_seq: i64,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let state = match payload.outcome {
        ExecutionOutcome::Cancelled => "cancelled",
        // A completed execution must not leave a tool operation open. Treat
        // that impossible state as failed rather than displaying a spinner
        // after its only execution owner has disappeared.
        ExecutionOutcome::Completed | ExecutionOutcome::Failed { .. } => "failed",
    };
    let stored_state = match payload.outcome {
        ExecutionOutcome::Cancelled => StoredExecutionStatus::Cancelled,
        ExecutionOutcome::Completed | ExecutionOutcome::Failed { .. } => {
            StoredExecutionStatus::Failed
        }
    };
    let reply_id = payload.reply_id.to_string();
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE agena_model_message_parts \
         SET status = ? \
         WHERE status IN (?, ?) \
           AND awaits_user_reply = 0 \
           AND activity_id IN ( \
             SELECT node_id FROM agena_content_nodes \
             WHERE owner_kind = 'assistant_reply' \
               AND owner_id = ? \
               AND state IN ('pending', 'in_progress') \
               AND json_extract(payload_json, '$.activity_type') != 'interaction' \
           )",
        [
            stored_state.into(),
            StoredExecutionStatus::Pending.into(),
            StoredExecutionStatus::InProgress.into(),
            reply_id.clone().into(),
        ],
    ))
    .await?;
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE agena_content_nodes \
         SET state = ?, revision_seq = ?, finished_at_ms = ? \
         WHERE owner_kind = 'assistant_reply' \
           AND owner_id = ? \
           AND state IN ('pending', 'in_progress') \
           AND revision_seq < ? \
           AND json_extract(payload_json, '$.activity_type') != 'interaction'",
        [
            state.into(),
            revision_seq.into(),
            payload.ts_ms.into(),
            reply_id.into(),
            revision_seq.into(),
        ],
    ))
    .await?;
    Ok(())
}

pub(crate) fn activity_payload(
    part: &MessagePart,
    role: Role,
) -> Option<agena_domain::ActivityPayload> {
    use agena_domain::{
        ActivityPayload, ErrorActivity, InteractionActivity, NoticeActivity,
        OperationActivity, OperationActivityError, ReasoningActivity, ResourceActivity,
        ResourceKind, ResourceReference, SkillReferenceActivity, TextArtifactActivity,
        TextSegmentActivity, ToolCallId,
    };
    match part.content.as_ref()? {
        // Assistant text parts that carry an ActivityId are interstitial body
        // segments (produced between tool calls); they render as their own
        // collapsible block. User text activities stay TextArtifact.
        PartContent::Text(text) if role == Role::Assistant => {
            Some(ActivityPayload::TextSegment(TextSegmentActivity {
                text: text.text.clone(),
            }))
        }
        PartContent::Text(text) => Some(ActivityPayload::TextArtifact(TextArtifactActivity {
            text: text.text.clone(),
            language: None,
            label: part.summary.clone(),
        })),
        PartContent::Activity(crate::message::RuntimeActivity::Reasoning(reasoning)) => {
            Some(ActivityPayload::Reasoning(ReasoningActivity {
                content: reasoning.clone(),
            }))
        }
        PartContent::Activity(crate::message::RuntimeActivity::Operation(operation)) => {
            // The compact `ToolResult` payload is the only durable tool data.
            // The human-facing detail Markdown is derived from it at render
            // time (`render_tool_payload_markdown`) and is never persisted.
            let data = operation
                .details
                .to_json_payload()
                .unwrap_or(serde_json::Value::Null);
            Some(ActivityPayload::Operation(OperationActivity {
                call_id: ToolCallId::new(
                    part.operation_id
                        .clone()
                        .unwrap_or_else(|| operation.call_id.to_string()),
                ),
                invocation: operation.invocation.clone(),
                title: operation.title.clone(),
                summary: operation.summary.clone(),
                data,
                // The durable projection carries no detail Markdown; it is
                // derived at snapshot load / lazy detail fetch time.
                markdown: String::new(),
                authorization: operation.authorization.clone(),
                error: operation
                    .error
                    .as_ref()
                    .map(|error| OperationActivityError {
                        problem: (&error.failure).into(),
                    }),
            }))
        }
        PartContent::Activity(crate::message::RuntimeActivity::Resource(attachment)) => {
            let item = attachment.attachments.first()?;
            let kind = match item.kind {
                crate::message::AttachmentKind::Image => ResourceKind::Image,
                crate::message::AttachmentKind::Audio => ResourceKind::Audio,
                crate::message::AttachmentKind::Video => ResourceKind::Video,
                crate::message::AttachmentKind::Pdf => ResourceKind::Pdf,
                crate::message::AttachmentKind::File if item.mime == "inode/directory" => {
                    ResourceKind::Directory
                }
                crate::message::AttachmentKind::File => ResourceKind::File,
            };
            let reference = match &item.source {
                crate::message::AttachmentSource::Url { url } => {
                    ResourceReference::Url { url: url.clone() }
                }
                crate::message::AttachmentSource::FileId { file_id } => {
                    ResourceReference::ProviderFile {
                        provider_id: "provider".to_owned(),
                        file_id: file_id.clone(),
                    }
                }
                crate::message::AttachmentSource::LocalPath { path } => {
                    ResourceReference::WorkspacePath { path: path.clone() }
                }
                crate::message::AttachmentSource::DataUrl { .. }
                | crate::message::AttachmentSource::Base64 { .. } => return None,
            };
            Some(ActivityPayload::Resource(ResourceActivity {
                kind,
                reference,
                name: item.summary_label(),
                media_type: (!item.mime.is_empty()).then(|| item.mime.clone()),
                size_bytes: item.size_bytes,
                width: item.width,
                height: item.height,
                duration_ms: item.duration_ms,
                page_count: item.page_count,
            }))
        }
        PartContent::Activity(crate::message::RuntimeActivity::SkillReference(skills)) => {
            let skill = skills.skills.first()?;
            Some(ActivityPayload::SkillReference(SkillReferenceActivity {
                name: skill.name.clone(),
                description: skill.description.clone(),
                instructions: skill.instructions.clone(),
                content_hash: skill.content_hash.clone(),
                source: skill.source.clone(),
                aliases: skill.aliases.clone(),
            }))
        }
        PartContent::Activity(crate::message::RuntimeActivity::Interaction(request)) => {
            Some(ActivityPayload::Interaction(match request {
                crate::message::RequestPart::UserInput(value) => InteractionActivity::UserInput {
                    request: value.request.clone(),
                    reply: value.reply.clone(),
                },
            }))
        }
        PartContent::Activity(crate::message::RuntimeActivity::Error(error)) => {
            Some(ActivityPayload::Error(ErrorActivity {
                problem: error.problem.clone(),
            }))
        }
        PartContent::Activity(crate::message::RuntimeActivity::Hook(hook)) => {
            Some(ActivityPayload::Notice(NoticeActivity {
                kind: "hook".to_owned(),
                summary: hook.summary.clone(),
                detail: hook.detail.clone(),
            }))
        }
        PartContent::Activity(crate::message::RuntimeActivity::Notice(notice)) => {
            Some(ActivityPayload::Notice(NoticeActivity {
                kind: notice.kind.clone(),
                summary: notice.summary.clone(),
                detail: notice.detail.clone(),
            }))
        }
    }
}

async fn next_content_position<C: ConnectionTrait>(
    db: &C,
    owner_kind: &str,
    owner_id: &str,
) -> Result<i64, DbErr> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT COALESCE(MAX(position), -1) + 1 AS next_position FROM (\
                 SELECT position FROM agena_content_nodes WHERE owner_kind = ? AND owner_id = ?
             )",
            [owner_kind.into(), owner_id.into()],
        ))
        .await?;
    row.map(|row| row.try_get("", "next_position"))
        .transpose()
        .map(|position| position.unwrap_or_default())
}

async fn project_part_content<C: ConnectionTrait>(
    db: &C,
    execution_id: agena_domain::ExecutionId,
    role: Role,
    part: &MessagePart,
    revision_seq: i64,
) -> Result<(), DbErr> {
    let reply = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT e.reply_id, r.turn_id \
             FROM agena_reply_executions e \
             JOIN agena_assistant_replies r ON r.reply_id = e.reply_id \
             WHERE e.execution_id = ? LIMIT 1",
            [execution_id.to_string().into()],
        ))
        .await?
        .ok_or_else(|| {
            DbErr::Custom(format!(
                "execution {execution_id} has no reply owner for transcript part {}",
                part.id
            ))
        })?;
    let reply_id: String = reply.try_get("", "reply_id")?;
    let turn_id: String = reply.try_get("", "turn_id")?;
    let (owner_kind, owner_id, actor) = match role {
        Role::User => ("turn_input", turn_id, "user"),
        Role::Assistant => ("assistant_reply", reply_id, "assistant"),
        Role::Tool => ("assistant_reply", reply_id, "tool"),
        Role::System => ("assistant_reply", reply_id, "runtime"),
    };
    let position = if let Some(activity_id) = part.activity_id {
        db.query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT position FROM agena_content_nodes WHERE node_id = ?",
            [activity_id.to_string().into()],
        ))
        .await?
        .map(|row| row.try_get("", "position"))
        .transpose()?
    } else if let Some(segment_id) = part.segment_id {
        db.query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT position FROM agena_content_nodes WHERE node_id = ?",
            [segment_id.to_string().into()],
        ))
        .await?
        .map(|row| row.try_get("", "position"))
        .transpose()?
    } else {
        None
    };
    let position = match position {
        Some(position) => position,
        None => next_content_position(db, owner_kind, owner_id.as_str()).await?,
    };

    if let Some(segment_id) = part.segment_id {
        let text = match part.content.as_ref() {
            Some(PartContent::Text(text)) if part.activity_id.is_none() => text.text.as_str(),
            _ => return Ok(()),
        };
        // v10 canonical text node: `agena_content_nodes` is the single
        // content store for text segments.
        // so readers can switch to the single content table.
        let node_state = match part.status {
            ExecutionStatus::Pending => "pending",
            ExecutionStatus::InProgress => "in_progress",
            _ => "completed",
        };
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO agena_content_nodes \
             (node_id, owner_kind, owner_id, node_type, actor, payload_json, text, state, \
              position, revision_seq, started_at_ms, finished_at_ms, created_at_ms, updated_at_ms) \
             VALUES (?, ?, ?, 'text', NULL, NULL, ?, ?, ?, ?, ?, NULL, ?, ?) \
             ON CONFLICT(node_id) DO UPDATE SET \
             text = excluded.text, state = excluded.state, revision_seq = excluded.revision_seq, \
             updated_at_ms = excluded.updated_at_ms \
             WHERE excluded.revision_seq >= agena_content_nodes.revision_seq",
            [
                segment_id.to_string().into(),
                owner_kind.into(),
                owner_id.clone().into(),
                text.into(),
                node_state.into(),
                position.into(),
                revision_seq.into(),
                part.created_at.timestamp_millis().into(),
                part.created_at.timestamp_millis().into(),
                chrono::Utc::now().timestamp_millis().into(),
            ],
        ))
        .await?;
        return Ok(());
    }

    let Some(activity_id) = part.activity_id else {
        return Ok(());
    };
    let Some(payload) = activity_payload(part, role) else {
        return Ok(());
    };
    let state = match part.status {
        ExecutionStatus::Pending => "pending",
        ExecutionStatus::InProgress => "in_progress",
        ExecutionStatus::Completed => "completed",
        ExecutionStatus::PolicyDenied
        | ExecutionStatus::UserDeclined
        | ExecutionStatus::CapabilityUnavailable
        | ExecutionStatus::ToolUnavailable
        | ExecutionStatus::Failed => "failed",
        ExecutionStatus::Cancelled => "cancelled",
    };
    let finished_at_ms = matches!(
        part.status,
        ExecutionStatus::Completed
            | ExecutionStatus::PolicyDenied
            | ExecutionStatus::UserDeclined
            | ExecutionStatus::CapabilityUnavailable
            | ExecutionStatus::ToolUnavailable
            | ExecutionStatus::Failed
            | ExecutionStatus::Cancelled
    )
    .then_some(part.created_at.timestamp_millis());
    // v10 unified content node mirror: activities also live in
    // `agena_content_nodes` so readers can switch to the single content table.
    let title = match &payload {
        agena_domain::ActivityPayload::Operation(operation) => operation.title.clone(),
        _ => String::new(),
    };
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO agena_content_nodes \
         (node_id, owner_kind, owner_id, node_type, actor, title, payload_json, text, state, \
          position, revision_seq, started_at_ms, finished_at_ms, created_at_ms, updated_at_ms) \
         VALUES (?, ?, ?, 'activity', ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(node_id) DO UPDATE SET \
         title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE agena_content_nodes.title END, \
         payload_json = excluded.payload_json, state = excluded.state, \
         revision_seq = excluded.revision_seq, finished_at_ms = excluded.finished_at_ms, \
         updated_at_ms = excluded.updated_at_ms \
         WHERE agena_content_nodes.owner_kind = excluded.owner_kind \
           AND agena_content_nodes.owner_id = excluded.owner_id \
           AND agena_content_nodes.position = excluded.position \
           AND excluded.revision_seq >= agena_content_nodes.revision_seq",
        [
            activity_id.to_string().into(),
            owner_kind.into(),
            owner_id.clone().into(),
            actor.into(),
            title.into(),
            serde_json::to_value(&payload)
                .map_err(|error| DbErr::Custom(format!("encode input activity: {error}")))?
                .into(),
            state.into(),
            position.into(),
            revision_seq.into(),
            part.created_at.timestamp_millis().into(),
            finished_at_ms.into(),
            part.created_at.timestamp_millis().into(),
            chrono::Utc::now().timestamp_millis().into(),
        ],
    ))
    .await?;
    Ok(())
}

async fn project_compaction_completed<C, W>(
    db: &C,
    _part_writer: &W,
    payload: &PromptCompactionCompletedEvent,
    revision_seq: i64,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
    W: ProjectionPartWriter<C> + ProjectionMessageWriter<C>,
{
    let reply = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT reply_id FROM agena_reply_executions WHERE execution_id = ? LIMIT 1",
            [payload.execution_id.to_string().into()],
        ))
        .await?
        .ok_or_else(|| {
            DbErr::Custom(format!(
                "compaction execution {} has no owning reply",
                payload.execution_id
            ))
        })?;
    let reply_id: String = reply.try_get("", "reply_id")?;
    let position = next_content_position(db, "assistant_reply", reply_id.as_str()).await?;
    let activity_payload = agena_domain::ActivityPayload::Notice(agena_domain::NoticeActivity {
        kind: "compaction".to_owned(),
        summary: "Prompt compaction completed".to_owned(),
        detail: Some(format!(
            "compaction of execution {} ({} tokens)",
            payload.execution_id, payload.activity.after_tokens
        )),
    });
    // v10 unified content node mirror for maintenance activities.
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO agena_content_nodes \
         (node_id, owner_kind, owner_id, node_type, actor, payload_json, text, state, \
          position, revision_seq, started_at_ms, finished_at_ms, created_at_ms, updated_at_ms) \
         VALUES (?, 'assistant_reply', ?, 'activity', 'runtime', ?, NULL, 'completed', ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(node_id) DO NOTHING",
        [
            payload.activity_id.to_string().into(),
            reply_id.clone().into(),
            serde_json::to_value(&activity_payload)
                .map_err(|error| DbErr::Custom(format!("encode compaction activity: {error}")))?
                .into(),
            position.into(),
            revision_seq.into(),
            payload.ts_ms.into(),
            payload.ts_ms.into(),
            payload.ts_ms.into(),
            payload.ts_ms.into(),
        ],
    )).await?;
    Ok(())
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
        message: model_message::Model,
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
        identity: agena_storage::ModelMessageOpenIdentity,
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
        message: model_message::Model,
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
        identity: agena_storage::ModelMessageOpenIdentity,
        status: ExecutionStatus,
        _updated_at_ms: i64,
    ) -> Result<(), DbErr> {
        let (column, value) = match identity {
            agena_storage::ModelMessageOpenIdentity::RunId(value) => ("run_id", value),
            agena_storage::ModelMessageOpenIdentity::ExecutionId(value) => ("execution_id", value),
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
    writer: Arc<dyn ModelMessageTransactionWriter<DatabaseTransaction>>,
}

impl TransactionProjectionPartWriter {
    fn new(writer: Arc<dyn ModelMessageTransactionWriter<DatabaseTransaction>>) -> Self {
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
                &ModelMessagePartWrite {
                    session_id,
                    part_id: part.id,
                    message_id: part.message_id,
                    part_index: part.part_index,
                    status: part.status,
                    kind: part.kind,
                    name: part.name.clone(),
                    summary: part.summary.clone(),
                    has_detail: part.has_detail,
                    awaits_user_reply: part.awaits_user_reply(),
                    activity_id: part.activity_id,
                    segment_id: part.segment_id,
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
        message: model_message::Model,
    ) -> Result<(), DbErr> {
        if message.model_turn_id != message.metadata.model_turn_id {
            return Err(DbErr::Custom(format!(
                "message {} has inconsistent turn identity: column {:?}, metadata {:?}",
                message.message_id, message.model_turn_id, message.metadata.model_turn_id
            )));
        }
        self.writer
            .upsert_message_in_transaction(
                transaction,
                &agena_storage::ModelMessageWrite {
                    message_id: message.message_id,
                    session_id: message.session_id,
                    model_turn_id: message.model_turn_id,
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
        identity: agena_storage::ModelMessageOpenIdentity,
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
async fn upsert_message_projection<C>(db: &C, row: model_message::Model) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    if row.model_turn_id != row.metadata.model_turn_id {
        return Err(DbErr::Custom(format!(
            "message {} has inconsistent turn identity: column {:?}, metadata {:?}",
            row.message_id, row.model_turn_id, row.metadata.model_turn_id
        )));
    }
    if let Some(existing) = model_message::Entity::find_by_id(row.message_id)
        .one(db)
        .await?
    {
        if existing.session_id != row.session_id {
            return Err(DbErr::Custom(format!(
                "message {} belongs to session {}, cannot reassign it to session {}",
                row.message_id, existing.session_id, row.session_id
            )));
        }
        if existing.model_turn_id != row.model_turn_id {
            return Err(DbErr::Custom(format!(
                "message {} turn identity is immutable: stored {:?}, received {:?}",
                row.message_id, existing.model_turn_id, row.model_turn_id
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
        "INSERT INTO agena_model_messages \
         (message_id, session_id, model_turn_id, execution_id, run_id, role, state, created_at_ms, updated_at_ms, metadata, provider_state, usage, part_count) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(message_id) DO UPDATE SET \
         execution_id = excluded.execution_id, run_id = excluded.run_id, \
         state = excluded.state, updated_at_ms = excluded.updated_at_ms, \
         metadata = excluded.metadata, provider_state = excluded.provider_state, usage = excluded.usage, \
         part_count = excluded.part_count \
         WHERE agena_model_messages.session_id = excluded.session_id \
           AND agena_model_messages.model_turn_id IS excluded.model_turn_id \
           AND agena_model_messages.role = excluded.role \
           AND agena_model_messages.created_at_ms = excluded.created_at_ms",
        [
            row.message_id.into(),
            row.session_id.into(),
            row.model_turn_id.into(),
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
        StoredExecutionStatus::PolicyDenied => 6,
        StoredExecutionStatus::UserDeclined => 7,
        StoredExecutionStatus::CapabilityUnavailable => 8,
        StoredExecutionStatus::ToolUnavailable => 9,
    }
}

#[cfg(test)]
async fn upsert_part_projection<C>(db: &C, session_id: i64, part: &MessagePart) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let owner = model_message::Entity::find_by_id(part.message_id)
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
    if let Some(existing) = model_message_part::Entity::find_by_id(part.id)
        .one(db)
        .await?
    {
        if existing.message_id != part.message_id {
            return Err(DbErr::Custom(format!(
                "part {} belongs to message {}, cannot reassign it to message {}",
                part.id, existing.message_id, part.message_id
            )));
        }
        if existing.part_index != part.part_index
            || existing.kind != part.kind.into()
            || existing.activity_id != part.activity_id.map(|id| id.to_string())
            || existing.segment_id != part.segment_id.map(|id| id.to_string())
            || existing.operation_id != part.operation_id
            || existing.created_at_ms != part.created_at.timestamp_millis()
        {
            return Err(DbErr::Custom(format!(
                "part {} immutable identity fields changed",
                part.id
            )));
        }
    }
    model_message_part::Entity::insert(model_message_part::ActiveModel {
        part_id: ActiveValue::Set(part.id),
        message_id: ActiveValue::Set(part.message_id),
        part_index: ActiveValue::Set(part.part_index),
        status: ActiveValue::Set(part.status.into()),
        kind: ActiveValue::Set(part.kind.into()),
        name: ActiveValue::Set(part.name.clone()),
        summary: ActiveValue::Set(part.summary.clone()),
        has_detail: ActiveValue::Set(part.has_detail),
        awaits_user_reply: ActiveValue::Set(part.awaits_user_reply()),
        activity_id: ActiveValue::Set(part.activity_id.map(|id| id.to_string())),
        segment_id: ActiveValue::Set(part.segment_id.map(|id| id.to_string())),
        operation_id: ActiveValue::Set(part.operation_id.clone()),
        created_at_ms: ActiveValue::Set(part.created_at.timestamp_millis()),
        content: ActiveValue::Set(part.content.clone()),
    })
    .on_conflict(
        OnConflict::column(model_message_part::Column::PartId)
            .update_columns([
                model_message_part::Column::Status,
                model_message_part::Column::Name,
                model_message_part::Column::Summary,
                model_message_part::Column::HasDetail,
                model_message_part::Column::AwaitsUserReply,
                model_message_part::Column::Content,
            ])
            .action_and_where(Expr::cust(
                "agena_model_message_parts.message_id = excluded.message_id \
                 AND agena_model_message_parts.part_index = excluded.part_index \
                 AND agena_model_message_parts.kind = excluded.kind \
                 AND agena_model_message_parts.activity_id IS excluded.activity_id \
                 AND agena_model_message_parts.segment_id IS excluded.segment_id \
                 AND agena_model_message_parts.operation_id IS excluded.operation_id \
                 AND agena_model_message_parts.created_at_ms = excluded.created_at_ms",
            ))
            .to_owned(),
    )
    .exec(db)
    .await?;
    let persisted = model_message_part::Entity::find_by_id(part.id)
        .one(db)
        .await?
        .ok_or_else(|| DbErr::Custom(format!("part {} disappeared after upsert", part.id)))?;
    if persisted.message_id != part.message_id
        || persisted.part_index != part.part_index
        || persisted.kind != part.kind.into()
        || persisted.activity_id != part.activity_id.map(|id| id.to_string())
        || persisted.segment_id != part.segment_id.map(|id| id.to_string())
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
    model_message_part::Entity::find()
        .filter(model_message_part::Column::MessageId.eq(message_id))
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
    if let Some(message) = model_message::Entity::find_by_id(message_id)
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
    if model_message::Entity::find_by_id(payload.message_id.raw())
        .one(db)
        .await?
        .is_none()
    {
        return Ok(());
    }

    let operation_parts = model_message_part::Entity::find()
        .filter(model_message_part::Column::MessageId.eq(payload.message_id.raw()))
        .filter(model_message_part::Column::Kind.eq(StoredPartKind::Activity))
        .filter(model_message_part::Column::OperationId.eq(payload.call_id.as_ref()))
        .all(db)
        .await?
        .into_iter()
        .filter(|part| {
            matches!(
                part.content.as_ref(),
                Some(PartContent::Activity(
                    crate::message::RuntimeActivity::Operation(_)
                ))
            )
        })
        .collect::<Vec<_>>();
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
    _session_id: i64,
    payload: &super::ToolCallCompleted,
    _revision_seq: i64,
) -> Result<(), DbErr>
where
    C: ConnectionTrait,
    W: ProjectionPartWriter<C> + ProjectionMessageWriter<C>,
{
    let operation_parts = model_message_part::Entity::find()
        .filter(model_message_part::Column::MessageId.eq(payload.message_id.raw()))
        .filter(model_message_part::Column::Kind.eq(StoredPartKind::Activity))
        .filter(model_message_part::Column::OperationId.eq(payload.call_id.as_ref()))
        .all(db)
        .await?
        .into_iter()
        .filter(|part| {
            matches!(
                part.content.as_ref(),
                Some(PartContent::Activity(
                    crate::message::RuntimeActivity::Operation(_)
                ))
            )
        })
        .collect::<Vec<_>>();
    match operation_parts.as_slice() {
        [_] => {}
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
    }
    // The terminal Operation content is projected by the durable
    // `MessagePartCheckpointed` emitted by `apply_tool_success*` before this
    // event is appended; replaying `tool_call_completed` only re-validates the
    // operation binding and advances the message projection timestamp.
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
    ) -> Result<Option<model_projection_state::Model>, DbErr> {
        model_projection_state::Entity::find_by_id(session_id)
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
                project_execution_started(
                    db,
                    part_writer,
                    payload,
                    event.meta.seq_session.unwrap_or(event.meta.seq_global),
                )
                .await?;
            }
            EventKind::CompactionCompleted(payload) => {
                ensure_projection_session(session_id, payload.session_id, "compaction_completed")?;
                project_compaction_completed(
                    db,
                    part_writer,
                    payload,
                    event.meta.seq_session.unwrap_or(event.meta.seq_global),
                )
                .await?;
            }
            EventKind::UserMessageAppended(payload) => {
                let metadata = source_if_missing(payload.metadata.clone(), MessageSource::User);
                part_writer
                    .upsert_message(
                        db,
                        model_message::Model {
                            message_id: payload.message_id.raw(),
                            session_id,
                            model_turn_id: metadata.model_turn_id,
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
                    project_part_content(
                        db,
                        payload.execution_id,
                        Role::User,
                        part,
                        event.meta.seq_session.unwrap_or(event.meta.seq_global),
                    )
                    .await?;
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
                let created_at_ms = payload.created_at.timestamp_millis();
                part_writer
                    .upsert_message(
                        db,
                        model_message::Model {
                            message_id: payload.message_id.raw(),
                            session_id,
                            model_turn_id: metadata.model_turn_id,
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
                    project_part_content(
                        db,
                        payload.execution_id,
                        Role::Assistant,
                        part,
                        event.meta.seq_session.unwrap_or(event.meta.seq_global),
                    )
                    .await?;
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
                update_tool_result_projection(
                    db,
                    part_writer,
                    session_id,
                    payload,
                    event.meta.seq_session.unwrap_or(event.meta.seq_global),
                )
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
                let effective_execution_id =
                    apply_message_part_update_on_connection(db, part_writer, update)
                        .await
                        .map_err(|err| {
                            DbErr::Custom(format!(
                                "project part update {} for message {}: {err}",
                                update.part.id, update.message_id
                            ))
                        })?;
                if let Some(execution_id) = effective_execution_id {
                    project_part_content(
                        db,
                        execution_id,
                        update.message_role,
                        &update.part,
                        event.meta.seq_session.unwrap_or(event.meta.seq_global),
                    )
                    .await?;
                }
            }
            EventKind::RunAborted(payload) => {
                let status = match payload.reason {
                    RunAbortReason::UserCancelled => ExecutionStatus::Cancelled,
                    RunAbortReason::Replaced | RunAbortReason::BudgetLimited => {
                        ExecutionStatus::Cancelled
                    }
                    RunAbortReason::ProcessRestart
                    | RunAbortReason::ProviderError
                    | RunAbortReason::Internal => ExecutionStatus::Failed,
                };
                part_writer
                    .terminalize_open_messages(
                        db,
                        session_id,
                        agena_storage::ModelMessageOpenIdentity::RunId(payload.run_id.to_string()),
                        status,
                        Utc::now().timestamp_millis(),
                    )
                    .await?;
            }
            EventKind::ExecutionFinished(payload) => {
                ensure_projection_session(session_id, payload.session_id, "execution_finished")?;
                let reply_waits_for_user = project_execution_finished(
                    db,
                    part_writer,
                    payload,
                    event.meta.seq_session.unwrap_or(event.meta.seq_global),
                )
                .await?;
                if reply_waits_for_user {
                    continue;
                }
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
                        agena_storage::ModelMessageOpenIdentity::ExecutionId(
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
) -> Result<Option<agena_domain::ExecutionId>, DbErr>
where
    C: ConnectionTrait,
    W: ProjectionPartWriter<C> + ProjectionMessageWriter<C>,
{
    let message_row = match model_message::Entity::find_by_id(update.message_id)
        .one(db)
        .await?
    {
        Some(row) => row,
        None => {
            let metadata = source_if_missing(
                update.message_metadata.clone(),
                role_default_source(update.message_role),
            );
            let row = model_message::Model {
                message_id: update.message_id,
                session_id: update.session_id,
                model_turn_id: metadata.model_turn_id,
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
            };
            part_writer.upsert_message(db, row.clone()).await?;
            row
        }
    };

    if message_row.model_turn_id != update.message_metadata.model_turn_id {
        return Err(DbErr::Custom(format!(
            "message {} checkpoint changed turn identity from {:?} to {:?}",
            update.message_id, message_row.model_turn_id, update.message_metadata.model_turn_id
        )));
    }

    let message_state_can_transition =
        ExecutionStatus::from(message_row.state).can_transition(update.message_state);
    if let Some(existing_part) = model_message_part::Entity::find_by_id(update.part.id)
        .one(db)
        .await?
        && !ExecutionStatus::from(existing_part.status).can_transition(update.part.status)
    {
        return Ok(None);
    }

    let effective_execution_id = match update.execution_id.map(Ok).or_else(|| {
        message_row.execution_id.as_deref().map(|value| {
            uuid::Uuid::parse_str(value)
                .map(agena_domain::ExecutionId)
                .map_err(|error| {
                    DbErr::Custom(format!(
                        "message {} has invalid execution identity {value}: {error}",
                        update.message_id
                    ))
                })
        })
    }) {
        Some(result) => Some(result?),
        None => None,
    };

    // A forked session rewrites message identities but keeps the original
    // `created_at`; the first checkpoint of a forked copy can therefore carry
    // a part whose `created_at` predates the fork. The projection must still
    // be able to attach it. Only the session owner matters for part
    // identity, so treat any timestamp as acceptable.
    if let Err(err) = part_writer
        .upsert_part(db, update.session_id, &update.part)
        .await
    {
        let reconcile_fork_copy = err.to_string().contains("cannot attach part")
            && model_message_part::Entity::find_by_id(update.part.id)
                .one(db)
                .await?
                .is_some_and(|existing| {
                    existing.message_id == update.part.message_id
                        && existing.kind == update.part.kind.into()
                        && existing.activity_id == update.part.activity_id.map(|id| id.to_string())
                        && existing.segment_id == update.part.segment_id.map(|id| id.to_string())
                });
        if !reconcile_fork_copy {
            return Err(err);
        }
        // The part already exists in the owning session with the same
        // identity; a forked copy raced the original. A replayed
        // checkpoint of the fork must not fail the whole projection.
    }

    let mut updated = message_row;
    if let Some(execution_id) = update.execution_id {
        updated.execution_id = Some(execution_id.to_string());
    }
    if let Some(run_id) = update.run_id {
        updated.run_id = Some(run_id.to_string());
    }
    if message_state_can_transition {
        updated.state = update.message_state.into();
    }
    updated.updated_at_ms = update.ts_ms;
    updated.part_count = count_parts_for_message(db, update.message_id).await? as i64;
    part_writer.upsert_message(db, updated).await?;
    Ok(effective_execution_id)
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
        model_message::Entity::find().filter(model_message::Column::SessionId.eq(session_id));
    query = match identity {
        "run_id" => query.filter(model_message::Column::RunId.eq(value)),
        "execution_id" => query.filter(model_message::Column::ExecutionId.eq(value)),
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
            let mut active: model_message::ActiveModel = message.into();
            active.state = ActiveValue::Set(status.into());
            active.updated_at_ms = ActiveValue::Set(Utc::now().timestamp_millis());
            active.update(db).await?;
        }

        // Execution-owned parts close with the execution. Interactive request
        // Activities deliberately remain pending until a later user reply.
        model_message_part::Entity::update_many()
            .col_expr(
                model_message_part::Column::Status,
                Expr::value(StoredExecutionStatus::from(status)),
            )
            .filter(model_message_part::Column::MessageId.eq(message_id))
            .filter(model_message_part::Column::Status.is_in([
                StoredExecutionStatus::Pending,
                StoredExecutionStatus::InProgress,
            ]))
            .filter(model_message_part::Column::AwaitsUserReply.eq(false))
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
    model_message::Entity::delete_many()
        .filter(model_message::Column::SessionId.eq(session_id))
        .exec(db)
        .await?;
    model_projection_state::Entity::delete_by_id(session_id)
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
        "INSERT INTO agena_model_projection_states \
         (session_id, last_seq_global, updated_at_ms) \
         VALUES (?, ?, ?) \
         ON CONFLICT(session_id) DO UPDATE SET \
         last_seq_global = MAX(agena_model_projection_states.last_seq_global, excluded.last_seq_global), \
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
