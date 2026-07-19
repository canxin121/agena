use super::{
    AppError, Message, MessageMetadata, MessagePart, MessageSource, MessageStatus, PartContent,
    Role, SessionAgentRestoreOutcome, SessionAgentSwitchOutcome, SessionCreateRequest,
    SessionListRequest, SessionManager, SessionRunOptions, SessionSummary, SessionUsage,
    SessionUsageLimitBasis, build_message,
};
use crate::session::Session;
use crate::session::prompt_window;

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
                && session.runtime.subtask.status == crate::session::SubtaskStatus::Running
                && !self.execution_registry.is_active(session_id).await
            {
                session.runtime.subtask.status = crate::session::SubtaskStatus::Interrupted;
                session.runtime.subtask.finished_at_ms =
                    Some(chrono::Utc::now().timestamp_millis());
                session.runtime.subtask.error = Some(
                    "subtask execution was interrupted by runtime shutdown or restart".to_string(),
                );
                let interrupted_at_ms = session.runtime.subtask.finished_at_ms;
                let lifecycle_event = session.parent_id.zip(session.task_id.clone()).map(
                    |(parent_session_id, task_id)| {
                        crate::event::EventKind::SubtaskStatusChanged(
                            crate::event::SubtaskStatusChangedEvent {
                                session_id,
                                parent_session_id,
                                task_id,
                                profile: session
                                    .runtime
                                    .execution
                                    .selection
                                    .agent
                                    .clone()
                                    .unwrap_or_else(|| "unknown".to_string()),
                                status: crate::session::SubtaskStatus::Interrupted,
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
    ) -> Option<crate::session::ExecutionLifecycle> {
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
            .dispatch_session_start(crate::plugin::SessionStartInput {
                session_id: session.id,
                source: crate::plugin::SessionStartSource::Startup,
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
                MessageStatus::Completed,
                vec![PartContent::text(additional_context)],
                MessageMetadata {
                    source: MessageSource::System,
                    turn_id: None,
                    parent_message_id: session
                        .last_conversation_message()
                        .map(|message| message.id),
                    generated_by_call_id: None,
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
                MessageStatus::Completed,
                vec![PartContent::text(initial_user_message)],
                MessageMetadata {
                    source: MessageSource::System,
                    turn_id: Some(initial_turn_id),
                    parent_message_id: session
                        .last_conversation_message()
                        .map(|message| message.id),
                    generated_by_call_id: None,
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

    pub async fn set_session_allowed_tools(
        &self,
        session_id: i64,
        allowed_tools: Vec<String>,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        session.runtime.set_allowed_tools(allowed_tools);
        self.persist_session_changes(session, Vec::new(), Vec::new(), None, state)
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
                session
                    .runtime
                    .execution
                    .agent_system_prompt
                    .as_deref()
                    .map(crate::agents::without_legacy_tool_protocol_prompt)
                    .map(str::trim)
                    .filter(|prompt| !prompt.is_empty())
                    .map(ToOwned::to_owned)
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
        let reserved_tokens = crate::session::estimate_auto_compaction_reserve_tokens(
            context_window_tokens,
            max_input_tokens,
            max_output_tokens,
            state.config.auto_compaction.reserved_tokens,
        );
        let context_auto_compact_limit_tokens =
            crate::session::estimate_auto_compaction_limit_tokens(
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
                    Some(crate::session::estimate_prompt_budget_threshold_tokens(
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

    pub async fn switch_session_agent(
        &self,
        session_id: i64,
        agent: Option<String>,
        push_previous: bool,
    ) -> Result<SessionAgentSwitchOutcome, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        let previous_agent = session.runtime.execution.selection.agent.clone();
        if push_previous {
            session
                .runtime
                .execution
                .agent_stack
                .push(previous_agent.clone());
        }

        let target_agent = agent
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let mut session = match target_agent {
            Some(agent_name) => {
                let mut options = self.run_options_from_session(&session, state.clone())?;
                options.agent_profile = Some(agent_name);
                self.apply_requested_agent_profile(session, &mut options, state.clone())
                    .await?
            }
            None => {
                self.clear_session_agent_profile(session, state.clone())
                    .await?
            }
        };
        let current_agent = session.runtime.execution.selection.agent.clone();
        let stack_depth = session.runtime.execution.agent_stack.len();
        session = self
            .persist_session_changes(session, Vec::new(), Vec::new(), None, state)
            .await?;
        Ok(SessionAgentSwitchOutcome {
            session_id: session.id,
            previous_agent,
            current_agent,
            stack_depth,
        })
    }

    pub async fn restore_session_agent(
        &self,
        session_id: i64,
    ) -> Result<SessionAgentRestoreOutcome, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        let previous_agent = session.runtime.execution.selection.agent.clone();
        let Some(target_agent) = session.runtime.execution.agent_stack.pop() else {
            return Ok(SessionAgentRestoreOutcome {
                session_id,
                restored: false,
                previous_agent,
                current_agent: session.runtime.execution.selection.agent,
                stack_depth: 0,
            });
        };

        let mut session = match target_agent {
            Some(agent_name) => {
                let mut options = self.run_options_from_session(&session, state.clone())?;
                options.agent_profile = Some(agent_name);
                self.apply_requested_agent_profile(session, &mut options, state.clone())
                    .await?
            }
            None => {
                self.clear_session_agent_profile(session, state.clone())
                    .await?
            }
        };
        let current_agent = session.runtime.execution.selection.agent.clone();
        let stack_depth = session.runtime.execution.agent_stack.len();
        session = self
            .persist_session_changes(session, Vec::new(), Vec::new(), None, state)
            .await?;
        Ok(SessionAgentRestoreOutcome {
            session_id: session.id,
            restored: true,
            previous_agent,
            current_agent,
            stack_depth,
        })
    }

    pub async fn set_session_permission(
        &self,
        session_id: i64,
        permission: crate::agent::PermissionConfig,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        session.runtime.execution.selection.permission = permission;
        let agent_permission = session
            .runtime
            .execution
            .selection
            .agent
            .as_deref()
            .map(|agent_name| {
                state
                    .tool_executor
                    .subagent_registry()
                    .require(agent_name)
                    .map(|profile| profile.frontmatter.permission.clone())
                    .map_err(|error| AppError::Config(error.to_string()))
            })
            .transpose()?;
        session.runtime.execution.effective_permission =
            self.resolve_effective_session_permission(&session, &state, agent_permission.as_ref());
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
                agent_profile: None,
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

    pub async fn list_projected_messages_page(
        &self,
        session_id: i64,
        include_full_parts: bool,
        cursor: Option<(i64, i64)>,
        limit: u64,
    ) -> Result<(Vec<Message>, bool, Option<(i64, i64)>), AppError> {
        self.store
            .list_projected_messages_page(session_id, include_full_parts, cursor, limit)
            .await
    }

    pub async fn list_projected_message_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<crate::session::ProjectedMessageHeader>, AppError> {
        self.store.list_projected_message_headers(session_id).await
    }

    pub async fn list_projected_message_headers_page(
        &self,
        session_id: i64,
        cursor: Option<(i64, i64)>,
        limit: u64,
    ) -> Result<
        (
            Vec<crate::session::ProjectedMessageHeader>,
            bool,
            Option<(i64, i64)>,
        ),
        AppError,
    > {
        self.store
            .list_projected_message_headers_page(session_id, cursor, limit)
            .await
    }

    pub async fn find_projected_message(
        &self,
        session_id: i64,
        message_id: i64,
        include_full_parts: bool,
    ) -> Result<Option<Message>, AppError> {
        self.store
            .find_projected_message(session_id, message_id, include_full_parts)
            .await
    }

    pub async fn find_projected_message_header(
        &self,
        session_id: i64,
        message_id: i64,
    ) -> Result<Option<crate::session::ProjectedMessageHeader>, AppError> {
        self.store
            .find_projected_message_header(session_id, message_id)
            .await
    }

    pub async fn list_projected_parts(
        &self,
        message_id: i64,
        include_full_parts: bool,
    ) -> Result<Vec<MessagePart>, AppError> {
        self.store
            .list_projected_parts(message_id, include_full_parts)
            .await
    }

    pub async fn find_projected_part(&self, part_id: i64) -> Result<Option<MessagePart>, AppError> {
        self.store.find_projected_part(part_id).await
    }

    pub async fn broadcast_session_end(
        &self,
        session_id: i64,
        reason: crate::plugin::SessionEndReason,
    ) {
        self.execution_state()
            .tool_executor
            .plugin_manager()
            .broadcast_session_end(crate::plugin::SessionEndInput { session_id, reason })
            .await;
    }

    pub async fn broadcast_active_session_end(&self, reason: crate::plugin::SessionEndReason) {
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
