use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::DbErr;

use crate::event::{DomainEvent, EventKind, EventPublisher, PublishContext};
use crate::message::Message;
use crate::session::SessionRuntimeState;

use super::{
    SessionView, SessionViewBuilder, TurnAbortReason, TurnAborted, TurnId, TurnStarted, fold_history,
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
        self.repair_hanging_turns(session_id).await?;
        let events = self.list_session_events(session_id).await?;

        let view: SessionView = fold_history::<SessionViewBuilder>(events.as_slice())
            .map_err(|err| DbErr::Custom(format!("session view fold failed: {err}")))?
            .map_err(|err| DbErr::Custom(format!("session view fold failed: {err}")))?;

        let mut runtime = view.runtime;
        if runtime == SessionRuntimeState::default() {
            runtime = base_runtime;
        }
        Ok(LoadedSessionProjection {
            messages: view.messages,
            runtime,
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

    /// Scan the tail of the history for any `TurnStarted(t)` without a
    /// matching `TurnCompleted(t)` / `TurnAborted(t)`. Append a synthetic
    /// `TurnAborted { reason: ProcessRestart }` for each — the
    /// `SessionViewBuilder` will then drop the dangling messages.
    async fn repair_hanging_turns(&self, session_id: i64) -> Result<(), DbErr> {
        use std::collections::HashSet;
        let events = self.list_session_events(session_id).await?;
        let mut started: HashSet<TurnId> = HashSet::new();
        for event in &events {
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
            return Ok(());
        }
        for turn_id in started {
            let kind = EventKind::TurnAborted(TurnAborted {
                turn_id,
                reason: TurnAbortReason::ProcessRestart,
                message: Some("process restart detected on session load".to_string()),
            });
            let ctx = PublishContext::for_session(session_id);
            self.publisher
                .publish(ctx, kind)
                .await
                .map_err(|err| DbErr::Custom(format!("publish abort failed: {err}")))?;
        }
        Ok(())
    }

    /// Append a batch of events for a session. Each entry is published
    /// through the unified [`EventPublisher`].
    pub(crate) async fn append_items(
        &self,
        session_id: i64,
        kinds: Vec<EventKind>,
        _now: DateTime<Utc>,
    ) -> Result<Vec<DomainEvent>, DbErr> {
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(kinds.len());
        for kind in kinds {
            let ctx = PublishContext::for_session(session_id);
            let event = self
                .publisher
                .publish(ctx, kind)
                .await
                .map_err(|err| DbErr::Custom(format!("publish history item failed: {err}")))?;
            out.push(event);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LoadedSessionProjection {
    pub messages: Vec<Message>,
    pub runtime: SessionRuntimeState,
    #[allow(dead_code)]
    pub last_seq: i64,
}
