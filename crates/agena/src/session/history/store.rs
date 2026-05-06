use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::DbErr;

use crate::event::{DomainEvent, EventKind, EventPublisher, PublishContext};
use crate::message::Message;
use crate::session::SessionRuntimeState;

use super::{
    SessionView, SessionViewBuilder, TurnAbortReason, TurnAborted, TurnId, TurnStarted,
    fold_history,
};

#[derive(Clone)]
pub(crate) struct SessionHistoryStore {
    publisher: Arc<EventPublisher>,
}

impl SessionHistoryStore {
    pub(crate) fn new(publisher: Arc<EventPublisher>) -> Self {
        Self { publisher }
    }

    pub(crate) async fn load_projection(
        &self,
        session_id: i64,
        base_runtime: SessionRuntimeState,
    ) -> Result<LoadedSessionProjection, DbErr> {
        let mut events = self.list_session_events(session_id).await?;
        let aborted = self
            .abort_hanging_turns(session_id, &events)
            .await?;
        events.extend(aborted);

        let view: SessionView = fold_history::<SessionViewBuilder>(events.as_slice())
            .map_err(|err| DbErr::Custom(format!("session view fold failed: {err}")))?
            .map_err(|err| DbErr::Custom(format!("session view fold failed: {err}")))?;

        Ok(LoadedSessionProjection {
            messages: view.messages,
            runtime: base_runtime,
            last_seq: view.last_seq,
        })
    }

    pub(crate) async fn list_session_events(
        &self,
        session_id: i64,
    ) -> Result<Vec<DomainEvent>, DbErr> {
        use crate::event::{EventFilter, Scope, StoreRange};
        let filter = EventFilter::new(Scope::Session { session_id });
        let mut all = Vec::new();
        let mut cursor = 0i64;
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

    /// Scan a previously loaded event slice for any `TurnStarted(t)` without a
    /// matching `TurnCompleted(t)` / `TurnAborted(t)`. Append a synthetic
    /// `TurnAborted { reason: ProcessRestart }` for each — the
    /// `SessionViewBuilder` will then drop the dangling messages.
    ///
    /// Returns the newly published abort events so callers can extend their
    /// in-memory event slice without re-reading the table.
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
        let mut out = Vec::with_capacity(started.len());
        for turn_id in started {
            let kind = EventKind::TurnAborted(TurnAborted {
                turn_id,
                reason: TurnAbortReason::ProcessRestart,
                message: Some("process restart detected on session load".to_string()),
            });
            let ctx = PublishContext::for_session(session_id);
            let event = self
                .publisher
                .publish(ctx, kind)
                .await
                .map_err(|err| DbErr::Custom(format!("publish abort failed: {err}")))?;
            out.push(event);
        }
        Ok(out)
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
        self.publisher
            .publish_batch(built)
            .await
            .map_err(|err| DbErr::Custom(format!("publish history batch failed: {err}")))
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LoadedSessionProjection {
    pub messages: Vec<Message>,
    pub runtime: SessionRuntimeState,
    #[allow(dead_code)]
    pub last_seq: i64,
}
