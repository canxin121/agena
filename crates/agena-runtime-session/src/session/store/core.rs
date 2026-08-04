use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::Utc;
use sea_orm::{DatabaseConnection, DbErr};
use tokio::sync::OnceCell;

use crate::{
    AppError,
    db::crud::session,
    event::{DomainEvent, EventKind, EventPublisher, PublishContext},
    message::Message,
};
use agena_domain::{SessionLifecycleState, UsageStats, UsageStatsQuery};
use agena_runtime::UsageStatRecord;

use super::{
    Session, SessionCache, SessionCachePolicy, SessionExportMeta, SessionHistoryStore,
    SessionStore, access_cache, event_execution_id_for_message, event_run_id_for_message,
    event_targets_message, rewrite_copied_domain_ids, rewrite_event_message_ids,
    rewrite_event_part_ids, rewrite_event_session_ids, session_from_model, session_from_model_db,
    timestamp_millis_to_utc, visit_event_message_ids, visit_event_part_ids,
};
use agena_domain::{SessionListRequest, SessionSummary};
use agena_storage::SequenceAllocator;
use agena_storage_sqlite::run_transaction_effects;

/// A lease whose heartbeat is older than this is considered stale and may be
/// reclaimed by reconciliation (the owning process is presumed crashed). The
/// authoritative definition lives in the lease layer so acquire-time stealing
/// and periodic reaping agree on the same threshold.
pub(crate) use agena_runtime_session_core::db::leases::LEASE_STALENESS_MS;

impl SessionStore {
    /// The owner of the session's active execution lease, if any.
    ///
    /// A lease is "active" while its heartbeat is fresh; a stale lease (e.g.
    /// from a process that crashed) is not reported here so startup
    /// reconciliation can reclaim the session.
    pub(crate) async fn active_lease_owner(&self, session_id: i64) -> Option<String> {
        let now = agena_runtime_session_core::db::leases::lease_now_ms();
        let row = agena_runtime_session_core::db::leases::lease(&self.db, session_id)
            .await
            .ok()?;
        let row = row?;
        (now - row.heartbeat_at_ms < LEASE_STALENESS_MS).then_some(row.owner_id)
    }

    pub(crate) async fn reconcile_interrupted_lifecycles(
        &self,
        session_id: i64,
    ) -> Result<(), AppError> {
        self.history
            .reconcile_interrupted_lifecycles(session_id)
            .await
            .map_err(AppError::from)?;
        // Lifecycle reconciliation advances the event-backed model-message
        // and canonical transcript projections without touching the session
        // row version. A cached Session therefore cannot use the row version
        // as an invalidation signal here.
        access_cache(self.cache.as_ref(), |guard| guard.discard(session_id));
        Ok(())
    }

