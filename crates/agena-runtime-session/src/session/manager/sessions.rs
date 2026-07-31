use super::{
    AppError, ExecutionStatus, Message, MessageMetadata, MessageSource, PartContent, Role,
    SessionCreateRequest, SessionListRequest, SessionManager, SessionRunOptions, SessionSummary,
    build_message,
};
use crate::session::Session;
use crate::session::prompt_window;
use agena_domain::{ModelRef, SessionUsage, SessionUsageLimitBasis};

impl SessionManager {
    pub async fn reconcile_interrupted_executions(&self) -> Result<(), AppError> {
        let state = self.execution_state();
        for session_id in self.workspace_session_ids().await? {
            self.store
                .reconcile_interrupted_lifecycles(session_id)
                .await?;
            let mut session = self
                .store
                .load_session(session_id, state.cache_policy())
                .await?;
            if session.is_subagent()
                && session.runtime.subtask.status == agena_domain::SubtaskStatus::Running
                && !self.execution_registry.is_active(session_id).await
            {
                session.runtime.subtask.status = agena_domain::SubtaskStatus::Interrupted;
                session.runtime.subtask.finished_at_ms =
                    Some(chrono::Utc::now().timestamp_millis());
                session.runtime.subtask.error = Some(
                    "subtask execution was interrupted by runtime shutdown or restart".to_string(),
                );
                let interrupted_at_ms = session.runtime.subtask.finished_at_ms;
                let lifecycle_event = session.parent_id.zip(session.task_id.clone()).map(
                    |(parent_session_id, task_id)| {
                        crate::event::EventKind::SubtaskStatusChanged(
                            agena_domain::SubtaskStatusChangedEvent {
                                session_id,
                                parent_session_id,
                                task_id,
                                access: session.runtime.execution.access,
                                status: agena_domain::SubtaskStatus::Interrupted,
                                resumed: false,
                                started_at_ms: session.runtime.subtask.started_at_ms,
                                finished_at_ms: interrupted_at_ms,
                                error: session.runtime.subtask.error.clone(),
                                ts_ms: interrupted_at_ms
                                    .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
                            },
                        )
                    },
                );
                let subtask = session.runtime.subtask.clone();
                session = self
                    .store
                    .update_subtask_state(session, subtask, state.cache_policy())
                    .await?;
                self.persist_session_changes(
                    session,
                    Vec::new(),
                    lifecycle_event.into_iter().collect(),
                    None,
                    state.clone(),
                )
                .await?;
            }
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
        let mut session = self
            .store
            .create_session(
                request.title,
                request.parent_session_id,
                state.cache_policy(),
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

        let mut injected_messages = Vec::new();
        if let Some(additional_context) = patch.additional_context {
            let ids = self.store.reserve_message_ids(1).await?;
            let system_message = build_message(
                ids,
                Role::System,
                ExecutionStatus::Completed,
                vec![PartContent::text(additional_context)],
                MessageMetadata {
                    source: MessageSource::System,
                    idempotency_key: None,
                    turn_id: None,
                    parent_message_id: session
                        .last_conversation_message()
                        .map(|message| message.id),
                    generated_by_call_id: None,
                    externally_initiated_tool: false,
                    model_provider_id: String::new(),
                    model_adapter_id: None,
                    model_id: String::new(),
                    model_thinking_mode: None,
                    model_speed_mode: None,
                },
            );
            session.messages.push(system_message.clone());
            injected_messages.push(system_message);
        }
        if let Some(initial_user_message) = patch.initial_user_message {
            let ids = self.store.reserve_message_ids(1).await?;
            let initial_turn_id = ids.message_id;
            let user_message = build_message(
                ids,
                Role::User,
                ExecutionStatus::Completed,
                vec![PartContent::text(initial_user_message)],
                MessageMetadata {
                    source: MessageSource::System,
                    idempotency_key: None,
                    turn_id: Some(initial_turn_id),
                    parent_message_id: session
                        .last_conversation_message()
                        .map(|message| message.id),
                    generated_by_call_id: None,
                    externally_initiated_tool: false,
                    model_provider_id: String::new(),
                    model_adapter_id: None,
                    model_id: String::new(),
                    model_thinking_mode: None,
                    model_speed_mode: None,
                },
            );
            session.messages.push(user_message.clone());
            injected_messages.push(user_message);
        }

        if injected_messages.is_empty() {
            return Ok(session);
        }

        self.persist_session_changes(session, injected_messages, Vec::new(), None, state)
            .await
    }

    pub async fn get_session(&self, session_id: i64) -> Result<Session, AppError> {
        let state = self.execution_state();
        self.store
            .load_session(session_id, state.cache_policy())
            .await
    }

    pub async fn rename_session(
        &self,
        session_id: i64,
        title: String,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        self.store
            .rename_session(session_id, title, state.cache_policy())
            .await
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
        let mut session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        if !self.apply_run_selection_to_session(&mut session, &options) {
            return Ok(session);
        }
        self.persist_session_changes(session, Vec::new(), Vec::new(), None, state)
            .await
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
        state.processor.model_metadata(&model)?;
        let mut session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        session.runtime.set_model_override(
            Some(model.provider_id.to_string()),
            model.adapter_id.as_ref().map(ToString::to_string),
            Some(model.model_id.to_string()),
        );
        self.persist_session_changes(session, Vec::new(), Vec::new(), None, state)
            .await
    }

    pub fn session_usage(&self, session: &Session) -> Result<SessionUsage, AppError> {
        let state = self.execution_state();
        let options = self.run_options_from_session(session, state.clone()).ok();
        let native_compaction_enabled = options
            .as_ref()
            .map(|options| {
                state
                    .processor
                    .provider_registry()
                    .native_compaction_enabled(&options.model)
            })
            .transpose()?
            .unwrap_or(false);
        let active_messages = options.as_ref().map_or_else(
            || prompt_window::active_prompt_messages(session),
            |options| {
                prompt_window::active_prompt_messages_for_model(
                    session,
                    Some(options.model.provider_id.as_ref()),
                    options.model.adapter_id.as_ref().map(AsRef::as_ref),
                    Some(options.model.model_id.as_ref()),
                    native_compaction_enabled,
                )
            },
        );
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution);
        let agena_tool_mode = options
            .as_ref()
            .and_then(|options| {
                state
                    .processor
                    .provider_registry()
                    .agena_tool_mode(&options.model)
                    .ok()
            })
            .unwrap_or_default();
        let tool_api_functions = if agena_tool_mode.is_disabled() {
            Vec::new()
        } else {
            scoped_executor.available_tool_api_bindings()
        };
        let request_system = options
            .as_ref()
            .and_then(|options| options.system.clone())
            .or_else(|| {
                Some(crate::identity::system_prompt(
                    session.is_subagent(),
                    session.runtime.execution.access,
                    state.tool_executor.workspace_root(),
                ))
            });
        let metadata = options
            .as_ref()
            .and_then(|options| state.processor.model_metadata(&options.model).ok())
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
                .processor
                .prompt_cache_shape(&options.model)
                .ok()
                .flatten();
            let continuation_supported =
                state.processor.supports_prompt_continuation(&options.model);
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
                active_messages.as_slice(),
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
            active_messages.as_slice(),
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

    pub async fn set_session_permission(
        &self,
        session_id: i64,
        permission: crate::authorization::PermissionConfig,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        session.runtime.execution.selection.permission = permission;
        session.runtime.execution.effective_permission =
            self.resolve_effective_session_permission(&session, &state);
        self.persist_session_changes(session, Vec::new(), Vec::new(), None, state)
            .await
    }

    pub async fn is_run_active(&self, session_id: i64) -> bool {
        self.execution_registry.is_active(session_id).await
    }

    pub async fn resolve_scheduled_run_options(
        &self,
        session_id: i64,
    ) -> Result<SessionRunOptions, AppError> {
        let session = self.get_session(session_id).await?;
        let state = self.execution_state();
        let model = self.model_from_session_or_default(&session, &state)?;
        self.apply_execution_context_to_run_options(
            &session,
            SessionRunOptions {
                model,
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
            },
        )
    }

    pub async fn workspace_session_ids(&self) -> Result<Vec<i64>, AppError> {
        self.store.list_workspace_session_ids().await
    }

    pub async fn list_projected_messages(
        &self,
        session_id: i64,
        include_full_parts: bool,
    ) -> Result<Vec<Message>, AppError> {
        self.store
            .list_projected_messages(session_id, include_full_parts)
            .await
    }

    /// Returns whether a persisted user message already owns an external
    /// idempotency key. Scheduler/connector sinks call this before replaying a
    /// delivery that may have been submitted just before a process crash.
    pub async fn has_user_message_idempotency_key(
        &self,
        session_id: i64,
        key: &str,
    ) -> Result<bool, AppError> {
        if key.trim().is_empty() {
            return Ok(false);
        }
        Ok(self
            .store
            .list_projected_messages(session_id, false)
            .await?
            .into_iter()
            .any(|message| {
                message.role == agena_domain::Role::User
                    && message.metadata.idempotency_key.as_deref() == Some(key)
            }))
    }

    pub async fn list_projected_message_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<crate::session::ProjectedMessageHeader>, AppError> {
        self.store.list_projected_message_headers(session_id).await
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

    pub async fn find_session_id_for_message(
        &self,
        message_id: i64,
    ) -> Result<Option<i64>, AppError> {
        self.store
            .find_projected_session_id_for_message(message_id)
            .await
    }

    pub async fn find_session_id_for_part(&self, part_id: i64) -> Result<Option<i64>, AppError> {
        self.store.find_projected_session_id_for_part(part_id).await
    }

    pub async fn list_session_summaries(
        &self,
        request: SessionListRequest,
    ) -> Result<Vec<SessionSummary>, AppError> {
        self.store.list_session_summaries(request).await
    }

    pub async fn list_session_events(
        &self,
        session_id: i64,
    ) -> Result<Vec<crate::event::DomainEvent>, AppError> {
        self.store.list_session_events(session_id).await
    }
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
            .map_err(|error| agena_runtime::SessionExecutionControlError::new(error.to_string()))
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
            .map_err(|error| agena_runtime::SessionExecutionControlError::new(error.to_string()))?;
        session.runtime().effective_model_ref().map_err(|error| {
            agena_runtime::SessionExecutionControlError::new(format!(
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
