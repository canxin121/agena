use super::*;

fn mode_request_override_for_adapter(
    request_override: &ModelSpeedModeRequestOverride,
    adapter_overrides: &std::collections::BTreeMap<String, ModelSpeedModeRequestOverride>,
    resolved_adapter_id: Option<&crate::model::AdapterId>,
) -> ModelSpeedModeRequestOverride {
    let mut merged = request_override.clone();
    if let Some(adapter_id) = resolved_adapter_id.map(crate::model::AdapterId::as_str)
        && let Some(adapter_override) = adapter_overrides.get(adapter_id)
    {
        merged = merged.merged_with(adapter_override);
    }
    merged
}

impl SessionManager {
    pub async fn reply_permission(
        &self,
        mut request: SessionPermissionReplyRequest,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        session = self
            .apply_requested_agent_profile(session, &mut request.options, state.clone())
            .await?;
        let pending = session
            .find_pending_permission_by_request_id(request.reply.request_id.as_str())
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "pending permission request not found: {}",
                    request.reply.request_id
                ))
            })?;

        let permission_request = session
            .pending_permission_request(&pending)
            .cloned()
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "pending permission request payload missing: {}",
                    request.reply.request_id
                ))
            })?;
        let reply_reason = request
            .reply
            .reason
            .clone()
            .unwrap_or_else(|| permission_request.reason.clone());

        {
            let permission_part = session.part_mut(&pending.request).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending permission part not found: {}",
                    request.reply.request_id
                ))
            })?;
            permission_part.set_content(PartContent::permission_request(
                PermissionRequestPart::pending(permission_request.clone())
                    .with_reply(request.reply.clone()),
            ));
            permission_part.status = ExecutionStatus::Completed;
        }

        let persisted_rule = persisted_rule_for_reply(
            &self.store,
            request.session_id,
            &permission_request.action,
            &request.reply,
            request.operator.as_deref(),
        )
        .await?;
        self.publisher
            .publish(
                crate::event::PublishContext::for_session(request.session_id),
                EventKind::PermissionReplied(PermissionRepliedEvent {
                    session_id: request.session_id,
                    request_id: request.reply.request_id.clone(),
                    kind: request.reply.kind,
                    reason: request.reply.reason.clone(),
                    scope: request.reply.scope.map(permission_scope_label),
                    ts_ms: Utc::now().timestamp_millis(),
                }),
            )
            .await
            .map_err(|err| AppError::Internal(format!("publish permission reply failed: {err}")))?;

        match request.reply.kind {
            PermissionReplyKind::AllowOnce | PermissionReplyKind::AllowAlways => {
                let resolved_tool = resolve_pending_tool(&session, &pending.tool)?;
                match self.execute_pending_tool_after_approval(
                    state.as_ref(),
                    session.id,
                    &resolved_tool,
                ) {
                    Ok(execution) => {
                        session = self
                            .apply_tool_success(
                                session,
                                &pending.tool,
                                execution,
                                persisted_rule.clone(),
                                state.clone(),
                            )
                            .await?;
                    }
                    Err(ToolError::UserInputRequired(input)) => {
                        session = self
                            .apply_user_input_request(session, &pending.tool, input, state.clone())
                            .await?;
                    }
                    Err(err) => {
                        session = self
                            .apply_tool_failure(
                                session,
                                &pending.tool,
                                err.to_string(),
                                persisted_rule.clone(),
                                state.clone(),
                            )
                            .await?;
                    }
                }
            }
            PermissionReplyKind::DenyOnce | PermissionReplyKind::DenyAlways => {
                session = self
                    .apply_tool_failure(
                        session,
                        &pending.tool,
                        reply_reason,
                        persisted_rule.clone(),
                        state.clone(),
                    )
                    .await?;
            }
        }

        let manager = self.background_handle();
        let session_id = request.session_id;
        let options = request.options;
        tokio::task::spawn(async move {
            manager
                .run_until_stable_for(session_id, session, &options, state)
                .await
        })
        .await
        .map_err(|err| AppError::Internal(format!("permission continuation task failed: {err}")))?
    }

    pub async fn reply_user_input(
        &self,
        mut request: SessionUserInputReplyRequest,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        session = self
            .apply_requested_agent_profile(session, &mut request.options, state.clone())
            .await?;
        let pending = session
            .find_pending_user_input_by_request_id(request.reply.request_id.as_str())
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "pending user input request not found: {}",
                    request.reply.request_id
                ))
            })?;

        let user_input_request = session
            .pending_user_input_request(&pending)
            .cloned()
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "pending user input request payload missing: {}",
                    request.reply.request_id
                ))
            })?;
        {
            let input_part = session.part_mut(&pending.request).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending user input part not found: {}",
                    request.reply.request_id
                ))
            })?;
            input_part.set_content(PartContent::user_input_request(
                UserInputRequestPart::pending(user_input_request.clone())
                    .with_reply(request.reply.clone()),
            ));
            input_part.status = ExecutionStatus::Completed;
        }

        let is_host_request = self
            .host_user_input_waiters
            .lock()
            .await
            .contains_key(request.reply.request_id.as_str());
        if is_host_request {
            let response = host_user_input_response(&user_input_request, &request.reply)?;
            let tool_part_ref = session
                .resolve_part_ref(&pending.tool.part)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "pending tool part not found: message={}, part={}",
                        pending.tool.part.message_id, pending.tool.part.part_id
                    ))
                })?;
            let assistant_message = session.messages[tool_part_ref.message_index].clone();
            session = self
                .persist_session_changes(
                    session,
                    vec![assistant_message],
                    Vec::new(),
                    None,
                    state.clone(),
                )
                .await?;
            if let Some(waiter) = self
                .host_user_input_waiters
                .lock()
                .await
                .remove(request.reply.request_id.as_str())
            {
                let _ = waiter.response.send(response);
                return Ok(session);
            }
            return Err(AppError::Internal(format!(
                "host user input waiter disappeared before reply delivery: {}",
                request.reply.request_id
            )));
        }
        if request.reply.request_id.starts_with("host-input:") {
            return Err(AppError::Internal(format!(
                "host user input waiter missing: {}",
                request.reply.request_id
            )));
        }

        match request.reply.kind {
            UserInputReplyKind::Submit => {
                let execution = user_input_execution(&user_input_request, &request.reply)?;
                session = self
                    .apply_tool_success(session, &pending.tool, execution, None, state.clone())
                    .await?;
            }
            UserInputReplyKind::Cancel => {
                let reason =
                    request.reply.reason.clone().unwrap_or_else(|| {
                        "user declined to answer requested questions".to_string()
                    });
                session = self
                    .apply_tool_failure(session, &pending.tool, reason, None, state.clone())
                    .await?;
            }
        }

        let manager = self.background_handle();
        let session_id = request.session_id;
        let options = request.options;
        tokio::task::spawn(async move {
            manager
                .run_until_stable_for(session_id, session, &options, state)
                .await
        })
        .await
        .map_err(|err| AppError::Internal(format!("user input continuation task failed: {err}")))?
    }

    /// Convenience wrapper that registers a fresh `TurnControl` for
    /// `session_id`, runs the loop, then unregisters. Used by entry points
    /// that don't already own a control (continuation-style: permission
    /// reply, user-input reply).
    async fn run_until_stable_for(
        &self,
        session_id: i64,
        session: Session,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let (control, steer_rx) = self.turn_registry.register(session_id).await;
        let result = self
            .run_until_stable(session, options, false, state, control.clone(), steer_rx)
            .await;
        self.turn_registry
            .unregister_if_matches(session_id, &control)
            .await;
        result
    }

    pub(super) fn spawn_idle_goal_run_if_needed(
        &self,
        session_id: i64,
        allow_goal_continuation: bool,
    ) {
        let manager = self.background_handle();
        tokio::spawn(async move {
            if manager.turn_registry.is_active(session_id).await {
                return;
            }

            let state = manager.execution_state();
            let session = match manager
                .store
                .load_session(session_id, state.cache_policy())
                .await
            {
                Ok(session) => session,
                Err(err) => {
                    tracing::warn!(
                        target: "agena::session::goal_runtime",
                        session_id,
                        error = %err,
                        "failed to load session for idle goal continuation"
                    );
                    return;
                }
            };

            if session.status() != SessionStatus::Idle {
                return;
            }
            match session.goal.as_ref() {
                None => return,
                Some(goal) if matches!(goal.status, GoalStatus::Completed | GoalStatus::Paused) => {
                    return;
                }
                _ => {}
            }
            if manager
                .goal_turn_directive(&session, allow_goal_continuation)
                .is_none()
            {
                return;
            }

            let options = match manager.resolve_scheduled_run_options(session_id).await {
                Ok(options) => options,
                Err(err) => {
                    tracing::warn!(
                        target: "agena::session::goal_runtime",
                        session_id,
                        error = %err,
                        "failed to resolve options for idle goal continuation"
                    );
                    return;
                }
            };

            let Some((control, steer_rx)) = manager
                .turn_registry
                .try_register_if_inactive(session_id)
                .await
            else {
                return;
            };
            let result = manager
                .run_until_stable(
                    session,
                    &options,
                    allow_goal_continuation,
                    state,
                    control.clone(),
                    steer_rx,
                )
                .await;
            if let Err(err) = result {
                tracing::warn!(
                    target: "agena::session::goal_runtime",
                    session_id,
                    error = %err,
                    "idle goal continuation failed"
                );
            }
            manager
                .turn_registry
                .unregister_if_matches(session_id, &control)
                .await;
        });
    }

    pub(super) async fn run_until_stable(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        allow_goal_continuation: bool,
        state: Arc<SessionManagerState>,
        control: Arc<TurnControl>,
        mut steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
    ) -> Result<Session, AppError> {
        let initial_options =
            self.apply_execution_context_to_run_options(&session, options.clone())?;
        let max_turn_loops = initial_options
            .max_turn_loops
            .unwrap_or(state.config.max_turn_loops);
        let mut continuation_available = allow_goal_continuation;
        for _ in 0..max_turn_loops {
            let current_options =
                self.apply_execution_context_to_run_options(&session, options.clone())?;
            if control.cancel.is_cancelled() {
                if control.is_superseded() {
                    return Ok(session);
                }
                self.persist_run_failed_event(
                    session.id,
                    "turn cancelled by user".to_string(),
                    state.clone(),
                )
                .await?;
                if let Some(paused) = self
                    .pause_active_goal_if_needed(session.id, state.clone())
                    .await?
                {
                    return Ok(paused);
                }
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

            if self.reconcile_goal_runtime(&mut session) {
                session = self
                    .persist_session_changes(session, Vec::new(), Vec::new(), None, state.clone())
                    .await?;
            }

            if let Some(hit) = crate::session::doom_loop::detect(
                session.messages.as_slice(),
                state.config.doom_loop,
            ) {
                tracing::warn!(
                    target: "agena::session::doom_loop",
                    session_id = session.id,
                    tool = %hit.tool_label,
                    repeat = hit.repeat_count,
                    "aborting turn: doom-loop detected"
                );
                self.persist_run_failed_event(session.id, hit.message(), state.clone())
                    .await?;
                return Ok(session);
            }

            let pending_tools = session.pending_tools();
            if !pending_tools.is_empty() {
                session = self
                    .resolve_pending_tools(session, pending_tools, state.clone())
                    .await?;
                continue;
            }

            let goal_turn_directive = self.goal_turn_directive(&session, continuation_available);
            match session.status() {
                SessionStatus::Idle => {
                    if goal_turn_directive.is_none() {
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
                                        model_verbosity: current_options.verbosity.clone(),
                                        model_parallel_tool_calls: current_options
                                            .request_override
                                            .parallel_tool_calls(),
                                        provider_metadata: None,
                                        tags: Vec::new(),
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
                }
                SessionStatus::AwaitingModel => {}
            }

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
                    "automatic session compaction triggered before model turn"
                );
                session = self
                    .auto_compact_session(session, &current_options, state.clone(), control.clone())
                    .await?;
            }

            let session_id = session.id;
            let model = format!(
                "{}/{}",
                current_options.model.provider_id, current_options.model.model_id
            );
            let message_count = session.messages.len();
            let pre_turn_input = crate::plugin::PreTurnInput {
                session_id,
                model: model.clone(),
                message_count,
            };
            state
                .tool_executor
                .plugin_manager()
                .broadcast_pre_turn(pre_turn_input)
                .await;

            let mut model_session = session;
            if let Some(directive) = goal_turn_directive.as_ref() {
                model_session = self
                    .append_goal_turn_directive_message(
                        model_session,
                        directive,
                        &current_options,
                        state.clone(),
                    )
                    .await?;
            }

            match self
                .run_model_turn(
                    model_session,
                    &current_options,
                    state.clone(),
                    control.clone(),
                )
                .await
            {
                Ok(mut next_session) => {
                    if goal_turn_directive.as_ref().is_some_and(|directive| {
                        directive.kind == GoalTurnDirectiveKind::Continuation
                    }) {
                        continuation_available = false;
                    }
                    if let Some(directive) = goal_turn_directive.as_ref()
                        && self.apply_goal_turn_directive(&mut next_session, directive)
                    {
                        next_session = self
                            .persist_session_changes(
                                next_session,
                                Vec::new(),
                                Vec::new(),
                                None,
                                state.clone(),
                            )
                            .await?;
                    }
                    session = next_session;
                    let post_turn_input = crate::plugin::PostTurnInput {
                        session_id: session.id,
                        model,
                        status: format!("{:?}", session.status()),
                        message_count: session.messages.len(),
                    };
                    state
                        .tool_executor
                        .plugin_manager()
                        .broadcast_post_turn(post_turn_input)
                        .await;
                }
                Err(err) => {
                    let post_turn_input = crate::plugin::PostTurnInput {
                        session_id,
                        model,
                        status: format!("error: {err}"),
                        message_count,
                    };
                    state
                        .tool_executor
                        .plugin_manager()
                        .broadcast_post_turn(post_turn_input)
                        .await;
                    return Err(err);
                }
            }
        }

        Err(AppError::Internal(
            "session manager exceeded max turn loop budget".to_string(),
        ))
    }

    pub(super) async fn run_model_turn(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
        control: Arc<TurnControl>,
    ) -> Result<Session, AppError> {
        let turn_span = tracing::info_span!(
            "session.turn",
            session_id = session.id,
            provider_id = %options.model.provider_id,
            model_id = %options.model.model_id,
        );
        {
            let active_messages = prompt_window::active_prompt_messages(&session);
            let scoped_executor = state
                .tool_executor
                .for_session_context(&session.runtime.execution);
            let tools = scoped_executor.available_tools_for_messages_and_loaded(
                active_messages.as_slice(),
                session.runtime.loaded_deferred_tools(),
            );
            let request_tools = tools.clone();
            let prompt_budget =
                self.prompt_budget_for_turn(&session, options, tools.as_slice(), state.as_ref());
            let provider_request_shape = state.processor.prompt_cache_shape(&options.model)?;
            let continuation_supported =
                state.processor.supports_prompt_continuation(&options.model);
            let prompt_request_options = PromptRequestOptions {
                provider_id: options.model.provider_id.as_str(),
                model_id: options.model.model_id.as_str(),
                system: options.system.as_deref(),
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
                continuation_reason = prepared.continuation_reason.as_str(),
                provider_request_shape_fingerprint = provider_request_shape_fingerprint
                    .as_deref()
                    .unwrap_or(""),
                provider_request_shape_changed = prepared
                    .continuation_diagnostic
                    .provider_shape_changed(),
                provider_request_shape_change_keys = ?provider_shape_change_keys,
                prompt_message_count = prepared.messages.len(),
                system_included = prepared.system.is_some(),
                "prepared prompt for session turn"
            );

            session.runtime.turn.record_model_request(
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
            let run = SessionRunRequest {
                session_id: session.id,
                model: options.model.clone(),
                model_thinking_mode: options.thinking_mode.clone(),
                model_speed_mode: options.speed_mode.clone(),
                model_parallel_tool_calls: options.request_override.parallel_tool_calls(),
                completion: options.completion_request(
                    prepared.system.clone(),
                    prepared.messages.clone(),
                    tools,
                    Some(prepared.prompt_cache_key.clone()),
                    prepared.previous_response_id.clone(),
                    Some(prepared.prompt_window_generation),
                ),
                next_message_id: processor_ids.message_id,
                part_ids: processor_ids.part_ids,
                next_call_id: session.next_call_id(),
                event_publisher: Some(Arc::clone(&self.publisher)),
                cancel: Some(control.cancel.clone()),
            };

            self.store
                .append_client_events(
                    session.id,
                    vec![EventKind::RunStarted(RunStartedEvent {
                        session_id: session.id,
                        ts_ms: Utc::now().timestamp_millis(),
                    })],
                )
                .await?;

            let processor_fut = state.processor.run_turn(run).instrument(turn_span.clone());
            let turn_outcome = tokio::select! {
                res = processor_fut => res,
                _ = control.cancel.cancelled() => {
                    Err(AppError::Internal("turn cancelled by user".to_string()))
                }
            };
            match turn_outcome {
                Ok(result) => {
                    let turn_id = result.turn_id;
                    let terminal_error = result.terminal_error;
                    if terminal_error.as_ref().is_some_and(is_user_cancelled_error)
                        && control.is_superseded()
                    {
                        return Ok(session);
                    }
                    let assistant_message = result
                        .state
                        .into_iter()
                        .find(|message| message.id == result.assistant_message_id)
                        .ok_or_else(|| {
                            AppError::Internal(format!(
                                "assistant message not found after processor turn: {}",
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
                                "failed to refresh provider request shape after turn; falling back to prepared shape"
                            );
                            prepared.provider_request_shape.clone()
                        }
                    };
                    let anchored_prompt_request_options = PromptRequestOptions {
                        provider_id: options.model.provider_id.as_str(),
                        model_id: options.model.model_id.as_str(),
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
                            options.model.provider_id.as_str(),
                            options.model.model_id.as_str(),
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

                    let mut turn_events: Vec<EventKind> = Vec::new();
                    turn_events.push(EventKind::TurnStarted(TurnStarted {
                        turn_id,
                        model_id: options.model.model_id.as_str().into(),
                        provider_id: options.model.provider_id.as_str().into(),
                        request_digest: None,
                    }));
                    turn_events.extend(result.history_items);
                    if let Some(err) = terminal_error.as_ref() {
                        turn_events.push(EventKind::TurnAborted(TurnAborted {
                            turn_id,
                            reason: TurnAbortReason::ProviderError,
                            message: Some(err.to_string()),
                        }));
                    } else {
                        turn_events.push(EventKind::TurnCompleted(TurnCompleted {
                            turn_id,
                            finish_reason: FinishReason::default(),
                        }));
                    }
                    let store = Arc::clone(&self.store);
                    let cache_policy = state.cache_policy();
                    persisted_session = tokio::task::spawn(async move {
                        store
                            .append_history_items(persisted_session, turn_events, cache_policy)
                            .await
                    })
                    .await
                    .map_err(|err| {
                        AppError::Internal(format!("history append task failed: {err}"))
                    })??;

                    if let Some(err) = terminal_error {
                        if is_user_cancelled_error(&err) {
                            if control.is_superseded() {
                                return Ok(persisted_session);
                            }
                            if let Some(paused) = self
                                .pause_active_goal_if_needed(persisted_session.id, state.clone())
                                .await?
                            {
                                persisted_session = paused;
                            }
                        }
                        self.persist_run_failed_event(persisted_session.id, err.to_string(), state)
                            .await?;
                        return Err(err);
                    }

                    Ok(persisted_session)
                }
                Err(err) => {
                    if is_user_cancelled_error(&err) {
                        if control.is_superseded() {
                            return Ok(session);
                        }
                        let _ = self
                            .pause_active_goal_if_needed(session.id, state.clone())
                            .await?;
                    }
                    self.persist_run_failed_event(session.id, err.to_string(), state)
                        .await?;
                    Err(err)
                }
            }
        }
    }

    fn prompt_budget_for_turn(
        &self,
        _session: &Session,
        options: &SessionRunOptions,
        tools: &[crate::plugin::registry::PluginEntry],
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
            options.system.as_deref(),
            tools,
        );

        PromptTurnBudget {
            max_prompt_chars,
            max_prompt_tokens: prompt_window::approximate_tokens_from_chars(max_prompt_chars),
            model_context_window_tokens: context_window_tokens,
        }
    }

    async fn resolve_pending_tools(
        &self,
        mut session: Session,
        pending_tools: Vec<SessionPendingTool>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
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
                        .apply_user_input_request(session, &resolved.pending, input, state)
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

    async fn prepare_concurrent_pending_tool(
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
            .prepare_bash_invocation(&prepared.invocation, session.id, resolved.call_id)
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
            tool_part.set_content(PartContent::Operation(OperationPart::pending(
                resolved.call_id,
                prepared.invocation,
                prepared.title_override.unwrap_or(current_title),
                resolved.lifecycle.clone(),
            )));
        }

        if let Err(err) = scoped_executor.enforce_plan_mode_for(&resolved.invocation, session.id) {
            tracing::debug!(
                target: "agena::session::tools",
                session_id = session.id,
                call_id = resolved.call_id,
                error = %err,
                "deferring plan-mode tool refusal to sequential failure handling"
            );
            *session = before_prepare;
            return Ok(None);
        }

        if !scoped_executor.is_concurrency_safe_invocation(&resolved.invocation) {
            *session = before_prepare;
            return Ok(None);
        }

        let permission_checks =
            match scoped_executor.collect_permission_checks_for_invocation(&resolved.invocation) {
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
    async fn execute_pending_tools_concurrently(
        &self,
        state: Arc<SessionManagerState>,
        session_id: i64,
        pending_tools: Vec<ResolvedPendingTool>,
    ) -> Result<Vec<Result<ToolInvocationExecution, ToolError>>, AppError> {
        // Cap concurrent blocking tool executions so a wide tool fan-out
        // cannot exhaust the tokio blocking pool.
        static TOOL_BLOCKING_LIMIT: std::sync::OnceLock<Arc<Semaphore>> =
            std::sync::OnceLock::new();
        let semaphore = TOOL_BLOCKING_LIMIT
            .get_or_init(|| Arc::new(Semaphore::new(32)))
            .clone();

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
                scoped_executor.execute_invocation_detailed(
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

    async fn resolve_pending_tool(
        &self,
        mut session: Session,
        pending_tool: SessionPendingTool,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let mut resolved = resolve_pending_tool(&session, &pending_tool)?;
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution);
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
            .prepare_bash_invocation(&prepared.invocation, session.id, resolved.call_id)
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
            tool_part.set_content(PartContent::Operation(OperationPart::pending(
                resolved.call_id,
                prepared.invocation,
                prepared.title_override.unwrap_or(current_title),
                resolved.lifecycle.clone(),
            )));
            session_changed = true;
        }

        // Plan-mode guardrail: refuse mutating tools while the session
        // is in plan mode.
        if let Err(err) = scoped_executor.enforce_plan_mode_for(&resolved.invocation, session.id) {
            return Box::pin(self.apply_tool_failure(
                session,
                &resolved.pending,
                err.to_string(),
                None,
                state,
            ))
            .await;
        }

        let permission_checks =
            match scoped_executor.collect_permission_checks_for_invocation(&resolved.invocation) {
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

        for check in permission_checks {
            let resolution = self
                .resolve_permission_decision(Some(session.id), &check)
                .await?;
            match resolution.decision {
                PermissionDecision::Allow => {}
                PermissionDecision::Ask { reason } => {
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
                    return self
                        .apply_permission_request(
                            session,
                            &resolved.pending,
                            check.action,
                            reason,
                            resolution.explanation,
                            source,
                            scope,
                            operator,
                            resolution.risk,
                            resolution.trace,
                            state,
                        )
                        .await;
                }
                PermissionDecision::Deny { reason } => {
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
            .execute_invocation_streaming(&resolved.invocation, session.id, resolved.call_id)
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
                self.apply_user_input_request(session, &resolved.pending, input, state)
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

    pub(crate) async fn resolve_tool_permission_check(
        &self,
        session_id: Option<i64>,
        check: &ToolPermissionCheck,
    ) -> Result<crate::permission::PermissionResolution, AppError> {
        self.resolve_permission_decision(session_id, check).await
    }

    async fn resolve_permission_decision(
        &self,
        session_id: Option<i64>,
        check: &ToolPermissionCheck,
    ) -> Result<crate::permission::PermissionResolution, AppError> {
        let key = permission_action_key(&check.action)?;
        let persisted_rule = self
            .store
            .resolve_permission_rule(key.as_str(), session_id)
            .await?;
        let mut resolution =
            resolve_permission_with_persisted_rule(check.decision.clone(), persisted_rule.as_ref());

        if persisted_rule.is_none() {
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

    pub(super) async fn require_immediate_tool_permissions(
        &self,
        session_id: i64,
        executor: &ToolExecutor,
        invocation: &ToolInvocation,
    ) -> Result<(), AppError> {
        for check in executor
            .collect_permission_checks_for_invocation(invocation)
            .map_err(tool_error_to_app_error)?
        {
            match self
                .resolve_permission_decision(Some(session_id), &check)
                .await?
                .decision
            {
                PermissionDecision::Allow => {}
                PermissionDecision::Ask { reason } => {
                    return Err(AppError::Internal(format!(
                        "permission confirmation required: {reason}"
                    )));
                }
                PermissionDecision::Deny { reason } => {
                    return Err(AppError::Internal(format!("permission denied: {reason}")));
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_permission_request(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        action: PermissionAction,
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
            request_id: resolved.operation_id.clone(),
            session_id: Some(session.id),
            action,
            reason: reason.clone(),
            explanation: explanation.clone(),
            source,
            scope,
            operator,
            risk,
            trace: trace.clone(),
            created_at: Utc::now(),
        };

        {
            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    resolved.pending.part.message_id, resolved.pending.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::Operation(OperationPart::pending(
                resolved.call_id,
                resolved.invocation.clone(),
                format!("Awaiting permission: {reason}"),
                resolved.lifecycle.clone(),
            )));
            tool_part.status = ExecutionStatus::Pending;
            tool_part.summary = Some(reason.clone());
        }

        let permission_part_id = self.store.reserve_part_id().await?;
        let permission_part = build_permission_part(
            permission_part_id,
            resolved.pending.part.message_id,
            resolved.operation_id.as_str(),
            PermissionRequestPart::pending(request.clone()),
        );
        session.messages[resolved.pending.part.message_index]
            .parts
            .push(permission_part.clone());

        let assistant_message = session.messages[resolved.pending.part.message_index].clone();
        self.publisher
            .publish(
                crate::event::PublishContext::for_session(session.id),
                EventKind::PermissionRequested(PermissionRequestedEvent {
                    session_id: session.id,
                    request_id: resolved.operation_id.clone(),
                    reason: reason.clone(),
                    explanation,
                    source: request.source.clone(),
                    scope: request.scope.map(permission_scope_label),
                    operator: request.operator.clone(),
                    risk: request.risk,
                    trace,
                    ts_ms: Utc::now().timestamp_millis(),
                }),
            )
            .await
            .map_err(|err| {
                AppError::Internal(format!("publish permission request failed: {err}"))
            })?;
        self.persist_session_changes(
            session,
            vec![assistant_message],
            Vec::new(),
            None,
            state.clone(),
        )
        .await
    }

    async fn apply_user_input_request(
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

    pub(super) async fn apply_user_input_request_with_id(
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
            questions: input.questions,
            created_at: Utc::now(),
        };

        {
            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    resolved.pending.part.message_id, resolved.pending.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::Operation(OperationPart::pending(
                resolved.call_id,
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
        }

        let input_part_id = self.store.reserve_part_id().await?;
        let input_part = build_user_input_part(
            input_part_id,
            resolved.pending.part.message_id,
            resolved.operation_id.as_str(),
            UserInputRequestPart::pending(request.clone()),
        );
        session.messages[resolved.pending.part.message_index]
            .parts
            .push(input_part.clone());

        let assistant_message = session.messages[resolved.pending.part.message_index].clone();
        self.persist_session_changes(session, vec![assistant_message], Vec::new(), None, state)
            .await
    }

    async fn apply_streaming_tool_execution(
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
                .store
                .load_session(session.id, state.cache_policy())
                .await?;
            let tool_part_ref = session
                .resolve_part_ref(&pending_tool.part)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "streaming tool part not found: message={}, part={}",
                        pending_tool.part.message_id, pending_tool.part.part_id
                    ))
                })?;
            let tool_part = session.part_mut(&tool_part_ref).ok_or_else(|| {
                AppError::Internal(format!(
                    "streaming tool part not found: message={}, part={}",
                    pending_tool.part.message_id, pending_tool.part.part_id
                ))
            })?;
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

            let assistant_message = session.messages[tool_part_ref.message_index].clone();
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

    async fn apply_tool_success(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        execution: ToolInvocationExecution,
        persisted_rule: Option<PersistedPermissionRule>,
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
        if let Some(loaded_tools) = loaded_tools_from_tool_output(&tool_output) {
            session.runtime.record_loaded_deferred_tools(&loaded_tools);
        }
        self.apply_tool_success_execution_context(&mut session, &resolved.invocation, &execution);

        {
            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    resolved.pending.part.message_id, resolved.pending.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::Operation(OperationPart::completed(
                resolved.call_id,
                resolved.invocation.clone(),
                output_text.clone(),
                blocks.clone(),
                execution.view.attachments.clone(),
                tool_output.clone(),
                lifecycle.clone(),
            )));
            tool_part.status = ExecutionStatus::Completed;
        }

        let assistant_message = session.messages[resolved.pending.part.message_index].clone();
        let tool_call_id = tool_call_id_for(&resolved);
        let completed_part = assistant_message
            .parts
            .iter()
            .find(|part| {
                part.kind == crate::message::PartKind::Operation
                    && part.operation_id.as_deref() == Some(tool_call_id.as_str())
            })
            .cloned();
        let tool_output_event = TranscriptToolOutput::Text {
            text: execution.view.output_text.clone(),
        };
        let session = self
            .persist_session_changes(
                session,
                vec![assistant_message.clone()],
                Vec::new(),
                persisted_rule.clone(),
                state.clone(),
            )
            .await?;
        let now = Utc::now();
        let turn_id = HistoryTurnId::new();
        let events = vec![EventKind::ToolCallCompleted(ToolCallCompleted {
            message_id: HistoryMessageId(assistant_message.id),
            call_id: tool_call_id,
            turn_id,
            tool_name: resolved.invocation.name.clone().into(),
            part: completed_part,
            output: tool_output_event,
            completed_at: now,
        })];
        self.store
            .append_history_items(session, events, state.cache_policy())
            .await
    }

    async fn apply_tool_failure(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        reason: String,
        persisted_rule: Option<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let blocks = text_result_blocks(reason.as_str());

        // Notify plugins about the tool failure (fire-and-forget).
        state.tool_executor.broadcast_tool_failure(
            &resolved.invocation,
            session.id,
            resolved.call_id,
            &reason,
        );

        {
            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    resolved.pending.part.message_id, resolved.pending.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::Operation(OperationPart::failed(
                resolved.call_id,
                resolved.invocation.clone(),
                reason.clone(),
                reason.clone(),
                blocks.clone(),
                Vec::new(),
                ToolOutput::default(),
                lifecycle.clone(),
            )));
            tool_part.status = ExecutionStatus::Failed;
        }

        let assistant_message = session.messages[resolved.pending.part.message_index].clone();
        let tool_call_id = tool_call_id_for(&resolved);
        let completed_part = assistant_message
            .parts
            .iter()
            .find(|part| {
                part.kind == crate::message::PartKind::Operation
                    && part.operation_id.as_deref() == Some(tool_call_id.as_str())
            })
            .cloned();
        let session = self
            .persist_session_changes(
                session,
                vec![assistant_message.clone()],
                Vec::new(),
                persisted_rule.clone(),
                state.clone(),
            )
            .await?;
        let now = Utc::now();
        let turn_id = HistoryTurnId::new();
        let events = vec![EventKind::ToolCallCompleted(ToolCallCompleted {
            message_id: HistoryMessageId(assistant_message.id),
            call_id: tool_call_id,
            turn_id,
            tool_name: resolved.invocation.name.clone().into(),
            part: completed_part,
            output: TranscriptToolOutput::Error { message: reason },
            completed_at: now,
        })];
        self.store
            .append_history_items(session, events, state.cache_policy())
            .await
    }

    pub(super) async fn persist_session_changes(
        &self,
        session: Session,
        touched_messages: Vec<Message>,
        client_events: Vec<EventKind>,
        persisted_rule: Option<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        self.store
            .persist(
                SessionCommit {
                    session,
                    touched_messages,
                    client_events,
                    persisted_rule,
                },
                state.cache_policy(),
            )
            .await
    }

    fn reconcile_goal_runtime(&self, session: &mut Session) -> bool {
        let Some(goal) = session.goal.as_ref() else {
            if session.runtime.goal.is_empty() {
                return false;
            }
            session.runtime.goal.clear();
            return true;
        };

        if goal.status == GoalStatus::Completed {
            if session.runtime.goal.is_empty() {
                return false;
            }
            session.runtime.goal.clear();
            return true;
        }

        let mut changed = false;
        if session
            .runtime
            .goal
            .pending_steering()
            .is_some_and(|pending| pending.goal_id != goal.id)
        {
            session.runtime.goal.clear_pending_steering();
            changed = true;
        }

        changed
    }

    pub(super) fn goal_turn_directive(
        &self,
        session: &Session,
        allow_continuation: bool,
    ) -> Option<GoalTurnDirective> {
        let goal = session.goal.as_ref()?;
        match goal.status {
            GoalStatus::Completed => None,
            GoalStatus::Paused => None,
            GoalStatus::Active => {
                if let Some(pending) = session.runtime.goal.pending_steering()
                    && pending.goal_id == goal.id
                    && pending.kind == GoalSteeringKind::ObjectiveUpdated
                {
                    return Some(GoalTurnDirective {
                        goal_id: goal.id,
                        kind: GoalTurnDirectiveKind::ObjectiveUpdated,
                        prompt: self
                            .render_goal_context(goal, GoalTurnDirectiveKind::ObjectiveUpdated),
                    });
                }
                if allow_continuation && session.status() == SessionStatus::Idle {
                    return Some(GoalTurnDirective {
                        goal_id: goal.id,
                        kind: GoalTurnDirectiveKind::Continuation,
                        prompt: self.render_goal_context(goal, GoalTurnDirectiveKind::Continuation),
                    });
                }
                None
            }
        }
    }

    pub(super) async fn append_goal_turn_directive_message(
        &self,
        mut session: Session,
        directive: &GoalTurnDirective,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let ids = self.store.reserve_message_ids(1).await?;
        let text = format!(
            "<goal_context>\n{}\n</goal_context>",
            directive.prompt.trim()
        );
        let goal_message = build_message(
            ids,
            Role::User,
            MessageStatus::Completed,
            vec![PartContent::text(text)],
            MessageMetadata {
                source: MessageSource::System,
                parent_message_id: session
                    .last_conversation_message()
                    .map(|message| message.id),
                generated_by_call_id: None,
                model_provider_id: options.model.provider_id.to_string(),
                model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
                model_id: options.model.model_id.to_string(),
                model_thinking_mode: options.thinking_mode.clone(),
                model_speed_mode: options.speed_mode.clone(),
                model_verbosity: options.verbosity.clone(),
                model_parallel_tool_calls: options.request_override.parallel_tool_calls(),
                provider_metadata: None,
                tags: Vec::new(),
            },
        );
        session.messages.push(goal_message.clone());
        session = self
            .persist_session_changes(
                session,
                vec![goal_message.clone()],
                Vec::new(),
                None,
                state.clone(),
            )
            .await?;

        let turn_id = HistoryTurnId::new();
        let history_items = vec![
            EventKind::TurnStarted(TurnStarted {
                turn_id,
                model_id: options.model.model_id.as_str().into(),
                provider_id: options.model.provider_id.as_str().into(),
                request_digest: None,
            }),
            EventKind::UserMessageAppended(UserMessageAppended {
                message_id: HistoryMessageId(goal_message.id),
                turn_id,
                created_at: goal_message.created_at,
                content: TranscriptContent::from_message_lossy(&goal_message),
                parts: goal_message.parts.clone(),
                metadata: goal_message.metadata.clone(),
            }),
            EventKind::TurnCompleted(TurnCompleted {
                turn_id,
                finish_reason: FinishReason::Stop,
            }),
        ];
        self.store
            .append_history_items(session, history_items, state.cache_policy())
            .await
    }

    fn apply_goal_turn_directive(
        &self,
        session: &mut Session,
        directive: &GoalTurnDirective,
    ) -> bool {
        match directive.kind {
            GoalTurnDirectiveKind::Continuation => false,
            GoalTurnDirectiveKind::ObjectiveUpdated => {
                if session
                    .runtime
                    .goal
                    .pending_steering()
                    .is_some_and(|pending| {
                        pending.goal_id == directive.goal_id
                            && pending.kind == GoalSteeringKind::ObjectiveUpdated
                    })
                {
                    session.runtime.goal.clear_pending_steering();
                    return true;
                }
                false
            }
        }
    }

    fn render_goal_context(&self, goal: &SessionGoal, kind: GoalTurnDirectiveKind) -> String {
        let objective = goal.objective.trim();
        match kind {
            GoalTurnDirectiveKind::ObjectiveUpdated => join_runtime_context_lines(&[
                "An active runtime goal has been set or updated.".to_string(),
                format!("Objective:\n{objective}"),
                "Continue making concrete progress toward this goal without waiting for additional user input. Use tools when needed, keep the work grounded in the current workspace, and call `update_goal` with `status = complete` once the objective is actually finished.".to_string(),
            ]),
            GoalTurnDirectiveKind::Continuation => join_runtime_context_lines(&[
                "Continue working toward the active runtime goal.".to_string(),
                format!("Objective:\n{objective}"),
                "Do not wait for the user just because the last turn ended. Make the next concrete move toward finishing the objective, explain the blocker if you are truly blocked, and call `update_goal` with `status = complete` once the objective is actually done.".to_string(),
            ]),
        }
    }

    pub(super) async fn publish_goal_event(
        &self,
        goal: &SessionGoal,
        session_id: i64,
    ) -> Result<(), AppError> {
        self.publisher
            .publish(
                crate::event::PublishContext::for_session(session_id),
                EventKind::SessionGoalUpdated(SessionGoalEvent {
                    session_id,
                    goal_id: Some(goal.id),
                    objective: Some(goal.objective.clone()),
                    status: Some(match goal.status {
                        GoalStatus::Active => "active".to_string(),
                        GoalStatus::Paused => "paused".to_string(),
                        GoalStatus::Completed => "completed".to_string(),
                    }),
                    completed_at_ms: goal.completed_at.map(|ts| ts.timestamp_millis()),
                    ts_ms: Utc::now().timestamp_millis(),
                }),
            )
            .await
            .map_err(|err| AppError::Internal(format!("publish goal event failed: {err}")))?;
        Ok(())
    }

    pub(super) fn apply_execution_context_to_run_options(
        &self,
        session: &Session,
        mut options: SessionRunOptions,
    ) -> Result<SessionRunOptions, AppError> {
        self.apply_selection_modes_to_run_options(session, &mut options)?;
        if let Some(system) = session.runtime.execution.system_prompt_override.as_ref() {
            options.system = Some(system.clone());
        }
        if options.temperature.is_none() {
            options.temperature = session
                .runtime
                .execution
                .agent_run
                .temperature
                .map(|value| value.0);
        }
        if options.temperature.is_none() {
            let execution = self.execution_state();
            let provider_registry = execution.processor.provider_registry();
            if let Ok(metadata) = provider_registry.model_metadata(&options.model) {
                options.temperature = metadata.parsed_default_temperature();
            }
        }
        if options.max_output_tokens.is_none() {
            options.max_output_tokens = session.runtime.execution.agent_run.max_output_tokens;
        }
        if options.max_turn_loops.is_none() {
            options.max_turn_loops = session.runtime.execution.agent_run.steps;
        }
        if options.agent_profile.is_none() {
            options.agent_profile = session.runtime.execution.selection.agent.clone();
        }
        Ok(options)
    }

    fn apply_selection_modes_to_run_options(
        &self,
        session: &Session,
        options: &mut SessionRunOptions,
    ) -> Result<(), AppError> {
        let state = self.execution_state();
        let effective_selection = state
            .config
            .default_selection
            .overlay_with_cascade(&session.runtime.execution.selection);
        let selection_model = effective_selection.model_ref().map_err(|error| {
            AppError::Internal(format!(
                "session {} contains invalid execution model selection: {error}",
                session.id
            ))
        })?;
        let modes_belong_to_options_model = selection_model
            .as_ref()
            .is_some_and(|model| model == &options.model);
        if options.thinking_mode.is_none() {
            options.thinking_mode = modes_belong_to_options_model
                .then(|| effective_selection.thinking_mode.clone())
                .flatten();
        }
        if options.speed_mode.is_none() {
            options.speed_mode = modes_belong_to_options_model
                .then(|| effective_selection.speed_mode.clone())
                .flatten();
        }
        if options.request_override.parallel_tool_calls().is_none() {
            options.request_override.set_parallel_tool_calls(
                modes_belong_to_options_model
                    .then_some(effective_selection.parallel_tool_calls)
                    .flatten(),
            );
        }
        if options.verbosity.is_none() {
            options.verbosity = modes_belong_to_options_model
                .then(|| effective_selection.verbosity.clone())
                .flatten();
        }
        self.apply_model_mode_requests(options)
    }

    fn apply_model_mode_requests(&self, options: &mut SessionRunOptions) -> Result<(), AppError> {
        let execution = self.execution_state();
        let provider_registry = execution.processor.provider_registry();
        let resolved_adapter_id = options.model.adapter_id.clone().or_else(|| {
            provider_registry
                .get(options.model.provider_id.as_str())
                .and_then(|provider| provider.default_adapter().cloned())
        });

        let requested_parallel_tool_calls = options.request_override.parallel_tool_calls();
        let mut merged_override = options.request_override.clone();
        merged_override.set_parallel_tool_calls(None);
        if let Some(thinking_mode_name) = options.thinking_mode.as_deref() {
            let thinking_modes = provider_registry.model_thinking_modes(&options.model)?;
            let thinking_mode = thinking_modes.get(thinking_mode_name).ok_or_else(|| {
                AppError::Config(format!(
                    "model `{}` has no thinking mode `{thinking_mode_name}`",
                    options.model
                ))
            })?;
            options.thinking = thinking_mode.thinking.clone();
            merged_override = merged_override.merged_with(&mode_request_override_for_adapter(
                &thinking_mode.request_override,
                &thinking_mode.adapter_overrides,
                resolved_adapter_id.as_ref(),
            ));
        }
        if let Some(speed_mode_name) = options.speed_mode.as_deref() {
            let speed_modes = provider_registry.model_speed_modes(&options.model)?;
            let speed_mode = speed_modes.get(speed_mode_name).ok_or_else(|| {
                AppError::Config(format!(
                    "model `{}` has no speed mode `{speed_mode_name}`",
                    options.model
                ))
            })?;
            merged_override = merged_override.merged_with(&mode_request_override_for_adapter(
                &speed_mode.request_override,
                &speed_mode.adapter_overrides,
                resolved_adapter_id.as_ref(),
            ));
        }
        if requested_parallel_tool_calls.is_some() {
            merged_override.set_parallel_tool_calls(requested_parallel_tool_calls);
        }
        options.request_override = merged_override;
        Ok(())
    }

    fn session_permission_defaults(
        &self,
        session: &Session,
        state: &SessionManagerState,
    ) -> crate::agent::PermissionConfig {
        let global_defaults = state
            .config
            .default_selection
            .permission
            .effective_with_defaults(&state.config.permission);
        session
            .runtime
            .execution
            .selection
            .permission
            .effective_with_defaults(&global_defaults)
    }

    fn model_from_session_selection(
        &self,
        session: &Session,
    ) -> Result<Option<ModelRef>, AppError> {
        session
            .runtime
            .execution
            .selection
            .model_ref()
            .map_err(|error| {
                AppError::Internal(format!(
                    "session {} contains invalid execution model selection: {error}",
                    session.id
                ))
            })
    }

    pub(super) fn default_model_from_config(
        &self,
        state: &SessionManagerState,
    ) -> Result<Option<ModelRef>, AppError> {
        let selection = &state.config.default_selection;
        let Some(provider_id) = selection.provider.as_deref() else {
            return Ok(None);
        };
        state
            .processor
            .provider_registry()
            .resolve_model_selection(
                provider_id,
                selection.adapter.as_deref(),
                selection.model.as_deref(),
            )
            .map(Some)
    }

    pub(super) fn model_from_session_or_default(
        &self,
        session: &Session,
        state: &SessionManagerState,
    ) -> Result<ModelRef, AppError> {
        self.model_from_session_selection(session)?
            .map(Ok)
            .unwrap_or_else(|| {
                self.default_model_from_config(state)?.ok_or_else(|| {
                    AppError::Internal(format!(
                        "model is required for session {}; set a session model or global default model",
                        session.id
                    ))
                })
            })
    }

    pub(super) fn run_options_from_session(
        &self,
        session: &Session,
        state: Arc<SessionManagerState>,
    ) -> Result<SessionRunOptions, AppError> {
        let model = self.model_from_session_or_default(session, &state)?;

        self.apply_execution_context_to_run_options(
            session,
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
                max_turn_loops: None,
            },
        )
    }

    pub(super) async fn clear_session_agent_profile(
        &self,
        mut session: Session,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        session.runtime.execution.selection.agent = None;
        session.runtime.execution.agent_mode = None;
        session.runtime.execution.agent_hidden = false;
        session.runtime.execution.agent_color = None;
        session.runtime.execution.system_prompt_override = None;
        session.runtime.set_allowed_tools(Vec::new());
        session.runtime.execution.agent_permission =
            self.session_permission_defaults(&session, &state);
        session.runtime.execution.agent_run = crate::agent::AgentRunConfig::default();
        session.runtime.set_model_override(None, None, None);
        session
            .runtime
            .set_model_mode_overrides(None, None, None, None);
        Ok(session)
    }

    pub(super) async fn apply_requested_agent_profile(
        &self,
        session: Session,
        options: &mut SessionRunOptions,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let requested = options
            .agent_profile
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let persisted = session
            .runtime
            .execution
            .selection
            .agent
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let requested_explicitly = requested.is_some();
        let persisted_explicitly = persisted.is_some();
        let effective = requested
            .or(persisted)
            .or_else(|| state.config.default_agent.clone());
        let Some(agent_name) = effective else {
            let mut session = session;
            session.runtime.execution.agent_permission =
                self.session_permission_defaults(&session, &state);
            return Ok(session);
        };
        let profile = state
            .tool_executor
            .subagent_registry()
            .require(agent_name.as_str())
            .map_err(|err| AppError::Config(err.to_string()))?;
        if session.is_subagent && !profile.frontmatter.mode.allows_subagent() {
            return Err(AppError::Config(format!(
                "agent profile '{}' is not available for subtask sessions",
                profile.name
            )));
        }
        if !session.is_subagent && !profile.frontmatter.mode.allows_root() {
            return Err(AppError::Config(format!(
                "agent profile '{}' is not available for root sessions",
                profile.name
            )));
        }
        options.agent_profile = Some(profile.name.clone());
        if session.runtime.execution.selection.agent.as_deref() == Some(profile.name.as_str())
            && session.runtime.execution.system_prompt_override.is_some()
        {
            *options = self.apply_execution_context_to_run_options(&session, options.clone())?;
            return Ok(session);
        }
        self.apply_agent_profile_to_session(
            session,
            options,
            profile,
            state,
            requested_explicitly || persisted_explicitly,
        )
        .await
    }

    async fn apply_agent_profile_to_session(
        &self,
        mut session: Session,
        options: &mut SessionRunOptions,
        profile: crate::agents::AgentProfile,
        state: Arc<SessionManagerState>,
        apply_profile_model_override: bool,
    ) -> Result<Session, AppError> {
        let next_allowed_tools = profile.frontmatter.allowed_tools.clone();
        let permission_defaults = self.session_permission_defaults(&session, &state);
        let next_permission = profile
            .frontmatter
            .permission
            .effective_with_defaults(&permission_defaults);
        let next_system = profile.prompt.trim().to_string();
        let next_model = self.resolve_root_agent_model(
            &session,
            options,
            &state,
            if apply_profile_model_override {
                Some(&profile.frontmatter.default)
            } else {
                None
            },
        )?;
        let next_model_provider_id = next_model.provider_id.to_string();
        let next_model_adapter_id = next_model.adapter_id.as_ref().map(ToString::to_string);
        let next_model_id = next_model.model_id.to_string();
        options.model = next_model.clone();
        self.apply_selection_modes_to_run_options(&session, options)?;
        let next_thinking_mode = options.thinking_mode.clone();
        let next_speed_mode = options.speed_mode.clone();
        let next_verbosity = options.verbosity.clone();
        let next_parallel_tool_calls = options.request_override.parallel_tool_calls();
        let next_run = crate::agent::AgentRunConfig {
            temperature: profile.frontmatter.temperature,
            max_output_tokens: profile.frontmatter.max_output_tokens,
            steps: profile.frontmatter.steps,
        };
        let changed = session.runtime.execution.selection.agent.as_deref()
            != Some(profile.name.as_str())
            || session.runtime.execution.agent_mode != Some(profile.frontmatter.mode)
            || session.runtime.execution.agent_hidden != profile.frontmatter.hidden
            || session.runtime.execution.agent_color != profile.frontmatter.color
            || session.runtime.execution.system_prompt_override.as_deref()
                != Some(next_system.as_str())
            || session.runtime.allowed_tools() != next_allowed_tools.as_slice()
            || session.runtime.execution.agent_permission != next_permission
            || session.runtime.execution.selection.provider.as_deref()
                != Some(next_model_provider_id.as_str())
            || session.runtime.execution.selection.adapter.as_deref()
                != next_model_adapter_id.as_deref()
            || session.runtime.execution.selection.model.as_deref() != Some(next_model_id.as_str())
            || session.runtime.execution.selection.thinking_mode != next_thinking_mode
            || session.runtime.execution.selection.speed_mode != next_speed_mode
            || session.runtime.execution.selection.verbosity != next_verbosity
            || session.runtime.execution.selection.parallel_tool_calls != next_parallel_tool_calls
            || session.runtime.execution.agent_run != next_run;
        session.runtime.execution.selection.agent = Some(profile.name.clone());
        session.runtime.execution.agent_mode = Some(profile.frontmatter.mode);
        session.runtime.execution.agent_hidden = profile.frontmatter.hidden;
        session.runtime.execution.agent_color = profile.frontmatter.color.clone();
        session.runtime.execution.system_prompt_override = Some(next_system);
        session.runtime.set_allowed_tools(next_allowed_tools);
        session.runtime.execution.agent_permission = next_permission;
        session.runtime.execution.agent_run = next_run.clone();
        session.runtime.set_model_override(
            Some(next_model_provider_id.clone()),
            next_model_adapter_id.clone(),
            Some(next_model_id.clone()),
        );
        session.runtime.set_model_mode_overrides(
            next_thinking_mode.clone(),
            next_speed_mode.clone(),
            next_verbosity.clone(),
            next_parallel_tool_calls,
        );
        options.model = next_model;
        options.thinking_mode = next_thinking_mode;
        options.speed_mode = next_speed_mode;
        options.verbosity = next_verbosity;
        options.system = session.runtime.execution.system_prompt_override.clone();
        if options.temperature.is_none() {
            options.temperature = next_run.temperature.map(|value| value.0);
        }
        if options.max_output_tokens.is_none() {
            options.max_output_tokens = next_run.max_output_tokens;
        }
        if options.max_turn_loops.is_none() {
            options.max_turn_loops = next_run.steps;
        }
        if !changed {
            return Ok(session);
        }
        self.persist_session_changes(session, Vec::new(), Vec::new(), None, state)
            .await
    }

    fn resolve_root_agent_model(
        &self,
        _session: &Session,
        options: &SessionRunOptions,
        state: &SessionManagerState,
        requested_default: Option<&crate::agents::AgentDefaultModelConfig>,
    ) -> Result<ModelRef, AppError> {
        let base_model = options.model.clone();
        match requested_default.filter(|value| !value.is_empty()) {
            Some(default_config) => self.resolve_agent_default_model_ref(
                state.processor.provider_registry(),
                &base_model,
                default_config,
            ),
            None => Ok(base_model),
        }
    }

    fn resolve_agent_default_model_ref(
        &self,
        provider_registry: &crate::provider::ProviderRegistry,
        base_model: &ModelRef,
        requested_default: &crate::agents::AgentDefaultModelConfig,
    ) -> Result<ModelRef, AppError> {
        if requested_default.is_empty() {
            return Ok(base_model.clone());
        }
        let requested_provider = requested_default
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let requested_adapter = requested_default
            .adapter
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let requested_model = requested_default
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let provider_changed =
            requested_provider.is_some_and(|provider| provider != base_model.provider_id.as_str());
        let provider_id = requested_provider.unwrap_or(base_model.provider_id.as_str());
        let base_adapter = (!provider_changed)
            .then(|| base_model.adapter_id.as_ref().map(|value| value.as_str()))
            .flatten();
        let adapter_id = requested_adapter.or(base_adapter);
        let base_model_id = (!provider_changed && requested_adapter.is_none())
            .then(|| base_model.model_id.as_str());
        let model_id = requested_model.or(base_model_id);
        provider_registry.resolve_model_selection(provider_id, adapter_id, model_id)
    }

    pub(super) fn apply_tool_success_execution_context(
        &self,
        session: &mut Session,
        invocation: &ToolInvocation,
        execution: &ToolInvocationExecution,
    ) {
        let payload_tool_name = payload_tool_name_for_invocation(invocation);
        if let Some(output) = crate::tool::ToolPayloadOutput::from_tool_output(
            payload_tool_name.as_str(),
            &execution.output,
        ) {
            match output {
                crate::tool::ToolPayloadOutput::EnterWorktree { path, .. } => {
                    session
                        .runtime
                        .set_effective_workspace_root(Some(PathBuf::from(path)));
                    return;
                }
                crate::tool::ToolPayloadOutput::ExitWorktree { .. } => {
                    session.runtime.set_effective_workspace_root(None);
                    return;
                }
                _ => {}
            }
        }

        match execution
            .view
            .metadata
            .get("agena.effect")
            .map(String::as_str)
        {
            Some("enter_worktree") => {
                if let Some(path) = custom_payload_value(&execution.output)
                    .and_then(|value| value.get("path").cloned())
                    .and_then(|value| value.as_str().map(str::to_string))
                {
                    session
                        .runtime
                        .set_effective_workspace_root(Some(PathBuf::from(path)));
                }
            }
            Some("exit_worktree") => {
                session.runtime.set_effective_workspace_root(None);
            }
            _ => {}
        }
    }

    async fn pause_active_goal_if_needed(
        &self,
        session_id: i64,
        state: Arc<SessionManagerState>,
    ) -> Result<Option<Session>, AppError> {
        let Some(updated) = self
            .store
            .pause_goal_if_active(session_id, state.cache_policy())
            .await?
        else {
            return Ok(None);
        };
        let goal = updated.goal.as_ref().ok_or_else(|| {
            AppError::Internal(format!("goal missing after pause for session {session_id}"))
        })?;
        self.publish_goal_event(goal, session_id).await?;
        Ok(Some(updated))
    }

    pub(super) async fn resume_paused_goal_if_needed(
        &self,
        session: Session,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        if !session
            .goal
            .as_ref()
            .is_some_and(|goal| goal.status == GoalStatus::Paused)
        {
            return Ok(session);
        }

        let session_id = session.id;
        let Some(updated) = self
            .store
            .resume_goal_if_paused(session_id, state.cache_policy())
            .await?
        else {
            return Ok(session);
        };
        let goal = updated.goal.as_ref().ok_or_else(|| {
            AppError::Internal(format!(
                "goal missing after resume for session {session_id}"
            ))
        })?;
        self.publish_goal_event(goal, session_id).await?;
        Ok(updated)
    }

    async fn persist_run_failed_event(
        &self,
        session_id: i64,
        reason: String,
        state: Arc<SessionManagerState>,
    ) -> Result<(), AppError> {
        let event = EventKind::RunFailed(RunFailedEvent {
            session_id,
            error: ErrorInfo {
                code: "session_run_failed".to_string(),
                message: reason,
            },
            ts_ms: Utc::now().timestamp_millis(),
        });
        let session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        let _ = self
            .persist_session_changes(session, Vec::new(), vec![event], None, state)
            .await?;
        Ok(())
    }

    pub(super) async fn find_child_session_for_task(
        &self,
        parent_session_id: i64,
        task_id: Option<&str>,
    ) -> Result<Option<Session>, AppError> {
        let Some(task_id) = task_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let summaries = self
            .list_session_summaries(SessionListRequest {
                offset: 0,
                limit: None,
                include_subagents: true,
            })
            .await?;
        let state = self.execution_state();
        for child_id in summaries
            .into_iter()
            .filter(|summary| summary.parent_id == Some(parent_session_id))
            .map(|summary| summary.id)
        {
            let session = self
                .store
                .load_session(child_id, state.cache_policy())
                .await?;
            if session.runtime.execution.task_id.as_deref() == Some(task_id) {
                return Ok(Some(session));
            }
        }
        Ok(None)
    }

    pub(super) fn subtask_run_options(
        &self,
        child: &Session,
        parent: &Session,
        state: &SessionManagerState,
        requested_model: Option<&str>,
        requested_default: Option<&crate::agents::AgentDefaultModelConfig>,
    ) -> Result<SessionRunOptions, AppError> {
        let requested_model = requested_model
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let base_model = match self.model_from_session_selection(child)? {
            Some(model) => model,
            None => match self.model_from_session_selection(parent)? {
                Some(model) => model,
                None => self.default_model_from_config(state)?.ok_or_else(|| {
                    AppError::Internal(
                        "subtask requires a child, parent, or global default model before it can run"
                            .to_string(),
                    )
                })?,
            },
        };
        let model = if let Some(model_id) = requested_model {
            self.resolve_requested_session_model_ref(&base_model, model_id)?
        } else if let Some(default_config) = requested_default.filter(|value| !value.is_empty()) {
            self.resolve_agent_default_model_ref(
                state.processor.provider_registry(),
                &base_model,
                default_config,
            )?
        } else {
            base_model
        };
        Ok(SessionRunOptions {
            model,
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: child.runtime.execution.system_prompt_override.clone(),
            temperature: child
                .runtime
                .execution
                .agent_run
                .temperature
                .map(|value| value.0),
            max_output_tokens: child.runtime.execution.agent_run.max_output_tokens,
            agent_profile: child.runtime.execution.selection.agent.clone(),
            max_turn_loops: child.runtime.execution.agent_run.steps,
        })
    }

    fn resolve_requested_session_model_ref(
        &self,
        base_model: &ModelRef,
        requested_model: &str,
    ) -> Result<ModelRef, AppError> {
        let requested_model = requested_model.trim();
        if requested_model.is_empty() {
            return Ok(base_model.clone());
        }

        if requested_model.matches('/').count() >= 2
            && let Some((provider_id, model_id)) = requested_model.split_once('/')
        {
            return ModelRef::try_new(provider_id, model_id).map_err(|error| {
                AppError::Config(format!(
                    "invalid requested model reference `{requested_model}`: {error}"
                ))
            });
        }

        let mut model = ModelRef::new(
            base_model.provider_id.to_string(),
            requested_model.to_string(),
        );
        model.adapter_id = base_model.adapter_id.clone();
        Ok(model)
    }

    /// Drain every pending steer message (non-blocking) and append each as
    /// a User message before the next model turn. A user steer becomes the
    /// next input the model sees.
    async fn drain_steer_input(
        &self,
        mut session: Session,
        steer_rx: &mut mpsc::UnboundedReceiver<Vec<PartContent>>,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        loop {
            let parts = match steer_rx.try_recv() {
                Ok(parts) => parts,
                Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                    return Ok(session);
                }
            };
            let ids = self.store.reserve_message_ids(parts.len()).await?;
            let user_message = build_message(
                ids,
                Role::User,
                MessageStatus::Completed,
                parts,
                MessageMetadata {
                    source: MessageSource::User,
                    parent_message_id: session.last_conversation_message().map(|m| m.id),
                    generated_by_call_id: None,
                    model_provider_id: options.model.provider_id.to_string(),
                    model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
                    model_id: options.model.model_id.to_string(),
                    model_thinking_mode: options.thinking_mode.clone(),
                    model_speed_mode: options.speed_mode.clone(),
                    model_verbosity: options.verbosity.clone(),
                    model_parallel_tool_calls: options.request_override.parallel_tool_calls(),
                    provider_metadata: None,
                    tags: Vec::new(),
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
        }
    }

    fn execute_pending_tool(
        &self,
        state: &SessionManagerState,
        session_id: i64,
        pending_tool: &ResolvedPendingTool,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let scoped_executor = state
            .tool_executor
            .for_session_context(&pending_tool.session_runtime.execution);
        scoped_executor.execute_invocation_detailed_with_prepared_shell(
            &pending_tool.invocation,
            session_id,
            pending_tool.call_id,
            pending_tool.prepared_shell_command.clone(),
        )
    }

    fn execute_pending_tool_after_approval(
        &self,
        state: &SessionManagerState,
        session_id: i64,
        pending_tool: &ResolvedPendingTool,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let scoped_executor = state
            .tool_executor
            .for_session_context(&pending_tool.session_runtime.execution);
        scoped_executor.execute_invocation_detailed_bypassing_permissions_with_prepared_shell(
            &pending_tool.invocation,
            session_id,
            pending_tool.call_id,
            pending_tool.prepared_shell_command.clone(),
        )
    }

    pub(super) fn execution_state(&self) -> Arc<SessionManagerState> {
        self.execution.load_full()
    }
}

fn join_runtime_context_lines(lines: &[String]) -> String {
    lines
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}
