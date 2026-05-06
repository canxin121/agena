use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use sea_orm::{DatabaseConnection, DbErr};
use tokio::sync::{Mutex as AsyncMutex, OnceCell};

use crate::{
    AppError,
    db::{
        crud::{permission_rule, session, workspace},
        tx::with_transaction_and_effects,
    },
    event::{DomainEvent, EventKind, EventPublisher, MessagePartUpdatedEvent, PublishContext},
    message::Message,
    permission::PermissionMode,
};

use super::{
    Session,
    cache::{SessionCache, SessionCachePolicy, SessionCacheStats},
    history::SessionHistoryStore,
    model::{SessionListRequest, SessionSummary},
};

pub(crate) struct SessionCommit {
    pub(crate) session: Session,
    pub(crate) touched_messages: Vec<Message>,
    pub(crate) client_events: Vec<EventKind>,
    pub(crate) persisted_rule: Option<(String, PermissionMode)>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessorPartIdAllocator {
    ids: Arc<AsyncMutex<GlobalIdAllocator>>,
}

impl ProcessorPartIdAllocator {
    fn new(ids: Arc<AsyncMutex<GlobalIdAllocator>>) -> Self {
        Self { ids }
    }

    pub(crate) async fn reserve(&self) -> Result<i64, AppError> {
        let mut allocator = self.ids.lock().await;
        if !allocator.initialized {
            return Err(AppError::Internal(
                "processor part allocator used before initialization".to_string(),
            ));
        }

        let part_id = allocator.next_part_id;
        allocator.next_part_id += 1;
        Ok(part_id)
    }

    #[cfg(test)]
    pub(crate) fn for_test(next_part_id: i64) -> Self {
        Self {
            ids: Arc::new(AsyncMutex::new(GlobalIdAllocator {
                initialized: true,
                next_message_id: 1,
                next_part_id,
            })),
        }
    }
}

pub(crate) struct SessionStore {
    db: DatabaseConnection,
    workspace_path: String,
    workspace_id: OnceCell<i64>,
    cache: Arc<Mutex<SessionCache>>,
    ids: Arc<AsyncMutex<GlobalIdAllocator>>,
    history: SessionHistoryStore,
    publisher: Arc<EventPublisher>,
}

impl SessionStore {
    pub(crate) fn new(
        db: DatabaseConnection,
        workspace_root: &Path,
        publisher: Arc<EventPublisher>,
    ) -> Self {
        Self {
            db: db.clone(),
            workspace_path: workspace_root.to_string_lossy().replace('\\', "/"),
            workspace_id: OnceCell::new(),
            cache: Arc::new(Mutex::new(SessionCache::default())),
            ids: Arc::new(AsyncMutex::new(GlobalIdAllocator::default())),
            history: SessionHistoryStore::new(Arc::clone(&publisher), db),
            publisher,
        }
    }

