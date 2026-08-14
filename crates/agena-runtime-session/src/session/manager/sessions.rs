use super::{
    AppError, SessionCreateRequest, SessionListRequest, SessionManager, SessionManagerState,
    SessionRunOptions, SessionSummary,
};
use crate::session::Session;
use crate::session::prompt_window;
use crate::session::store::new_part_from_content;
use agena_domain::{ModelRef, SessionUsage, SessionUsageLimitBasis};
use agena_runtime_contracts::part_content::TypedContent;
use agena_storage::store::{PartRole, PartState};
use std::collections::HashMap;

impl SessionManager {
    pub async fn reconcile_interrupted_executions(&self) -> Result<(), AppError> {
        for session_id in self.workspace_session_ids().await? {
            self.reconcile_interrupted_session(session_id).await?;
        }
        Ok(())
    }

    /// Reconcile one session's interrupted lifecycles and subagent subtask
    /// state without scanning unrelated sessions. Skips sessions with a live
    /// execution lease: another process is actively running them, so their
    /// RunStarted entries are not interrupted and must not be aborted.
    pub(super) async fn reconcile_interrupted_session(
        &self,
        session_id: i64,
    ) -> Result<(), AppError> {
        // A live cross-process run means another process is actively running
        // this session; its interrupted work is not ours to abort (17.4 2b).
        let presentation = self.store.session_state(session_id).await?;
        if matches!(
            presentation.state,
            agena_storage::store::SessionState::Running
                | agena_storage::store::SessionState::AwaitingUser
        ) {
            return Ok(());
        }
        // Abort the session's in-flight run markers (17.4 2c). The engine
        // owns lease freshness; an interrupted (stale-lease) run is failed.
        self.store.reconcile(session_id).await?;
        let mut session = self.store.load_session(session_id).await?;
        if session.is_subagent()
            && session.runtime.subtask.status == agena_domain::SubtaskStatus::Running
            && !self.execution_registry.is_active(session_id).await
        {
            session.runtime.subtask.status = agena_domain::SubtaskStatus::Interrupted;
            session.runtime.subtask.finished_at_ms = Some(chrono::Utc::now().timestamp_millis());
            session.runtime.subtask.failure = Some(interrupted_subtask_failure());
            let interrupted_at_ms = session.runtime.subtask.finished_at_ms;
            let subtask_started_at_ms = session.runtime.subtask.started_at_ms;
            let subtask_failure = session
                .runtime
                .subtask
                .failure
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| {
                    AppError::Internal(format!("serialize subtask failure: {error}"))
                })?;
            self.store
                .update_subtask_state(
                    session,
                    Some(
                        agena_domain::SubtaskStatus::Interrupted
                            .as_ref()
                            .to_string(),
                    ),
                    subtask_started_at_ms,
                    interrupted_at_ms,
                    subtask_failure,
                )
                .await?;
        }
        Ok(())
    }

    /// Lazy interrupted-run reconciliation for the session the user is about
    /// to open, plus its subagent children (whose state is displayed under the
    /// parent in the session tree). Runs at most once per session per process,
    /// replacing the startup full-workspace scan that delayed `tui` on large
    /// databases. Stale execution leases are still stolen atomically on demand
    /// by `register`, and per-run/per-load cleanup keeps sessions current
    /// after the first open.
    async fn reconcile_session_on_open(&self, session_id: i64) -> Result<(), AppError> {
        {
            let mut reconciled = self.reconciled_sessions.lock().await;
            if !reconciled.insert(session_id) {
                return Ok(());
            }
        }
        // A live cross-process run belongs to another process, and a pending
        // interaction means the run is deliberately paused for the user.
        // Neither condition is an interrupted execution (17.4 steps 2a-2b).
        let presentation = self.store.session_state(session_id).await?;
        if matches!(
            presentation.state,
            agena_storage::store::SessionState::Running
                | agena_storage::store::SessionState::AwaitingUser
        ) {
            return Ok(());
        }
        self.reconcile_interrupted_session(session_id).await?;
        let session = self.store.load_session(session_id).await?;
        let root_id = session.root_id;
        for summary in self.store.list_session_tree(root_id).await? {
            if summary.parent_id == Some(session_id) {
                self.reconcile_interrupted_session(summary.id).await?;
            }
        }
        Ok(())
    }

    /// Reclaim stale execution leases (from crashed processes) and reconcile
    /// the interrupted runs of the reclaimed sessions. Called periodically by
    /// a maintenance loop so a running process can recover another process's
    /// crashed run without waiting for a restart.
    pub async fn reap_stale_leases(&self) -> Result<(), AppError> {
        // Engine-owned maintenance: reap stale leases and GC orphan parts
        // (14.2). Reconcile-on-open handles the interrupted-run recovery for
        // each reclaimed session the next time it is loaded.
        let outcome = self
            .store
            .maintenance()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        if !outcome.reaped_sessions.is_empty() || outcome.gc_deleted_parts > 0 {
            tracing::info!(
                target: "agena_session::maintenance",
                reaped_sessions = ?outcome.reaped_sessions,
                gc_deleted_parts = outcome.gc_deleted_parts,
                "maintenance reaped stale leases and GC'd orphan parts"
            );
        }
        Ok(())
    }

    pub async fn active_execution(
        &self,
        session_id: i64,
    ) -> Option<agena_domain::ExecutionLifecycle> {
        self.execution_registry.execution(session_id).await
    }
    pub async fn create_session(&self, request: SessionCreateRequest) -> Result<Session, AppError> {
        let state = self.execution_state();
        let workspace_id = self.current_workspace_id().await?;
        let relation_kind = if request.parent_session_id.is_some() {
            agena_domain::SessionRelationKind::Child
        } else {
            agena_domain::SessionRelationKind::Root
        };
        let mut session = self
            .store
            .create_session(
                workspace_id,
                request.parent_session_id,
                relation_kind,
                None,
                request.title,
                None,
                None,
            )
            .await?;

        let patch = match state
            .tool_executor
            .plugin_manager()
            .dispatch_session_start(agena_plugin_host::SessionStartInput {
                session_id: session.id,
                source: agena_plugin_host::SessionStartSource::Startup,
                workspace_root: state.tool_executor.workspace_root().display().to_string(),
                model: None,
            })
            .await
        {
            Ok(patch) => patch,
            Err(err) => {
                tracing::warn!(
                    target: "agena_plugin_host::session_start",
                    session_id = session.id,
                    "session.start hook failed during session creation: {err}"
                );
                return Ok(session);
            }
        };

        let mut injected_new_parts = Vec::new();
        if let Some(additional_context) = patch.additional_context {
            injected_new_parts.push(new_part_from_content(
                "text",
                PartRole::System,
                &TypedContent::Text(crate::session::store::text_content(additional_context)),
                PartState::Completed,
            )?);
        }
        if let Some(initial_user_message) = patch.initial_user_message {
            injected_new_parts.push(new_part_from_content(
                "text",
                PartRole::User,
                &TypedContent::Text(crate::session::store::text_content(initial_user_message)),
                PartState::Completed,
            )?);
        }

        let hook_runs = state
            .tool_executor
            .plugin_manager()
            .drain_hook_runs(session.id);
        // The `user_send` receipt must exist before session.start hook parts can
        // be grouped beneath it when no assistant run exists yet. Create it up
        // front — empty when only hooks need a home. An all-terminal input batch
        // creates a terminal receipt; `record_hook_runs` appends through the
        // terminal-safe settle path without reopening it. The parts carry their
        // own role so the reloaded projection preserves the System/User
        // distinction.
        if !injected_new_parts.is_empty() || !hook_runs.is_empty() {
            let outcome = self
                .store
                .submit_user_run(session.id, injected_new_parts, None)
                .await?;
            let mut projected = session.parts().to_vec();
            projected.extend(outcome.parts);
            session.install_projected_parts(projected);
        }
        // Record the session.start hook runs observed during creation (plus
        // any unattributed runs, such as config/provider.list, that happened
        // before this session existed) as transcript activity.
        if !hook_runs.is_empty() {
            session = self
                .record_hook_runs(session, hook_runs, state.clone())
                .await?;
        }
        Ok(session)
    }

    pub async fn get_session(&self, session_id: i64) -> Result<Session, AppError> {
        let state = self.execution_state();
        // Reconcile interrupted runs lazily on first open so the transcript
        // shows aborted (not stuck in-progress) replies without scanning
        // unrelated sessions at startup.
        self.reconcile_session_on_open(session_id).await?;
        let mut session = self.store.load_session(session_id).await?;
        // The persisted runtime contains a historical effective-permission
        // snapshot. It is not the source of truth after a config reload or a
        // live session-permission edit. Rebuild it from the current shared
        // overlays before any caller creates a scoped executor from it.
        self.refresh_execution_policy(&mut session, &state);
        Ok(session)
    }

    pub async fn rename_session(
        &self,
        session_id: i64,
        title: String,
    ) -> Result<Session, AppError> {
        self.store.rename_session(session_id, title).await
    }

    /// Replace one session's persisted model selection without starting a
    /// run. Future turns resolve from this session-local selection.
    pub async fn update_session_selection(
        &self,
        session_id: i64,
        options: SessionRunOptions,
    ) -> Result<Session, AppError> {
        if self.execution_registry.is_active(session_id).await {
            return Err(AppError::Config(format!(
                "cannot change the model selection while session {session_id} has an active run"
            )));
        }
        let state = self.execution_state();
        let mut session = self.store.load_session(session_id).await?;
        if !self.apply_run_selection_to_session(&mut session, &options) {
            return Ok(session);
        }
        // Persist the selection to `sessions.config_json`. `persist_session_changes`
        // is a no-op without changed parts, which would leave the override
        // in-memory only — invisible to `selected_model()` and the TUI's
        // post-send sync after the next store load.
        let persisted = self.store.persist_execution_config(session).await?;
        if let Ok(mut permissions) = state.shared_session_permissions.write() {
            permissions.insert(
                session_id,
                persisted.runtime.execution.selection.permission.clone(),
            );
        }
        Ok(persisted)
    }

    /// Persist a validated model override for future session turns. This is
    /// intentionally rejected while a generation is active so a plugin cannot
    /// change the provider/model underneath an in-flight completion.
    pub async fn set_session_model_override(
        &self,
        session_id: i64,
        model: ModelRef,
    ) -> Result<Session, AppError> {
        if self.execution_registry.is_active(session_id).await {
            return Err(AppError::Config(format!(
                "cannot change the model selection while session {session_id} has an active run"
            )));
        }
        let state = self.execution_state();
        state.provider_registry.model_metadata(&model)?;
        let mut session = self.store.load_session(session_id).await?;
        session.runtime.set_model_override(
            Some(model.provider_id.to_string()),
            model.adapter_id.as_ref().map(ToString::to_string),
            Some(model.model_id.to_string()),
        );
        self.store.persist_execution_config(session).await
    }

    pub async fn session_usage_async(&self, session: &Session) -> Result<SessionUsage, AppError> {
        let state = self.execution_state();
        let scoped_executor = state
            .tool_executor
            .for_session_context_async(&session.runtime.execution)
            .await;
        let options = self
            .session_usage_run_options(session, state.as_ref(), &scoped_executor)
            .ok();
        let native_compaction_enabled = options
            .as_ref()
            .map(|options| {
                state
                    .provider_registry
                    .native_compaction_enabled(&options.model)
            })
            .transpose()?
            .unwrap_or(false);
        let agena_tool_mode = options
            .as_ref()
            .and_then(|options| state.provider_registry.agena_tool_mode(&options.model).ok())
            .unwrap_or_default();
        let tool_api_functions = if agena_tool_mode.is_disabled() {
            Vec::new()
        } else {
            scoped_executor.available_tool_api_bindings_async().await
        };
        self.session_usage_with_catalog(
            session,
            state.as_ref(),
            &options,
            native_compaction_enabled,
            tool_api_functions,
        )
    }

    fn session_usage_run_options(
        &self,
        session: &Session,
        state: &SessionManagerState,
        scoped_executor: &crate::tool::ToolExecutor,
    ) -> Result<SessionRunOptions, AppError> {
        let model = self.model_from_session_or_default(session, state)?;
        let mut options = SessionRunOptions {
            model,
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: None,
        };
        self.apply_selection_modes_to_run_options(session, &mut options)?;
        options.system =
            Some(self.assemble_session_system_prompt_with_executor(scoped_executor, None));
        if let Ok(metadata) = state.provider_registry.model_metadata(&options.model) {
            options.temperature = metadata.parsed_default_temperature();
        }
        Ok(options)
    }

    fn session_usage_with_catalog(
        &self,
        session: &Session,
        state: &SessionManagerState,
        options: &Option<SessionRunOptions>,
        native_compaction_enabled: bool,
        tool_api_functions: Vec<crate::tool::ToolApiBinding>,
    ) -> Result<SessionUsage, AppError> {
        let request_system = options
            .as_ref()
            .and_then(|options| options.system.clone())
            .or_else(|| Some(crate::identity::system_prompt()));
        let metadata = options
            .as_ref()
            .and_then(|options| state.provider_registry.model_metadata(&options.model).ok())
            .unwrap_or_default();
        let context_window_tokens = metadata.limits.context_window_tokens;
        let max_input_tokens = metadata.limits.max_input_tokens;
        let max_output_tokens = options
            .as_ref()
            .and_then(|options| options.max_output_tokens)
            .or(metadata.limits.max_output_tokens);
        let reserved_tokens = agena_runtime::estimate_auto_compaction_reserve_tokens(
            context_window_tokens,
            max_input_tokens,
            max_output_tokens,
            state.config.auto_compaction.reserved_tokens,
        );
        let context_auto_compact_limit_tokens =
            agena_runtime::estimate_auto_compaction_limit_tokens(
                context_window_tokens,
                max_input_tokens,
                max_output_tokens,
                state.config.auto_compaction.reserved_tokens,
            );
        let (auto_compact_limit_tokens, limit_basis) =
            if let Some(limit) = context_auto_compact_limit_tokens {
                (Some(limit), Some(SessionUsageLimitBasis::ContextWindow))
            } else {
                (
                    Some(agena_runtime::estimate_prompt_budget_threshold_tokens(
                        context_window_tokens,
                        max_output_tokens,
                    )),
                    Some(SessionUsageLimitBasis::PromptThreshold),
                )
            };

        let prompt_fingerprints = options.as_ref().map(|options| {
            let provider_request_shape = state
                .provider_registry
                .prompt_cache_shape(&options.model)
                .ok()
                .flatten();
            let continuation_supported = state
                .provider_registry
                .supports_prompt_continuation(&options.model)
                .unwrap_or(false);
            prompt_window::prompt_request_fingerprints(
                &crate::session::prompt_window::PromptRequestOptions {
                    provider_id: options.model.provider_id.as_ref(),
                    adapter_id: options.model.adapter_id.as_ref().map(AsRef::as_ref),
                    model_id: options.model.model_id.as_ref(),
                    system: request_system.as_deref(),
                    temperature: options.temperature,
                    max_output_tokens: options.max_output_tokens,
                    tool_api_functions: tool_api_functions.as_slice(),
                    provider_request_shape: provider_request_shape.as_ref(),
                    continuation_supported,
                    native_compaction_enabled,
                },
            )
        });
        let projected_tokens = prompt_fingerprints.as_ref().and_then(|fingerprints| {
            prompt_window::estimate_prompt_tokens_from_runtime(
                session,
                session.active_window_parts(),
                fingerprints.system_fingerprint.as_str(),
                fingerprints.request_options_fingerprint.as_str(),
            )
            .map(|estimate| estimate.total_tokens)
        });
        let measured_prompt_tokens = session.runtime.prompt_tokens.prompt_tokens();
        let provider_compaction = options.as_ref().and_then(|options| {
            prompt_window::provider_compaction_for_model(
                session,
                options.model.provider_id.as_ref(),
                options.model.adapter_id.as_ref().map(AsRef::as_ref),
                options.model.model_id.as_ref(),
                native_compaction_enabled,
            )
        });
        let approximate_tokens = prompt_window::approximate_total_request_tokens_with_compaction(
            session.active_window_parts(),
            request_system.as_deref(),
            tool_api_functions.as_slice(),
            provider_compaction.as_ref(),
        );
        let current_tokens = measured_prompt_tokens
            .into_iter()
            .chain(projected_tokens)
            .chain(std::iter::once(approximate_tokens))
            .max()
            .unwrap_or_default();
        Ok(SessionUsage {
            measured_prompt_tokens,
            current_tokens,
            projected_tokens,
            limit_tokens: auto_compact_limit_tokens,
            limit_basis,
            reserved_tokens,
            model_context_window_tokens: context_window_tokens,
            model_max_input_tokens: max_input_tokens,
            model_max_output_tokens: max_output_tokens,
        })
    }

    /// The session's **effective** model selection after resolving the default
    /// model and applying every mode cascade/registry default.
    ///
    /// Unlike `execution.selection` (which holds only the persisted per-session
    /// override and is empty for default-model sessions), this resolves the
    /// concrete provider/adapter/model that would actually run plus the
    /// effective thinking/speed/verbosity modes.
    pub fn session_model_status(
        &self,
        session: &Session,
    ) -> Result<agena_domain::ModelSelectionConfig, AppError> {
        let state = self.execution_state();
        let model = self.model_from_session_or_default(session, &state)?;
        let mut options = SessionRunOptions {
            model,
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: None,
        };
        self.apply_selection_modes_to_run_options(session, &mut options)?;
        Ok(agena_domain::ModelSelectionConfig {
            provider: Some(options.model.provider_id.to_string()),
            adapter: options.model.adapter_id.as_ref().map(ToString::to_string),
            model: Some(options.model.model_id.to_string()),
            thinking_mode: options.thinking_mode,
            speed_mode: options.speed_mode,
            verbosity: options.verbosity,
            parallel_tool_calls: options.request_override.parallel_tool_calls(),
        })
    }

    pub async fn set_session_permission(
        &self,
        session_id: i64,
        permission: crate::authorization::PermissionConfig,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self.store.load_session(session_id).await?;
        let previous_shared_permission = state
            .shared_session_permissions
            .write()
            .ok()
            .and_then(|mut permissions| permissions.insert(session_id, permission.clone()));
        session.runtime.execution.selection.permission = permission;
        session.runtime.execution.effective_permission =
            self.resolve_effective_session_permission(&session, &state);
        let persisted = match self
            .persist_session_changes(session, Vec::new(), None, state.clone())
            .await
        {
            Ok(persisted) => persisted,
            Err(error) => {
                if let Ok(mut permissions) = state.shared_session_permissions.write() {
                    match previous_shared_permission {
                        Some(previous) => {
                            permissions.insert(session_id, previous);
                        }
                        None => {
                            permissions.remove(&session_id);
                        }
                    }
                }
                return Err(error);
            }
        };
        // An execution may already hold an older in-memory Session snapshot.
        // Keep the shared overlay in sync with the durable write so its next
        // permission preflight observes this update immediately.
        if let Ok(mut permissions) = state.shared_session_permissions.write() {
            permissions.insert(
                session_id,
                persisted.runtime.execution.selection.permission.clone(),
            );
        }
        Ok(persisted)
    }

    pub async fn is_run_active(&self, session_id: i64) -> bool {
        self.execution_registry.is_active(session_id).await
    }

    pub async fn workspace_session_ids(&self) -> Result<Vec<i64>, AppError> {
        let workspace_id = self.current_workspace_id().await?;
        let summaries = self
            .store
            .list_session_summaries(
                workspace_id,
                agena_domain::SessionListRequest {
                    offset: 0,
                    limit: None,
                    include_subagents: true,
                    ..Default::default()
                },
            )
            .await?;
        Ok(summaries.into_iter().map(|summary| summary.id).collect())
    }

    pub async fn list_projected_runs(
        &self,
        session_id: i64,
        include_full_parts: bool,
    ) -> Result<Vec<crate::session_query_service::SessionProjectedRun>, AppError> {
        // v2 has no separate "projected" transcript: the canonical read is
        // the aggregate rebuilt from parts (`load_session`). `include_full_parts`
        // is retained for callers that historically fetched headers only; the
        // aggregate always carries full parts.
        let _ = include_full_parts;
        let session = self.store.load_session(session_id).await?;
        super::history::projected_runs_from_parts(session.parts())
    }

    pub async fn broadcast_session_end(
        &self,
        session_id: i64,
        reason: agena_plugin_host::SessionEndReason,
    ) {
        self.execution_state()
            .tool_executor
            .plugin_manager()
            .broadcast_session_end(agena_plugin_host::SessionEndInput { session_id, reason })
            .await;
    }

    pub async fn broadcast_active_session_end(&self, reason: agena_plugin_host::SessionEndReason) {
        let session_ids = self.execution_registry.active_session_ids().await;
        for session_id in session_ids {
            self.broadcast_session_end(session_id, reason).await;
        }
    }

    pub async fn list_session_summaries(
        &self,
        request: SessionListRequest,
    ) -> Result<Vec<SessionSummary>, AppError> {
        let workspace_id = self.current_workspace_id().await?;
        self.store
            .list_session_summaries(workspace_id, request)
            .await
    }

    /// Fetch one session's summary row as the shared domain DTO, or `None`.
    pub async fn get_session_summary(
        &self,
        session_id: i64,
    ) -> Result<Option<SessionSummary>, AppError> {
        self.store.get_session_summary(session_id).await
    }

    /// Session counts per workspace (13.5 `workspace_counts`).
    pub async fn session_counts_by_workspace(
        &self,
        workspace_ids: &[i64],
    ) -> Result<HashMap<i64, i64>, AppError> {
        self.store.session_counts_by_workspace(workspace_ids).await
    }
}

