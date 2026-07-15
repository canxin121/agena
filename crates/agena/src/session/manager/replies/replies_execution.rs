use super::{
    AggregatedPermissionOutcome, AggregatedPermissionRequest, AppError, Arc, DecisionTraceStep,
    EventKind, ExecutionControl, ExecutionSource, ExecutionStatus, FinishReason,
    InteractiveRequestPart, MessageMetadata, MessageSource, MessageStatus, OperationPart,
    PartContent, PermissionAction, PermissionDecision, PermissionRequest, PermissionRequestedEvent,
    PermissionRiskLevel, PermissionScope, PersistedPermissionRule, PolicySourceKind,
    PromptRequestOptions, PromptTurnBudget, ProviderPromptAnchor, RequestPart, ResolvedPendingTool,
    Role, RunAbortReason, RunAborted, RunCompleted, RunStarted, SessionManager,
    SessionManagerState, SessionPendingTool, SessionRunOptions, SessionRunRequest,
    SessionRunTermination, SessionUsageLimitBasis, StreamingToolExecution, ToolError,
    ToolInvocationExecution, ToolOutput, ToolPermissionCheck, UserInputRequest, Utc, WorkflowState,
    append_resolved_message_part, apply_advisory_permission_decision, ask_user_title,
    assistant_message_for_part, build_message, build_request_part, completed_lifecycle,
    execution_control_to_app_error, max_permission_risk, mpsc, operation_blocks_from_tool_output,
    pending_operation_for_resolved, pending_tool_part_not_found_error,
    pending_tools_include_gateway_call, permission_action_key, permission_scope_label,
    permission_subject, plugin_risk_to_core, push_unique_permission_action, resolve_pending_tool,
    resolve_permission_with_persisted_rules, responses_api_request_metadata,
    risk_for_permission_decision, run_abort_reason, should_execute_pending_tools_concurrently,
    text_result_blocks, tool_name, update_resolved_tool_message,
};
use crate::session::Session;
use crate::session::prompt_window;
use tracing::Instrument;

