use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use sea_orm::{DatabaseConnection, DbErr, EntityTrait, QueryOrder};
use tokio::sync::{Mutex as AsyncMutex, OnceCell};

use crate::{
    AppError,
    db::{
        crud::{message, permission_rule, session, session_history, session_runtime, workspace},
        entities,
        tx::with_transaction_and_effects,
    },
    event::{MessagePartUpdatedEvent, SessionEvent},
    message::Message,
    permission::PermissionMode,
};

use super::{
    Session, SessionEventRecord,
    cache::{SessionCache, SessionCachePolicy, SessionCacheStats},
    history::{
        HistoryItem, PromptWindowInvalidationReason, PromptWindowInvalidated, SessionHistoryStore,
        SessionRolledBack, history_items_from_message_snapshot, history_items_from_runtime_diff,
    },
    model::{SessionListRequest, SessionSummary},
};

pub(crate) struct SessionCommit {
    pub(crate) session: Session,
    pub(crate) touched_messages: Vec<Message>,
    pub(crate) client_events: Vec<SessionEvent>,
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
}

impl SessionStore {
    pub(crate) fn new(db: DatabaseConnection, workspace_root: &Path) -> Self {
        Self {
            db: db.clone(),
            workspace_path: workspace_root.to_string_lossy().replace('\\', "/"),
            workspace_id: OnceCell::new(),
            cache: Arc::new(Mutex::new(SessionCache::default())),
            ids: Arc::new(AsyncMutex::new(GlobalIdAllocator::default())),
            history: SessionHistoryStore::new(db),
        }
    }

    #[cfg(test)]
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