    pub(crate) async fn reconcile_unmatched_runs(
        &self,
        session_id: i64,
        reason: agena_domain::RunAbortReason,
    ) -> Result<(), AppError> {
        self.history
            .reconcile_unmatched_runs(session_id, reason)
            .await
            .map_err(AppError::from)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        db: DatabaseConnection,
        workspace_root: &Path,
        publisher: Arc<EventPublisher>,
        ids: Arc<dyn SequenceAllocator>,
        workspace_repository: Arc<dyn agena_storage::WorkspaceRepository>,
        permission_rule_repository: Arc<dyn agena_storage::PermissionRuleRepository>,
        permission_rule_transaction_writer: Arc<
            dyn agena_storage::PermissionRuleTransactionWriter<sea_orm::DatabaseTransaction>,
        >,
        session_stats_repository: Arc<dyn agena_storage::SessionStatsRepository>,
        usage_repository: Arc<dyn agena_storage::UsageRepository>,
        session_mutation_repository: Arc<dyn agena_storage::SessionMutationRepository>,
        projection_lookup_repository: Arc<dyn agena_storage::ProjectionLookupRepository>,
        message_projection_repository: Arc<dyn agena_storage::ModelMessageRepository>,
        message_projection_transaction_writer: Arc<
            dyn agena_storage::ModelMessageTransactionWriter<sea_orm::DatabaseTransaction>,
        >,
        session_summary_repository: Arc<dyn agena_storage::SessionSummaryRepository>,
    ) -> Self {
        Self {
            db: db.clone(),
            workspace_path: workspace_root.to_string_lossy().replace('\\', "/"),
            workspace_id: OnceCell::new(),
            id_seed: OnceCell::new(),
            cache: Arc::new(Mutex::new(SessionCache::default())),
            ids,
            history: SessionHistoryStore::new(
                Arc::clone(&publisher),
                db,
                Arc::clone(&message_projection_repository),
                message_projection_transaction_writer,
            ),
            publisher,
            workspace_repository,
            permission_rule_repository,
            permission_rule_transaction_writer,
            session_stats_repository,
            usage_repository,
            session_mutation_repository,
            projection_lookup_repository,
            session_summary_repository,
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

    /// Update the authoritative delegated-task lifecycle independently from
    /// the rest of runtime JSON. Normal session writes re-read these columns,
    /// so a detached provider future cannot overwrite a timeout/cancellation.
    pub(crate) async fn update_subtask_state(
        &self,
        mut value: Session,
        subtask: crate::session::SubtaskRuntimeState,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let session_id = value.id;
        let updated = session::update_subtask_state(&self.db, session_id, subtask.clone())
            .await?
            .ok_or_else(|| AppError::Internal(format!("session not found: {session_id}")))?;
        let persisted = session_from_model(updated)?;
        value.apply_persisted_metadata(&persisted);
        value.runtime.subtask = subtask;
        value.refresh_derived();
        access_cache(self.cache.as_ref(), |guard| {
            guard.insert(value.clone(), cache_policy);
        });
        Ok(value)
    }

    pub(crate) async fn create_session(
        &self,
        title: String,
        parent_session_id: Option<i64>,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let lineage = parent_session_id.map(|_| session::SessionLineageInput::CHILD);
        self.create_session_inner(
            title,
            parent_session_id,
            lineage,
            None,
            SessionLifecycleState::Ready,
            cache_policy,
        )
        .await
    }

    /// Same as [`create_session`] but marks the new row as a subagent
    /// session so user-facing list APIs hide it by default.
    pub(crate) async fn create_subagent_session(
        &self,
        title: String,
        parent_session_id: i64,
        task_id: String,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        self.create_session_inner(
            title,
            Some(parent_session_id),
            Some(session::SessionLineageInput::subagent()),
            Some(task_id),
            SessionLifecycleState::Ready,
            cache_policy,
        )
        .await
    }

    async fn create_session_inner(
        &self,
        title: String,
        parent_session_id: Option<i64>,
        lineage: Option<session::SessionLineageInput>,
        task_id: Option<String>,
        lifecycle_state: SessionLifecycleState,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let workspace_id = self.workspace_id().await?;
        let cache = Arc::clone(&self.cache);
        let session = run_transaction_effects(&self.db, move |txn, effects| {
            let title = title.clone();
            let cache = Arc::clone(&cache);
            Box::pin(async move {
                let created = session::create_session_in_transaction(
                    txn,
                    workspace_id,
                    parent_session_id,
                    title,
                    lineage,
                    task_id,
                    lifecycle_state,
                )
                .await?;
                let session = session_from_model_db(created)?;

                if lifecycle_state == SessionLifecycleState::Ready {
                    let session_for_cache = session.clone();
                    effects.push(async move {
                        access_cache(cache.as_ref(), |guard| {
                            guard.insert(session_for_cache, cache_policy);
                        });
                    });
                }

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

    pub(crate) async fn list_projected_message_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<crate::session::history::ProjectedMessageHeader>, AppError> {
        Ok(self
            .history
            .list_projected_message_headers(session_id)
            .await?)
    }

    pub(crate) async fn find_projected_session_id_for_message(
        &self,
        message_id: i64,
    ) -> Result<Option<i64>, AppError> {
        self.projection_lookup_repository
            .session_id_for_message(message_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))
    }

    pub(crate) async fn find_projected_session_id_for_part(
        &self,
        part_id: i64,
    ) -> Result<Option<i64>, AppError> {
        self.projection_lookup_repository
            .session_id_for_part(part_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))
    }

    pub(crate) async fn list_workspace_session_ids(&self) -> Result<Vec<i64>, AppError> {
        let Some(workspace_id) = self.lookup_workspace_id().await? else {
            return Ok(Vec::new());
        };

        let summaries = self
            .session_summary_repository
            .list(agena_storage::SessionSummaryListQuery {
                workspace_id: Some(workspace_id),
                roots_only: false,
                parent_id: None,
                search: None,
                before_updated_at_ms: None,
                before_id: None,
                offset: 0,
                limit: u64::MAX,
                include_subagents: true,
            })
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        Ok(summaries.into_iter().map(|summary| summary.id).collect())
    }

    pub(crate) async fn find_subagent_by_task_id(
        &self,
        parent_session_id: i64,
        task_id: &str,
        cache_policy: SessionCachePolicy,
    ) -> Result<Option<Session>, AppError> {
        let Some(summary) = self
            .session_summary_repository
            .get_subagent_by_task_id(parent_session_id, task_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
        else {
            return Ok(None);
        };
        self.load_session(summary.id, cache_policy).await.map(Some)
    }

    /// Return every session that shares the same tree root, ordered by
    /// `(depth, id)`. Useful for UI tree rendering and bulk export.
    pub(crate) async fn list_session_tree(
        &self,
        root_id: i64,
    ) -> Result<Vec<SessionSummary>, AppError> {
        let records = self
            .session_summary_repository
            .list_tree(root_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let mut out = Vec::with_capacity(records.len());
        for record in records {
            let summary = record.summary;
            let message_count = u64::try_from(record.message_count).map_err(|_| {
                AppError::Internal(format!(
                    "invalid negative message count for session {}",
                    summary.id
                ))
            })?;
            let child_session_count = u64::try_from(record.child_session_count).map_err(|_| {
                AppError::Internal(format!(
                    "invalid negative child count for session {}",
                    summary.id
                ))
            })?;
            let last_message_at = record
                .last_message_at_ms
                .map(timestamp_millis_to_utc)
                .transpose()?;
            out.push(SessionSummary {
                task_id: summary.task_id,
                subtask_access: summary.subtask_access,
                subtask_status: summary.subtask_status,
                id: summary.id,
                parent_id: summary.parent_id,
                depth: summary.depth,
                root_id: summary.root_id,
                workspace_id: summary.workspace_id,
                title: summary.title,
                version: summary.version,
                relation_kind: summary.relation_kind,
                lifecycle_state: summary.lifecycle_state,
                source_cutoff_seq_global: summary.source_cutoff_seq_global,
                source_message_id: summary.source_message_id,
                created_at: timestamp_millis_to_utc(summary.created_at_ms)?,
                updated_at: timestamp_millis_to_utc(summary.updated_at_ms)?,
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

        let query = agena_storage::SessionSummaryListQuery {
            workspace_id: Some(workspace_id),
            roots_only: false,
            parent_id: None,
            search: None,
            before_updated_at_ms: None,
            before_id: None,
            offset: request.offset,
            limit: request.limit.unwrap_or(u64::MAX),
            include_subagents: request.include_subagents,
        };
        let summaries = self
            .session_summary_repository
            .list(query)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        if summaries.is_empty() {
            return Ok(Vec::new());
        }

        let session_ids = summaries
            .iter()
            .map(|summary| summary.id)
            .collect::<Vec<_>>();
        let message_stats = self
            .session_stats_repository
            .event_stats(&session_ids)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let child_counts = self
            .session_stats_repository
            .child_counts(&session_ids)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;

        let mut out = Vec::with_capacity(summaries.len());
        for summary in summaries {
            let message_stats = message_stats.get(&summary.id).copied();
            let message_count = message_stats
                .map(|stats| u64::try_from(stats.message_count))
                .transpose()
                .map_err(|_| {
                    AppError::Internal(format!(
                        "invalid negative message count for session {}",
                        summary.id
                    ))
                })?
                .unwrap_or_default();
            let child_session_count = child_counts
                .get(&summary.id)
                .copied()
                .map(u64::try_from)
                .transpose()
                .map_err(|_| {
                    AppError::Internal(format!(
                        "invalid negative child session count for session {}",
                        summary.id
                    ))
                })?
                .unwrap_or_default();
            let last_message_at = message_stats
                .and_then(|stats| stats.last_message_at_ms)
                .map(timestamp_millis_to_utc)
                .transpose()?;
            out.push(SessionSummary {
                task_id: summary.task_id,
                subtask_access: summary.subtask_access,
                subtask_status: summary.subtask_status,
                id: summary.id,
                parent_id: summary.parent_id,
                depth: summary.depth,
                root_id: summary.root_id,
                workspace_id: summary.workspace_id,
                title: summary.title,
                version: summary.version,
                relation_kind: summary.relation_kind,
                lifecycle_state: summary.lifecycle_state,
                source_cutoff_seq_global: summary.source_cutoff_seq_global,
                source_message_id: summary.source_message_id,
                created_at: timestamp_millis_to_utc(summary.created_at_ms)?,
                updated_at: timestamp_millis_to_utc(summary.updated_at_ms)?,
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
            let summary = self
                .session_summary_repository
                .get(session_id)
                .await
                .map_err(|error| AppError::Internal(error.to_string()))?
                .ok_or_else(|| AppError::Internal(format!("session not found: {session_id}")))?;
            if summary.lifecycle_state != SessionLifecycleState::Ready {
                return Err(AppError::Internal(format!(
                    "session is not ready: {session_id}"
                )));
            }
            if session.version == summary.version {
                return Ok(session);
            }

            let session_model = session::get_session_by_id(&self.db, session_id)
                .await?
                .ok_or_else(|| AppError::Internal(format!("session not found: {session_id}")))?;
            let mut refreshed = session_from_model(session_model)?;
            let projection = self
                .history
                .load_projection(session_id, refreshed.runtime.clone())
                .await?;
            refreshed.install_projected_messages(projection.messages);
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
        if session_model.lifecycle_state != SessionLifecycleState::Ready {
            return Err(AppError::Internal(format!(
                "session is not ready: {session_id}"
            )));
        }
        let mut session = session_from_model(session_model)?;
        let projection = self
            .history
            .load_projection(session_id, session.runtime.clone())
            .await?;
        session.install_projected_messages(projection.messages);
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
        let Some(updated) = self
            .session_mutation_repository
            .rename(session_id, title.clone())
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
        else {
            return Err(AppError::Internal(format!(
                "session not found: {session_id}"
            )));
        };
        if let Some(session) = access_cache(self.cache.as_ref(), |guard| {
            guard.update(session_id, |session| {
                session.title = title;
                session.refresh_derived();
            })
        })
        .flatten()
        {
            return Ok(session);
        }
        self.load_session(updated.id, cache_policy).await
    }

    pub(crate) async fn fork_session(
        &self,
        source: Session,
        at_message_id: Option<i64>,
        title: String,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let source_prompt_window = source.runtime.prompt_window.clone();
        let events = self.history.list_session_events(source.id).await?;
        if events.is_empty() {
            return self
                .create_fork_from_history_items(
                    source.id,
                    title,
                    Vec::new(),
                    source_prompt_window,
                    session::SessionLineageInput::fork(0, at_message_id),
                    cache_policy,
                )
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
                let run_finished_seq = event_run_id_for_message(&target_event.kind, message_id)
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
                    .unwrap_or(target_seq);
                let execution_finished_seq =
                    event_execution_id_for_message(&target_event.kind, message_id)
                        .and_then(|execution_id| {
                            events
                                .iter()
                                .filter(|event| event.meta.seq_global >= target_seq)
                                .find_map(|event| match &event.kind {
                                    EventKind::ExecutionFinished(payload)
                                        if payload.execution_id == execution_id =>
                                    {
                                        Some(event.meta.seq_global)
                                    }
                                    _ => None,
                                })
                        })
                        .unwrap_or(target_seq);
                run_finished_seq.max(execution_finished_seq)
            }
        };

        let items = events
            .into_iter()
            .filter(|event| event.meta.seq_global <= cutoff_seq && event.kind.is_persistent())
            .map(|event| event.kind)
            .collect::<Vec<_>>();

        self.create_fork_from_history_items(
            source.id,
            title,
            items,
            source_prompt_window,
            session::SessionLineageInput::fork(cutoff_seq, at_message_id),
            cache_policy,
        )
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
        let source_prompt_window = source.runtime.prompt_window.clone();
        let events = self.history.list_session_events(source.id).await?;
        if events.is_empty() {
            return Err(AppError::Internal(format!(
                "message not found in session {}: {message_id}",
                source.id
            )));
        }

        let target_execution_ids = events
            .iter()
            .filter(|event| event_targets_message(&event.kind, message_id))
            .filter_map(|event| event_execution_id_for_message(&event.kind, message_id))
            .collect::<std::collections::HashSet<_>>();
        let rewind_start_seq = events
            .iter()
            .filter(|event| {
                event_targets_message(&event.kind, message_id)
                    || matches!(
                        &event.kind,
                        EventKind::ExecutionStarted(payload)
                            if target_execution_ids.contains(&payload.execution_id)
                    )
            })
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

        self.create_fork_from_history_items(
            source.id,
            title,
            items,
            source_prompt_window,
            session::SessionLineageInput::rewind(cutoff_seq, message_id),
            cache_policy,
        )
        .await
    }

    async fn create_fork_from_history_items(
        &self,
        source_session_id: i64,
        title: String,
        mut items: Vec<EventKind>,
        mut source_prompt_window: crate::session::PromptWindowRuntime,
        lineage: session::SessionLineageInput,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let message_id_map = self.remap_copied_history_ids(&mut items).await?;
        if !remap_prompt_window_for_fork(&mut source_prompt_window, &message_id_map) {
            source_prompt_window.compaction = None;
        }

        let child = self
            .create_session_inner(
                title,
                Some(source_session_id),
                Some(lineage),
                None,
                SessionLifecycleState::Creating,
                cache_policy,
            )
            .await?;
        let child_id = child.id;
        for item in &mut items {
            rewrite_event_session_ids(item, child.id);
        }
        // Silent: subscribers should not observe copied history as fresh
        // activity. Storage ids must already be remapped at this boundary so
        // projection upserts cannot steal the source session's rows.
        let build_result = async {
            let child = self
                .append_history_items_silent(child, items, cache_policy)
                .await?;
            self.inherit_prompt_window_on_fork(child, source_prompt_window, cache_policy)
                .await
        }
        .await;

        match build_result {
            Ok(mut child) => {
                let ready = session::set_session_lifecycle(
                    &self.db,
                    child_id,
                    SessionLifecycleState::Ready,
                    None,
                )
                .await?
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "created branch vanished before activation: {child_id}"
                    ))
                })?;
                let persisted = session_from_model(ready)?;
                child.apply_persisted_metadata(&persisted);
                child.refresh_derived();
                access_cache(self.cache.as_ref(), |guard| {
                    guard.insert(child.clone(), cache_policy);
                });
                Ok(child)
            }
            Err(error) => {
                let failure = error.failure();
                tracing::error!(
                    failure_id = %failure.id,
                    session_id = child_id,
                    diagnostic = %error,
                    "failed to build session branch"
                );
                if let Err(mark_error) = session::set_session_lifecycle(
                    &self.db,
                    child_id,
                    SessionLifecycleState::Failed,
                    Some(failure),
                )
                .await
                {
                    tracing::error!(
                        session_id = child_id,
                        "failed to mark incomplete branch as failed: {mark_error}"
                    );
                }
                access_cache(self.cache.as_ref(), |guard| guard.discard(child_id));
                Err(error)
            }
        }
    }

    pub(crate) async fn usage_stats(&self, query: UsageStatsQuery) -> Result<UsageStats, AppError> {
        let generated_at = Utc::now();
        let Some(workspace_id) = self.lookup_workspace_id().await? else {
            return Ok(agena_runtime::summarize_usage_records(
                &[],
                &query,
                generated_at,
            ));
        };

        let rows = self
            .usage_repository
            .list(
                workspace_id,
                &query.session_ids,
                query.include_subagents,
                query.from.as_ref().map(|value| value.timestamp_millis()),
                query.to.as_ref().map(|value| value.timestamp_millis()),
            )
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let records = rows
            .into_iter()
            .map(|row| {
                let usage: agena_provider::CompletionUsage =
                    serde_json::from_value(row.usage.value)
                        .map_err(|error| AppError::Internal(error.to_string()))?;
                Ok(UsageStatRecord {
                    session_id: row.session_id,
                    session_title: row.session_title,
                    is_subagent: row.is_subagent,
                    created_at: timestamp_millis_to_utc(row.created_at_ms)?,
                    provider_id: row.provider_id,
                    model_id: row.model_id,
                    usage,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;

        Ok(agena_runtime::summarize_usage_records(
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
            source_session_id: model.id,
            title: model.title.clone(),
            source_parent_id: model.parent_id,
            source_relation_kind: model.relation_kind,
            source_cutoff_seq_global: model.source_cutoff_seq_global,
            source_message_id: model.source_message_id,
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
        // Import always creates an independent root session. A delegated-task
        // lifecycle references a parent that is not part of this single-
        // session bundle, so replaying it would create an orphan task event.
        events.retain(|kind| !matches!(kind, EventKind::SubtaskStatusChanged(_)));

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
        rewrite_copied_domain_ids(&mut events);

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
        let mut imported_runtime = meta.runtime_state;
        imported_runtime.rewrite_storage_ids(id_offset, part_id_offset);
        imported_runtime.subtask = Default::default();
        imported_runtime.execution.permission_ceiling = Default::default();
        imported_runtime.execution.effective_workspace_root = None;
        if !imported_runtime.prompt_tokens.is_empty()
            || !imported_runtime.provider_anchors.is_empty()
            || !imported_runtime.execution.is_empty()
            || !imported_runtime.prompt_window.is_empty()
        {
            let runtime = imported_runtime.clone();
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
            session.runtime = imported_runtime;
            session.refresh_derived();
            access_cache(self.cache.as_ref(), |guard| {
                guard.insert(session.clone(), cache_policy);
            });
        }
        Ok(session)
    }

    async fn inherit_prompt_window_on_fork(
        &self,
        mut child: Session,
        source_prompt_window: crate::session::PromptWindowRuntime,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let checkpoint_retained =
            source_prompt_window
                .compaction
                .as_ref()
                .is_some_and(|checkpoint| {
                    child
                        .messages
                        .iter()
                        .any(|message| message.id == checkpoint.compacted_through_message_id)
                });
        if !checkpoint_retained {
            return Ok(child);
        }
        child.runtime.prompt_window = source_prompt_window;
        child.runtime.clear_prompt_tokens();
        child.runtime.clear_provider_anchors();
        let expected_version = Some(child.version);
        self.persist(
            super::SessionCommit {
                session: child,
                checkpoints: Vec::new(),
                client_events: Vec::new(),
                persisted_rules: Vec::new(),
                expected_version,
            },
            cache_policy,
        )
        .await
    }
}

fn remap_prompt_window_for_fork(
    prompt_window: &mut crate::session::PromptWindowRuntime,
    message_id_map: &BTreeMap<i64, i64>,
) -> bool {
    let Some(compaction) = prompt_window.compaction.as_mut() else {
        return true;
    };
    let remap = |id: &mut i64| {
        if *id <= 0 {
            return true;
        }
        let Some(mapped) = message_id_map.get(id).copied() else {
            return false;
        };
        *id = mapped;
        true
    };

    let mut fully_mapped = remap(&mut compaction.compacted_through_message_id);
    if let crate::session::PromptCompactionContent::TextSummary {
        recent_messages, ..
    } = &mut compaction.content
    {
        for message in recent_messages {
            fully_mapped &= remap(&mut message.id);
        }
    }
    fully_mapped
}