    /// Publish a single [`EventKind`] for the session. Best-effort: failures
    /// are surfaced as `AppError`.
    async fn publish_event(&self, session_id: i64, kind: EventKind) -> Result<(), AppError> {
        let ctx = PublishContext::for_session(session_id);
        self.publisher
            .publish(ctx, kind)
            .await
            .map_err(|err| AppError::Internal(format!("publish event failed: {err}")))?;
        Ok(())
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    pub(crate) async fn create_session(
        &self,
        title: String,
        parent_session_id: Option<i64>,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let workspace_id = self.workspace_id().await?;
        let cache = Arc::clone(&self.cache);
        let session = with_transaction_and_effects(&self.db, move |txn, effects| {
            let title = title.clone();
            let cache = Arc::clone(&cache);
            Box::pin(async move {
                let created =
                    session::create_session(txn, workspace_id, parent_session_id, title).await?;
                let session = session_from_model_db(created)?;

                let session_for_cache = session.clone();
                effects.push(async move {
                    with_cache(cache.as_ref(), |guard| {
                        guard.insert(session_for_cache, cache_policy);
                    });
                });

                Ok(session)
            })
        })
        .await?;

        Ok(session)
    }

    /// Read every event for a session from the unified store, in
    /// `seq_global` order.
    pub(crate) async fn list_session_events(
        &self,
        session_id: i64,
    ) -> Result<Vec<DomainEvent>, AppError> {
        Ok(self.history.list_session_events(session_id).await?)
    }

    /// Per-session message-count and last-message-time stats, computed by
    /// projecting the unified event log.
    pub(crate) async fn session_message_stats_for_ids(
        &self,
        session_ids: &[i64],
    ) -> Result<
        std::collections::HashMap<i64, crate::db::crud::session::SessionMessageStats>,
        AppError,
    > {
        let mut out = std::collections::HashMap::with_capacity(session_ids.len());
        for &session_id in session_ids {
            let events = self.history.list_session_events(session_id).await?;
            let view = super::history::fold_session_view(events.as_slice())
                .map_err(|err| AppError::Internal(format!("project session view: {err}")))?;
            let message_count = view.messages.len() as i64;
            let last_message_at_ms = view
                .messages
                .iter()
                .map(|m| m.created_at.timestamp_millis())
                .max();
            if message_count == 0 && last_message_at_ms.is_none() {
                continue;
            }
            out.insert(
                session_id,
                crate::db::crud::session::SessionMessageStats {
                    message_count,
                    last_message_at_ms,
                },
            );
        }
        Ok(out)
    }

    pub(crate) async fn list_workspace_session_ids(&self) -> Result<Vec<i64>, AppError> {
        let Some(workspace_id) = self.lookup_workspace_id().await? else {
            return Ok(Vec::new());
        };

        Ok(session::list_session_ids_by_workspace_id(&self.db, workspace_id).await?)
    }

    pub(crate) async fn list_session_summaries(
        &self,
        request: SessionListRequest,
    ) -> Result<Vec<SessionSummary>, AppError> {
        let Some(workspace_id) = self.lookup_workspace_id().await? else {
            return Ok(Vec::new());
        };

        let session_models =
            session::list_sessions_by_workspace_id_with_request(&self.db, workspace_id, request)
                .await?;
        if session_models.is_empty() {
            return Ok(Vec::new());
        }

        let session_ids = session_models
            .iter()
            .map(|model| model.id)
            .collect::<Vec<_>>();
        let message_stats = self.session_message_stats_for_ids(&session_ids).await?;
        let child_counts =
            session::child_session_counts_by_parent_ids(&self.db, session_ids.as_slice()).await?;

        session_models
            .into_iter()
            .map(|model| {
                let message_stats = message_stats.get(&model.id).copied();
                let message_count = message_stats
                    .map(|stats| u64::try_from(stats.message_count))
                    .transpose()
                    .map_err(|_| {
                        AppError::Internal(format!(
                            "invalid negative message count for session {}",
                            model.id
                        ))
                    })?
                    .unwrap_or_default();
                let child_session_count = child_counts
                    .get(&model.id)
                    .copied()
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| {
                        AppError::Internal(format!(
                            "invalid negative child session count for session {}",
                            model.id
                        ))
                    })?
                    .unwrap_or_default();
                let last_message_at = message_stats
                    .and_then(|stats| stats.last_message_at_ms)
                    .map(timestamp_millis_to_utc)
                    .transpose()?;

                Ok(SessionSummary {
                    id: model.id,
                    parent_id: model.parent_id,
                    workspace_id: model.workspace_id,
                    title: model.title,
                    version: model.version,
                    created_at: timestamp_millis_to_utc(model.created_at_ms)?,
                    updated_at: timestamp_millis_to_utc(model.updated_at_ms)?,
                    message_count,
                    child_session_count,
                    last_message_at,
                })
            })
            .collect()
    }

    pub(crate) async fn load_session(
        &self,
        session_id: i64,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        if let Some(session) = with_cache(self.cache.as_ref(), |guard| {
            guard.get(session_id, cache_policy)
        })
        .flatten()
        {
            return Ok(session);
        }

        let session_model = session::get_session_by_id(&self.db, session_id)
            .await?
            .ok_or_else(|| AppError::Internal(format!("session not found: {session_id}")))?;
        let mut session = session_from_model(session_model)?;
        let projection = self
            .history
            .load_projection(session_id, session.runtime.clone())
            .await?;
        session.replace_messages(projection.messages);
        session.runtime = projection.runtime;
        session.refresh_derived();

        with_cache(self.cache.as_ref(), |guard| {
            guard.insert(session.clone(), cache_policy);
        });

        Ok(session)
    }