    pub(crate) async fn list_session_events(
        &self,
        session_id: i64,
    ) -> Result<Vec<SessionEventRecord>, AppError> {
        Ok(session_runtime::list_session_events(&self.db, session_id).await?)
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
        let message_stats =
            session::session_message_stats_by_session_ids(&self.db, session_ids.as_slice()).await?;
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

    pub(crate) async fn rewind_to_message(
        &self,
        session_id: i64,
        message_id: i64,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let cache = Arc::clone(&self.cache);
        let session = with_transaction_and_effects(&self.db, move |txn, effects| {
            let cache = Arc::clone(&cache);
            Box::pin(async move {
                let Some(existing) = session::get_session_by_id(txn, session_id).await? else {
                    return Err(DbErr::Custom(format!("session not found: {session_id}")));
                };
                let target = entities::message::Entity::find_by_id(message_id)
                    .one(txn)
                    .await?
                    .ok_or_else(|| {
                        DbErr::Custom(format!("message not found for rewind: {message_id}"))
                    })?;
                if target.session_id != session_id {
                    return Err(DbErr::Custom(format!(
                        "message {message_id} does not belong to session {session_id}"
                    )));
                }

                super::history::ensure_legacy_imported(txn, session_id).await?;
                let next_history_seq = session_history::latest_history_seq(txn, session_id)
                    .await?
                    .unwrap_or(0);
                let next_runtime =
                    rewind_runtime_state(existing.runtime_state.clone().unwrap_or_default());
                super::history::append_items(
                    txn,
                    session_id,
                    next_history_seq,
                    vec![
                        HistoryItem::SessionRolledBack(SessionRolledBack {
                            target_message_id: message_id,
                            target_seq: Some(next_history_seq),
                        }),
                        HistoryItem::PromptWindowInvalidated(PromptWindowInvalidated {
                            generation: next_runtime.prompt_window.generation,
                            reason: PromptWindowInvalidationReason::Rewind,
                        }),
                    ],
                    Utc::now(),
                )
                .await?;

                let records = session_history::list_history_records(txn, session_id).await?;
                let projection = super::history::replay_history(records.as_slice())
                    .map_err(|err| DbErr::Custom(format!("failed to replay session history: {err}")))?;
                message::delete_messages_by_session_id(txn, session_id).await?;
                for message in &projection.messages {
                    message::insert_message_with_parts(txn, session_id, message).await?;
                }

                let updated = session::touch_session_updated_at(txn, session_id, projection.runtime.clone())
                    .await?
                    .ok_or_else(|| {
                        DbErr::Custom(format!("session disappeared while rewinding: {session_id}"))
                    })?;
                let mut session = session_from_model_db(updated)?;
                session.replace_messages(projection.messages);

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

    pub(crate) async fn append_history_items(
        &self,
        mut session: Session,
        items: Vec<HistoryItem>,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        if items.is_empty() {
            return Ok(session);
        }
        let session_id = session.id;
        let now = Utc::now();
        let cache = Arc::clone(&self.cache);
        let session_for_cache_effect = session.clone();
        let (updated_session, projected_messages, projected_runtime) = with_transaction_and_effects(&self.db, move |txn, effects| {
            let cache = Arc::clone(&cache);
            let items = items.clone();
            Box::pin(async move {
                super::history::ensure_legacy_imported(txn, session_id).await?;
                let next_history_seq = session_history::latest_history_seq(txn, session_id)
                    .await?
                    .unwrap_or(0);
                super::history::append_items(txn, session_id, next_history_seq, items, now)
                    .await?;
                let records = session_history::list_history_records(txn, session_id).await?;
                let projection = super::history::replay_history(records.as_slice())
                    .map_err(|err| DbErr::Custom(format!("failed to replay session history: {err}")))?;
                message::delete_messages_by_session_id(txn, session_id).await?;
                for message in &projection.messages {
                    message::insert_message_with_parts(txn, session_id, message).await?;
                }
                let updated = session::touch_session_updated_at(txn, session_id, projection.runtime.clone())
                    .await?
                    .ok_or_else(|| DbErr::Custom(format!("session not found: {session_id}")))?;
                let updated_session = session_from_model_db(updated)?;
                let updated_session_for_cache = updated_session.clone();
                let projected_messages = projection.messages;
                let projected_runtime = projection.runtime;
                let mut session_for_cache = session_for_cache_effect.clone();
                session_for_cache.replace_messages(projected_messages.clone());
                session_for_cache.runtime = projected_runtime.clone();
                effects.push(async move {
                    with_cache(cache.as_ref(), |guard| {
                        let mut cached_session = session_for_cache;
                        cached_session.apply_persisted_metadata(&updated_session_for_cache);
                        cached_session.refresh_derived();
                        guard.insert(cached_session, cache_policy);
                    });
                });
                Ok((updated_session, projected_messages, projected_runtime))
            })
        })
        .await?;
        session.apply_persisted_metadata(&updated_session);
        session.replace_messages(projected_messages);
        session.runtime = projected_runtime;
        session.refresh_derived();
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
        let session_id = session.id;
        let touched_messages = ordered_unique_touched_messages(&session, touched_messages);
        let now = Utc::now();
        let ts_ms = now.timestamp_millis();
        for message in &touched_messages {
            for part in &message.parts {
                client_events.push(SessionEvent::MessagePartUpdated(MessagePartUpdatedEvent {
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
                let existing_runtime = session::get_session_by_id(txn, session_id)
                    .await?
                    .ok_or_else(|| DbErr::Custom(format!("session not found: {session_id}")))?
                    .runtime_state
                    .unwrap_or_default();
                super::history::ensure_legacy_imported(txn, session_id).await?;
                let mut next_history_seq = session_history::latest_history_seq(txn, session_id)
                    .await?
                    .unwrap_or(0);
                for message in &touched_messages {
                    let records = super::history::append_items(
                        txn,
                        session_id,
                        next_history_seq,
                        history_items_from_message_snapshot(message),
                        now,
                    )
                    .await?;
                    next_history_seq = records
                        .last()
                        .map(|record| record.seq)
                        .unwrap_or(next_history_seq);
                    message::upsert_message_with_parts(txn, session_id, message).await?;
                }
                let runtime_items = history_items_from_runtime_diff(&existing_runtime, &session_runtime);
                let records = super::history::append_items(
                    txn,
                    session_id,
                    next_history_seq,
                    runtime_items,
                    now,
                )
                .await?;
                next_history_seq = records
                    .last()
                    .map(|record| record.seq)
                    .unwrap_or(next_history_seq);
                let _ = next_history_seq;

                if let Some((action_key, mode)) = persisted_rule {
                    permission_rule::upsert_rule(txn, action_key.as_str(), mode).await?;
                }

                let updated_session =
                    session::touch_session_updated_at(txn, session_id, session_runtime)
                        .await?
                        .ok_or_else(|| DbErr::Custom(format!("session not found: {session_id}")))?;
                let updated_session = session_from_model_db(updated_session)?;

                let mut next_seq = session_runtime::latest_event_seq(txn, session_id)
                    .await?
                    .unwrap_or(0);
                for event in client_events {
                    next_seq += 1;
                    session_runtime::append_session_event(txn, session_id, next_seq, event, now)
                        .await?;
                }

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

        session.apply_persisted_metadata(&updated_session);
        session.refresh_derived();
        Ok(session)
    }

    pub(crate) async fn append_client_events(
        &self,
        session_id: i64,
        client_events: Vec<SessionEvent>,
    ) -> Result<(), AppError> {
        if client_events.is_empty() {
            return Ok(());
        }

        let now = Utc::now();
        with_transaction_and_effects(&self.db, move |txn, _effects| {
            let events = client_events.clone();
            Box::pin(async move {
                let mut next_seq = session_runtime::latest_event_seq(txn, session_id)
                    .await?
                    .unwrap_or(0);
                for event in events {
                    next_seq += 1;
                    session_runtime::append_session_event(txn, session_id, next_seq, event, now)
                        .await?;
                }
                Ok(())
            })
        })
        .await?;

        Ok(())
    }

    pub(crate) async fn append_client_projection(
        &self,
        session_id: i64,
        message_snapshot: Option<Message>,
        client_events: Vec<SessionEvent>,
        cache_policy: SessionCachePolicy,
    ) -> Result<(), AppError> {
        if client_events.is_empty() && message_snapshot.is_none() {
            return Ok(());
        }

        let now = Utc::now();
        let cache = Arc::clone(&self.cache);
        with_transaction_and_effects(&self.db, move |txn, effects| {
            let events = client_events.clone();
            let message_snapshot = message_snapshot.clone();
            let cache = Arc::clone(&cache);
            Box::pin(async move {
                if let Some(message) = message_snapshot.as_ref() {
                    super::history::ensure_legacy_imported(txn, session_id).await?;
                    let next_history_seq = session_history::latest_history_seq(txn, session_id)
                        .await?
                        .unwrap_or(0);
                    super::history::append_message_snapshot(
                        txn,
                        session_id,
                        next_history_seq,
                        message,
                        now,
                    )
                    .await?;
                    message::upsert_message_with_parts(txn, session_id, message).await?;
                }

                let mut next_seq = session_runtime::latest_event_seq(txn, session_id)
                    .await?
                    .unwrap_or(0);
                for event in events {
                    next_seq += 1;
                    session_runtime::append_session_event(txn, session_id, next_seq, event, now)
                        .await?;
                }

                if let Some(message) = message_snapshot {
                    effects.push(async move {
                        with_cache(cache.as_ref(), |guard| {
                            let Some(mut cached_session) = guard.get(session_id, cache_policy)
                            else {
                                return;
                            };
                            upsert_session_message(&mut cached_session, message);
                            guard.insert(cached_session, cache_policy);
                        });
                    });
                }

                Ok(())
            })
        })
        .await?;

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

        let next_message_id = entities::message::Entity::find()
            .order_by_desc(entities::message::Column::Id)
            .one(&self.db)
            .await?
            .map(|model| model.id + 1)
            .unwrap_or(1);
        let next_part_id = entities::message_part::Entity::find()
            .order_by_desc(entities::message_part::Column::Id)
            .one(&self.db)
            .await?
            .map(|model| model.id + 1)
            .unwrap_or(1);

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

fn upsert_session_message(session: &mut Session, message: Message) {
    if let Some(index) = session
        .messages
        .iter()
        .position(|item| item.id == message.id)
    {
        session.messages[index] = message;
    } else {
        session.messages.push(message);
        session
            .messages
            .sort_by_key(|item| (item.created_at, item.id));
    }
    session.refresh_derived();
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, time::Duration};

    use chrono::Utc;
    use sea_orm::Database;

    use super::{SessionCachePolicy, SessionCommit, SessionStore, ordered_unique_touched_messages};
    use crate::{
        db::{crud::session_history, init_schema},
        message::Message,
        role::Role,
        session::{Session, history::HistoryItem},
    };

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
    async fn persist_appends_history_without_rewriting_prior_payloads() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("failed to create sqlite db");
        init_schema(&db).await.expect("failed to init schema");

        let workspace_root = std::env::temp_dir();
        let store = SessionStore::new(db.clone(), workspace_root.as_path());
        let session = store
            .create_session("append-only".to_string(), None, test_cache_policy())
            .await
            .expect("session should create");

        let mut message = Message::prompt_text(Role::User, "first");
        message.id = 1;
        message.parts[0].id = 1;
        message.parts[0].message_id = 1;
        let mut first_session = session.clone();
        first_session.messages.push(message.clone());
        let first_session = store
            .persist(
                SessionCommit {
                    session: first_session,
                    touched_messages: vec![message.clone()],
                    client_events: Vec::new(),
                    persisted_rule: None,
                },
                test_cache_policy(),
            )
            .await
            .expect("first persist should succeed");

        let first_records = session_history::list_history_records(&db, session.id)
            .await
            .expect("history should load");
        assert_eq!(first_records.len(), 1);
        let HistoryItem::MessageSnapshotRecorded(first_payload) = &first_records[0].item else {
            panic!("expected message snapshot history item");
        };
        assert_eq!(first_payload.message.as_text_lossy(), "first");

        let mut updated_message = message;
        updated_message.parts[0].set_content(crate::message::PartContent::text("second"));
        let mut second_session = first_session;
        second_session.messages[0] = updated_message.clone();
        store
            .persist(
                SessionCommit {
                    session: second_session,
                    touched_messages: vec![updated_message],
                    client_events: Vec::new(),
                    persisted_rule: None,
                },
                test_cache_policy(),
            )
            .await
            .expect("second persist should succeed");

        let records = session_history::list_history_records(&db, session.id)
            .await
            .expect("history should load");
        assert_eq!(records.len(), 2);
        let HistoryItem::MessageSnapshotRecorded(first_payload) = &records[0].item else {
            panic!("expected first message snapshot history item");
        };
        let HistoryItem::MessageSnapshotRecorded(second_payload) = &records[1].item else {
            panic!("expected second message snapshot history item");
        };
        assert_eq!(first_payload.message.as_text_lossy(), "first");
        assert_eq!(second_payload.message.as_text_lossy(), "second");
    }

    #[tokio::test]
    async fn processor_part_allocator_does_not_reuse_ids_after_large_stream() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("failed to create sqlite db");
        init_schema(&db).await.expect("failed to init schema");

        let workspace_root = std::env::temp_dir();
        let store = SessionStore::new(db, workspace_root.as_path());
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