fn interrupted_subtask_failure() -> agena_failure::Failure {
    agena_failure::Failure::new(
        agena_failure::FailureCode::new("subtask.interrupted"),
        agena_failure::FailureCategory::DependencyUnavailable,
        agena_failure::FailureResponsibility::System,
        agena_failure::RetryDirective::AfterUserAction,
        agena_failure::RecoveryDirective::Retry,
        agena_failure::FailureImpact::OperationFailed,
        agena_failure::UserPresentation::new(
            "subtask-interrupted",
            "The subtask was interrupted by a runtime restart. Retry the subtask.",
        ),
    )
}

#[async_trait::async_trait]
impl agena_runtime::SessionExecutionControl for SessionManager {
    async fn active_execution(&self, session_id: i64) -> Option<agena_domain::ExecutionLifecycle> {
        SessionManager::active_execution(self, session_id).await
    }

    async fn cancel_execution(
        &self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) -> Result<agena_domain::CancellationResult, agena_runtime::SessionExecutionControlError> {
        SessionManager::cancel_execution(self, session_id, execution_id)
            .await
            .map_err(|error| {
                agena_runtime::SessionExecutionControlError::internal(error.to_string())
            })
    }

    async fn list_scheduled_jobs(&self) -> Vec<agena_scheduler::ScheduledJob> {
        let Some(scheduler) = self.tool_executor().scheduler().cloned() else {
            return Vec::new();
        };
        scheduler.list().await
    }