    pub(crate) async fn fork_session(
        &self,
        source: Session,
        at_event_seq: i64,
        title: String,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let events = self.history.list_session_events(source.id).await?;
        if events.is_empty() {
            if at_event_seq != 0 {
                return Err(AppError::Internal(format!(
                    "event seq not found for session {}: {}",
                    source.id, at_event_seq
                )));
            }
            return self
                .create_session(title, Some(source.id), cache_policy)
                .await;
        }

        if !events
            .iter()
            .any(|event| event.meta.seq_global == at_event_seq)
        {
            return Err(AppError::Internal(format!(
                "event seq not found for session {}: {}",
                source.id, at_event_seq
            )));
        }

        let items = events
            .into_iter()
            .filter(|event| event.meta.seq_global <= at_event_seq && event.kind.is_persistent())
            .map(|event| event.kind)
            .collect::<Vec<_>>();

        let child = self
            .create_session(title, Some(source.id), cache_policy)
            .await?;
        self.append_history_items(child, items, cache_policy).await
    }

    pub(crate) async fn rewind_to_message(
        &self,
        session_id: i64,
        message_id: i64,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        // Validate that `message_id` belongs to this session by projecting
        // the current event log and checking membership.
        let pre_events = self.history.list_session_events(session_id).await?;
        let pre_view = super::history::fold_session_view(pre_events.as_slice())
            .map_err(|err| AppError::Internal(format!("failed to project session view: {err}")))?;
        if !pre_view.messages.iter().any(|m| m.id == message_id) {
            return Err(AppError::Internal(format!(
                "message not found for rewind: {message_id}"
            )));
        }

        // Compact every message that lives at or after the rewind target.
        // The transcript projector treats `Compacted` as "drop from prompt".
        let revisions: Vec<EventKind> = pre_view
            .messages
            .iter()
            .filter(|m| m.id >= message_id)
            .map(|m| {
                EventKind::MessageRevised(super::history::MessageRevised {
                    target_message_id: m.id,
                    kind: super::history::RevisionKind::Compacted,
                })
            })
            .collect();
        for kind in revisions {
            self.publish_event(session_id, kind).await?;
        }

        let new_runtime_base = session::get_session_by_id(&self.db, session_id)
            .await?
            .ok_or_else(|| AppError::Internal(format!("session not found: {session_id}")))?
            .runtime_state
            .unwrap_or_default();
        let next_runtime = rewind_runtime_state(new_runtime_base);

        let cache = Arc::clone(&self.cache);
        let session = with_transaction_and_effects(&self.db, move |txn, effects| {
            let cache = Arc::clone(&cache);
            let next_runtime = next_runtime.clone();
            Box::pin(async move {
                let updated = session::touch_session_updated_at(txn, session_id, next_runtime)
                    .await?
                    .ok_or_else(|| {
                        DbErr::Custom(format!("session disappeared while rewinding: {session_id}"))
                    })?;
                let session = session_from_model_db(updated)?;
                let session_for_cache = session.clone();
                effects.push(async move {
                    with_cache(cache.as_ref(), |guard| {
                        guard.insert(session_for_cache, cache_policy);
                    });
                });
                Ok(session)
            })
        })
        .await?;

        // Re-project after the publish so the cached session reflects the
        // compaction.
        let post_events = self.history.list_session_events(session_id).await?;
        let post_view = super::history::fold_session_view(post_events.as_slice())
            .map_err(|err| AppError::Internal(format!("failed to project session view: {err}")))?;
        let mut session = session;
        session.replace_messages(post_view.messages);
        with_cache(self.cache.as_ref(), |guard| {
            guard.insert(session.clone(), cache_policy);
        });
        Ok(session)
    }

    pub(crate) async fn append_history_items(
        &self,
        mut session: Session,
        items: Vec<EventKind>,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        if items.is_empty() {
            return Ok(session);
        }
        session.sync_runtime_turn_state();
        let session_id = session.id;
        let now = Utc::now();
        let runtime_to_persist = session.runtime.clone();

        // Publish every item via the unified publisher.
        self.history.append_items(session_id, items, now).await?;

        // Re-project from the unified store and update session state.
        let events = self.history.list_session_events(session_id).await?;
        let view = super::history::fold_session_view(events.as_slice())
            .map_err(|err| AppError::Internal(format!("failed to project session view: {err}")))?;

        let updated = with_transaction_and_effects(&self.db, move |txn, _effects| {
            let runtime = runtime_to_persist.clone();
            Box::pin(async move {
                let updated = session::touch_session_updated_at(txn, session_id, runtime)
                    .await?
                    .ok_or_else(|| DbErr::Custom(format!("session not found: {session_id}")))?;
                session_from_model_db(updated)
            })
        })
        .await?;

        session.apply_persisted_metadata(&updated);
        session.replace_messages(view.messages);
        session.runtime = updated.runtime.clone();
        session.refresh_derived();

        let session_for_cache = session.clone();
        with_cache(self.cache.as_ref(), |guard| {
            guard.insert(session_for_cache, cache_policy);
        });

        Ok(session)
    }

