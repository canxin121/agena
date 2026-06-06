use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder};
use tokio::sync::{Mutex as AsyncMutex, OnceCell};

use crate::{
    AppError,
    db::{
        crud::{permission_rule, session, workspace},
        entities,
        tx::with_transaction_and_effects,
    },
    event::{
        DomainEvent, EventKind, EventPublisher, MessagePartUpdatedEvent, PermissionRuleEvent,
        PublishContext,
    },
    message::Message,
    permission::{PermissionMode, PermissionScope, PersistedPermissionRule},
    role::Role,
    session::cost::{UsageStatRecord, UsageStats, UsageStatsQuery},
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
    pub(crate) persisted_rules: Vec<PersistedPermissionRule>,
}

#[derive(Debug, Clone)]
struct PersistedRuleEventMeta {
    rule: PersistedPermissionRule,
    rule_id: i64,
    created: bool,
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
    pub(crate) fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    pub(crate) async fn create_session(
        &self,
        title: String,
        parent_session_id: Option<i64>,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        self.create_session_inner(title, parent_session_id, false, cache_policy)
            .await
    }

    /// Same as [`create_session`] but marks the new row as a subagent
    /// session so user-facing list APIs hide it by default.
    pub(crate) async fn create_subagent_session(
        &self,
        title: String,
        parent_session_id: i64,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        self.create_session_inner(title, Some(parent_session_id), true, cache_policy)
            .await
    }