    fn scheduler_available(&self) -> bool {
        self.tool_executor().scheduler().is_some()
    }

    async fn selected_model(
        &self,
        session_id: i64,
    ) -> Result<Option<agena_domain::ModelRef>, agena_runtime::SessionExecutionControlError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| {
                agena_runtime::SessionExecutionControlError::internal(error.to_string())
            })?;
        session.runtime().effective_model_ref().map_err(|error| {
            agena_runtime::SessionExecutionControlError::internal(format!(
                "session {session_id} contains invalid persisted model reference: {error}"
            ))
        })
    }

    fn cache_stats(&self) -> agena_domain::SessionCacheStats {
        SessionManager::cache_stats(self)
    }

    fn snapshot_status(
        &self,
        workspace_root: &std::path::Path,
    ) -> Option<agena_runtime::RuntimeSnapshotStatus> {
        let executor = self.tool_executor();
        let registry = executor.snapshot_registry()?;
        let active = agena_runtime::list_active_snapshots(registry)
            .into_iter()
            .map(|snapshot| agena_runtime::RuntimeActiveSnapshot {
                session_id: snapshot.session_id,
                path: snapshot.path.display().to_string(),
                branch: snapshot.branch,
                backend: snapshot.backend,
                created_here: snapshot.created_here,
            })
            .collect();
        let managed = agena_runtime::list_managed_snapshots(workspace_root, registry)
            .into_iter()
            .map(|snapshot| agena_runtime::RuntimeManagedSnapshot {
                stale: snapshot.is_stale(),
                path: snapshot.path.display().to_string(),
                session_id: snapshot.session_id,
                branch: snapshot.branch,
                backend: snapshot.backend,
                registered_with_git: snapshot.registered_with_git,
                registered_with_rift: snapshot.registered_with_rift,
            })
            .collect();
        Some(agena_runtime::RuntimeSnapshotStatus { active, managed })
    }
}