    pub(crate) async fn persist(
        &self,
        commit: SessionCommit,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let SessionCommit {
            mut session,
            touched_messages,
            mut client_events,
            persisted_rule,
        } = commit;
        session.sync_runtime_turn_state();
        let session_id = session.id;
        let touched_messages = ordered_unique_touched_messages(&session, touched_messages);
        let now = Utc::now();
        let ts_ms = now.timestamp_millis();
        for message in &touched_messages {
            for part in &message.parts {
                client_events.push(EventKind::MessagePartUpdated(MessagePartUpdatedEvent {
                    session_id,
                    message_id: message.id,
                    message_role: message.role,
                    message_state: message.state,
                    message_created_at: message.created_at,
                    part: part.clone(),
                    ts_ms,
                }));
            }
        }

        let cache = Arc::clone(&self.cache);
        let session_for_cache = session.clone();
        let session_runtime = session.runtime.clone();
        let updated_session = with_transaction_and_effects(&self.db, move |txn, effects| {
            let cache = Arc::clone(&cache);
            Box::pin(async move {
                if let Some((action_key, mode)) = persisted_rule {
                    permission_rule::upsert_rule(txn, action_key.as_str(), mode).await?;
                }

                let updated_session =
                    session::touch_session_updated_at(txn, session_id, session_runtime)
                        .await?
                        .ok_or_else(|| DbErr::Custom(format!("session not found: {session_id}")))?;
                let updated_session = session_from_model_db(updated_session)?;

                let updated_session_for_cache = updated_session.clone();
                effects.push(async move {
                    with_cache(cache.as_ref(), |guard| {
                        let mut cached_session = session_for_cache;
                        cached_session.apply_persisted_metadata(&updated_session_for_cache);
                        cached_session.refresh_derived();
                        guard.insert(cached_session, cache_policy);
                    });
                });

                Ok(updated_session)
            })
        })
        .await?;

        // Publish every queued event after the row update commits.
        for kind in client_events {
            self.publish_event(session_id, kind).await?;
        }

        session.apply_persisted_metadata(&updated_session);
        session.refresh_derived();
        Ok(session)
    }

    pub(crate) async fn append_client_events(
        &self,
        session_id: i64,
        client_events: Vec<EventKind>,
    ) -> Result<(), AppError> {
        for kind in client_events {
            self.publish_event(session_id, kind).await?;
        }
        Ok(())
    }

    pub(crate) async fn resolve_permission_mode(
        &self,
        action_key: &str,
    ) -> Result<Option<PermissionMode>, AppError> {
        Ok(permission_rule::resolve_rule(&self.db, action_key).await?)
    }

    pub(crate) fn prune_cache(&self, cache_policy: SessionCachePolicy) {
        with_cache(self.cache.as_ref(), |guard| {
            guard.prune(cache_policy);
        });
    }

    pub(crate) fn cache_stats(&self) -> SessionCacheStats {
        with_cache(self.cache.as_ref(), |guard| guard.stats()).unwrap_or_default()
    }

    pub(crate) async fn reserve_message_ids(
        &self,
        part_count: usize,
    ) -> Result<ReservedMessageIds, AppError> {
        let mut allocator = self.ids.lock().await;
        self.ensure_id_allocator(&mut allocator).await?;

        let message_id = allocator.next_message_id;
        allocator.next_message_id += 1;

        let first_part_id = allocator.next_part_id;
        allocator.next_part_id += part_count as i64;
        let part_ids = (0..part_count)
            .map(|index| first_part_id + index as i64)
            .collect::<Vec<_>>();

        Ok(ReservedMessageIds {
            message_id,
            part_ids,
        })
    }

    pub(crate) async fn reserve_part_id(&self) -> Result<i64, AppError> {
        let mut allocator = self.ids.lock().await;
        self.ensure_id_allocator(&mut allocator).await?;

        let part_id = allocator.next_part_id;
        allocator.next_part_id += 1;
        Ok(part_id)
    }

