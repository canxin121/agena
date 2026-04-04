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
        crud::{message, permission_rule, session, session_runtime, workspace},
        entities,
        tx::with_transaction_and_effects,
    },
    event::{MessagePartUpdatedEvent, SessionEvent},
    message::Message,
    permission::PermissionMode,
};

use super::{
    Session, SessionEventRecord,
    cache::{SessionCache, SessionCachePolicy},
    model::{SessionListRequest, SessionSummary},
};

const PROCESSOR_PART_ID_BLOCK: i64 = 1024;

pub(crate) struct SessionCommit {
    pub(crate) session: Session,
    pub(crate) touched_messages: Vec<Message>,
    pub(crate) client_events: Vec<SessionEvent>,
    pub(crate) persisted_rule: Option<(String, PermissionMode)>,
}

pub(crate) struct SessionStore {
    db: DatabaseConnection,
    workspace_path: String,
    workspace_id: OnceCell<i64>,
    cache: Arc<Mutex<SessionCache>>,
    ids: AsyncMutex<GlobalIdAllocator>,
}

impl SessionStore {
    pub(crate) fn new(db: DatabaseConnection, workspace_root: &Path) -> Self {
        Self {
            db,
            workspace_path: workspace_root.to_string_lossy().replace('\\', "/"),
            workspace_id: OnceCell::new(),
            cache: Arc::new(Mutex::new(SessionCache::default())),
            ids: AsyncMutex::new(GlobalIdAllocator::default()),
        }
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
        session.replace_messages(message::list_messages_with_parts(&self.db, session_id).await?);

        with_cache(self.cache.as_ref(), |guard| {
            guard.insert(session.clone(), cache_policy);
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
        let session_id = session.id;
        let mut unique_messages = std::collections::HashMap::new();
        for message in touched_messages {
            unique_messages.insert(message.id, message);
        }
        let touched_messages = unique_messages.into_values().collect::<Vec<_>>();
        let now = Utc::now();
        let ts_ms = now.timestamp_millis();
        for message in &touched_messages {
            for part in &message.parts {
                client_events.push(SessionEvent::MessagePartUpdated(MessagePartUpdatedEvent {
                    session_id,
                    message_id: message.id,
                    part: part.clone(),
                    ts_ms,
                }));
            }
        }

        let cache = Arc::clone(&self.cache);
        let session_for_cache = session.clone();
        let updated_session = with_transaction_and_effects(&self.db, move |txn, effects| {
            let cache = Arc::clone(&cache);
            Box::pin(async move {
                for message in &touched_messages {
                    message::upsert_message_with_parts(txn, session_id, message).await?;
                }

                if let Some((action_key, mode)) = persisted_rule {
                    permission_rule::upsert_rule(txn, action_key.as_str(), mode).await?;
                }

                let updated_session = session::touch_session_updated_at(txn, session_id)
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
            first_part_id: allocator.next_part_id,
        };
        allocator.next_message_id += 1;
        allocator.next_part_id += PROCESSOR_PART_ID_BLOCK;
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct ReservedProcessorIds {
    pub(crate) message_id: i64,
    pub(crate) first_part_id: i64,
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
    session.updated_at = updated_at;
    Ok(session)
}

fn session_from_model_db(model: crate::db::entities::session::Model) -> Result<Session, DbErr> {
    let created_at = timestamp_millis_to_utc_db(model.created_at_ms)?;
    let updated_at = timestamp_millis_to_utc_db(model.updated_at_ms)?;
    let mut session = Session::new(model.id, model.workspace_id, model.title, created_at);
    session.parent_id = model.parent_id;
    session.version = model.version;
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
