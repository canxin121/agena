use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::Utc;
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder};
use tokio::sync::{Mutex as AsyncMutex, OnceCell};

use crate::{
    AppError,
    db::{crud::session, entities, tx::run_transaction_effects},
    event::{DomainEvent, EventKind, EventPublisher, PublishContext},
    message::Message,
    role::Role,
    session::cost::{UsageStatRecord, UsageStats, UsageStatsQuery},
};

use super::{
    GlobalIdAllocator, SESSION_EXPORT_SCHEMA, SESSION_EXPORT_SCHEMA_MIN, Session, SessionCache,
    SessionCachePolicy, SessionExportMeta, SessionHistoryStore, SessionListRequest, SessionStore,
    SessionSummary, access_cache, event_run_id_for_message, event_targets_message,
    nonempty_or_unknown, rewrite_event_message_ids, rewrite_event_part_ids,
    rewrite_event_session_ids, session_from_model, session_from_model_db, timestamp_millis_to_utc,
    visit_event_message_ids, visit_event_part_ids,
};

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
    pub(crate) async fn publish_event(
        &self,
        session_id: i64,
        kind: EventKind,
    ) -> Result<(), AppError> {
        let ctx = PublishContext::for_session(session_id);
        self.publisher
            .publish(ctx, kind)
            .await
            .map_err(|err| AppError::Internal(format!("publish event failed: {err}")))?;
        Ok(())
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

    pub(crate) async fn create_session_inner(
        &self,
        title: String,
        parent_session_id: Option<i64>,
        is_subagent: bool,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let workspace_id = self.workspace_id().await?;
        let cache = Arc::clone(&self.cache);
        let session = run_transaction_effects(&self.db, move |txn, effects| {
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
                    access_cache(cache.as_ref(), |guard| {
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
        if let Some(session) = access_cache(self.cache.as_ref(), |guard| {
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

            access_cache(self.cache.as_ref(), |guard| {
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

        access_cache(self.cache.as_ref(), |guard| {
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
        access_cache(self.cache.as_ref(), |guard| {
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

    /// Create a child branch that keeps only the persistent history before
    /// `message_id`. Unlike [`Self::fork_session`], the supplied message is
    /// *not* retained. This is the operation used by conversational rewind:
    /// the selected message is the first one to retract, not the last one to
    /// keep.
    pub(crate) async fn fork_session_before_message(
        &self,
        source: Session,
        message_id: i64,
        title: String,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let events = self.history.list_session_events(source.id).await?;
        if events.is_empty() {
            return Err(AppError::Internal(format!(
                "message not found in session {}: {message_id}",
                source.id
            )));
        }

        let rewind_start_seq = events
            .iter()
            .filter(|event| event_targets_message(&event.kind, message_id))
            .map(|event| event.meta.seq_global)
            .min()
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "message not found in session {}: {message_id}",
                    source.id
                ))
            })?;
        let cutoff_seq = events
            .iter()
            .filter(|event| event.meta.seq_global < rewind_start_seq && event.kind.is_persistent())
            .map(|event| event.meta.seq_global)
            .max()
            .unwrap_or(0);
        let items = events
            .into_iter()
            .filter(|event| event.meta.seq_global <= cutoff_seq && event.kind.is_persistent())
            .map(|event| event.kind)
            .collect::<Vec<_>>();

        let child = self
            .create_session(title, Some(source.id), cache_policy)
            .await?;
        // Silent: subscribers should not observe a rewind copy as fresh
        // activity.
        self.append_history_items_silent(child, items, cache_policy)
            .await
    }

    pub(crate) async fn usage_stats(&self, query: UsageStatsQuery) -> Result<UsageStats, AppError> {
        let generated_at = Utc::now();
        let Some(workspace_id) = self.lookup_workspace_id().await? else {
            return Ok(crate::session::cost::summarize_usage_records(
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
            return Ok(crate::session::cost::summarize_usage_records(
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

        Ok(crate::session::cost::summarize_usage_records(
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
            let updated = run_transaction_effects(&self.db, move |txn, _effects| {
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
            access_cache(self.cache.as_ref(), |guard| {
                guard.insert(session.clone(), cache_policy);
            });
        }
        Ok(session)
    }
}