    pub(crate) async fn reserve_processor_ids(&self) -> Result<ReservedProcessorIds, AppError> {
        let mut allocator = self.ids.lock().await;
        self.ensure_id_allocator(&mut allocator).await?;

        let ids = ReservedProcessorIds {
            message_id: allocator.next_message_id,
            part_ids: ProcessorPartIdAllocator::new(Arc::clone(&self.ids)),
        };
        allocator.next_message_id += 1;
        Ok(ids)
    }

    async fn workspace_id(&self) -> Result<i64, AppError> {
        let workspace_id = self
            .workspace_id
            .get_or_try_init(|| async {
                workspace::ensure_workspace_id(&self.db, self.workspace_path.as_str())
                    .await
                    .map_err(AppError::from)
            })
            .await?;
        Ok(*workspace_id)
    }

    async fn lookup_workspace_id(&self) -> Result<Option<i64>, AppError> {
        if let Some(workspace_id) = self.workspace_id.get() {
            return Ok(Some(*workspace_id));
        }

        let workspace_id =
            workspace::get_workspace_id_by_path(&self.db, self.workspace_path.as_str()).await?;
        if let Some(workspace_id) = workspace_id {
            let _ = self.workspace_id.set(workspace_id);
        }
        Ok(workspace_id)
    }

    async fn ensure_id_allocator(&self, allocator: &mut GlobalIdAllocator) -> Result<(), AppError> {
        if allocator.initialized {
            return Ok(());
        }

        // The legacy `message` and `message_part` SQL tables are gone. Derive
        // the next message-id by scanning every session's projected view and
        // taking the max message id observed. Part ids are not persisted in
        // the append-only event log (the projection synthesises them) so the
        // allocator simply restarts at 1.
        let mut max_message_id: i64 = 0;
        for session_id in crate::db::crud::session::list_all_session_ids(&self.db).await? {
            let events = self.history.list_session_events(session_id).await?;
            let view = super::history::fold_session_view(events.as_slice())
                .map_err(|err| AppError::Internal(format!("session view fold failed: {err}")))?;
            for message in &view.messages {
                if message.id > max_message_id {
                    max_message_id = message.id;
                }
            }
        }
        let next_message_id = max_message_id + 1;
        let next_part_id: i64 = 1;

        if !allocator.initialized {
            allocator.initialized = true;
            allocator.next_message_id = next_message_id;
            allocator.next_part_id = next_part_id;
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct GlobalIdAllocator {
    initialized: bool,
    next_message_id: i64,
    next_part_id: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct ReservedMessageIds {
    pub(crate) message_id: i64,
    pub(crate) part_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReservedProcessorIds {
    pub(crate) message_id: i64,
    pub(crate) part_ids: ProcessorPartIdAllocator,
}

fn with_cache<T>(
    cache: &Mutex<SessionCache>,
    op: impl FnOnce(&mut SessionCache) -> T,
) -> Option<T> {
    match cache.lock() {
        Ok(mut guard) => Some(op(&mut guard)),
        Err(_) => {
            tracing::warn!("session cache lock poisoned; falling back to database state");
            None
        }
    }
}

fn session_from_model(model: crate::db::entities::session::Model) -> Result<Session, AppError> {
    let created_at = timestamp_millis_to_utc(model.created_at_ms)?;
    let updated_at = timestamp_millis_to_utc(model.updated_at_ms)?;
    let mut session = Session::new(model.id, model.workspace_id, model.title, created_at);
    session.parent_id = model.parent_id;
    session.version = model.version;
    session.runtime = model.runtime_state.unwrap_or_default();
    session.updated_at = updated_at;
    Ok(session)
}

fn session_from_model_db(model: crate::db::entities::session::Model) -> Result<Session, DbErr> {
    let created_at = timestamp_millis_to_utc_db(model.created_at_ms)?;
    let updated_at = timestamp_millis_to_utc_db(model.updated_at_ms)?;
    let mut session = Session::new(model.id, model.workspace_id, model.title, created_at);
    session.parent_id = model.parent_id;
    session.version = model.version;
    session.runtime = model.runtime_state.unwrap_or_default();
    session.updated_at = updated_at;
    Ok(session)
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<DateTime<Utc>, AppError> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| AppError::Internal(format!("invalid timestamp millis: {timestamp_ms}")))
}

fn timestamp_millis_to_utc_db(timestamp_ms: i64) -> Result<DateTime<Utc>, DbErr> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| DbErr::Custom(format!("invalid timestamp millis: {timestamp_ms}")))
}

fn rewind_runtime_state(
    mut runtime: crate::session::SessionRuntimeState,
) -> crate::session::SessionRuntimeState {
    let next_generation = runtime.prompt_window.generation.saturating_add(1);
    runtime = crate::session::SessionRuntimeState::default();
    runtime.prompt_window.generation = next_generation;
    runtime
}

fn ordered_unique_touched_messages(
    session: &Session,
    touched_messages: Vec<Message>,
) -> Vec<Message> {
    let session_order = session
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| (message.id, index))
        .collect::<std::collections::HashMap<_, _>>();

    let mut latest_by_id = std::collections::HashMap::new();
    for message in touched_messages {
        latest_by_id.insert(message.id, message);
    }

    let mut ordered = latest_by_id.into_values().collect::<Vec<_>>();
    ordered.sort_by_key(|message| {
        (
            session_order
                .get(&message.id)
                .copied()
                .unwrap_or(usize::MAX),
            message.id,
        )
    });
    ordered
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, time::Duration};

