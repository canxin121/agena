use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr,
    EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
};

use crate::db::entities::{activity_message, activity_part, session_snapshot};
use crate::event::{
    DomainEvent, EventFilter, EventKind, EventPublisher, MessagePartUpdatedEvent, PublishContext,
    Scope, StoreRange,
};
use crate::message::{Message, MessagePart, MessageSource};
use crate::session::SessionRuntimeState;

use super::{
    FinishReason, MessageRevised, RevisionKind, SessionView, SessionViewBuilder, SystemNoticeKind,
    TurnAbortReason, TurnAborted, TurnId, TurnStarted,
    fold_history,
};
use crate::role::Role;

/// Persisted form of a [`SessionView`] snapshot. Only the fields needed to
/// reconstruct a `LoadedSessionProjection` participate; runtime state is
/// authoritative on `agena_sessions.runtime_state_json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SnapshotPayload {
    messages: Vec<Message>,
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
        // Try the snapshot fast path first: if a snapshot exists, fold only
        // the events that landed *after* `last_seq` against the saved view.
        let snapshot = self.load_snapshot(session_id).await.ok().flatten();

        let after_seq = snapshot.as_ref().map(|s| s.last_seq).unwrap_or(0);
        let mut events = self
            .list_session_events_after(session_id, after_seq)
            .await?;
        let aborted = self.abort_hanging_turns(session_id, &events).await?;
        events.extend(aborted);

        let view: SessionView = match snapshot {
            Some(model) => {
                // Fold the tail against the cached message list. The fold is
                // idempotent on top of the materialised messages because
                // every projection event we replay either adds a new
                // message or updates one already present.
                let payload: SnapshotPayload = serde_json::from_value(model.view)
                    .map_err(|err| DbErr::Custom(format!("decode session snapshot: {err}")))?;
                let tail: SessionView = fold_history::<SessionViewBuilder>(events.as_slice())
                    .map_err(|err| DbErr::Custom(format!("session view fold failed: {err}")))?
                    .map_err(|err| DbErr::Custom(format!("session view fold failed: {err}")))?;
                let mut messages = payload.messages;
                // Tail messages override / extend snapshot messages keyed by id.
                for message in tail.messages {
                    if let Some(slot) = messages.iter_mut().find(|m| m.id == message.id) {
                        *slot = message;
                    } else {
                        messages.push(message);
                    }
                }
                SessionView {
                    messages,
                    last_seq: tail.last_seq.max(model.last_seq),
                }
            }
            None => fold_history::<SessionViewBuilder>(events.as_slice())
                .map_err(|err| DbErr::Custom(format!("session view fold failed: {err}")))?
                .map_err(|err| DbErr::Custom(format!("session view fold failed: {err}")))?,
        };

        // Best-effort snapshot write so the next load can take the fast path.
        // Failures are logged but never propagated — losing a snapshot is
        // free; the next load just re-folds.
        let snapshot_payload = SnapshotPayload {
            messages: view.messages.clone(),
        };
        if let Err(err) = self
            .write_snapshot(session_id, view.last_seq, &snapshot_payload)
            .await
        {
            tracing::warn!(
                error = %err,
                session_id,
                last_seq = view.last_seq,
                "failed to persist session view snapshot"
            );
        }

        Ok(LoadedSessionProjection {
            messages: view.messages,
            runtime: base_runtime,
            last_seq: view.last_seq,
        })
    }

    pub(crate) async fn list_projected_messages(
        &self,
        session_id: i64,
        include_full_parts: bool,
    ) -> Result<Vec<Message>, DbErr> {
        let message_rows = activity_message::Entity::find()
            .filter(activity_message::Column::SessionId.eq(session_id))
            .filter(activity_message::Column::IsCompacted.eq(false))
            .order_by_asc(activity_message::Column::CreatedAtMs)
            .order_by_asc(activity_message::Column::MessageId)
            .all(&self.db)
            .await?;

        if message_rows.is_empty() {
            return Ok(Vec::new());
        }

        let message_ids = message_rows
            .iter()
            .map(|row| row.message_id)
            .collect::<Vec<_>>();
        let part_rows = activity_part::Entity::find()
            .filter(activity_part::Column::MessageId.is_in(message_ids))
            .order_by_asc(activity_part::Column::MessageId)
            .order_by_asc(activity_part::Column::PartIndex)
            .all(&self.db)
            .await?;

        let mut parts_by_message =
            std::collections::BTreeMap::<i64, Vec<MessagePart>>::new();
        for row in part_rows {
            let part = MessagePart {
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
                content: if include_full_parts { row.content } else { None },
            };
            parts_by_message.entry(row.message_id).or_default().push(part);
        }

        message_rows
            .into_iter()
            .map(|row| {
                Ok(Message {
                    id: row.message_id,
                    role: row.role,
                    state: row.state,
                    parts: parts_by_message.remove(&row.message_id).unwrap_or_default(),
                    created_at: timestamp_millis_to_utc(row.created_at_ms)?,
                    metadata: row.metadata,
                    usage: row.usage,
                    finish: row.finish,
                })
            })
            .collect()
    }

    pub(crate) async fn find_projected_message(
        &self,
        session_id: i64,
        message_id: i64,
        include_full_parts: bool,
    ) -> Result<Option<Message>, DbErr> {
        let messages = self
            .list_projected_messages(session_id, include_full_parts)
            .await?;
        Ok(messages.into_iter().find(|message| message.id == message_id))
    }

    pub(crate) async fn list_projected_parts(
        &self,
        message_id: i64,
        include_full_parts: bool,
    ) -> Result<Vec<MessagePart>, DbErr> {
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
                    content: if include_full_parts { row.content } else { None },
                })
            })
            .collect()
    }

    pub(crate) async fn find_projected_part(&self, part_id: i64) -> Result<Option<MessagePart>, DbErr> {
        let row = activity_part::Entity::find_by_id(part_id).one(&self.db).await?;
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

    pub(crate) async fn find_session_id_for_part(&self, part_id: i64) -> Result<Option<i64>, DbErr> {
        let row = activity_part::Entity::find_by_id(part_id).one(&self.db).await?;
        Ok(row.map(|row| row.session_id))
    }

    async fn load_snapshot(
        &self,
        session_id: i64,
    ) -> Result<Option<session_snapshot::Model>, DbErr> {
        session_snapshot::Entity::find()
            .filter(session_snapshot::Column::SessionId.eq(session_id))
            .one(&self.db)
            .await
    }

    async fn write_snapshot(
        &self,
        session_id: i64,
        last_seq: i64,
        payload: &SnapshotPayload,
    ) -> Result<(), DbErr> {
        let view_json = serde_json::to_value(payload)
            .map_err(|err| DbErr::Custom(format!("encode session snapshot: {err}")))?;
        let now_ms = Utc::now().timestamp_millis();
        let backend = self.db.get_database_backend();

        // Try INSERT first. If a row already exists, fall back to a
        // monotonically-guarded UPDATE: only overwrite when our `last_seq`
        // is strictly newer than the persisted one. Two concurrent loads
        // can both reach this point — the older fold would otherwise stomp
        // on a newer fold and silently demote the cached projection.
        let active = session_snapshot::ActiveModel {
            session_id: ActiveValue::Set(session_id),
            last_seq: ActiveValue::Set(last_seq),
            view: ActiveValue::Set(view_json.clone()),
            updated_at_ms: ActiveValue::Set(now_ms),
        };
        if session_snapshot::Entity::insert(active)
            .exec(&self.db)
            .await
            .is_ok()
        {
            return Ok(());
        }

        use sea_orm::Statement;
        let stmt = Statement::from_sql_and_values(
            backend,
            "UPDATE agena_session_snapshots \
             SET last_seq = ?, view_json = ?, updated_at_ms = ? \
             WHERE session_id = ? AND last_seq < ?",
            [
                last_seq.into(),
                view_json.into(),
                now_ms.into(),
                session_id.into(),
                last_seq.into(),
            ],
        );
        // rows_affected == 0 just means a newer snapshot already won; that
        // is the desired no-op outcome, not an error.
        self.db.execute(stmt).await.map(|_| ())
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

    /// Persist a synthetic `TurnAborted { ProcessRestart }` for any
    /// `TurnStarted` that lacks a matching `TurnCompleted` / `TurnAborted` in
    /// `events`. Returns the freshly published events so the caller can fold
    /// them into the view in one pass.
    async fn abort_hanging_turns(
        &self,
        session_id: i64,
        events: &[DomainEvent],
    ) -> Result<Vec<DomainEvent>, DbErr> {
        use std::collections::HashSet;
        let mut started: HashSet<TurnId> = HashSet::new();
        for event in events {
            match &event.kind {
                EventKind::TurnStarted(TurnStarted { turn_id, .. }) => {
                    started.insert(*turn_id);
                }
                EventKind::TurnCompleted(payload) => {
                    started.remove(&payload.turn_id);
                }
                EventKind::TurnAborted(payload) => {
                    started.remove(&payload.turn_id);
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
            .map(|turn_id| {
                let kind = EventKind::TurnAborted(TurnAborted {
                    turn_id,
                    reason: TurnAbortReason::ProcessRestart,
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
        self.apply_projection_events(session_id, built.as_slice()).await?;
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
        self.apply_projection_events(session_id, built.as_slice()).await?;
        Ok(built)
    }

    /// Drop the cached projection snapshot for a session. Called after
    /// timeline-mutating operations (rewind/unrewind/fork-copy) so the next
    /// `load_projection` re-folds from the authoritative event log.
    pub(crate) async fn invalidate_snapshot(&self, session_id: i64) -> Result<(), DbErr> {
        session_snapshot::Entity::delete_many()
            .filter(session_snapshot::Column::SessionId.eq(session_id))
            .exec(&self.db)
            .await
            .map(|_| ())
    }

    pub(crate) async fn apply_message_part_update(
        &self,
        update: &MessagePartUpdatedEvent,
    ) -> Result<(), DbErr> {
        upsert_part_projection(&self.db, update.session_id, &update.part).await?;

        let Some(message_row) = activity_message::Entity::find_by_id(update.message_id)
            .one(&self.db)
            .await?
        else {
            return Ok(());
        };
        let mut active: activity_message::ActiveModel = message_row.into();
        active.state = ActiveValue::Set(update.message_state);
        active.updated_at_ms = ActiveValue::Set(update.ts_ms);
        active.part_count = ActiveValue::Set(
            count_parts_for_message(&self.db, update.message_id).await? as i64,
        );
        active.update(&self.db).await?;
        Ok(())
    }

    async fn apply_projection_events(
        &self,
        session_id: i64,
        events: &[DomainEvent],
    ) -> Result<(), DbErr> {
        for event in events {
            match &event.kind {
                EventKind::UserMessageAppended(payload) => {
                    let metadata = with_source_if_missing(
                        payload.metadata.clone(),
                        MessageSource::User,
                    );
                    upsert_message_projection(
                        &self.db,
                        activity_message::Model {
                            message_id: payload.message_id.raw(),
                            session_id,
                            role: Role::User,
                            state: crate::message::ExecutionStatus::Completed,
                            created_at_ms: payload.created_at.timestamp_millis(),
                            updated_at_ms: payload.created_at.timestamp_millis(),
                            metadata,
                            usage: None,
                            finish: None,
                            part_count: payload.parts.len() as i64,
                            is_compacted: false,
                        },
                    )
                    .await?;
                    for part in &payload.parts {
                        upsert_part_projection(&self.db, session_id, part).await?;
                    }
                }
                EventKind::AssistantMessageCompleted(payload) => {
                    let metadata = with_source_if_missing(
                        payload.metadata.clone(),
                        MessageSource::Assistant,
                    );
                    upsert_message_projection(
                        &self.db,
                        activity_message::Model {
                            message_id: payload.message_id.raw(),
                            session_id,
                            role: Role::Assistant,
                            state: crate::message::ExecutionStatus::Completed,
                            created_at_ms: payload.created_at.timestamp_millis(),
                            updated_at_ms: payload.created_at.timestamp_millis(),
                            metadata,
                            usage: payload.usage.clone(),
                            finish: finish_reason_label(payload.finish_reason),
                            part_count: payload.parts.len() as i64,
                            is_compacted: false,
                        },
                    )
                    .await?;
                    for part in &payload.parts {
                        upsert_part_projection(&self.db, session_id, part).await?;
                    }
                }
                EventKind::ToolCallCompleted(payload) => {
                    let synthetic_part = project_tool_result_part(payload)?;
                    upsert_message_projection(
                        &self.db,
                        activity_message::Model {
                            message_id: payload.message_id.raw(),
                            session_id,
                            role: Role::Tool,
                            state: crate::message::ExecutionStatus::Completed,
                            created_at_ms: payload.completed_at.timestamp_millis(),
                            updated_at_ms: payload.completed_at.timestamp_millis(),
                            metadata: Default::default(),
                            usage: None,
                            finish: None,
                            part_count: 1,
                            is_compacted: false,
                        },
                    )
                    .await?;
                    upsert_part_projection(&self.db, session_id, &synthetic_part).await?;
                }
                EventKind::SystemNoticeAppended(payload) => {
                    let synthetic_part = project_system_notice_part(payload);
                    upsert_message_projection(
                        &self.db,
                        activity_message::Model {
                            message_id: payload.message_id.raw(),
                            session_id,
                            role: Role::System,
                            state: crate::message::ExecutionStatus::Completed,
                            created_at_ms: payload.created_at.timestamp_millis(),
                            updated_at_ms: payload.created_at.timestamp_millis(),
                            metadata: Default::default(),
                            usage: None,
                            finish: None,
                            part_count: 1,
                            is_compacted: matches!(payload.kind, SystemNoticeKind::RewindCheckpoint)
                                .then_some(true)
                                .unwrap_or(false),
                        },
                    )
                    .await?;
                    if !matches!(payload.kind, SystemNoticeKind::RewindCheckpoint) {
                        upsert_part_projection(&self.db, session_id, &synthetic_part).await?;
                    }
                }
                EventKind::MessageRevised(MessageRevised {
                    target_message_id,
                    kind,
                }) => {
                    if let Some(row) = activity_message::Entity::find_by_id(*target_message_id)
                        .one(&self.db)
                        .await?
                    {
                        let mut active: activity_message::ActiveModel = row.into();
                        match kind {
                            RevisionKind::Compacted => {
                                active.is_compacted = ActiveValue::Set(true);
                            }
                            RevisionKind::Uncompacted => {
                                active.is_compacted = ActiveValue::Set(false);
                            }
                            RevisionKind::ToolResultPruned { .. }
                            | RevisionKind::AttachmentStripped { .. } => {}
                        }
                        active.updated_at_ms =
                            ActiveValue::Set(event.meta.created_at.timestamp_millis());
                        active.update(&self.db).await?;
                    }
                }
                EventKind::MessagePartUpdated(update) => {
                    self.apply_message_part_update(update).await?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LoadedSessionProjection {
    pub messages: Vec<Message>,
    pub runtime: SessionRuntimeState,
    #[allow(dead_code)]
    pub last_seq: i64,
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<chrono::DateTime<Utc>, DbErr> {
    chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| DbErr::Custom(format!("timestamp out of range: {timestamp_ms}")))
}

fn finish_reason_label(reason: FinishReason) -> Option<String> {
    Some(match reason {
        FinishReason::Stop => "stop",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::MaxTokens => "max_tokens",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Error => "error",
        FinishReason::Other => "other",
    }
    .to_string())
}

fn with_source_if_missing(
    mut metadata: crate::message::MessageMetadata,
    source: MessageSource,
) -> crate::message::MessageMetadata {
    metadata.source = source;
    metadata
}

async fn upsert_message_projection(
    db: &DatabaseConnection,
    row: activity_message::Model,
) -> Result<(), DbErr> {
    if let Some(existing) = activity_message::Entity::find_by_id(row.message_id)
        .one(db)
        .await?
    {
        let mut active: activity_message::ActiveModel = existing.into();
        active.session_id = ActiveValue::Set(row.session_id);
        active.role = ActiveValue::Set(row.role);
        active.state = ActiveValue::Set(row.state);
        active.created_at_ms = ActiveValue::Set(row.created_at_ms);
        active.updated_at_ms = ActiveValue::Set(row.updated_at_ms);
        active.metadata = ActiveValue::Set(row.metadata);
        active.usage = ActiveValue::Set(row.usage);
        active.finish = ActiveValue::Set(row.finish);
        active.part_count = ActiveValue::Set(row.part_count);
        active.is_compacted = ActiveValue::Set(row.is_compacted);
        active.update(db).await?;
        return Ok(());
    }

    activity_message::ActiveModel {
        message_id: ActiveValue::Set(row.message_id),
        session_id: ActiveValue::Set(row.session_id),
        role: ActiveValue::Set(row.role),
        state: ActiveValue::Set(row.state),
        created_at_ms: ActiveValue::Set(row.created_at_ms),
        updated_at_ms: ActiveValue::Set(row.updated_at_ms),
        metadata: ActiveValue::Set(row.metadata),
        usage: ActiveValue::Set(row.usage),
        finish: ActiveValue::Set(row.finish),
        part_count: ActiveValue::Set(row.part_count),
        is_compacted: ActiveValue::Set(row.is_compacted),
    }
    .insert(db)
    .await?;
    Ok(())
}

async fn upsert_part_projection(
    db: &DatabaseConnection,
    session_id: i64,
    part: &MessagePart,
) -> Result<(), DbErr> {
    if let Some(existing) = activity_part::Entity::find_by_id(part.id).one(db).await? {
        let mut active: activity_part::ActiveModel = existing.into();
        active.message_id = ActiveValue::Set(part.message_id);
        active.session_id = ActiveValue::Set(session_id);
        active.part_index = ActiveValue::Set(part.part_index);
        active.status = ActiveValue::Set(part.status);
        active.kind = ActiveValue::Set(part.kind);
        active.name = ActiveValue::Set(part.name.clone());
        active.summary = ActiveValue::Set(part.summary.clone());
        active.has_detail = ActiveValue::Set(part.has_detail);
        active.operation_id = ActiveValue::Set(part.operation_id.clone());
        active.created_at_ms = ActiveValue::Set(part.created_at.timestamp_millis());
        active.content = ActiveValue::Set(part.content.clone());
        active.update(db).await?;
        return Ok(());
    }

    activity_part::ActiveModel {
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
    }
    .insert(db)
    .await?;
    Ok(())
}

async fn count_parts_for_message(db: &DatabaseConnection, message_id: i64) -> Result<u64, DbErr> {
    Ok(activity_part::Entity::find()
        .filter(activity_part::Column::MessageId.eq(message_id))
        .count(db)
        .await?)
}

fn project_system_notice_part(
    payload: &super::SystemNoticeAppended,
) -> MessagePart {
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

fn project_tool_result_part(
    payload: &super::ToolCallCompleted,
) -> Result<MessagePart, DbErr> {
    let summary = match &payload.output {
        super::transcript::TranscriptToolOutput::Text { text } => text.clone(),
        super::transcript::TranscriptToolOutput::Pruned { replacement } => replacement.clone(),
        super::transcript::TranscriptToolOutput::Error { message } => message.clone(),
    };
    let mut part = MessagePart::with_content(
        payload.message_id.raw(),
        payload.message_id.raw(),
        payload.completed_at,
        crate::message::ExecutionStatus::Completed,
        crate::message::PartContent::text(summary),
    );
    part.part_index = 0;
    part.operation_id = Some(payload.call_id.as_str().to_owned());
    Ok(part)
}