    async fn create_session_inner(
        &self,
        title: String,
        parent_session_id: Option<i64>,
        is_subagent: bool,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let workspace_id = self.workspace_id().await?;
        let cache = Arc::clone(&self.cache);
        let session = with_transaction_and_effects(&self.db, move |txn, effects| {
            let title = title.clone();
            let cache = Arc::clone(&cache);
            Box::pin(async move {
                let created = session::create_session_with_options(
                    txn,
                    workspace_id,
                    parent_session_id,
                    title,
                    is_subagent,
                )
                .await?;
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

    pub(crate) async fn list_projected_messages(
        &self,
        session_id: i64,
        include_full_parts: bool,
    ) -> Result<Vec<Message>, AppError> {
        Ok(self
            .history
            .list_projected_messages(session_id, include_full_parts)
            .await?)
    }

    pub(crate) async fn list_projected_messages_page(
        &self,
        session_id: i64,
        include_full_parts: bool,
        cursor: Option<(i64, i64)>,
        limit: u64,
    ) -> Result<(Vec<Message>, bool, Option<(i64, i64)>), AppError> {
        Ok(self
            .history
            .list_projected_messages_page(session_id, include_full_parts, cursor, limit)
            .await?)
    }

    pub(crate) async fn list_projected_message_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<crate::session::history::ProjectedMessageHeader>, AppError> {
        Ok(self
            .history
            .list_projected_message_headers(session_id)
            .await?)
    }

    pub(crate) async fn list_projected_message_headers_page(
        &self,
        session_id: i64,
        cursor: Option<(i64, i64)>,
        limit: u64,
    ) -> Result<
        (
            Vec<crate::session::history::ProjectedMessageHeader>,
            bool,
            Option<(i64, i64)>,
        ),
        AppError,
    > {
        Ok(self
            .history
            .list_projected_message_headers_page(session_id, cursor, limit)
            .await?)
    }

    pub(crate) async fn find_projected_message(
        &self,
        session_id: i64,
        message_id: i64,
        include_full_parts: bool,
    ) -> Result<Option<Message>, AppError> {
        Ok(self
            .history
            .find_projected_message(session_id, message_id, include_full_parts)
            .await?)
    }

    pub(crate) async fn find_projected_message_header(
        &self,
        session_id: i64,
        message_id: i64,
    ) -> Result<Option<crate::session::history::ProjectedMessageHeader>, AppError> {
        Ok(self
            .history
            .find_projected_message_header(session_id, message_id)
            .await?)
    }

    pub(crate) async fn list_projected_parts(
        &self,
        message_id: i64,
        include_full_parts: bool,
    ) -> Result<Vec<crate::message::MessagePart>, AppError> {
        Ok(self
            .history
            .list_projected_parts(message_id, include_full_parts)
            .await?)
    }

    pub(crate) async fn find_projected_part(
        &self,
        part_id: i64,
    ) -> Result<Option<crate::message::MessagePart>, AppError> {
        Ok(self.history.find_projected_part(part_id).await?)
    }

    pub(crate) async fn find_projected_session_id_for_message(
        &self,
        message_id: i64,
    ) -> Result<Option<i64>, AppError> {
        Ok(self.history.find_session_id_for_message(message_id).await?)
    }

    pub(crate) async fn find_projected_session_id_for_part(
        &self,
        part_id: i64,
    ) -> Result<Option<i64>, AppError> {
        Ok(self.history.find_session_id_for_part(part_id).await?)
    }

    pub(crate) async fn list_workspace_session_ids(&self) -> Result<Vec<i64>, AppError> {
        let Some(workspace_id) = self.lookup_workspace_id().await? else {
            return Ok(Vec::new());
        };

        Ok(session::list_session_ids_by_workspace_id(&self.db, workspace_id).await?)
    }

    /// Return every session that shares the same tree root, ordered by
    /// `(depth, id)`. Useful for UI tree rendering and bulk export.
    pub(crate) async fn list_session_tree(
        &self,
        root_id: i64,
    ) -> Result<Vec<SessionSummary>, AppError> {
        let models = session::list_session_tree(&self.db, root_id).await?;
        if models.is_empty() {
            return Ok(Vec::new());
        }
        let ids = models.iter().map(|m| m.id).collect::<Vec<_>>();
        // Single grouped event-log query instead of N×fold; tree views can
        // contain hundreds of sessions and re-folding each one was the
        // dominant cost.
        let stats = session::session_event_stats_for_ids(&self.db, &ids).await?;
        let child_counts =
            session::child_session_counts_by_parent_ids(&self.db, ids.as_slice()).await?;
        let mut out = Vec::with_capacity(models.len());
        for model in models {
            let s = stats.get(&model.id).copied();
            let message_count = s
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
                        "invalid negative child count for session {}",
                        model.id
                    ))
                })?
                .unwrap_or_default();
            let last_message_at = s
                .and_then(|stats| stats.last_message_at_ms)
                .map(timestamp_millis_to_utc)
                .transpose()?;
            out.push(SessionSummary {
                is_subagent: model.is_subagent,
                id: model.id,
                parent_id: model.parent_id,
                depth: model.depth,
                root_id: model.root_id,
                workspace_id: model.workspace_id,
                title: model.title,
                version: model.version,
                created_at: timestamp_millis_to_utc(model.created_at_ms)?,
                updated_at: timestamp_millis_to_utc(model.updated_at_ms)?,
                message_count,
                child_session_count,
                last_message_at,
            });
        }
        Ok(out)
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
        let message_stats = session::session_event_stats_for_ids(&self.db, &session_ids).await?;
        let child_counts =
            session::child_session_counts_by_parent_ids(&self.db, session_ids.as_slice()).await?;

        let mut out = Vec::with_capacity(session_models.len());
        for model in session_models {
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

            out.push(SessionSummary {
                is_subagent: model.is_subagent,
                id: model.id,
                parent_id: model.parent_id,
                depth: model.depth,
                root_id: model.root_id,
                workspace_id: model.workspace_id,
                title: model.title,
                version: model.version,
                created_at: timestamp_millis_to_utc(model.created_at_ms)?,
                updated_at: timestamp_millis_to_utc(model.updated_at_ms)?,
                message_count,
                child_session_count,
                last_message_at,
            });
        }
        Ok(out)
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
            let session_model = session::get_session_by_id(&self.db, session_id)
                .await?
                .ok_or_else(|| AppError::Internal(format!("session not found: {session_id}")))?;
            if session.version == session_model.version {
                return Ok(session);
            }

            let mut refreshed = session_from_model(session_model)?;
            let projection = self
                .history
                .load_projection(session_id, refreshed.runtime.clone())
                .await?;
            refreshed.replace_messages(projection.messages);
            refreshed.runtime = projection.runtime;
            refreshed.refresh_derived();

            with_cache(self.cache.as_ref(), |guard| {
                guard.insert(refreshed.clone(), cache_policy);
            });

            return Ok(refreshed);
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

    pub(crate) async fn rename_session(
        &self,
        session_id: i64,
        title: String,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let updated = session::rename_session(&self.db, session_id, title)
            .await?
            .ok_or_else(|| AppError::Internal(format!("session not found: {session_id}")))?;
        let mut session = session_from_model(updated)?;
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
        at_message_id: Option<i64>,
        title: String,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let events = self.history.list_session_events(source.id).await?;
        if events.is_empty() {
            return self
                .create_session(title, Some(source.id), cache_policy)
                .await;
        }

        let cutoff_seq = match at_message_id {
            None => events
                .iter()
                .rfind(|e| e.kind.is_persistent())
                .map(|e| e.meta.seq_global)
                .unwrap_or(0),
            Some(message_id) => {
                let target_event = events
                    .iter()
                    .filter(|e| event_targets_message(&e.kind, message_id))
                    .max_by_key(|e| e.meta.seq_global)
                    .ok_or_else(|| {
                        AppError::Internal(format!(
                            "message not found in session {}: {}",
                            source.id, message_id
                        ))
                    })?;
                let target_seq = target_event.meta.seq_global;
                event_run_id_for_message(&target_event.kind, message_id)
                    .and_then(|run_id| {
                        events
                            .iter()
                            .filter(|e| e.meta.seq_global >= target_seq)
                            .find_map(|event| match &event.kind {
                                EventKind::RunCompleted(payload) if payload.run_id == run_id => {
                                    Some(event.meta.seq_global)
                                }
                                _ => None,
                            })
                    })
                    .unwrap_or(target_seq)
            }
        };

        let items = events
            .into_iter()
            .filter(|event| event.meta.seq_global <= cutoff_seq && event.kind.is_persistent())
            .map(|event| event.kind)
            .collect::<Vec<_>>();

        let child = self
            .create_session(title, Some(source.id), cache_policy)
            .await?;
        // Silent: subscribers should not observe a fork copy as fresh activity.
        self.append_history_items_silent(child, items, cache_policy)
            .await
    }

    pub(crate) async fn usage_stats(&self, query: UsageStatsQuery) -> Result<UsageStats, AppError> {
        let generated_at = Utc::now();
        let Some(workspace_id) = self.lookup_workspace_id().await? else {
            return Ok(super::cost::summarize_usage_records(
                &[],
                &query,
                generated_at,
            ));
        };

        let sessions = entities::session::Entity::find()
            .filter(entities::session::Column::WorkspaceId.eq(workspace_id))
            .all(&self.db)
            .await?;
        if sessions.is_empty() {
            return Ok(super::cost::summarize_usage_records(
                &[],
                &query,
                generated_at,
            ));
        }

        let session_meta = sessions
            .iter()
            .map(|session| {
                (
                    session.id,
                    (
                        session.title.clone(),
                        session.is_subagent,
                        session.workspace_id,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let session_ids = sessions
            .iter()
            .map(|session| session.id)
            .collect::<Vec<_>>();

        let mut statement = entities::activity_message::Entity::find()
            .filter(entities::activity_message::Column::SessionId.is_in(session_ids))
            .filter(entities::activity_message::Column::Role.eq(Role::Assistant))
            .filter(entities::activity_message::Column::Usage.is_not_null())
            .order_by_asc(entities::activity_message::Column::CreatedAtMs)
            .order_by_asc(entities::activity_message::Column::MessageId);

        if let Some(from) = query.from.as_ref() {
            statement = statement.filter(
                entities::activity_message::Column::CreatedAtMs.gte(from.timestamp_millis()),
            );
        }
        if let Some(to) = query.to.as_ref() {
            statement = statement
                .filter(entities::activity_message::Column::CreatedAtMs.lte(to.timestamp_millis()));
        }

        let message_models = statement.all(&self.db).await?;
        let mut records = Vec::with_capacity(message_models.len());
        for message in message_models {
            let Some(usage) = message.usage else {
                continue;
            };
            let Some((title, is_subagent, _workspace_id)) = session_meta.get(&message.session_id)
            else {
                continue;
            };
            let provider_id = nonempty_or_unknown(message.metadata.model_provider_id.as_str());
            let model_id = nonempty_or_unknown(message.metadata.model_id.as_str());
            records.push(UsageStatRecord {
                session_id: message.session_id,
                session_title: title.clone(),
                is_subagent: *is_subagent,
                created_at: timestamp_millis_to_utc(message.created_at_ms)?,
                provider_id,
                model_id,
                usage,
            });
        }

        Ok(super::cost::summarize_usage_records(
            &records,
            &query,
            generated_at,
        ))
    }

    pub(crate) async fn export_session_jsonl(&self, session_id: i64) -> Result<String, AppError> {
        let model = session::get_session_by_id(&self.db, session_id)
            .await?
            .ok_or_else(|| AppError::Internal(format!("session not found: {session_id}")))?;
        let events = self.history.list_session_events(session_id).await?;
        let meta = SessionExportMeta {
            schema: SESSION_EXPORT_SCHEMA,
            source_session_id: model.id,
            parent_id: model.parent_id,
            depth: model.depth,
            root_id: model.root_id,
            title: model.title.clone(),
            created_at_ms: model.created_at_ms,
            updated_at_ms: model.updated_at_ms,
            runtime_state: model.runtime_state.clone().unwrap_or_default(),
            source_workspace_path: Some(self.workspace_path.clone()),
        };

        let mut out = String::new();
        let meta_line = serde_json::to_string(&meta)
            .map_err(|err| AppError::Internal(format!("encode export meta: {err}")))?;
        out.push_str(&meta_line);
        out.push('\n');
        for event in events {
            if !event.kind.is_persistent() {
                continue;
            }
            let line = serde_json::to_string(&event.kind)
                .map_err(|err| AppError::Internal(format!("encode export event: {err}")))?;
            out.push_str(&line);
            out.push('\n');
        }
        Ok(out)
    }

    /// Import a JSONL bundle produced by [`export_session_jsonl`]. Creates a
    /// fresh session in this store's workspace, copies the title, and replays
    /// every persistent event payload through the publisher. Returns the
    /// newly-created session.
    pub(crate) async fn import_session_jsonl(
        &self,
        bundle: &str,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let mut lines = bundle.lines();
        let header = lines
            .next()
            .ok_or_else(|| AppError::Internal("import bundle is empty".to_string()))?;
        let meta: SessionExportMeta = serde_json::from_str(header)
            .map_err(|err| AppError::Internal(format!("decode export meta: {err}")))?;
        if meta.schema < SESSION_EXPORT_SCHEMA_MIN || meta.schema > SESSION_EXPORT_SCHEMA {
            return Err(AppError::Internal(format!(
                "unsupported export schema: {} (supported {SESSION_EXPORT_SCHEMA_MIN}..={SESSION_EXPORT_SCHEMA})",
                meta.schema
            )));
        }

        let mut events = Vec::new();
        for (idx, line) in lines.enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let kind: EventKind = serde_json::from_str(line).map_err(|err| {
                AppError::Internal(format!("decode export event line {}: {err}", idx + 2))
            })?;
            if !kind.is_persistent() {
                continue;
            }
            events.push(kind);
        }

        // Re-map every message id in the imported event stream onto a fresh
        // contiguous range we reserve from the global allocator. Without this
        // the imported events would collide with whatever message ids the
        // current process has already handed out — fork/run appends after
        // the import would silently overwrite an imported message.
        let mut max_imported_id: i64 = 0;
        let mut max_imported_part_id: i64 = 0;
        for event in &events {
            visit_event_message_ids(event, |id| max_imported_id = max_imported_id.max(id));
            visit_event_part_ids(event, |id| {
                max_imported_part_id = max_imported_part_id.max(id)
            });
        }
        let id_offset = if max_imported_id > 0 {
            self.reserve_message_id_block(max_imported_id).await?
        } else {
            0
        };
        let part_id_offset = if max_imported_part_id > 0 {
            self.reserve_part_id_block(max_imported_part_id).await?
        } else {
            0
        };
        if id_offset != 0 {
            for event in &mut events {
                rewrite_event_message_ids(event, |id| id + id_offset);
            }
        }
        if part_id_offset != 0 {
            for event in &mut events {
                rewrite_event_part_ids(event, |id| id + part_id_offset);
            }
        }

        let session = self.create_session(meta.title, None, cache_policy).await?;
        let new_session_id = session.id;
        for event in &mut events {
            rewrite_event_session_ids(event, new_session_id);
        }
        // Silent: imported events are historical; subscribers should not see
        // them as fresh activity.
        let mut session = self
            .append_history_items_silent(session, events, cache_policy)
            .await?;

        // Restore the exported runtime state — provider anchors, prompt token
        // accounting and execution context — onto the new row. Without this
        // round-trip the import would lose every cache hint and the next run
        // would re-prime caches from scratch.
        if !meta.runtime_state.prompt_tokens.is_empty()
            || !meta.runtime_state.provider_anchors.is_empty()
            || !meta.runtime_state.execution.is_empty()
            || !meta.runtime_state.prompt_window.is_empty()
        {
            let runtime = meta.runtime_state.clone();
            let updated = with_transaction_and_effects(&self.db, move |txn, _effects| {
                let runtime = runtime.clone();
                Box::pin(async move {
                    session::touch_session_updated_at(txn, new_session_id, runtime)
                        .await?
                        .ok_or_else(|| {
                            DbErr::Custom(format!(
                                "imported session vanished while restoring runtime: {new_session_id}"
                            ))
                        })
                })
            })
            .await?;
            let persisted = session_from_model_db(updated)?;
            session.apply_persisted_metadata(&persisted);
            session.runtime = meta.runtime_state;
            session.refresh_derived();
            with_cache(self.cache.as_ref(), |guard| {
                guard.insert(session.clone(), cache_policy);
            });
        }
        Ok(session)
    }

    /// Reserve `count` consecutive message ids from the global allocator and
    /// return the offset to add to each imported message id so the imported
    /// range slots into the freshly reserved block:
    ///
    ///   imported id ∈ [1..=count]   → new id ∈ [first..=first+count-1]
    ///
    /// Returns `first - 1` so callers can simply do `new = imported + offset`.
    async fn reserve_message_id_block(&self, count: i64) -> Result<i64, AppError> {
        debug_assert!(count > 0);
        let mut allocator = self.ids.lock().await;
        self.ensure_id_allocator(&mut allocator).await?;
        let first = allocator.next_message_id;
        allocator.next_message_id = first.saturating_add(count);
        Ok(first - 1)
    }

    async fn reserve_part_id_block(&self, count: i64) -> Result<i64, AppError> {
        debug_assert!(count > 0);
        let mut allocator = self.ids.lock().await;
        self.ensure_id_allocator(&mut allocator).await?;
        let first = allocator.next_part_id;
        allocator.next_part_id = first.saturating_add(count);
        Ok(first - 1)
    }

    pub(crate) async fn append_history_items(
        &self,
        session: Session,
        items: Vec<EventKind>,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        self.append_history_items_inner(session, items, cache_policy, false)
            .await
    }

    /// Same as [`Self::append_history_items`] but persists the events without
    /// broadcasting them on the in-process bus. Use for replay-only flows
    /// (fork copy, JSONL import) so subscribers don't observe historical
    /// reconstructions as fresh activity.
    pub(crate) async fn append_history_items_silent(
        &self,
        session: Session,
        items: Vec<EventKind>,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        self.append_history_items_inner(session, items, cache_policy, true)
            .await
    }

    async fn append_history_items_inner(
        &self,
        mut session: Session,
        items: Vec<EventKind>,
        cache_policy: SessionCachePolicy,
        silent: bool,
    ) -> Result<Session, AppError> {
        if items.is_empty() {
            return Ok(session);
        }
        session.sync_runtime_run_state();
        let session_id = session.id;
        let now = Utc::now();
        let runtime_to_persist = session.runtime.clone();

        // Persist via the unified publisher; broadcast only when not silent.
        if silent {
            self.history.append_items_silent(session_id, items).await?;
        } else {
            self.history.append_items(session_id, items, now).await?;
        }
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

        let projection = self
            .history
            .load_projection(session_id, updated.runtime.clone())
            .await?;
        session.apply_persisted_metadata(&updated);
        session.replace_messages(projection.messages);
        session.runtime = projection.runtime;

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
            persisted_rules,
        } = commit;
        session.sync_runtime_run_state();
        let session_id = session.id;
        let touched_messages = ordered_unique_touched_messages(&session, touched_messages);
        let now = Utc::now();
        let ts_ms = now.timestamp_millis();
        let mut ordered_client_events = Vec::new();
        for message in &touched_messages {
            for part in &message.parts {
                ordered_client_events.push(EventKind::MessagePartUpdated(
                    MessagePartUpdatedEvent {
                        session_id,
                        message_id: message.id,
                        message_role: message.role,
                        message_state: message.state,
                        message_created_at: message.created_at,
                        part: part.clone(),
                        ts_ms,
                    },
                ));
            }
        }
        ordered_client_events.extend(client_events);
        client_events = ordered_client_events;
        let cache = Arc::clone(&self.cache);
        let session_for_cache = session.clone();
        let session_runtime = session.runtime.clone();
        let (updated_session, persisted_rules_for_event) =
            with_transaction_and_effects(&self.db, move |txn, effects| {
                let cache = Arc::clone(&cache);
                Box::pin(async move {
                    let mut persisted_rules_for_event = Vec::new();
                    for rule in &persisted_rules {
                        let (model, created) = permission_rule::upsert_rule(txn, rule).await?;
                        persisted_rules_for_event.push(PersistedRuleEventMeta {
                            rule: rule.clone(),
                            rule_id: model.id,
                            created,
                        });
                    }

                    let updated_session =
                        session::touch_session_updated_at(txn, session_id, session_runtime)
                            .await?
                            .ok_or_else(|| {
                                DbErr::Custom(format!("session not found: {session_id}"))
                            })?;
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

                    Ok((updated_session, persisted_rules_for_event))
                })
            })
            .await?;

        // Publish every queued event after the row update commits.
        for kind in client_events {
            self.publish_event(session_id, kind).await?;
        }
        for meta in persisted_rules_for_event {
            let event_kind = if meta.rule.revoked_at_ms.is_some() {
                EventKind::PermissionRuleRevoked(permission_rule_event_from_rule(
                    meta.rule_id,
                    &meta.rule,
                    session_id,
                ))
            } else if meta.created {
                EventKind::PermissionRuleCreated(permission_rule_event_from_rule(
                    meta.rule_id,
                    &meta.rule,
                    session_id,
                ))
            } else {
                EventKind::PermissionRuleUpdated(permission_rule_event_from_rule(
                    meta.rule_id,
                    &meta.rule,
                    session_id,
                ))
            };
            self.publish_event(session_id, event_kind).await?;
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

    pub(crate) async fn resolve_permission_rules(
        &self,
        action_key: &str,
        session_id: Option<i64>,
    ) -> Result<Vec<PersistedPermissionRule>, AppError> {
        let workspace_id = self.lookup_workspace_id().await?;
        let rules =
            permission_rule::resolve_rules(&self.db, action_key, session_id, workspace_id).await?;
        rules
            .iter()
            .map(persisted_permission_rule_from_model)
            .collect()
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

    pub(crate) async fn current_workspace_id(&self) -> Result<i64, AppError> {
        self.workspace_id().await
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

        // Stream every persistent event once and take the highest message id
        // we observe. This replaces the per-session `fold_session_view` walk
        // that used to dominate startup on instances with many sessions —
        // event-store iteration is O(events) and avoids re-projecting each
        // session.
        use crate::event::{EventFilter, Scope, StoreRange};
        let filter = EventFilter::new(Scope::Global);
        let mut max_message_id: i64 = 0;
        let mut max_part_id: i64 = 0;
        let mut cursor: i64 = 0;
        loop {
            let chunk = self
                .publisher
                .store()
                .range(
                    &filter,
                    StoreRange {
                        after_seq_global: cursor,
                        limit: 4096,
                    },
                )
                .await
                .map_err(|err| {
                    AppError::Internal(format!("scan events for id allocator: {err}"))
                })?;
            if chunk.is_empty() {
                break;
            }
            cursor = chunk.last().map(|e| e.meta.seq_global).unwrap_or(cursor);
            for event in &chunk {
                visit_event_message_ids(&event.kind, |id| {
                    if id > max_message_id {
                        max_message_id = id;
                    }
                });
                visit_event_part_ids(&event.kind, |id| {
                    if id > max_part_id {
                        max_part_id = id;
                    }
                });
            }
        }
        let next_message_id = max_message_id + 1;
        let next_part_id = max_part_id + 1;

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

/// Wire-format version for [`SessionExportMeta`]. Bumped whenever the meta
/// shape or replay semantics change; old bundles whose `schema` is outside
/// `[SESSION_EXPORT_SCHEMA_MIN..=SESSION_EXPORT_SCHEMA]` are rejected.
const SESSION_EXPORT_SCHEMA: u32 = 2;
/// Lowest schema we still accept on import. Bump in lockstep with
/// [`SESSION_EXPORT_SCHEMA`] when a breaking change lands.
const SESSION_EXPORT_SCHEMA_MIN: u32 = 1;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SessionExportMeta {
    schema: u32,
    /// Original session id at export time. Used for audit / cross-machine
    /// correlation; the new session always gets a fresh auto-increment id.
    #[serde(default)]
    source_session_id: i64,
    parent_id: Option<i64>,
    depth: i64,
    root_id: i64,
    title: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    #[serde(default)]
    runtime_state: crate::session::SessionRuntimeState,
    /// Filesystem path of the source workspace at export time. Optional —
    /// empty when exporter cannot resolve a path or for schema=1 bundles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_workspace_path: Option<String>,
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

fn nonempty_or_unknown(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

fn persisted_permission_rule_from_model(
    model: &crate::db::entities::permission_rule::Model,
) -> Result<PersistedPermissionRule, AppError> {
    let mode = permission_rule::mode_from_string(model.mode.as_str()).map_err(|_| {
        AppError::Internal(format!(
            "invalid permission mode in persisted rule {}",
            model.id
        ))
    })?;
    let scope = permission_rule::scope_from_string(model.scope.as_str()).map_err(|_| {
        AppError::Internal(format!(
            "invalid permission scope in persisted rule {}",
            model.id
        ))
    })?;
    Ok(PersistedPermissionRule {
        action_key: model.action_key.clone(),
        mode,
        scope,
        session_id: model.session_id,
        workspace_id: model.workspace_id,
        source: model.source.clone(),
        reason: model.reason.clone(),
        operator: model.operator.clone(),
        revoked_at_ms: model.revoked_at_ms,
        revoked_reason: model.revoked_reason.clone(),
        revoked_by: model.revoked_by.clone(),
    })
}

fn permission_mode_label(mode: PermissionMode) -> String {
    match mode {
        PermissionMode::Allow => "allow".to_string(),
        PermissionMode::Ask => "ask".to_string(),
        PermissionMode::Deny => "deny".to_string(),
    }
}

fn permission_scope_label(scope: PermissionScope) -> String {
    match scope {
        PermissionScope::Session => "session".to_string(),
        PermissionScope::Workspace => "workspace".to_string(),
        PermissionScope::Global => "global".to_string(),
    }
}

fn permission_rule_event_from_rule(
    rule_id: i64,
    rule: &PersistedPermissionRule,
    fallback_session_id: i64,
) -> PermissionRuleEvent {
    PermissionRuleEvent {
        session_id: rule.session_id.or(Some(fallback_session_id)),
        rule_id,
        action_key: rule.action_key.clone(),
        mode: permission_mode_label(rule.mode),
        scope: permission_scope_label(rule.scope),
        source: rule.source.clone(),
        reason: rule.reason.clone(),
        operator: rule.operator.clone(),
        revoked_reason: rule.revoked_reason.clone(),
        revoked_by: rule.revoked_by.clone(),
        ts_ms: Utc::now().timestamp_millis(),
    }
}

fn session_from_model(model: crate::db::entities::session::Model) -> Result<Session, AppError> {
    let created_at = timestamp_millis_to_utc(model.created_at_ms)?;
    let updated_at = timestamp_millis_to_utc(model.updated_at_ms)?;
    let mut session = Session::new(model.id, model.workspace_id, model.title, created_at);
    session.parent_id = model.parent_id;
    session.depth = model.depth;
    session.root_id = model.root_id;
    session.version = model.version;
    session.is_subagent = model.is_subagent;
    session.runtime = model.runtime_state.unwrap_or_default();
    session.updated_at = updated_at;
    Ok(session)
}

fn session_from_model_db(model: crate::db::entities::session::Model) -> Result<Session, DbErr> {
    let created_at = timestamp_millis_to_utc_db(model.created_at_ms)?;
    let updated_at = timestamp_millis_to_utc_db(model.updated_at_ms)?;
    let mut session = Session::new(model.id, model.workspace_id, model.title, created_at);
    session.parent_id = model.parent_id;
    session.depth = model.depth;
    session.root_id = model.root_id;
    session.version = model.version;
    session.is_subagent = model.is_subagent;
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

/// Returns true when `kind` is a persistent history event tied to
/// `message_id`. Used by `fork_session` to map a message-level cutoff onto
/// the underlying event sequence.
fn event_targets_message(kind: &EventKind, message_id: i64) -> bool {
    match kind {
        EventKind::UserMessageAppended(payload) => payload.message_id.raw() == message_id,
        EventKind::AssistantMessageCompleted(payload) => payload.message_id.raw() == message_id,
        EventKind::ToolCallIssued(payload) => payload.message_id.raw() == message_id,
        EventKind::ToolCallCompleted(payload) => payload.message_id.raw() == message_id,
        EventKind::SystemNoticeAppended(payload) => payload.message_id.raw() == message_id,
        _ => false,
    }
}

fn event_run_id_for_message(kind: &EventKind, message_id: i64) -> Option<super::ids::RunId> {
    match kind {
        EventKind::UserMessageAppended(payload) if payload.message_id.raw() == message_id => {
            Some(payload.run_id)
        }
        EventKind::AssistantMessageCompleted(payload) if payload.message_id.raw() == message_id => {
            Some(payload.run_id)
        }
        EventKind::ToolCallIssued(payload) if payload.message_id.raw() == message_id => {
            Some(payload.run_id)
        }
        EventKind::ToolCallCompleted(payload) if payload.message_id.raw() == message_id => {
            Some(payload.run_id)
        }
        _ => None,
    }
}

/// Visit every `message_id` carried by the persistent variants of `kind`.
/// Stays in sync with [`rewrite_event_message_ids`] — anything visited there
/// must be visited here too, otherwise import will under-reserve and
/// imported ids will collide with later live ids.
fn visit_event_message_ids(kind: &EventKind, mut visit: impl FnMut(i64)) {
    match kind {
        EventKind::UserMessageAppended(p) => {
            visit(p.message_id.raw());
            visit_message_metadata_ids(&p.metadata, &mut visit);
            for part in &p.parts {
                visit(part.message_id);
            }
        }
        EventKind::AssistantMessageCompleted(p) => {
            visit(p.message_id.raw());
            visit_message_metadata_ids(&p.metadata, &mut visit);
            for part in &p.parts {
                visit(part.message_id);
            }
        }
        EventKind::ToolCallIssued(p) => visit(p.message_id.raw()),
        EventKind::ToolCallCompleted(p) => {
            visit(p.message_id.raw());
            if let Some(part) = &p.part {
                visit(part.message_id);
            }
        }
        EventKind::SystemNoticeAppended(p) => {
            visit(p.message_id.raw());
        }
        EventKind::MessagePartUpdated(p) => {
            visit(p.message_id);
            visit(p.part.message_id);
        }
        // Non-persistent / unaffected variants:
        EventKind::ExecutionStarted(_)
        | EventKind::ExecutionFailed(_)
        | EventKind::StreamError(_)
        | EventKind::MessagePartDelta(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::PluginEvent(_) => {}
    }
}

fn visit_message_metadata_ids(
    metadata: &crate::message::MessageMetadata,
    mut visit: impl FnMut(i64),
) {
    if let Some(parent_message_id) = metadata.parent_message_id {
        visit(parent_message_id);
    }
}

fn visit_event_part_ids(kind: &EventKind, mut visit: impl FnMut(i64)) {
    match kind {
        EventKind::UserMessageAppended(p) => {
            for part in &p.parts {
                visit(part.id);
            }
        }
        EventKind::AssistantMessageCompleted(p) => {
            for part in &p.parts {
                visit(part.id);
            }
        }
        EventKind::MessagePartUpdated(p) => {
            visit(p.part.id);
        }
        EventKind::ExecutionStarted(_)
        | EventKind::ExecutionFailed(_)
        | EventKind::StreamError(_)
        | EventKind::MessagePartDelta(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::PluginEvent(_)
        | EventKind::ToolCallIssued(_)
        | EventKind::SystemNoticeAppended(_) => {}
        EventKind::ToolCallCompleted(p) => {
            if let Some(part) = &p.part {
                visit(part.id);
            }
        }
    }
}

/// Rewrite every `message_id` in `kind` through `f`. Mirror of
/// [`visit_event_message_ids`].
fn rewrite_event_message_ids(kind: &mut EventKind, mut f: impl FnMut(i64) -> i64) {
    use crate::session::ids::MessageId;
    match kind {
        EventKind::UserMessageAppended(p) => {
            p.message_id = MessageId(f(p.message_id.raw()));
            rewrite_message_metadata_ids(&mut p.metadata, &mut f);
            for part in &mut p.parts {
                part.message_id = f(part.message_id);
            }
        }
        EventKind::AssistantMessageCompleted(p) => {
            p.message_id = MessageId(f(p.message_id.raw()));
            rewrite_message_metadata_ids(&mut p.metadata, &mut f);
            for part in &mut p.parts {
                part.message_id = f(part.message_id);
            }
        }
        EventKind::ToolCallIssued(p) => {
            p.message_id = MessageId(f(p.message_id.raw()));
        }
        EventKind::ToolCallCompleted(p) => {
            p.message_id = MessageId(f(p.message_id.raw()));
            if let Some(part) = &mut p.part {
                part.message_id = f(part.message_id);
            }
        }
        EventKind::SystemNoticeAppended(p) => {
            p.message_id = MessageId(f(p.message_id.raw()));
        }
        EventKind::MessagePartUpdated(p) => {
            p.message_id = f(p.message_id);
            p.part.message_id = f(p.part.message_id);
        }
        EventKind::ExecutionStarted(_)
        | EventKind::ExecutionFailed(_)
        | EventKind::StreamError(_)
        | EventKind::MessagePartDelta(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::PluginEvent(_) => {}
    }
}

fn rewrite_message_metadata_ids(
    metadata: &mut crate::message::MessageMetadata,
    mut f: impl FnMut(i64) -> i64,
) {
    if let Some(parent_message_id) = metadata.parent_message_id.as_mut() {
        *parent_message_id = f(*parent_message_id);
    }
}

/// Rewrite every `part_id` in `kind` through `f`. Mirror of
/// [`visit_event_part_ids`].
fn rewrite_event_part_ids(kind: &mut EventKind, mut f: impl FnMut(i64) -> i64) {
    match kind {
        EventKind::UserMessageAppended(p) => {
            for part in &mut p.parts {
                part.id = f(part.id);
            }
        }
        EventKind::AssistantMessageCompleted(p) => {
            for part in &mut p.parts {
                part.id = f(part.id);
            }
        }
        EventKind::MessagePartUpdated(p) => {
            p.part.id = f(p.part.id);
        }
        EventKind::ExecutionStarted(_)
        | EventKind::ExecutionFailed(_)
        | EventKind::StreamError(_)
        | EventKind::MessagePartDelta(_)
        | EventKind::CommandBegin(_)
        | EventKind::CommandOutputDelta(_)
        | EventKind::CommandEnd(_)
        | EventKind::PermissionRequested(_)
        | EventKind::PermissionReplied(_)
        | EventKind::PermissionRuleCreated(_)
        | EventKind::PermissionRuleUpdated(_)
        | EventKind::PermissionRuleRevoked(_)
        | EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::ToolCallIssued(_)
        | EventKind::SystemNoticeAppended(_)
        | EventKind::PluginEvent(_) => {}
        EventKind::ToolCallCompleted(p) => {
            if let Some(part) = &mut p.part {
                part.id = f(part.id);
            }
        }
    }
}

fn rewrite_event_session_ids(kind: &mut EventKind, session_id: i64) {
    match kind {
        EventKind::ExecutionStarted(p) => p.session_id = session_id,
        EventKind::ExecutionFailed(p) => p.session_id = session_id,
        EventKind::StreamError(p) => p.session_id = session_id,
        EventKind::MessagePartUpdated(p) => p.session_id = session_id,
        EventKind::MessagePartDelta(p) => p.session_id = session_id,
        EventKind::CommandBegin(p) => p.context.session_id = session_id,
        EventKind::CommandOutputDelta(p) => p.context.session_id = session_id,
        EventKind::CommandEnd(p) => p.context.session_id = session_id,
        EventKind::PermissionRequested(p) => p.session_id = session_id,
        EventKind::PermissionReplied(p) => p.session_id = session_id,
        EventKind::PermissionRuleCreated(p)
        | EventKind::PermissionRuleUpdated(p)
        | EventKind::PermissionRuleRevoked(p) => {
            if p.session_id.is_some() {
                p.session_id = Some(session_id);
            }
        }
        EventKind::RunStarted(_)
        | EventKind::RunCompleted(_)
        | EventKind::RunAborted(_)
        | EventKind::UserMessageAppended(_)
        | EventKind::AssistantMessageCompleted(_)
        | EventKind::ToolCallIssued(_)
        | EventKind::ToolCallCompleted(_)
        | EventKind::SystemNoticeAppended(_)
        | EventKind::PluginEvent(_) => {}
    }
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