    use chrono::Utc;
    use sea_orm::Database;

    use super::{SessionCachePolicy, SessionStore, ordered_unique_touched_messages};
    use crate::{
        db::init_schema, event::EventPublisher, message::Message, role::Role, session::Session,
    };

    fn test_publisher(db: &sea_orm::DatabaseConnection) -> std::sync::Arc<EventPublisher> {
        use crate::event::{EventBus, EventStore, InProcessEventBus, SequenceAllocator};
        let store_dyn: std::sync::Arc<dyn EventStore<crate::event::EventKind>> =
            std::sync::Arc::new(crate::db::SeaEventStore::<crate::event::EventKind>::new(
                std::sync::Arc::new(db.clone()),
            ));
        let bus: std::sync::Arc<dyn EventBus<crate::event::EventKind>> =
            std::sync::Arc::new(InProcessEventBus::<crate::event::EventKind>::new(64));
        let seq = std::sync::Arc::new(SequenceAllocator::new());
        std::sync::Arc::new(EventPublisher::new(seq, store_dyn, bus))
    }

    #[allow(dead_code)]
    fn test_cache_policy() -> SessionCachePolicy {
        SessionCachePolicy {
            max_sessions: 32,
            ttl: Duration::from_secs(60),
            max_bytes: 16 * 1024 * 1024,
        }
    }

    #[test]
    fn ordered_unique_touched_messages_preserves_session_order_and_latest_snapshot() {
        let now = Utc::now();
        let mut first = Message::prompt_text(Role::User, "first");
        first.id = 1;
        let mut second_old = Message::prompt_text(Role::Assistant, "second-old");
        second_old.id = 2;
        let mut second_new = Message::prompt_text(Role::Assistant, "second-new");
        second_new.id = 2;
        second_new.finish = Some("done".to_string());
        let mut third = Message::prompt_text(Role::Tool, "third");
        third.id = 3;

        let session = Session::new(99, 1, "ordered", now).with_messages(vec![
            first.clone(),
            second_new.clone(),
            third.clone(),
        ]);

        let ordered = ordered_unique_touched_messages(
            &session,
            vec![third, second_old, first, second_new.clone()],
        );

        assert_eq!(
            ordered.iter().map(|message| message.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(ordered[1].finish.as_deref(), Some("done"));
        assert_eq!(ordered[1].as_text_lossy(), "second-new");
    }

    #[tokio::test]
    async fn processor_part_allocator_does_not_reuse_ids_after_large_stream() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("failed to create sqlite db");
        init_schema(&db).await.expect("failed to init schema");

        let workspace_root = std::env::temp_dir();
        let publisher = test_publisher(&db);
        let store = SessionStore::new(db, workspace_root.as_path(), publisher);
        let reserved = store
            .reserve_processor_ids()
            .await
            .expect("processor ids should reserve");

        let mut allocated = Vec::new();
        for _ in 0..1_100 {
            allocated.push(
                reserved
                    .part_ids
                    .reserve()
                    .await
                    .expect("processor part id should reserve"),
            );
        }

        let unique = allocated.iter().copied().collect::<HashSet<_>>();
        let next_part_id = store
            .reserve_part_id()
            .await
            .expect("subsequent part id should reserve");

        assert_eq!(allocated.first().copied(), Some(1));
        assert_eq!(allocated.last().copied(), Some(1_100));
        assert_eq!(unique.len(), allocated.len());
        assert_eq!(next_part_id, 1_101);
        assert!(!unique.contains(&next_part_id));
    }
}