impl SessionManager {
    pub(in crate::session::manager) async fn run_until_stable(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        allow_goal_continuation: bool,
        base_run_source: ExecutionSource,
        state: Arc<SessionManagerState>,
        control: Arc<ExecutionControl>,
        mut steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
    ) -> Result<Session, AppError> {
        let _ = allow_goal_continuation;
        loop {
            let current_options =
                self.apply_execution_context_to_run_options(&session, options.clone())?;
            if control.cancel.is_cancelled() {
                return Ok(session);
            }

            session = self
                .drain_steer_input(session, &mut steer_rx, &current_options, state.clone())
                .await?;

            let current_options =
                self.apply_execution_context_to_run_options(&session, options.clone())?;
            session.refresh_derived();
            if session.blocked() {
                return Ok(session);
            }

            if let Some(hit) = crate::session::doom_loop::detect(
                session.messages.as_slice(),
                crate::session::DoomLoopPolicy::default(),
            ) {
                tracing::warn!(
                    target: "agena::session::doom_loop",
                    session_id = session.id,
                    tool = %hit.tool_label,
                    repeat = hit.repeat_count,
                    "aborting run: doom-loop detected"
                );
                return Err(AppError::Internal(hit.message()));
            }

            let pending_tools = session.pending_tools();
            if !pending_tools.is_empty() {
                control
                    .transition(crate::session::ExecutionPhase::ExecutingTools)
                    .await
                    .map_err(execution_control_to_app_error)?;
                session = self
                    .resolve_pending_tools(session, pending_tools, &current_options, state.clone())
                    .await?;
                continue;
            }

            match session.workflow_state() {
                WorkflowState::Quiescent => {
                    let last_assistant_text = session
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m.role == crate::role::Role::Assistant)
                        .map(|m| m.as_text_lossy());
                    let stop_input = crate::plugin::AgentStopInput {
                        session_id: session.id,
                        stop_hook_active: false,
                        last_assistant_message: last_assistant_text,
                    };
                    match state
                        .tool_executor
                        .plugin_manager()
                        .dispatch_agent_stop(stop_input)
                        .await
                    {
                        Ok(patch) if patch.continue_with_message.is_some() => {
                            let follow_up = patch.continue_with_message.unwrap_or_default();
                            let ids = self.store.reserve_message_ids(1).await?;
                            let user_message = build_message(
                                ids,
                                Role::User,
                                MessageStatus::Completed,
                                vec![PartContent::text(follow_up)],
                                MessageMetadata {
                                    source: MessageSource::System,
                                    parent_message_id: session
                                        .last_conversation_message()
                                        .map(|m| m.id),
                                    generated_by_call_id: None,
                                    model_provider_id: current_options
                                        .model
                                        .provider_id
                                        .to_string(),
                                    model_adapter_id: current_options
                                        .model
                                        .adapter_id
                                        .as_ref()
                                        .map(ToString::to_string),
                                    model_id: current_options.model.model_id.to_string(),
                                    model_thinking_mode: current_options.thinking_mode.clone(),
                                    model_speed_mode: current_options.speed_mode.clone(),
                                },
                            );
                            session.messages.push(user_message.clone());
                            session = self
                                .persist_session_changes(
                                    session,
                                    vec![user_message],
                                    Vec::new(),
                                    None,
                                    state.clone(),
                                )
                                .await?;
                            continue;
                        }
                        Ok(_) => return Ok(session),
                        Err(err) => {
                            tracing::warn!(
                                target: "agena_plugin_host::agent_stop",
                                "agent.stop hook failed (stopping normally): {err}"
                            );
                            return Ok(session);
                        }
                    }
                }
                WorkflowState::ReadyForModel => {}
                WorkflowState::ToolPending | WorkflowState::Blocked => {
                    return Err(AppError::Internal(
                        "workflow changed after pending-operation resolution".to_string(),
                    ));
                }
            }

            control
                .transition(crate::session::ExecutionPhase::PreparingModel)
                .await
                .map_err(execution_control_to_app_error)?;

            let last_message_id = session.messages.last().map(|message| message.id);
            let already_auto_compacted_at_boundary = session
                .runtime
                .prompt_window
                .compaction
                .as_ref()
                .and_then(|compaction| compaction.compacted_by_message_id)
                == last_message_id;
            let session_usage = self.session_usage(&session)?;
            if state.config.auto_compaction.enabled
                && !already_auto_compacted_at_boundary
                && session_usage.limit_basis == Some(SessionUsageLimitBasis::ContextWindow)
                && let Some(limit_tokens) = session_usage.limit_tokens
                && session_usage
                    .projected_tokens
                    .unwrap_or(session_usage.current_tokens)
                    >= limit_tokens
            {
                let projected_tokens = session_usage
                    .projected_tokens
                    .unwrap_or(session_usage.current_tokens);
                tracing::info!(
                    target: "agena::session::compact",
                    session_id = session.id,
                    current_tokens = session_usage.current_tokens,
                    projected_tokens,
                    usable_tokens = limit_tokens,
                    reserved_tokens = session_usage.reserved_tokens.unwrap_or_default(),
                    "automatic session compaction triggered before model run"
                );
                session = Box::pin(self.auto_compact_session(
                    session,
                    &current_options,
                    state.clone(),
                    control.clone(),
                ))
                .await?;
            }

            let session_id = session.id;
            let model = format!(
                "{}/{}",
                current_options.model.provider_id, current_options.model.model_id
            );
            let message_count = session.messages.len();
            let pre_run_input = crate::plugin::PreRunInput {
                session_id,
                model: model.clone(),
                message_count,
            };
            state
                .tool_executor
                .plugin_manager()
                .broadcast_pre_run(pre_run_input)
                .await;

            match Box::pin(self.run_model_turn(
                session,
                &current_options,
                base_run_source,
                state.clone(),
                control.clone(),
            ))
            .await
            {
                Ok(next_session) => {
                    session = next_session;
                    let post_run_input = crate::plugin::PostRunInput {
                        session_id: session.id,
                        model,
                        status: format!("{:?}", session.workflow_state()),
                        message_count: session.messages.len(),
                    };
                    state
                        .tool_executor
                        .plugin_manager()
                        .broadcast_post_run(post_run_input)
                        .await;
                }
                Err(err) => {
                    let post_run_input = crate::plugin::PostRunInput {
                        session_id,
                        model,
                        status: format!("error: {err}"),
                        message_count,
                    };
                    state
                        .tool_executor
                        .plugin_manager()
                        .broadcast_post_run(post_run_input)
                        .await;
                    return Err(err);
                }
            }
        }
    }

    pub(in crate::session::manager) async fn run_model_turn(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        run_source: ExecutionSource,
        state: Arc<SessionManagerState>,
        control: Arc<ExecutionControl>,
    ) -> Result<Session, AppError> {
        let run_span = tracing::info_span!(
            "session.run",
            session_id = session.id,
            provider_id = %options.model.provider_id,
            model_id = %options.model.model_id,
        );
        {
            let active_messages = prompt_window::active_prompt_messages(&session);
            let scoped_executor = state
                .tool_executor
                .for_session_context(&session.runtime.execution);
            let tool_protocol = scoped_executor.model_tool_prompt_text();
            let tools = scoped_executor.available_model_tools();
            let request_tools = tools.clone();
            let request_system = super::merge_system_prompt_with_tool_protocol(
                options.system.as_deref(),
                tool_protocol.as_deref(),
            );
            let prompt_budget = self.prompt_budget_for_run(
                &session,
                options,
                request_system.as_deref(),
                tools.as_slice(),
                state.as_ref(),
            );
            let provider_request_shape = state.processor.prompt_cache_shape(&options.model)?;
            let continuation_supported =
                state.processor.supports_prompt_continuation(&options.model);
            let prompt_request_options = PromptRequestOptions {
                provider_id: options.model.provider_id.as_ref(),
                model_id: options.model.model_id.as_ref(),
                system: request_system.as_deref(),
                temperature: options.temperature,
                max_output_tokens: options.max_output_tokens,
                tools: tools.as_slice(),
                provider_request_shape: provider_request_shape.as_ref(),
                continuation_supported,
            };
            let prompt_fingerprints =
                prompt_window::prompt_request_fingerprints(&prompt_request_options);
            let prompt_exceeds_runtime_budget = prompt_window::estimate_prompt_tokens_from_runtime(
                &session,
                active_messages.as_slice(),
                prompt_fingerprints.system_fingerprint.as_str(),
                prompt_fingerprints.request_options_fingerprint.as_str(),
            )
            .is_some_and(|estimate| estimate.total_tokens > prompt_budget.max_prompt_tokens);
            if prompt_exceeds_runtime_budget
                || state.processor.prompt_exceeds_budget(
                    active_messages.as_slice(),
                    prompt_budget.max_prompt_chars,
                )
            {
                tracing::warn!(
                    session_id = session.id,
                    prompt_message_count = active_messages.len(),
                    max_prompt_chars = prompt_budget.max_prompt_chars,
                    max_prompt_tokens = prompt_budget.max_prompt_tokens,
                    "prompt exceeds configured budget threshold; preserving append-only provider prefix and sending the full prompt"
                );
            }

            let prepared = prompt_window::build_prepared_prompt(&session, prompt_request_options);
            let provider_request_shape_fingerprint = prepared
                .provider_request_shape
                .as_ref()
                .map(crate::provider::PromptCacheShape::fingerprint);
            let provider_shape_change_keys = prepared
                .continuation_diagnostic
                .provider_shape_change_keys();
            tracing::debug!(
                session_id = session.id,
                provider_id = %options.model.provider_id,
                model_id = %options.model.model_id,
                prompt_window_generation = prepared.prompt_window_generation,
                prompt_cache_key = %prepared.prompt_cache_key,
                previous_response_id_present = prepared.previous_response_id.is_some(),
                continuation_reason = prepared.continuation_reason.as_ref(),
                provider_request_shape_fingerprint = provider_request_shape_fingerprint
                    .as_deref()
                    .unwrap_or(""),
                provider_request_shape_changed = prepared
                    .continuation_diagnostic
                    .provider_shape_changed(),
                provider_request_shape_change_keys = ?provider_shape_change_keys,
                prompt_message_count = prepared.messages.len(),
                system_included = prepared.system.is_some(),
                "prepared prompt for session run"
            );

            session.runtime.workflow.record_model_request(
                run_source,
                options.model.provider_id.to_string(),
                options.model.adapter_id.as_ref().map(ToString::to_string),
                options.model.model_id.to_string(),
                options.thinking_mode.clone(),
                options.speed_mode.clone(),
                options.verbosity.clone(),
                options.request_override.parallel_tool_calls(),
                prepared.prompt_cache_key.clone(),
                prepared.prompt_window_generation,
            );
            session = self
                .persist_session_changes(session, Vec::new(), Vec::new(), None, state.clone())
                .await?;

            let processor_ids = self.store.reserve_processor_ids().await?;
            let run_id = crate::session::RunId::new();
            let turn_started_at_unix_ms = Utc::now().timestamp_millis();
            let provider_tools = state
                .processor
                .provider_registry()
                .provider_tools_config(&options.model)?;
            let mut completion = options.completion_request(
                prepared.system.clone(),
                prepared.messages.clone(),
                tools,
                provider_tools,
                Some(prepared.prompt_cache_key.clone()),
                prepared.previous_response_id.clone(),
                Some(prepared.prompt_window_generation),
            );
            completion.responses_api_metadata = Some(
                responses_api_request_metadata(
                    &session,
                    prepared.prompt_cache_key.as_str(),
                    prepared.prompt_window_generation,
                    run_id,
                    turn_started_at_unix_ms,
                )
                .await,
            );
            let run = SessionRunRequest {
                run_id,
                execution_id: control.execution_id(),
                session_id: session.id,
                model: options.model.clone(),
                model_thinking_mode: options.thinking_mode.clone(),
                model_speed_mode: options.speed_mode.clone(),
                completion,
                next_message_id: processor_ids.message_id,
                part_ids: processor_ids.part_ids,
                next_call_id: session.next_call_id(),
                event_publisher: Some(Arc::clone(&self.publisher)),
                cancel: Some(control.cancel.clone()),
            };

            // The attempt start is durable before the provider worker begins.
            // Startup reconciliation can therefore close every interrupted
            // attempt, including a process crash before the first token.
            session = self
                .store
                .append_history_items(
                    session,
                    vec![EventKind::RunStarted(RunStarted {
                        execution_id: control.execution_id(),
                        run_id,
                        source: run_source,
                        model_id: options.model.model_id.as_ref().into(),
                        provider_id: options.model.provider_id.as_ref().into(),
                        request_digest: None,
                    })],
                    state.cache_policy(),
                )
                .await?;

            // `SessionProcessor` is the sole owner of cooperative model-stream
            // cancellation. Never race it with an outer select: dropping this
            // future would skip message and part terminalization.
            control
                .transition(crate::session::ExecutionPhase::StreamingModel)
                .await
                .map_err(execution_control_to_app_error)?;
            let run_outcome = state
                .processor
                .run_turn(run)
                .instrument(run_span.clone())
                .await;
            match run_outcome {
                Ok(result) => {
                    let run_id = result.run_id;
                    let termination = result.termination;
                    let assistant_message = result
                        .state
                        .into_iter()
                        .find(|message| message.id == result.assistant_message_id)
                        .ok_or_else(|| {
                            AppError::Internal(format!(
                                "assistant message not found after processor run: {}",
                                result.assistant_message_id
                            ))
                        })?;
                    let transcript_digest = {
                        let mut transcript_messages =
                            prompt_window::active_prompt_messages(&session);
                        transcript_messages.push(assistant_message.clone());
                        prompt_window::prompt_transcript_digest(transcript_messages.as_slice())
                    };
                    let anchored_provider_request_shape = match state
                        .processor
                        .prompt_cache_shape(&options.model)
                    {
                        Ok(shape) => shape,
                        Err(err) => {
                            tracing::warn!(
                                session_id = session.id,
                                provider_id = %options.model.provider_id,
                                model_id = %options.model.model_id,
                                error = %err,
                                "failed to refresh provider request shape after run; falling back to prepared shape"
                            );
                            prepared.provider_request_shape.clone()
                        }
                    };
                    let anchored_prompt_request_options = PromptRequestOptions {
                        provider_id: options.model.provider_id.as_ref(),
                        model_id: options.model.model_id.as_ref(),
                        system: options.system.as_deref(),
                        temperature: options.temperature,
                        max_output_tokens: options.max_output_tokens,
                        tools: request_tools.as_slice(),
                        provider_request_shape: anchored_provider_request_shape.as_ref(),
                        continuation_supported,
                    };
                    let anchored_fingerprints = prompt_window::prompt_request_fingerprints(
                        &anchored_prompt_request_options,
                    );
                    if let Some(usage) = assistant_message.usage.as_ref() {
                        session.runtime.record_prompt_tokens(
                            assistant_message.id,
                            usage,
                            prepared.prompt_window_generation,
                            prompt_budget.model_context_window_tokens,
                            anchored_fingerprints.system_fingerprint.clone(),
                            anchored_fingerprints.request_options_fingerprint.clone(),
                            transcript_digest.clone(),
                        );
                    }
                    if let Some(response_id) =
                        prompt_window::extract_response_id(result.provider_metadata.as_ref())
                    {
                        session.runtime.set_provider_anchor(ProviderPromptAnchor {
                            provider_id: options.model.provider_id.to_string(),
                            model_id: options.model.model_id.to_string(),
                            previous_response_id: response_id,
                            assistant_message_id: assistant_message.id,
                            prompt_window_generation: prepared.prompt_window_generation,
                            system_fingerprint: anchored_fingerprints.system_fingerprint,
                            request_options_fingerprint: anchored_fingerprints
                                .request_options_fingerprint,
                            provider_request_shape: anchored_provider_request_shape,
                            transcript_digest,
                        });
                    } else {
                        session.runtime.clear_provider_anchor(
                            options.model.provider_id.as_ref(),
                            options.model.model_id.as_ref(),
                        );
                    }
                    drop(request_tools);
                    drop(prepared);

                    let client_events = result.client_events;
                    session.messages.push(assistant_message.clone());
                    let mut persisted_session = self
                        .persist_session_changes(
                            session,
                            vec![assistant_message],
                            client_events,
                            None,
                            state.clone(),
                        )
                        .await?;

                    let mut run_events: Vec<EventKind> = Vec::new();
                    run_events.extend(result.history_items);
                    match &termination {
                        SessionRunTermination::Completed => {
                            run_events.push(EventKind::RunCompleted(RunCompleted {
                                run_id,
                                finish_reason: FinishReason::default(),
                            }));
                        }
                        SessionRunTermination::Cancelled => {
                            run_events.push(EventKind::RunAborted(RunAborted {
                                run_id,
                                reason: RunAbortReason::UserCancelled,
                                message: Some("execution cancelled".to_string()),
                            }));
                        }
                        SessionRunTermination::Failed(error) => {
                            run_events.push(EventKind::RunAborted(RunAborted {
                                run_id,
                                reason: RunAbortReason::ProviderError,
                                message: Some(error.to_string()),
                            }));
                        }
                    }
                    let store = Arc::clone(&self.store);
                    let cache_policy = state.cache_policy();
                    persisted_session = tokio::task::spawn(async move {
                        store
                            .append_history_items(persisted_session, run_events, cache_policy)
                            .await
                    })
                    .await
                    .map_err(|err| {
                        AppError::Internal(format!("history append task failed: {err}"))
                    })??;

                    match termination {
                        SessionRunTermination::Completed => Ok(persisted_session),
                        SessionRunTermination::Cancelled => Err(AppError::Cancelled),
                        SessionRunTermination::Failed(error) => Err(error),
                    }
                }
                Err(err) => {
                    let reason = run_abort_reason(&err);
                    self.store
                        .append_history_items(
                            session,
                            vec![EventKind::RunAborted(RunAborted {
                                run_id,
                                reason,
                                message: Some(err.to_string()),
                            })],
                            state.cache_policy(),
                        )
                        .await?;
                    Err(err)
                }
            }
        }
    }

    pub(in crate::session::manager) fn prompt_budget_for_run(
        &self,
        _session: &Session,
        options: &SessionRunOptions,
        system: Option<&str>,
        tools: &[crate::plugin::registry::RegisteredTool],
        state: &SessionManagerState,
    ) -> PromptTurnBudget {
        let fallback_budget = state.processor.max_prompt_chars();
        let metadata = state
            .processor
            .model_metadata(&options.model)
            .unwrap_or_default();
        let context_window_tokens = metadata.limits.context_window_tokens;
        let max_prompt_chars = prompt_window::prompt_char_budget(
            context_window_tokens,
            options
                .max_output_tokens
                .or(metadata.limits.max_output_tokens),
            fallback_budget,
            system,
            tools,
        );

        PromptTurnBudget {
            max_prompt_chars,
            max_prompt_tokens: prompt_window::approximate_tokens_from_chars(max_prompt_chars),
            model_context_window_tokens: context_window_tokens,
        }
    }

    pub(in crate::session::manager) async fn resolve_pending_tools(
        &self,
        mut session: Session,
        pending_tools: Vec<SessionPendingTool>,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        // `parallel_tool_calls: false` is a model-request contract, not just
        // a provider hint. Keep provider-emitted calls in their transcript
        // order even when every individual tool is otherwise safe to fan out.
        // Gateway `agena.tools.call` invocations are also always serialized:
        // their host callbacks identify the outer pending operation, and a
        // concurrent fan-out can attach a nested approval to the wrong call
        // or overwrite its pending request state.
        if !should_execute_pending_tools_concurrently(&options.request_override)
            || pending_tools_include_gateway_call(&session, pending_tools.as_slice())
        {
            if let Some(tool) = session.next_pending_tool() {
                return self.resolve_pending_tool(session, tool, state).await;
            }
            return Ok(session);
        }

        let mut resolved_tools = Vec::new();
        for pending_tool in pending_tools {
            let Some(resolved) = self
                .prepare_concurrent_pending_tool(&mut session, &pending_tool, state.as_ref())
                .await?
            else {
                break;
            };
            resolved_tools.push(resolved);
        }

        if resolved_tools.len() < 2 {
            if let Some(tool) = session.next_pending_tool() {
                return self.resolve_pending_tool(session, tool, state).await;
            }
            return Ok(session);
        }

        let executions = self
            .execute_pending_tools_concurrently(state.clone(), session.id, resolved_tools.clone())
            .await?;
        for (resolved, result) in resolved_tools.into_iter().zip(executions) {
            match result {
                Ok(execution) => {
                    session = self
                        .apply_tool_success(
                            session,
                            &resolved.pending,
                            execution,
                            None,
                            state.clone(),
                        )
                        .await?;
                }
                Err(ToolError::UserInputRequired(input)) => {
                    return self
                        .apply_user_input_request(session, &resolved.pending, *input, state)
                        .await;
                }
                Err(err) => {
                    session = self
                        .apply_tool_failure(
                            session,
                            &resolved.pending,
                            err.to_string(),
                            None,
                            state.clone(),
                        )
                        .await?;
                }
            }
        }

        Ok(session)
    }

    pub(in crate::session::manager) async fn prepare_concurrent_pending_tool(
        &self,
        session: &mut Session,
        pending_tool: &SessionPendingTool,
        state: &SessionManagerState,
    ) -> Result<Option<ResolvedPendingTool>, AppError> {
        let before_prepare = session.clone();
        let mut resolved = resolve_pending_tool(session, pending_tool)?;
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution);
        if let Err(err) = scoped_executor.validate_advertised_tool_identity(
            &resolved.invocation,
            resolved.advertised_tool_identity.as_deref(),
        ) {
            *session = before_prepare;
            tracing::debug!(
                target: "agena::session::tools",
                session_id = session.id,
                call_id = resolved.call_id,
                error = %err,
                "deferring stale tool call to sequential failure handling"
            );
            return Ok(None);
        }
        let prepared = match scoped_executor.prepare_invocation(
            &resolved.invocation,
            session.id,
            resolved.call_id,
        ) {
            Ok(prepared) => prepared,
            Err(err) => {
                tracing::debug!(
                    target: "agena::session::tools",
                    session_id = session.id,
                    call_id = resolved.call_id,
                    error = %err,
                    "deferring tool preparation error to sequential failure handling"
                );
                *session = before_prepare;
                return Ok(None);
            }
        };
        let (prepared_invocation, prepared_shell_command) = match scoped_executor
            .prepare_process_invocation(&prepared.invocation, session.id, resolved.call_id)
        {
            Ok(prepared) => prepared,
            Err(err) => {
                tracing::debug!(
                    target: "agena::session::tools",
                    session_id = session.id,
                    call_id = resolved.call_id,
                    error = %err,
                    "deferring shell preparation error to sequential failure handling"
                );
                *session = before_prepare;
                return Ok(None);
            }
        };
        resolved.prepared_shell_command = prepared_shell_command;
        resolved.invocation = prepared_invocation;
        if prepared.invocation != resolved.invocation || prepared.title_override.is_some() {
            let current_title = match session
                .part(&resolved.pending.part)
                .and_then(|part| part.content.as_ref())
            {
                Some(PartContent::Operation(operation)) => operation.title.clone(),
                _ => format!("Tool {}", tool_name(&resolved.invocation)),
            };

            resolved.invocation = prepared.invocation.clone();
            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    resolved.pending.part.message_id, resolved.pending.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::Operation(pending_operation_for_resolved(
                &resolved,
                prepared.invocation,
                prepared.title_override.unwrap_or(current_title),
                resolved.lifecycle.clone(),
            )));
        }

        if !scoped_executor.is_concurrency_safe_invocation(&resolved.invocation) {
            *session = before_prepare;
            return Ok(None);
        }

        let permission_checks = match scoped_executor
            .collect_permission_checks_for_invocation_in_session(
                &resolved.invocation,
                Some(session.id),
            ) {
            Ok(checks) => checks,
            Err(err) => {
                tracing::debug!(
                    target: "agena::session::tools",
                    session_id = session.id,
                    call_id = resolved.call_id,
                    error = %err,
                    "deferring permission-check error to sequential failure handling"
                );
                *session = before_prepare;
                return Ok(None);
            }
        };

        for check in permission_checks {
            if !matches!(
                self.resolve_permission_decision(Some(session.id), &check)
                    .await?
                    .decision,
                PermissionDecision::Allow
            ) {
                *session = before_prepare;
                return Ok(None);
            }
        }

        Ok(Some(resolved))
    }

    #[tracing::instrument(skip(self, state, pending_tools), fields(session_id, tool_count = pending_tools.len()))]
    pub(in crate::session::manager) async fn execute_pending_tools_concurrently(
        &self,
        state: Arc<SessionManagerState>,
        session_id: i64,
        pending_tools: Vec<ResolvedPendingTool>,
    ) -> Result<Vec<Result<ToolInvocationExecution, ToolError>>, AppError> {
        // Cap concurrent blocking tool executions using the live runtime
        // configuration so reloads can tune the fan-out without a process
        // restart or a hidden process-global constant.
        let semaphore = Arc::clone(&state.tool_execution_semaphore);

        let mut handles = Vec::with_capacity(pending_tools.len());
        for pending_tool in pending_tools {
            let executor = state.tool_executor.clone();
            let scoped_executor =
                executor.for_session_context(&pending_tool.session_runtime.execution);
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|err| AppError::Internal(format!("tool semaphore closed: {err}")))?;
            handles.push(tokio::task::spawn_blocking(move || {
                let _permit = permit;
                scoped_executor.validate_advertised_tool_identity(
                    &pending_tool.invocation,
                    pending_tool.advertised_tool_identity.as_deref(),
                )?;
                scoped_executor.execute_invocation_detailed_bypassing_permissions(
                    &pending_tool.invocation,
                    session_id,
                    pending_tool.call_id,
                )
            }));
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            results.push(handle.await.map_err(|err| {
                AppError::Internal(format!("concurrent tool task failed: {err}"))
            })?);
        }
        Ok(results)
    }

    pub(in crate::session::manager) async fn resolve_pending_tool(
        &self,
        mut session: Session,
        pending_tool: SessionPendingTool,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let mut resolved = resolve_pending_tool(&session, &pending_tool)?;
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution);
        if let Err(err) = scoped_executor.validate_advertised_tool_identity(
            &resolved.invocation,
            resolved.advertised_tool_identity.as_deref(),
        ) {
            return Box::pin(self.apply_tool_failure(
                session,
                &resolved.pending,
                err.to_string(),
                None,
                state,
            ))
            .await;
        }
        let prepared = match scoped_executor.prepare_invocation(
            &resolved.invocation,
            session.id,
            resolved.call_id,
        ) {
            Ok(prepared) => prepared,
            Err(err) => {
                return Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    err.to_string(),
                    None,
                    state,
                ))
                .await;
            }
        };
        let (prepared_invocation, prepared_shell_command) = match scoped_executor
            .prepare_process_invocation(&prepared.invocation, session.id, resolved.call_id)
        {
            Ok(prepared) => prepared,
            Err(err) => {
                return Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    err.to_string(),
                    None,
                    state,
                ))
                .await;
            }
        };
        resolved.prepared_shell_command = prepared_shell_command;
        resolved.invocation = prepared_invocation;
        let mut session_changed = false;
        if prepared.invocation != resolved.invocation || prepared.title_override.is_some() {
            let current_title = match session
                .part(&resolved.pending.part)
                .and_then(|part| part.content.as_ref())
            {
                Some(PartContent::Operation(operation)) => operation.title.clone(),
                _ => format!("Tool {}", tool_name(&resolved.invocation)),
            };

            resolved.invocation = prepared.invocation.clone();
            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    resolved.pending.part.message_id, resolved.pending.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::Operation(pending_operation_for_resolved(
                &resolved,
                prepared.invocation,
                prepared.title_override.unwrap_or(current_title),
                resolved.lifecycle.clone(),
            )));
            session_changed = true;
        }

        let permission_checks = match scoped_executor
            .collect_permission_checks_for_invocation_in_session(
                &resolved.invocation,
                Some(session.id),
            ) {
            Ok(checks) => checks,
            Err(err) => {
                return Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    err.to_string(),
                    None,
                    state,
                ))
                .await;
            }
        };

        match self
            .aggregate_permission_outcome(Some(session.id), permission_checks.as_slice())
            .await?
        {
            AggregatedPermissionOutcome::Allow => {}
            AggregatedPermissionOutcome::Request(request) => {
                let request = *request;
                return self
                    .apply_permission_request(
                        session,
                        &resolved.pending,
                        request.action,
                        request.related_actions,
                        request.requested_actions,
                        request.reason,
                        request.explanation,
                        request.source,
                        request.scope,
                        request.operator,
                        request.risk,
                        request.trace,
                        state,
                    )
                    .await;
            }
            AggregatedPermissionOutcome::Deny { reason } => {
                return Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    reason,
                    None,
                    state,
                ))
                .await;
            }
        }

        if session_changed {
            let assistant_message = session.messages[resolved.pending.part.message_index].clone();
            session = self
                .persist_session_changes(
                    session,
                    vec![assistant_message],
                    Vec::new(),
                    None,
                    state.clone(),
                )
                .await?;
        }

        let streaming_tool = match state
            .tool_executor
            .for_session_context(&session.runtime.execution)
            .execute_invocation_streaming_after_authorization(
                &resolved.invocation,
                session.id,
                resolved.call_id,
            )
            .await
        {
            Ok(stream) => stream,
            Err(err) => {
                return Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    err.to_string(),
                    None,
                    state,
                ))
                .await;
            }
        };

        if let Some(stream) = streaming_tool {
            return self
                .apply_streaming_tool_execution(session, &resolved.pending, stream, state)
                .await;
        }

        match self.execute_pending_tool(state.as_ref(), session.id, &resolved) {
            Ok(execution) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                self.apply_tool_success(session, &resolved.pending, execution, None, state)
                    .await
            }
            Err(ToolError::UserInputRequired(input)) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                self.apply_user_input_request(session, &resolved.pending, *input, state)
                    .await
            }
            Err(err) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    err.to_string(),
                    None,
                    state,
                ))
                .await
            }
        }
    }

    pub async fn resolve_tool_permission_check(
        &self,
        session_id: Option<i64>,
        check: &ToolPermissionCheck,
    ) -> Result<crate::permission::PermissionResolution, AppError> {
        self.resolve_permission_decision(session_id, check).await
    }

    pub(in crate::session::manager) async fn resolve_permission_decision(
        &self,
        session_id: Option<i64>,
        check: &ToolPermissionCheck,
    ) -> Result<crate::permission::PermissionResolution, AppError> {
        let key = permission_action_key(&check.action)?;
        let persisted_rules = self
            .store
            .resolve_permission_rules(key.as_str(), session_id)
            .await?;
        let mut resolution =
            resolve_permission_with_persisted_rules(check.decision.clone(), &persisted_rules);

        if persisted_rules.is_empty() {
            let plugins = self
                .execution_state()
                .tool_executor
                .plugin_manager()
                .clone();
            if !plugins.is_empty() {
                let default_decision = match resolution.decision {
                    PermissionDecision::Allow => crate::plugin::PermissionDecision::Allow,
                    PermissionDecision::Deny { .. } => crate::plugin::PermissionDecision::Deny,
                    PermissionDecision::Ask { .. } => crate::plugin::PermissionDecision::Prompt,
                };
                let req = crate::plugin::PermissionAskInput {
                    session_id: session_id.unwrap_or(-1),
                    action: format!("{:?}", check.action),
                    subject: permission_subject(&check.action),
                    default_decision,
                };
                match plugins.dispatch_permission_ask_blocking(req) {
                    Ok(Some(crate::plugin::host::PermissionAskOutcome::Decision {
                        plugin_id,
                        decision: crate::plugin::PermissionDecision::Allow,
                        authority,
                    })) => {
                        resolution.decision = PermissionDecision::Allow;
                        resolution.risk = PermissionRiskLevel::Low;
                        resolution.explanation = format!(
                            "allowed by plugin decision from {plugin_id} ({})",
                            authority.trust_level
                        );
                        resolution.trace.push(DecisionTraceStep {
                            source_kind: PolicySourceKind::PluginAdvice,
                            summary: format!(
                                "allowed by plugin decision from {plugin_id} (trust={}, capabilities={})",
                                authority.trust_level,
                                authority.plugin_capabilities.join(", ")
                            ),
                            source: Some(plugin_id),
                            scope: None,
                            operator: None,
                        });
                    }
                    Ok(Some(crate::plugin::host::PermissionAskOutcome::Decision {
                        plugin_id,
                        decision: crate::plugin::PermissionDecision::Deny,
                        authority,
                    })) => {
                        resolution.decision = PermissionDecision::Deny {
                            reason: format!("denied by plugin {plugin_id}"),
                        };
                        resolution.risk = PermissionRiskLevel::High;
                        resolution.explanation = format!(
                            "denied by plugin decision from {plugin_id} ({})",
                            authority.trust_level
                        );
                        resolution.trace.push(DecisionTraceStep {
                            source_kind: PolicySourceKind::PluginAdvice,
                            summary: format!(
                                "denied by plugin decision from {plugin_id} (trust={}, capabilities={})",
                                authority.trust_level,
                                authority.plugin_capabilities.join(", ")
                            ),
                            source: Some(plugin_id),
                            scope: None,
                            operator: None,
                        });
                    }
                    Ok(Some(crate::plugin::host::PermissionAskOutcome::Decision {
                        plugin_id,
                        decision: crate::plugin::PermissionDecision::Prompt,
                        authority,
                    })) => {
                        resolution.decision = PermissionDecision::Ask {
                            reason: resolution.explanation.clone(),
                        };
                        resolution.risk = PermissionRiskLevel::Medium;
                        resolution.trace.push(DecisionTraceStep {
                            source_kind: PolicySourceKind::PluginAdvice,
                            summary: format!(
                                "plugin {plugin_id} requested confirmation (trust={}, capabilities={})",
                                authority.trust_level,
                                authority.plugin_capabilities.join(", ")
                            ),
                            source: Some(plugin_id),
                            scope: None,
                            operator: None,
                        });
                    }
                    Ok(Some(crate::plugin::host::PermissionAskOutcome::Advice {
                        plugin_id,
                        advice,
                        authority,
                    })) => {
                        let explanation = if advice.reason.trim().is_empty() {
                            format!("permission advised by plugin {plugin_id}")
                        } else {
                            format!("{} (plugin: {plugin_id})", advice.reason)
                        };
                        resolution.explanation = explanation.clone();
                        let plugin_risk = plugin_risk_to_core(advice.risk);
                        resolution.decision = apply_advisory_permission_decision(
                            resolution.decision.clone(),
                            advice.decision,
                            &explanation,
                        );
                        resolution.risk = max_permission_risk(
                            max_permission_risk(resolution.risk, plugin_risk),
                            risk_for_permission_decision(&resolution.decision),
                        );
                        resolution.trace.push(DecisionTraceStep {
                            source_kind: PolicySourceKind::PluginAdvice,
                            summary: format!(
                                "{} (trust={}, capabilities={})",
                                explanation,
                                authority.trust_level,
                                authority.plugin_capabilities.join(", ")
                            ),
                            source: Some(plugin_id),
                            scope: None,
                            operator: None,
                        });
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(
                            target: "agena_plugin_host::permission",
                            "permission plugin failed: {err}"
                        );
                    }
                }
            }
        }

        Ok(resolution)
    }

    pub(in crate::session::manager) async fn aggregate_permission_outcome(
        &self,
        session_id: Option<i64>,
        checks: &[ToolPermissionCheck],
    ) -> Result<AggregatedPermissionOutcome, AppError> {
        let mut related_actions = Vec::with_capacity(checks.len());
        let mut requested_actions = Vec::new();
        let mut primary_request: Option<AggregatedPermissionRequest> = None;

        for check in checks {
            let action = check.action.clone();
            push_unique_permission_action(&mut related_actions, action.clone());
            let resolution = self.resolve_permission_decision(session_id, check).await?;
            match resolution.decision {
                PermissionDecision::Allow => {}
                PermissionDecision::Deny { reason } => {
                    return Ok(AggregatedPermissionOutcome::Deny { reason });
                }
                PermissionDecision::Ask { reason } => {
                    push_unique_permission_action(&mut requested_actions, action.clone());
                    let (source, scope, operator) = match resolution.source {
                        crate::permission::PermissionResolutionSource::PersistedRule {
                            scope,
                            source,
                            operator,
                            ..
                        } => (Some(source), Some(scope), operator),
                        crate::permission::PermissionResolutionSource::StaticPolicy => {
                            (Some("static_policy".to_string()), None, None)
                        }
                    };

                    if let Some(existing) = primary_request.as_mut() {
                        existing.risk = max_permission_risk(existing.risk, resolution.risk);
                        existing.trace.extend(resolution.trace);
                    } else {
                        primary_request = Some(AggregatedPermissionRequest {
                            action,
                            related_actions: Vec::new(),
                            requested_actions: Vec::new(),
                            reason,
                            explanation: resolution.explanation,
                            source,
                            scope,
                            operator,
                            risk: resolution.risk,
                            trace: resolution.trace,
                        });
                    }
                }
            }
        }

        if let Some(mut request) = primary_request {
            request.related_actions = related_actions;
            request.requested_actions = requested_actions;
            if request.requested_actions.len() > 1 {
                let additional = request.requested_actions.len() - 1;
                request.reason = format!(
                    "{} (plus {additional} more permission checks for this tool call)",
                    request.reason
                );
            }
            return Ok(AggregatedPermissionOutcome::Request(Box::new(request)));
        }

        Ok(AggregatedPermissionOutcome::Allow)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::session::manager) async fn apply_permission_request(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        action: PermissionAction,
        related_actions: Vec<PermissionAction>,
        requested_actions: Vec<PermissionAction>,
        reason: String,
        explanation: String,
        source: Option<String>,
        scope: Option<PermissionScope>,
        operator: Option<String>,
        risk: crate::permission::PermissionRiskLevel,
        trace: Vec<crate::permission::DecisionTraceStep>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let request_id = resolve_pending_tool(&session, pending_tool)?.operation_id;
        self.apply_permission_request_with_id(
            session,
            pending_tool,
            request_id,
            action,
            related_actions,
            requested_actions,
            reason,
            explanation,
            source,
            scope,
            operator,
            risk,
            trace,
            state,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::session::manager) async fn apply_permission_request_with_id(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        request_id: String,
        action: PermissionAction,
        related_actions: Vec<PermissionAction>,
        requested_actions: Vec<PermissionAction>,
        reason: String,
        explanation: String,
        source: Option<String>,
        scope: Option<PermissionScope>,
        operator: Option<String>,
        risk: crate::permission::PermissionRiskLevel,
        trace: Vec<crate::permission::DecisionTraceStep>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let request = PermissionRequest {
            request_id,
            session_id: Some(session.id),
            action,
            related_actions: related_actions.clone(),
            requested_actions: requested_actions.clone(),
            reason: reason.clone(),
            explanation: explanation.clone(),
            source,
            scope,
            operator,
            risk,
            trace: trace.clone(),
            created_at: Utc::now(),
        };

        let _assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                tool_part.set_content(PartContent::Operation(pending_operation_for_resolved(
                    &resolved,
                    resolved.invocation.clone(),
                    format!("Awaiting permission: {reason}"),
                    resolved.lifecycle.clone(),
                )));
                tool_part.status = ExecutionStatus::Pending;
                tool_part.summary = Some(reason.clone());
            })?;

        let permission_request_part =
            RequestPart::Permission(InteractiveRequestPart::pending(request.clone()));
        let assistant_message = match self.upsert_existing_pending_request_part(
            &mut session,
            &resolved,
            request.request_id.as_str(),
            crate::message::PendingInteractiveRequestKind::Permission,
            permission_request_part,
        )? {
            Some(message) => message,
            None => {
                let permission_part_id = self.store.reserve_part_id().await?;
                append_resolved_message_part(
                    &mut session,
                    &resolved,
                    build_request_part(
                        permission_part_id,
                        resolved.pending.part.message_id,
                        resolved.operation_id.as_str(),
                        RequestPart::Permission(InteractiveRequestPart::pending(request.clone())),
                    ),
                )?
            }
        };
        let session_id = session.id;
        self.persist_session_changes(
            session,
            vec![assistant_message],
            vec![EventKind::PermissionRequested(PermissionRequestedEvent {
                session_id,
                request_id: request.request_id.clone(),
                action: request.action.clone(),
                related_actions,
                requested_actions,
                reason: reason.clone(),
                explanation,
                source: request.source.clone(),
                scope: request.scope.map(permission_scope_label),
                operator: request.operator.clone(),
                risk: request.risk,
                trace,
                ts_ms: Utc::now().timestamp_millis(),
            })],
            None,
            state.clone(),
        )
        .await
    }

    pub(in crate::session::manager) async fn apply_user_input_request(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        input: crate::message::AskUserToolInput,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let request_id = resolve_pending_tool(&session, pending_tool)?.operation_id;
        self.apply_user_input_request_with_id(session, pending_tool, input, request_id, state)
            .await
    }

    pub(in crate::session::manager) async fn apply_user_input_request_with_id(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        input: crate::message::AskUserToolInput,
        request_id: String,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let request = UserInputRequest {
            request_id,
            session_id: Some(session.id),
            title: input.title,
            body_markdown: input.body_markdown,
            kind: input.kind,
            submit_label: input.submit_label,
            cancel_label: input.cancel_label,
            auto_resolution_ms: input.auto_resolution_ms,
            questions: input.questions,
            created_at: Utc::now(),
        };

        let _assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                tool_part.set_content(PartContent::Operation(pending_operation_for_resolved(
                    &resolved,
                    resolved.invocation.clone(),
                    ask_user_title(&request),
                    resolved.lifecycle.clone(),
                )));
                tool_part.status = ExecutionStatus::Pending;
                tool_part.summary = Some(match request.questions.len() {
                    0 => "Ask user".to_string(),
                    1 => "Waiting for answer".to_string(),
                    count => format!("Waiting for {count} answers"),
                });
            })?;

        let user_input_request_part =
            RequestPart::UserInput(InteractiveRequestPart::pending(request.clone()));
        let assistant_message = match self.upsert_existing_pending_request_part(
            &mut session,
            &resolved,
            request.request_id.as_str(),
            crate::message::PendingInteractiveRequestKind::UserInput,
            user_input_request_part,
        )? {
            Some(message) => message,
            None => {
                let input_part_id = self.store.reserve_part_id().await?;
                append_resolved_message_part(
                    &mut session,
                    &resolved,
                    build_request_part(
                        input_part_id,
                        resolved.pending.part.message_id,
                        resolved.operation_id.as_str(),
                        RequestPart::UserInput(InteractiveRequestPart::pending(request.clone())),
                    ),
                )?
            }
        };
        self.persist_session_changes(session, vec![assistant_message], Vec::new(), None, state)
            .await
    }

    pub(in crate::session::manager) async fn apply_streaming_tool_execution(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        mut stream: StreamingToolExecution,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let stream_id = stream.stream_id.clone();
        while let Some(chunk) = stream.chunks.recv().await {
            let Some(delta) = chunk.text_delta.as_deref() else {
                continue;
            };
            if delta.is_empty() {
                continue;
            }

            session = self
                .append_streaming_tool_output_delta(session.id, pending_tool, delta, state.clone())
                .await?;
        }

        let execution = match stream.end.await {
            Ok(Ok(execution)) => execution,
            Ok(Err(err)) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                return self
                    .apply_tool_failure(session, pending_tool, err.to_string(), None, state)
                    .await;
            }
            Err(_) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                return self
                    .apply_tool_failure(
                        session,
                        pending_tool,
                        format!("tool stream ended without terminal result: {stream_id}"),
                        None,
                        state,
                    )
                    .await;
            }
        };

        let session = self
            .store
            .load_session(session.id, state.cache_policy())
            .await?;
        self.apply_tool_success(session, pending_tool, execution, None, state)
            .await
    }

    /// Persist one text chunk for a pending tool operation. This is shared by
    /// ordinary direct streaming invocations and streaming targets executed
    /// through the tools.call gateway.
    pub(in crate::session::manager) async fn append_streaming_tool_output_delta(
        &self,
        session_id: i64,
        pending_tool: &SessionPendingTool,
        delta: &str,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let mut session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        let tool_part_ref = session
            .resolve_part_ref(&pending_tool.part)
            .ok_or_else(|| pending_tool_part_not_found_error(&pending_tool.part))?;
        {
            let tool_part = session
                .part_mut(&tool_part_ref)
                .ok_or_else(|| pending_tool_part_not_found_error(&pending_tool.part))?;
            if !tool_part.append_tool_output_delta(delta) {
                return Err(AppError::Internal(format!(
                    "streaming tool part refused output delta: message={}, part={}",
                    pending_tool.part.message_id, pending_tool.part.part_id
                )));
            }
            if matches!(
                tool_part.status,
                ExecutionStatus::Pending | ExecutionStatus::InProgress
            ) {
                tool_part.status = ExecutionStatus::InProgress;
            }
        }

        let assistant_message = assistant_message_for_part(&session, &pending_tool.part)?;
        self.persist_session_changes(session, vec![assistant_message], Vec::new(), None, state)
            .await
    }

    pub(in crate::session::manager) async fn apply_tool_success(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        execution: ToolInvocationExecution,
        persisted_rule: Option<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        self.apply_tool_success_with_rules(
            session,
            pending_tool,
            execution,
            persisted_rule.into_iter().collect(),
            state,
        )
        .await
    }

    pub(in crate::session::manager) async fn apply_tool_success_with_rules(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        execution: ToolInvocationExecution,
        persisted_rules: Vec<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let tool_output = execution.output.clone();
        let output_text = execution.view.output_text.clone();
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let blocks = operation_blocks_from_tool_output(
            &resolved.invocation,
            &tool_output,
            execution.view.attachments.as_slice(),
            output_text.as_str(),
        );
        let completion_title = {
            let execution_title = execution.view.title.trim();
            if !execution_title.is_empty() {
                execution_title.to_string()
            } else {
                session
                    .part(&resolved.pending.part)
                    .and_then(|part| part.content.as_ref())
                    .and_then(|content| match content {
                        PartContent::Operation(operation) => Some(operation.title.clone()),
                        _ => None,
                    })
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| format!("Tool {}", tool_name(&resolved.invocation)))
            }
        };
        self.apply_tool_success_execution_context(&mut session, &resolved.invocation, &execution);

        let assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::completed(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    output_text.clone(),
                    blocks.clone(),
                    execution.view.attachments.clone(),
                    tool_output.clone(),
                    lifecycle.clone(),
                );
                operation.set_title(completion_title.clone());
                operation.result.metadata.extend(
                    execution.view.metadata.iter().map(|(key, value)| {
                        (key.clone(), serde_json::Value::String(value.clone()))
                    }),
                );
                tool_part.set_content(PartContent::Operation(operation));
                tool_part.status = ExecutionStatus::Completed;
            })?;

        self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            persisted_rules,
            state,
        )
        .await
    }

    pub(in crate::session::manager) async fn apply_tool_failure(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        reason: String,
        persisted_rule: Option<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        self.apply_tool_failure_with_rules(
            session,
            pending_tool,
            reason,
            persisted_rule.into_iter().collect(),
            state,
        )
        .await
    }

    pub(in crate::session::manager) async fn apply_tool_failure_with_rules(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        reason: String,
        persisted_rules: Vec<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let blocks = text_result_blocks(reason.as_str());
        let failure_title = session
            .part(&resolved.pending.part)
            .and_then(|part| part.content.as_ref())
            .and_then(|content| match content {
                PartContent::Operation(operation) => Some(operation.title.clone()),
                _ => None,
            })
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| format!("Tool {}", tool_name(&resolved.invocation)));

        // Notify plugins about the tool failure (fire-and-forget).
        state.tool_executor.broadcast_tool_failure(
            &resolved.invocation,
            session.id,
            resolved.call_id,
            &reason,
        );

        let assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::failed(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    reason.clone(),
                    reason.clone(),
                    blocks.clone(),
                    Vec::new(),
                    ToolOutput::default(),
                    lifecycle.clone(),
                );
                operation.set_title(failure_title.clone());
                tool_part.set_content(PartContent::Operation(operation));
                tool_part.status = ExecutionStatus::Failed;
            })?;

        self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            persisted_rules,
            state,
        )
        .await
    }
}
