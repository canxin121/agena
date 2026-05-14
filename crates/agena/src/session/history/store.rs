use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter,
};

use crate::db::entities::session_snapshot;
use crate::event::{
    DomainEvent, EventFilter, EventKind, EventPublisher, PublishContext, Scope, StoreRange,
};
use crate::message::Message;
use crate::session::SessionRuntimeState;

use super::{
    SessionView, SessionViewBuilder, TurnAbortReason, TurnAborted, TurnId, TurnStarted,
    fold_history,
};

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
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LoadedSessionProjection {
    pub messages: Vec<Message>,
    pub runtime: SessionRuntimeState,
    #[allow(dead_code)]
    pub last_seq: i64,
}
