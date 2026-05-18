use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection,
    DbErr, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    sea_query::OnConflict,
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
    TurnAbortReason, TurnAborted, TurnId, TurnStarted, fold_history,
};
use crate::role::Role;

/// Persisted form of a [`SessionView`] snapshot. Only the fields needed to
/// reconstruct a `LoadedSessionProjection` participate; runtime state is
/// authoritative on `agena_sessions.runtime_state_json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SnapshotPayload {
    messages: Vec<Message>,
}

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
    pub finish: Option<String>,
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
        let message_rows = self.list_projected_message_rows(session_id).await?;

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

    pub(crate) async fn list_projected_messages_page(
        &self,
        session_id: i64,
        include_full_parts: bool,
        cursor: Option<(i64, i64)>,
        limit: u64,
    ) -> Result<(Vec<Message>, bool, Option<(i64, i64)>), DbErr> {
        let (message_rows, has_more, next_cursor) = self
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

    pub(crate) async fn list_projected_message_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<ProjectedMessageHeader>, DbErr> {
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
        let row = activity_message::Entity::find_by_id(message_id)
            .filter(activity_message::Column::SessionId.eq(session_id))
            .filter(activity_message::Column::IsCompacted.eq(false))
            .one(&self.db)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let parts = self
            .list_projected_parts(message_id, include_full_parts)
            .await?;
        Ok(Some(projected_message_from_row(row, parts)?))
    }

    pub(crate) async fn find_projected_message_header(
        &self,
        session_id: i64,
        message_id: i64,
    ) -> Result<Option<ProjectedMessageHeader>, DbErr> {
        let row = activity_message::Entity::find_by_id(message_id)
            .filter(activity_message::Column::SessionId.eq(session_id))
            .filter(activity_message::Column::IsCompacted.eq(false))
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

    async fn load_snapshot(
        &self,
        session_id: i64,
    ) -> Result<Option<session_snapshot::Model>, DbErr> {
        session_snapshot::Entity::find()
            .filter(session_snapshot::Column::SessionId.eq(session_id))
            .one(&self.db)
            .await
    }

    async fn list_projected_message_rows(
        &self,
        session_id: i64,
    ) -> Result<Vec<activity_message::Model>, DbErr> {
        activity_message::Entity::find()
            .filter(activity_message::Column::SessionId.eq(session_id))
            .filter(activity_message::Column::IsCompacted.eq(false))
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
            .filter(activity_message::Column::IsCompacted.eq(false))
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

    pub(crate) async fn apply_message_part_update(
        &self,
        update: &MessagePartUpdatedEvent,
    ) -> Result<(), DbErr> {
        let Some(message_row) = activity_message::Entity::find_by_id(update.message_id)
            .one(&self.db)
            .await?
        else {
            return Ok(());
        };

        upsert_part_projection(&self.db, update.session_id, &update.part).await?;

        let mut active: activity_message::ActiveModel = message_row.into();
        active.state = ActiveValue::Set(update.message_state);
        active.updated_at_ms = ActiveValue::Set(update.ts_ms);
        active.part_count =
            ActiveValue::Set(count_parts_for_message(&self.db, update.message_id).await? as i64);
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
                    let metadata =
                        with_source_if_missing(payload.metadata.clone(), MessageSource::User);
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
                    .await
                    .map_err(|err| {
                        DbErr::Custom(format!(
                            "project user message {}: {err}",
                            payload.message_id.raw()
                        ))
                    })?;
                    for part in &payload.parts {
                        upsert_part_projection(&self.db, session_id, part)
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
                    .await
                    .map_err(|err| {
                        DbErr::Custom(format!(
                            "project assistant message {}: {err}",
                            payload.message_id.raw()
                        ))
                    })?;
                    for part in &payload.parts {
                        upsert_part_projection(&self.db, session_id, part)
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
                    project_tool_call_issued(&self.db, session_id, payload)
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
                    update_tool_result_projection(&self.db, session_id, payload)
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
                            is_compacted: matches!(
                                payload.kind,
                                SystemNoticeKind::RewindCheckpoint
                            )
                            .then_some(true)
                            .unwrap_or(false),
                        },
                    )
                    .await
                    .map_err(|err| {
                        DbErr::Custom(format!(
                            "project system notice {}: {err}",
                            payload.message_id.raw()
                        ))
                    })?;
                    if !matches!(payload.kind, SystemNoticeKind::RewindCheckpoint) {
                        upsert_part_projection(&self.db, session_id, &synthetic_part)
                            .await
                            .map_err(|err| {
                                DbErr::Custom(format!(
                                    "project system notice part {} for message {}: {err}",
                                    synthetic_part.id, synthetic_part.message_id
                                ))
                            })?;
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
                    self.apply_message_part_update(update)
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
        usage: row.usage,
        finish: row.finish,
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
        finish: row.finish,
        part_count,
    })
}

fn finish_reason_label(reason: FinishReason) -> Option<String> {
    Some(
        match reason {
            FinishReason::Stop => "stop",
            FinishReason::ToolCalls => "tool_calls",
            FinishReason::MaxTokens => "max_tokens",
            FinishReason::ContentFilter => "content_filter",
            FinishReason::Error => "error",
            FinishReason::Other => "other",
        }
        .to_string(),
    )
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
         (message_id, session_id, role, state, created_at_ms, updated_at_ms, metadata, usage, finish, part_count, is_compacted) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(message_id) DO UPDATE SET \
         session_id = excluded.session_id, role = excluded.role, state = excluded.state, \
         created_at_ms = excluded.created_at_ms, updated_at_ms = excluded.updated_at_ms, \
         metadata = excluded.metadata, usage = excluded.usage, finish = excluded.finish, \
         part_count = excluded.part_count, is_compacted = excluded.is_compacted",
        [
            row.message_id.into(),
            row.session_id.into(),
            role_db_value(row.role).into(),
            execution_status_db_value(row.state).into(),
            row.created_at_ms.into(),
            row.updated_at_ms.into(),
            metadata,
            usage,
            row.finish.into(),
            row.part_count.into(),
            row.is_compacted.into(),
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

async fn upsert_part_projection(
    db: &DatabaseConnection,
    session_id: i64,
    part: &MessagePart,
) -> Result<(), DbErr> {
    if let Some(operation_id) = part.operation_id.as_deref() {
        let existing = activity_part::Entity::find()
            .select_only()
            .column(activity_part::Column::PartId)
            .filter(activity_part::Column::MessageId.eq(part.message_id))
            .filter(activity_part::Column::OperationId.eq(operation_id))
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

async fn count_parts_for_message(db: &DatabaseConnection, message_id: i64) -> Result<u64, DbErr> {
    Ok(activity_part::Entity::find()
        .filter(activity_part::Column::MessageId.eq(message_id))
        .count(db)
        .await?)
}

async fn touch_message_projection(
    db: &DatabaseConnection,
    message_id: i64,
    updated_at_ms: i64,
) -> Result<(), DbErr> {
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

async fn project_tool_call_issued(
    db: &DatabaseConnection,
    session_id: i64,
    payload: &super::ToolCallIssued,
) -> Result<(), DbErr> {
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

async fn update_tool_result_projection(
    db: &DatabaseConnection,
    session_id: i64,
    payload: &super::ToolCallCompleted,
) -> Result<(), DbErr> {
    let existing = activity_part::Entity::find()
        .filter(activity_part::Column::MessageId.eq(payload.message_id.raw()))
        .filter(activity_part::Column::OperationId.eq(payload.call_id.as_str()))
        .one(db)
        .await?;

    let output_text = match &payload.output {
        super::transcript::TranscriptToolOutput::Text { text } => text.clone(),
        super::transcript::TranscriptToolOutput::Pruned { replacement } => replacement.clone(),
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
        super::transcript::TranscriptToolOutput::Text { .. }
        | super::transcript::TranscriptToolOutput::Pruned { .. } => {
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
        super::transcript::TranscriptToolOutput::Text { .. }
        | super::transcript::TranscriptToolOutput::Pruned { .. } => {
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

fn synthetic_tool_call_part_id(message_id: i64, call_id: &str) -> i64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in message_id.to_le_bytes().iter().chain(call_id.as_bytes()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    -((hash & 0x0000_3fff_ffff_ffff) as i64) - 1_000_000
}
