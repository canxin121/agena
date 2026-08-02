use super::{
    AggregatedPermissionOutcome, AggregatedPermissionRequest, AppError, Arc, EventKind,
    ExecutionControl, ExecutionStatus, InteractiveRequestPart, MessageCheckpoint, MessageMetadata,
    MessageSource, OperationPart, PartContent, PersistedPermissionRule, PromptRequestOptions,
    PromptTurnBudget, ProviderPromptAnchor, RequestPart, ResolvedPendingTool, RunAborted,
    RunCompleted, RunStarted, SessionManager, SessionManagerState, SessionPendingTool,
    SessionRunOptions, SessionRunRequest, SessionRunTermination, StreamingToolExecution, ToolError,
    ToolInvocationExecution, ToolPermissionCheck, Utc, append_resolved_message_part,
    ask_user_title, assistant_message_for_part, build_message, build_request_part,
    completed_lifecycle, execution_control_to_app_error, is_authorization_phase_title,
    max_permission_risk, operation_authorization, operation_blocks_from_tool_output,
    pending_operation_for_resolved, pending_tool_part_not_found_error, permission_action_key,
    permission_scope_label, push_unique_permission_action, resolve_pending_tool,
    responses_api_request_metadata, run_abort_reason, should_execute_pending_tools_concurrently,
    terminal_operation_title, tool_name, update_resolved_tool_message,
};
use crate::session::Session;
use crate::session::prompt_window;
use agena_domain::UserInputRequest;
use agena_domain::{
    DecisionTraceStep, ExecutionPhase, ExecutionSource, FinishReason, PermissionAction,
    PermissionDecision, PermissionRequest, PermissionRequestedEvent, PermissionRiskLevel,
    PermissionScope, PolicySourceKind, Role, RunAbortReason,
};
use tracing::Instrument;

use super::super::StableRunContext;

/// True for tools whose operation is scoped to concrete paths (filesystem
/// read/write tools such as `fs.write` / `fs.apply_patch`). Arbitrary
/// execution tools (shell/process) are never path-scoped even when they
/// declare filesystem effects, because their declared paths are derived
/// from free-form input, not authoritative: a user who allows writes inside
/// the workspace has not authorized arbitrary command execution.
fn is_path_scoped_tool(tags: &[String]) -> bool {
    let path_scoped = tags
        .iter()
        .any(|tag| tag == "filesystem_read" || tag == "filesystem_write");
    let arbitrary_execution = tags.iter().any(|tag| tag == "shell" || tag == "process");
    path_scoped && !arbitrary_execution
}

/// One member of a provider-emitted tool batch after preflight.
///
/// Preflight deliberately separates permission discovery from execution. A
/// batch must publish *every* independently actionable permission request
/// before the session enters its blocked state; discovering only the first
/// request turns a parallel model response into an accidental serial queue.
enum PendingToolBatchMember {
    Ready(ResolvedPendingTool),
    AwaitingPermission {
        resolved: ResolvedPendingTool,
        request: Box<AggregatedPermissionRequest>,
    },
    Sequential(SessionPendingTool),
}

/// Fully prepared sequential invocation plus the authorization checks that
/// must be resolved before it may execute.
///
/// Synchronous preflight lives outside the async resolver so the compiler
/// does not embed every preparation error path and terminalization future in
/// one multi-megabyte poll frame.
struct PreparedPendingToolExecution {
    resolved: ResolvedPendingTool,
    permission_checks: Vec<ToolPermissionCheck>,
    session_changed: bool,
}

enum PendingToolPreparationError {
    Session(AppError),
    Tool(ToolError),
}

impl From<ToolError> for PendingToolPreparationError {
    fn from(error: ToolError) -> Self {
        Self::Tool(error)
    }
}

impl SessionManager {
    /// Build a per-operation sink for process lifecycle/output events. Shell
    /// execution is synchronous and may run on a blocking worker, so delivery
    /// is handed back to Tokio immediately and never blocks the pipe reader.
    pub(in crate::session::manager) fn command_event_sink_for_pending(
        &self,
        session_id: i64,
        pending: &ResolvedPendingTool,
    ) -> agena_tool::ToolRuntimeEventSink {
        let publisher = Arc::clone(&self.publisher);
        let handle = tokio::runtime::Handle::current();
        let message_id = pending.pending.part.message_id;
        let part_id = pending.pending.part.part_id;
        let call_id = pending.call_id;
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<EventKind>();
        handle.spawn(async move {
            while let Some(kind) = event_rx.recv().await {
                if let Err(error) = publisher
                    .publish(crate::event::PublishContext::for_session(session_id), kind)
                    .await
                {
                    tracing::debug!(
                        target: "agena::session::command_events",
                        session_id,
                        error = %error,
                        "failed to publish live command event"
                    );
                }
            }
        });

        Arc::new(move |event| {
            let kind = match event {
                agena_tool::ToolRuntimeEvent::CommandBegin(mut event) => {
                    event.context.session_id = session_id;
                    event.context.call_id = call_id;
                    event.context.message_id = Some(message_id);
                    event.context.part_id = Some(part_id);
                    EventKind::CommandBegin(event)
                }
                agena_tool::ToolRuntimeEvent::CommandOutputDelta(mut event) => {
                    event.context.session_id = session_id;
                    event.context.call_id = call_id;
                    event.context.message_id = Some(message_id);
                    event.context.part_id = Some(part_id);
                    EventKind::CommandOutputDelta(event)
                }
                agena_tool::ToolRuntimeEvent::CommandEnd(mut event) => {
                    event.context.session_id = session_id;
                    event.context.call_id = call_id;
                    event.context.message_id = Some(message_id);
                    event.context.part_id = Some(part_id);
                    EventKind::CommandEnd(event)
                }
            };
            // A single queue preserves begin → delta → end ordering even
            // though the shell itself runs on a blocking worker. Spawning one
            // publisher task per chunk allowed Tokio scheduling to reorder
            // stdout deltas and made the browser occasionally drop output.
            let _ = event_tx.send(kind);
        })
    }

    pub(in crate::session::manager) fn command_event_sink_for_pending_if_needed(
        &self,
        session_id: i64,
        pending: &ResolvedPendingTool,
    ) -> Option<agena_tool::ToolRuntimeEventSink> {
        let command_capable = pending.prepared_shell_command.is_some()
            || matches!(
                pending.invocation.name.as_str(),
                "shell"
                    | "shell.run"
                    | "agena.shell.run"
                    | "powershell"
                    | "powershell.run"
                    | "agena.powershell.run"
                    | "process"
                    | "process.run"
                    | "agena.process.run"
            );
        command_capable.then(|| self.command_event_sink_for_pending(session_id, pending))
    }

    pub(in crate::session::manager) async fn run_until_stable(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        context: StableRunContext,
    ) -> Result<Session, AppError> {
        let StableRunContext {
            allow_goal_continuation,
            base_run_source,
            mut active_model_turn_id,
            state,
            control,
            mut steer_rx,
            usage_budget,
        } = context;
        let _ = allow_goal_continuation;
        let mut reactive_compaction_attempted = false;
        let mut force_model_retry = false;
        // Provider continuation is an execution-local decision. It is never
        // reconstructed from "some tool part is terminal" after a restart.
        // This flag becomes true only at command entry, after the entire
        // pending tool batch reaches a barrier, or after new steer input.
        let mut model_requested = true;
        let mut observed_user_message_id = session
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::User)
            .map(|message| message.id);
        loop {
            let current_options =
                self.apply_execution_context_to_run_options(&session, options.clone())?;
            if control.cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }

            session = self
                .drain_steer_input(session, &mut steer_rx, &current_options, state.clone())
                .await?;

            let latest_user = session
                .messages
                .iter()
                .rev()
                .find(|message| message.role == Role::User);
            if latest_user.map(|message| message.id) != observed_user_message_id {
                active_model_turn_id = latest_user
                    .and_then(|message| message.metadata.model_turn_id)
                    .or_else(|| latest_user.map(|message| message.id));
                observed_user_message_id = latest_user.map(|message| message.id);
                model_requested = true;
            }

            let mut current_options =
                self.apply_execution_context_to_run_options(&session, options.clone())?;
            if let Some(budget) = usage_budget.as_ref() {
                let aggregate_usage = session.aggregate_usage();
                if let Some(message) = budget.prevents_next_model_turn(&aggregate_usage) {
                    return Err(AppError::SubtaskBudgetExceeded(message));
                }
                budget.cap_output_tokens(&aggregate_usage, &mut current_options);
            }
            let interaction_epoch = control.interaction_epoch();
            session.refresh_derived();
            if session.blocked() {
                control
                    .transition(ExecutionPhase::AwaitingInteraction)
                    .await
                    .map_err(execution_control_to_app_error)?;

                // Re-read after observing the signal generation. This closes
                // the race where the final reply commits between our stale
                // in-memory blocked check and installation of the waiter.
                session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                session.refresh_derived();
                if !session.blocked() {
                    continue;
                }

                tokio::select! {
                    biased;
                    _ = control.cancel.cancelled() => return Err(AppError::Cancelled),
                    _ = control.wait_for_interaction_after(interaction_epoch) => {}
                }
                session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                continue;
            }

            if let Some(hit) = crate::session::doom_loop::detect(
                session.messages.as_slice(),
                agena_domain::DoomLoopPolicy::default(),
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
                    .transition(ExecutionPhase::ExecutingTools)
                    .await
                    .map_err(execution_control_to_app_error)?;
                // Tool resolution is itself a large async state machine. A
                // heap boundary here prevents its branch storage from being
                // embedded in the provider-loop future and overflowing a
                // normal Tokio worker stack when a permission continuation
                // reaches the tool batch immediately.
                session = Box::pin(self.resolve_pending_tools(
                    session,
                    pending_tools,
                    &current_options,
                    state.clone(),
                ))
                .await?;
                session.refresh_derived();
                model_requested = !session.blocked() && session.pending_tools().is_empty();
                continue;
            }

            if !model_requested && !force_model_retry {
                let last_assistant_text = session
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == agena_domain::Role::Assistant)
                    .map(|m| m.as_text_lossy());
                let stop_input = agena_plugin_host::AgentStopInput {
                    session_id: session.id,
                    stop_hook_active: false,
                    last_assistant_message: last_assistant_text,
                };
                match state
                    .tool_executor
                    .plugin_manager()
                    .dispatch_agent_stop_cancellable(stop_input, Some(control.cancel.clone()))
                    .await
                {
                    Ok(patch) if patch.continue_with_message.is_some() => {
                        let follow_up = patch.continue_with_message.unwrap_or_default();
                        let ids = self.store.reserve_message_ids(1).await?;
                        let follow_up_turn_id = ids.message_id;
                        let user_message = build_message(
                            ids,
                            Role::User,
                            ExecutionStatus::Completed,
                            vec![PartContent::text(follow_up)],
                            MessageMetadata {
                                source: MessageSource::System,
                                idempotency_key: None,
                                model_turn_id: Some(follow_up_turn_id),
                                parent_message_id: session
                                    .last_conversation_message()
                                    .map(|m| m.id),
                                generated_by_call_id: None,
                                externally_initiated_tool: false,
                                model_provider_id: current_options.model.provider_id.to_string(),
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
                        let checkpoint = MessageCheckpoint::all(&user_message);
                        session = self
                            .persist_session_changes(
                                session,
                                vec![checkpoint],
                                Vec::new(),
                                None,
                                state.clone(),
                            )
                            .await?;
                        model_requested = true;
                        continue;
                    }
                    Ok(_) => return Ok(session),
                    Err(err) => {
                        if control.cancel.is_cancelled() {
                            return Err(AppError::Cancelled);
                        }
                        tracing::warn!(
                            target: "agena_plugin_host::agent_stop",
                            "agent.stop hook failed (stopping normally): {err}"
                        );
                        return Ok(session);
                    }
                }
            }
            // Reaching this point consumes the one-shot retry authorization.
            // It must not survive a successful model turn and trigger another
            // model call after the session becomes quiescent.
            force_model_retry = false;

            control
                .transition(ExecutionPhase::PreparingModel)
                .await
                .map_err(execution_control_to_app_error)?;

            let last_message_id = session
                .last_conversation_message()
                .map(|message| message.id);
            let already_auto_compacted_at_boundary = session
                .runtime
                .prompt_window
                .compaction
                .as_ref()
                .map(|compaction| compaction.compacted_through_message_id)
                == last_message_id;
            let session_usage = self.session_usage(&session)?;
            if state.config.auto_compaction.enabled
                && !session.runtime.prompt_window.auto_compaction_disabled
                && !already_auto_compacted_at_boundary
                && session_usage.limit_basis.is_some()
                && let Some(limit_tokens) = session_usage.limit_tokens
                && session_usage.current_tokens >= limit_tokens
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
            let pre_run_input = agena_plugin_host::PreRunInput {
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
                active_model_turn_id,
                state.clone(),
                control.clone(),
            ))
            .await
            {
                Ok(next_session) => {
                    session = next_session;
                    model_requested = false;
                    if active_model_turn_id.is_none() {
                        active_model_turn_id = session
                            .messages
                            .iter()
                            .rev()
                            .find(|message| message.role == Role::Assistant)
                            .and_then(|message| message.metadata.model_turn_id);
                    }
                    reactive_compaction_attempted = false;
                    let post_run_input = agena_plugin_host::PostRunInput {
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
                    let post_run_input = agena_plugin_host::PostRunInput {
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
                    if state.config.auto_compaction.enabled
                        && !reactive_compaction_attempted
                        && err.provider_error_kind()
                            == Some(agena_provider::ProviderErrorKind::ContextOverflow)
                    {
                        reactive_compaction_attempted = true;
                        let reloaded = self
                            .store
                            .load_session(session_id, state.cache_policy())
                            .await?;
                        let generation = reloaded.runtime.prompt_window.generation;
                        let compacted = Box::pin(self.reactive_compact_session(
                            reloaded,
                            &current_options,
                            state.clone(),
                            control.clone(),
                        ))
                        .await?;
                        if compacted.runtime.prompt_window.generation > generation {
                            tracing::info!(
                                target: "agena::session::compact",
                                session_id,
                                "provider context overflow recovered by reactive compaction; retrying once"
                            );
                            session = compacted;
                            force_model_retry = true;
                            continue;
                        }
                    }
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
        model_turn_id: Option<i64>,
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
            let provider_registry = state.processor.provider_registry();
            let native_compaction_enabled =
                provider_registry.native_compaction_enabled(&options.model)?;
            let active_messages = prompt_window::active_prompt_messages_for_model(
                &session,
                Some(options.model.provider_id.as_ref()),
                options.model.adapter_id.as_ref().map(AsRef::as_ref),
                Some(options.model.model_id.as_ref()),
                native_compaction_enabled,
            );
            let scoped_executor = state
                .tool_executor
                .for_session_context(&session.runtime.execution);
            let agena_tool_mode = provider_registry.agena_tool_mode(&options.model)?;
            let tool_api_functions = if agena_tool_mode.is_disabled() {
                Vec::new()
            } else {
                scoped_executor.available_tool_api_bindings()
            };
            let request_tool_api_functions = tool_api_functions.clone();
            let request_system = options.system.clone();
            let prompt_budget = self.prompt_budget_for_run(
                &session,
                options,
                request_system.as_deref(),
                tool_api_functions.as_slice(),
                state.as_ref(),
            );
            let provider_request_shape = state.processor.prompt_cache_shape(&options.model)?;
            let continuation_supported =
                state.processor.supports_prompt_continuation(&options.model);
            let prompt_request_options = PromptRequestOptions {
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
                .map(agena_provider::PromptCacheShape::fingerprint);
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

            let processor_ids = self.store.reserve_processor_ids().await?;
            let run_id = agena_domain::RunId::new();
            let turn_started_at_unix_ms = Utc::now().timestamp_millis();
            let mut completion = super::super::completion_request(
                options,
                prepared.system.clone(),
                prepared.messages.clone(),
                tool_api_functions,
                Some(prepared.prompt_cache_key.clone()),
                prepared.previous_response_id.clone(),
                Some(prepared.prompt_window_generation),
            );
            completion.provider_compaction = prepared.provider_compaction.clone();
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
                turn_id: control.turn_id(),
                reply_id: control.reply_id(),
                session_id: session.id,
                model_turn_id,
                completion_parent_message_id: session
                    .last_conversation_message()
                    .map(|message| message.id),
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
                .transition(ExecutionPhase::StreamingModel)
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
                            prompt_window::active_prompt_messages_for_model(
                                &session,
                                Some(options.model.provider_id.as_ref()),
                                options.model.adapter_id.as_ref().map(AsRef::as_ref),
                                Some(options.model.model_id.as_ref()),
                                native_compaction_enabled,
                            );
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
                        adapter_id: options.model.adapter_id.as_ref().map(AsRef::as_ref),
                        model_id: options.model.model_id.as_ref(),
                        system: options.system.as_deref(),
                        temperature: options.temperature,
                        max_output_tokens: options.max_output_tokens,
                        tool_api_functions: request_tool_api_functions.as_slice(),
                        provider_request_shape: anchored_provider_request_shape.as_ref(),
                        continuation_supported,
                        native_compaction_enabled,
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
                    drop(request_tool_api_functions);
                    drop(prepared);

                    let client_events = result.client_events;
                    session.messages.push(assistant_message.clone());
                    // Tool Operations already publish their authoritative
                    // invocation/result checkpoints from the processor. Text
                    // and reasoning stream live updates ephemerally, so only
                    // those parts need a final durable snapshot here.
                    let final_part_ids = assistant_message
                        .parts
                        .iter()
                        .filter(|part| {
                            !matches!(
                                part.content.as_ref(),
                                Some(PartContent::Activity(
                                    crate::message::RuntimeActivity::Operation(_)
                                ))
                            )
                        })
                        .map(|part| part.id)
                        .collect::<Vec<_>>();
                    let checkpoints = (!final_part_ids.is_empty())
                        .then(|| MessageCheckpoint::parts(assistant_message.id, final_part_ids));
                    let mut persisted_session = self
                        .persist_session_changes(
                            session,
                            checkpoints.into_iter().collect(),
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
                                failure: None,
                            }));
                        }
                        SessionRunTermination::Failed(error) => {
                            let failure = error.failure();
                            tracing::warn!(
                                failure_id = %failure.id,
                                session_id = persisted_session.id,
                                diagnostic = %error,
                                "session run failed"
                            );
                            run_events.push(EventKind::RunAborted(RunAborted {
                                run_id,
                                reason: RunAbortReason::ProviderError,
                                failure: Some((&failure).into()),
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
                    let failure = err.failure();
                    tracing::warn!(
                        failure_id = %failure.id,
                        session_id = session.id,
                        diagnostic = %err,
                        "session run aborted by an execution failure"
                    );
                    self.store
                        .append_history_items(
                            session,
                            vec![EventKind::RunAborted(RunAborted {
                                run_id,
                                reason,
                                failure: Some((&failure).into()),
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
        tool_api_functions: &[crate::tool::ToolApiBinding],
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
            metadata.limits.max_input_tokens,
            options
                .max_output_tokens
                .or(metadata.limits.max_output_tokens),
            fallback_budget,
            system,
            tool_api_functions,
        );

        PromptTurnBudget {
            max_prompt_chars,
            max_prompt_tokens: agena_runtime::estimate_prompt_tokens_from_chars(max_prompt_chars),
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
        // Protocol `tools_call` invocations are safe to fan out too.
        // Host-side interactive requests serialize only their short
        // load-and-persist section, keyed by call id, rather than serializing
        // complete tool executions around user approval.
        if !should_execute_pending_tools_concurrently(&options.request_override) {
            if let Some(tool) = session.next_pending_tool() {
                return Box::pin(self.resolve_pending_tool(session, tool, state)).await;
            }
            return Ok(session);
        }

        let mut ready_tools = Vec::new();
        let mut permission_requests = Vec::new();
        let mut sequential_tools = Vec::new();
        for pending_tool in pending_tools {
            match Box::pin(self.prepare_pending_tool_batch_member(
                &mut session,
                &pending_tool,
                state.as_ref(),
            ))
            .await?
            {
                PendingToolBatchMember::Ready(resolved) => ready_tools.push(resolved),
                PendingToolBatchMember::AwaitingPermission { resolved, request } => {
                    permission_requests.push((resolved, request));
                }
                PendingToolBatchMember::Sequential(pending) => sequential_tools.push(pending),
            }
        }

        // Operation-owned authorization requests are durable batch members,
        // not a side effect of whichever tool happens to be first in
        // transcript order. Persist every request before waiting so the UI
        // can present all approvals at once and each reply addresses its exact
        // Operation.
        for (resolved, request) in permission_requests {
            session = Box::pin(self.apply_permission_request(
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
                state.clone(),
            ))
            .await?;
        }

        // The parallel worker path intentionally handles only detailed,
        // non-streaming executions. Keep a singleton on the sequential path
        // so streaming tool output continues to checkpoint through its normal
        // lifecycle; a batch needs at least two ready members to fan out.
        if ready_tools.len() == 1 {
            sequential_tools.push(
                ready_tools
                    .pop()
                    .expect("checked singleton ready tool")
                    .pending,
            );
        } else if !ready_tools.is_empty() {
            let executions = Box::pin(self.execute_pending_tools_concurrently(
                state.clone(),
                session.id,
                ready_tools.clone(),
            ))
            .await?;
            // A concurrent gateway tool can persist nested permission/input
            // request parts while its execution is suspended. Merge results
            // into the latest projection so completing outer calls cannot
            // replace those request/reply records with another snapshot.
            session = self
                .store
                .load_session(session.id, state.cache_policy())
                .await?;
            for (resolved, result) in ready_tools.into_iter().zip(executions) {
                session = Box::pin(self.apply_tool_execution_result(
                    session,
                    &resolved.pending,
                    result,
                    state.clone(),
                ))
                .await?;
            }
        }

        // A non-concurrency-safe invocation still executes in transcript
        // order. It is reached only after all Ask outcomes above are already
        // visible, so it can never hide a later permission behind a slow
        // earlier call.
        if !session.blocked() {
            for pending_tool in sequential_tools {
                if session
                    .part(&pending_tool.part)
                    .is_some_and(|part| part.status == ExecutionStatus::Pending)
                {
                    session =
                        Box::pin(self.resolve_pending_tool(session, pending_tool, state.clone()))
                            .await?;
                }
            }
        }

        Ok(session)
    }

    async fn prepare_pending_tool_batch_member(
        &self,
        session: &mut Session,
        pending_tool: &SessionPendingTool,
        state: &SessionManagerState,
    ) -> Result<PendingToolBatchMember, AppError> {
        self.refresh_execution_policy(session, state);
        let before_prepare = session.clone();
        let mut resolved = resolve_pending_tool(session, pending_tool)?;
        let cancellation = self.execution_registry.cancellation_token(session.id).await;
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution)
            .with_cancellation_token(cancellation);
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
            return Ok(PendingToolBatchMember::Sequential(pending_tool.clone()));
        }
        let prepared = match scoped_executor.prepare_invocation(
            &resolved.invocation,
            session.id,
            resolved.call_id,
        ) {
            Ok(prepared) => prepared,
            Err(err) => {
                if matches!(&err, ToolError::Cancelled) {
                    return Err(AppError::Cancelled);
                }
                tracing::debug!(
                    target: "agena::session::tools",
                    session_id = session.id,
                    call_id = resolved.call_id,
                    error = %err,
                    "deferring tool preparation error to sequential failure handling"
                );
                *session = before_prepare;
                return Ok(PendingToolBatchMember::Sequential(pending_tool.clone()));
            }
        };
        let (prepared_invocation, prepared_shell_command) = match scoped_executor
            .prepare_shell_invocation(&prepared.invocation, session.id, resolved.call_id)
        {
            Ok(prepared) => prepared,
            Err(err) => {
                if matches!(&err, ToolError::Cancelled) {
                    return Err(AppError::Cancelled);
                }
                tracing::debug!(
                    target: "agena::session::tools",
                    session_id = session.id,
                    call_id = resolved.call_id,
                    error = %err,
                    "deferring shell preparation error to sequential failure handling"
                );
                *session = before_prepare;
                return Ok(PendingToolBatchMember::Sequential(pending_tool.clone()));
            }
        };
        resolved.prepared_shell_command = prepared_shell_command;
        resolved.invocation = prepared_invocation;
        if prepared.invocation != resolved.invocation || prepared.title_override.is_some() {
            let authorization = operation_authorization(session, &resolved);
            let current_title = match session
                .part(&resolved.pending.part)
                .and_then(|part| part.content.as_ref())
            {
                Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(
                    operation,
                ))) => operation.title.clone(),
                _ => format!("Tool {}", tool_name(&resolved.invocation)),
            };

            resolved.invocation = prepared.invocation.clone();
            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    resolved.pending.part.message_id, resolved.pending.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::operation(pending_operation_for_resolved(
                &resolved,
                prepared.invocation,
                prepared.title_override.unwrap_or(current_title),
                resolved.lifecycle.clone(),
                authorization,
            )));
        }

        let permission_checks = match scoped_executor
            .collect_permission_checks_for_invocation_in_session(
                &resolved.invocation,
                Some(session.id),
            ) {
            Ok(checks) => checks,
            Err(err) => {
                if matches!(&err, ToolError::Cancelled) {
                    return Err(AppError::Cancelled);
                }
                tracing::debug!(
                    target: "agena::session::tools",
                    session_id = session.id,
                    call_id = resolved.call_id,
                    error = %err,
                    "deferring permission-check error to sequential failure handling"
                );
                *session = before_prepare;
                return Ok(PendingToolBatchMember::Sequential(pending_tool.clone()));
            }
        };

        let approved_actions = session.operation_permission_approved_actions(
            resolved.pending.part.message_id,
            &resolved.operation_id,
        );
        let permission_checks = permission_checks
            .into_iter()
            .filter(|check| !approved_actions.contains(&check.action))
            .collect::<Vec<_>>();
        let permission_outcome = if permission_checks.is_empty() {
            AggregatedPermissionOutcome::Allow
        } else {
            self.aggregate_permission_outcome(Some(session), permission_checks.as_slice())
                .await?
        };

        match permission_outcome {
            AggregatedPermissionOutcome::Request(request) => {
                return Ok(PendingToolBatchMember::AwaitingPermission { resolved, request });
            }
            AggregatedPermissionOutcome::Deny(_) => {
                *session = before_prepare;
                return Ok(PendingToolBatchMember::Sequential(pending_tool.clone()));
            }
            AggregatedPermissionOutcome::Allow => {}
        }

        if !scoped_executor.is_concurrency_safe_invocation(&resolved.invocation) {
            *session = before_prepare;
            return Ok(PendingToolBatchMember::Sequential(pending_tool.clone()));
        }

        Ok(PendingToolBatchMember::Ready(resolved))
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
        let cancellation = self.execution_registry.cancellation_token(session_id).await;

        let mut handles = Vec::with_capacity(pending_tools.len());
        for pending_tool in pending_tools {
            let executor = state.tool_executor.clone();
            let command_event_sink =
                self.command_event_sink_for_pending_if_needed(session_id, &pending_tool);
            let scoped_executor = executor
                .for_session_context(&pending_tool.session_runtime.execution)
                .with_cancellation_token(cancellation.clone())
                .with_command_event_sink(command_event_sink);
            let acquire = semaphore.clone().acquire_owned();
            let permit = match cancellation.as_ref() {
                Some(cancellation) => tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Err(AppError::Cancelled),
                    permit = acquire => permit,
                },
                None => acquire.await,
            }
            .map_err(|err| AppError::Internal(format!("tool semaphore closed: {err}")))?;
            handles.push(tokio::task::spawn_blocking(move || {
                let _permit = permit;
                scoped_executor.validate_advertised_tool_identity(
                    &pending_tool.invocation,
                    pending_tool.advertised_tool_identity.as_deref(),
                )?;
                scoped_executor.execute_invocation_detailed_with_prepared_shell(
                    &pending_tool.invocation,
                    session_id,
                    pending_tool.call_id,
                    pending_tool.prepared_shell_command.clone(),
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

    fn prepare_pending_tool_execution(
        &self,
        session: &mut Session,
        mut resolved: ResolvedPendingTool,
        state: &SessionManagerState,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<PreparedPendingToolExecution, PendingToolPreparationError> {
        self.refresh_execution_policy(session, state);
        // `resolved` was created before this refresh in the sequential path.
        // Carry the live execution context forward so the later execution
        // phase cannot reintroduce the persisted stale permission snapshot.
        resolved.session_runtime = session.runtime.clone();
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution)
            .with_cancellation_token(cancellation);
        scoped_executor.validate_advertised_tool_identity(
            &resolved.invocation,
            resolved.advertised_tool_identity.as_deref(),
        )?;
        let prepared = scoped_executor.prepare_invocation(
            &resolved.invocation,
            session.id,
            resolved.call_id,
        )?;
        let (prepared_invocation, prepared_shell_command) = scoped_executor
            .prepare_shell_invocation(&prepared.invocation, session.id, resolved.call_id)?;
        resolved.prepared_shell_command = prepared_shell_command;
        resolved.invocation = prepared_invocation;

        let mut session_changed = false;
        if prepared.invocation != resolved.invocation || prepared.title_override.is_some() {
            let authorization = operation_authorization(session, &resolved);
            let current_title = match session
                .part(&resolved.pending.part)
                .and_then(|part| part.content.as_ref())
            {
                Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(
                    operation,
                ))) => operation.title.clone(),
                _ => format!("Tool {}", tool_name(&resolved.invocation)),
            };

            resolved.invocation = prepared.invocation.clone();
            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                PendingToolPreparationError::Session(AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    resolved.pending.part.message_id, resolved.pending.part.part_id
                )))
            })?;
            tool_part.set_content(PartContent::operation(pending_operation_for_resolved(
                &resolved,
                prepared.invocation,
                prepared.title_override.unwrap_or(current_title),
                resolved.lifecycle.clone(),
                authorization,
            )));
            session_changed = true;
        }

        let permission_checks = scoped_executor
            .collect_permission_checks_for_invocation_in_session(
                &resolved.invocation,
                Some(session.id),
            )?;
        Ok(PreparedPendingToolExecution {
            resolved,
            permission_checks,
            session_changed,
        })
    }

    async fn apply_pending_tool_start_error(
        &self,
        mut session: Session,
        pending: &SessionPendingTool,
        error: ToolError,
        state: Arc<SessionManagerState>,
        reload_specialized_state: bool,
    ) -> Result<Session, AppError> {
        if reload_specialized_state
            && matches!(
                &error,
                ToolError::PolicyDenied(_)
                    | ToolError::UserDeclined(_)
                    | ToolError::CapabilityUnavailable(_)
                    | ToolError::ToolUnavailable(_)
            )
        {
            session = self
                .store
                .load_session(session.id, state.cache_policy())
                .await?;
        }

        match error {
            ToolError::Cancelled => {
                Box::pin(self.apply_tool_cancellation(session, pending, state)).await
            }
            ToolError::PolicyDenied(denial) => {
                Box::pin(self.apply_tool_policy_denied(session, pending, *denial, state)).await
            }
            ToolError::UserDeclined(decline) => {
                Box::pin(self.apply_tool_user_declined(
                    session,
                    pending,
                    *decline,
                    Vec::new(),
                    state,
                ))
                .await
            }
            ToolError::CapabilityUnavailable(unavailable) => {
                Box::pin(self.apply_tool_capability_unavailable(
                    session,
                    pending,
                    *unavailable,
                    state,
                ))
                .await
            }
            ToolError::ToolUnavailable(unavailable) => {
                Box::pin(self.apply_tool_unavailable(session, pending, *unavailable, state)).await
            }
            error => Box::pin(self.apply_tool_error(session, pending, error, None, state)).await,
        }
    }

    pub(in crate::session::manager) async fn resolve_pending_tool(
        &self,
        mut session: Session,
        pending_tool: SessionPendingTool,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let cancellation = self.execution_registry.cancellation_token(session.id).await;
        let resolved = resolve_pending_tool(&session, &pending_tool)?;
        let PreparedPendingToolExecution {
            resolved,
            permission_checks,
            session_changed,
        } = match self.prepare_pending_tool_execution(
            &mut session,
            resolved,
            state.as_ref(),
            cancellation.clone(),
        ) {
            Ok(prepared) => prepared,
            Err(PendingToolPreparationError::Session(error)) => return Err(error),
            Err(PendingToolPreparationError::Tool(error)) => {
                return Box::pin(self.apply_pending_tool_start_error(
                    session,
                    &pending_tool,
                    error,
                    state,
                    false,
                ))
                .await;
            }
        };

        let approved_actions = session.operation_permission_approved_actions(
            resolved.pending.part.message_id,
            &resolved.operation_id,
        );
        let permission_checks = permission_checks
            .into_iter()
            .filter(|check| !approved_actions.contains(&check.action))
            .collect::<Vec<_>>();
        let permission_outcome = if permission_checks.is_empty() {
            AggregatedPermissionOutcome::Allow
        } else {
            Box::pin(
                self.aggregate_permission_outcome(Some(&session), permission_checks.as_slice()),
            )
            .await?
        };

        match permission_outcome {
            AggregatedPermissionOutcome::Allow => {}
            AggregatedPermissionOutcome::Request(request) => {
                let request = *request;
                return Box::pin(self.apply_permission_request(
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
                ))
                .await;
            }

            AggregatedPermissionOutcome::Deny(denial) => {
                return Box::pin(self.apply_tool_policy_denied(
                    session,
                    &resolved.pending,
                    *denial,
                    state,
                ))
                .await;
            }
        }

        if session_changed {
            session = Box::pin(self.persist_session_changes(
                session,
                vec![MessageCheckpoint::part(
                    resolved.pending.part.message_id,
                    resolved.pending.part.part_id,
                )],
                Vec::new(),
                None,
                state.clone(),
            ))
            .await?;
        }

        let streaming_tool = match Box::pin(
            state
                .tool_executor
                .for_session_context(&session.runtime.execution)
                .with_cancellation_token(cancellation.clone())
                .execute_invocation_streaming(&resolved.invocation, session.id, resolved.call_id),
        )
        .await
        {
            Ok(stream) => stream,
            Err(error) => {
                return Box::pin(self.apply_pending_tool_start_error(
                    session,
                    &resolved.pending,
                    error,
                    state,
                    true,
                ))
                .await;
            }
        };

        if let Some(stream) = streaming_tool {
            return Box::pin(self.apply_streaming_tool_execution(
                session,
                &resolved.pending,
                stream,
                state,
                cancellation,
            ))
            .await;
        }

        let manager = self.background_handle();
        let execution_state = state.clone();
        let execution_resolved = resolved.clone();
        let execution_session_id = session.id;
        let execution = tokio::task::spawn_blocking(move || {
            manager.execute_pending_tool(
                execution_state.as_ref(),
                execution_session_id,
                &execution_resolved,
                cancellation,
            )
        })
        .await
        .map_err(|err| AppError::Internal(format!("tool execution task failed: {err}")))?;

        let session = self
            .store
            .load_session(session.id, state.cache_policy())
            .await?;
        Box::pin(self.apply_tool_execution_result(session, &resolved.pending, execution, state))
            .await
    }

    /// Apply the one canonical state transition for a completed tool attempt.
    /// Every execution path (sequential, parallel, and permission-resumed)
    /// must pass through this method so an error can never leave the original
    /// operation pending and accidentally request permission a second time.
    pub(in crate::session::manager) async fn apply_tool_execution_result(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        execution: Result<ToolInvocationExecution, ToolError>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        match execution {
            Ok(execution) => {
                Box::pin(self.apply_tool_success(session, pending_tool, execution, None, state))
                    .await
            }
            Err(ToolError::UserInputRequired(input)) => {
                Box::pin(self.apply_user_input_request(session, pending_tool, *input, state)).await
            }
            Err(ToolError::Cancelled) => {
                Box::pin(self.apply_tool_cancellation(session, pending_tool, state)).await
            }
            Err(ToolError::PolicyDenied(denial)) => {
                Box::pin(self.apply_tool_policy_denied(session, pending_tool, *denial, state)).await
            }
            Err(ToolError::UserDeclined(decline)) => {
                Box::pin(self.apply_tool_user_declined(
                    session,
                    pending_tool,
                    *decline,
                    Vec::new(),
                    state,
                ))
                .await
            }
            Err(ToolError::CapabilityUnavailable(unavailable)) => {
                Box::pin(self.apply_tool_capability_unavailable(
                    session,
                    pending_tool,
                    *unavailable,
                    state,
                ))
                .await
            }
            Err(ToolError::ToolUnavailable(unavailable)) => {
                Box::pin(self.apply_tool_unavailable(session, pending_tool, *unavailable, state))
                    .await
            }
            Err(error) => {
                Box::pin(self.apply_tool_error(session, pending_tool, error, None, state)).await
            }
        }
    }

    pub async fn resolve_tool_permission_check(
        &self,
        session_id: Option<i64>,
        check: &ToolPermissionCheck,
    ) -> Result<agena_domain::PermissionResolution, AppError> {
        self.resolve_permission_decision(session_id, check).await
    }

    pub(in crate::session::manager) async fn resolve_permission_decision(
        &self,
        session_id: Option<i64>,
        check: &ToolPermissionCheck,
    ) -> Result<agena_domain::PermissionResolution, AppError> {
        let cancellation = match session_id {
            Some(session_id) => self.execution_registry.cancellation_token(session_id).await,
            None => None,
        };
        if cancellation
            .as_ref()
            .is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
        {
            return Err(AppError::Cancelled);
        }
        let key = permission_action_key(&check.action)?;
        let state = self.execution_state();
        let snapshot = self.rule_snapshot(&state, session_id).await?;
        let resolution =
            agena_permission::rules::apply_rules(&check.decision, snapshot.rules_for(key.as_str()));
        tracing::debug!(
            target: "agena::permission",
            session_id,
            action = key.as_str(),
            static_decision = ?check.decision,
            persisted_rule_count = snapshot.rules_for(key.as_str()).len(),
            resolved_decision = ?resolution.decision,
            "resolved tool permission"
        );
        Ok(resolution)
    }

    pub(in crate::session::manager) async fn aggregate_permission_outcome(
        &self,
        session: Option<&Session>,
        checks: &[ToolPermissionCheck],
    ) -> Result<AggregatedPermissionOutcome, AppError> {
        let mut related_actions = Vec::with_capacity(checks.len());
        let mut requested_actions = Vec::new();
        let mut primary_request: Option<AggregatedPermissionRequest> = None;
        let mut denial: Option<agena_domain::PolicyDeniedResult> = None;

        if checks.is_empty() {
            return Ok(AggregatedPermissionOutcome::Allow);
        }
        let session_id = session.map(|session| session.id);
        let state = self.execution_state();
        let snapshot = self.rule_snapshot(&state, session_id).await?;
        let managed_project_root =
            agena_runtime::project_state_dir(state.tool_executor.workspace_root())
                .to_string_lossy()
                .into_owned();
        let context = agena_permission::DecisionContext {
            managed_project_root: Some(managed_project_root.as_str()),
        };
                let budget = self.auto_budget(session_id);

        // Phase 0: path-granted tool-ask override. A path-scoped tool whose
        // *every* concrete path check is allowed by the path policy performs
        // exactly the operation the user authorized (for example
        // `path.workspace.write = allow`). Its tool-level default `ask`
        // (e.g. `tools.default = ask` without a `filesystem_write` tag
        // allowlist) must not re-ask for that same concrete operation,
        // otherwise the "workspace write: allow" setting never takes effect.
        // Tool-level `Deny` stays authoritative, and a single non-allowed
        // path check disables the override so external or unlisted paths
        // still go through their own policy.
                let mut tool_ask_overridden_by_paths = false;
        if let Some(tool_check) = checks
            .iter()
            .find(|check| matches!(check.action, PermissionAction::Tool { .. }))
            && matches!(tool_check.decision, PermissionDecision::Ask { .. })
            && is_path_scoped_tool(&tool_check.tags)
        {
            let mut path_check_count = 0usize;
            let mut all_paths_allowed = true;
            for check in checks
                .iter()
                .filter(|check| matches!(check.action, PermissionAction::PathAccess { .. }))
            {
                path_check_count += 1;
                let key = permission_action_key(&check.action)?;
                let path_resolution = agena_permission::rules::apply_rules(
                    &check.decision,
                    snapshot.rules_for(key.as_str()),
                );
                if !matches!(path_resolution.decision, PermissionDecision::Allow) {
                    all_paths_allowed = false;
                }
            }
            tool_ask_overridden_by_paths = path_check_count > 0 && all_paths_allowed;
        }

        // Phase 1: synchronous pipeline (static policy + rule snapshot +
        // fast path + heuristics + denial budget) for every check. Checks
        // that still need the classifier are deferred to phase 2.
        let mut decisions: Vec<(PermissionAction, agena_domain::PermissionResolution, bool)> =
            Vec::with_capacity(checks.len());
        let mut candidates = Vec::new();
        for check in checks {
            let action = check.action.clone();
            push_unique_permission_action(&mut related_actions, action.clone());
            let key = permission_action_key(&check.action)?;
            let mut resolution = agena_permission::rules::apply_rules(
                &check.decision,
                snapshot.rules_for(key.as_str()),
            );
                        if tool_ask_overridden_by_paths
                && matches!(check.action, PermissionAction::Tool { .. })
                && matches!(resolution.decision, PermissionDecision::Ask { .. })
            {
                tracing::debug!(
                    target: "agena::permission",
                    action = key.as_str(),
                    "path-granted override lifted the tool-level ask because every path check is allowed"
                );
                resolution.decision = PermissionDecision::Allow;
            }
            let was_auto = matches!(&resolution.decision, PermissionDecision::Auto { .. });
            if was_auto {
                let mut spec = agena_domain::ActionSpec::from_action(&check.action);
                if let agena_domain::ActionSpec::Tool { tags, .. } = &mut spec {
                    *tags = check.tags.clone();
                }
                match agena_permission::decide_sync(&resolution.decision, &spec, &context, &budget)
                {
                    agena_permission::SyncOutcome::Final(decision) => {
                        resolution.decision = decision;
                    }
                    agena_permission::SyncOutcome::Classifier(candidate) => {
                        candidates.push((action, resolution, candidate));
                        continue;
                    }
                }
            }
            decisions.push((action, resolution, was_auto));
        }

        // Phase 2: one shared classifier context (model, transcript, recent
        // decisions) serves every candidate from this batch.
        if !candidates.is_empty() {
            let outcomes = self
                .classify_auto_candidates(
                    session,
                    &state,
                    session_id,
                    candidates
                        .iter()
                        .map(|(_, _, candidate)| candidate.clone())
                        .collect(),
                )
                .await;
            for ((action, mut resolution, candidate), outcome) in
                candidates.into_iter().zip(outcomes)
            {
                resolution.decision = match outcome {
                    Ok(true) => PermissionDecision::Allow,
                                        Ok(false) => PermissionDecision::Deny {
                        reason: agena_permission::deny_reason(format!(
                            "automatic approval classifier denied the action: {}",
                            candidate.policy_reason
                        )),
                    },
                    Err(()) => PermissionDecision::Ask {
                        reason: format!(
                            "automatic approval classifier unavailable; falling back to confirmation: {}",
                            candidate.policy_reason
                        ),
                    },
                };
                decisions.push((action, resolution, true));
            }
        }

        // Phase 3: aggregate final decisions.
        for (action, resolution, auto_approval) in decisions {
            let decision = match resolution.decision {
                PermissionDecision::Auto { reason } => PermissionDecision::Ask {
                    reason: format!(
                        "automatic approval was not resolved; falling back to confirmation: {reason}"
                    ),
                },
                decision => decision,
            };
            match decision {
                PermissionDecision::Allow => {}
                PermissionDecision::Deny { reason } => {
                    let (source, scope, operator, authority, rule_id, rule_revision_ms) =
                        match &resolution.source {
                            agena_domain::PermissionResolutionSource::PersistedRule {
                                rule_id,
                                revision_ms,
                                scope,
                                source,
                                operator,
                                ..
                            } => (
                                Some(source.clone()),
                                Some(*scope),
                                operator.clone(),
                                agena_domain::PermissionAuthorityKind::PersistedRule,
                                *rule_id,
                                *revision_ms,
                            ),
                            agena_domain::PermissionResolutionSource::StaticPolicy => {
                                let plugin_step = resolution.trace.iter().rev().find(|step| {
                                    step.source_kind == PolicySourceKind::PluginAdvice
                                });
                                (
                                    plugin_step
                                        .and_then(|step| step.source.clone())
                                        .or_else(|| Some("static_policy".to_string())),
                                    None,
                                    plugin_step.and_then(|step| step.operator.clone()),
                                    if auto_approval {
                                        agena_domain::PermissionAuthorityKind::AutoApprovalModel
                                    } else if plugin_step.is_some() {
                                        agena_domain::PermissionAuthorityKind::PluginPolicy
                                    } else {
                                        agena_domain::PermissionAuthorityKind::StaticPolicy
                                    },
                                    None,
                                    None,
                                )
                            }
                        };
                    if let Some(existing) = denial.as_mut() {
                        push_unique_permission_action(&mut existing.denied_actions, action.clone());
                        existing.risk = max_permission_risk(existing.risk, resolution.risk);
                        existing.trace.extend(resolution.trace);
                    } else {
                        denial = Some(agena_domain::PolicyDeniedResult {
                            action: action.clone(),
                            related_actions: Vec::new(),
                            denied_actions: vec![action],
                            reason,
                            explanation: resolution.explanation,
                            source,
                            scope,
                            operator,
                            authority,
                            rule_id,
                            rule_revision_ms,
                            risk: resolution.risk,
                            trace: resolution.trace,
                        });
                    }
                }
                PermissionDecision::Ask { reason } | PermissionDecision::Auto { reason } => {
                    push_unique_permission_action(&mut requested_actions, action.clone());
                    let (source, scope, operator) = match resolution.source {
                        agena_domain::PermissionResolutionSource::PersistedRule {
                            scope,
                            source,
                            operator,
                            ..
                        } => (Some(source), Some(scope), operator),
                        agena_domain::PermissionResolutionSource::StaticPolicy => {
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

        if let Some(mut denial) = denial {
            denial.related_actions = related_actions;
            return Ok(AggregatedPermissionOutcome::Deny(Box::new(denial)));
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
        risk: PermissionRiskLevel,
        trace: Vec<DecisionTraceStep>,
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
        risk: PermissionRiskLevel,
        trace: Vec<DecisionTraceStep>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let existing_permission_replied = session
            .part(&resolved.pending.part)
            .and_then(|part| part.content.as_ref())
            .and_then(|content| match content {
                PartContent::Activity(crate::message::RuntimeActivity::Operation(operation)) => {
                    operation
                        .authorization
                        .find(request_id.as_str())
                        .map(|permission| permission.reply.is_some())
                }
                _ => None,
            });
        if existing_permission_replied == Some(true) {
            return Box::pin(self.apply_tool_error(
                session,
                pending_tool,
                ToolError::plugin(format!(
                    "permission request {request_id} is already resolved; the same operation cannot request it again"
                )),
                None,
                state,
            ))
            .await;
        }
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

        let assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(
                    operation,
                ))) = tool_part.content.as_mut()
                else {
                    return;
                };
                operation.authorization.push_pending(request.clone());
                operation.set_summary(format!("Awaiting approval · {reason}"));
                tool_part.status = ExecutionStatus::Pending;
                tool_part.summary = Some(operation.summary.clone());
            })?;
        let session_id = session.id;
        let events = if existing_permission_replied.is_some() {
            Vec::new()
        } else {
            vec![EventKind::PermissionRequested(PermissionRequestedEvent {
                session_id,
                operation_id: resolved.operation_id.clone(),
                call_id: resolved.call_id,
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
            })]
        };
        self.persist_session_changes(
            session,
            vec![MessageCheckpoint::part(
                assistant_message.id,
                resolved.pending.part.part_id,
            )],
            events,
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
        let authorization = operation_authorization(&session, &resolved);

        let _assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                tool_part.set_content(PartContent::operation(pending_operation_for_resolved(
                    &resolved,
                    resolved.invocation.clone(),
                    ask_user_title(&request),
                    resolved.lifecycle.clone(),
                    authorization.clone(),
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
            agena_domain::PendingInteractiveRequestKind::UserInput,
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
        let checkpoint = MessageCheckpoint::parts(
            assistant_message.id,
            assistant_message
                .parts
                .iter()
                .filter(|part| part.operation_id.as_deref() == Some(resolved.operation_id.as_str()))
                .map(|part| part.id),
        );
        self.persist_session_changes(session, vec![checkpoint], Vec::new(), None, state)
            .await
    }

    pub(in crate::session::manager) async fn apply_streaming_tool_execution(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        mut stream: StreamingToolExecution,
        state: Arc<SessionManagerState>,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<Session, AppError> {
        let stream_id = stream.stream_id.clone();
        loop {
            let chunk = match cancellation.as_ref() {
                Some(cancellation) => tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        return self.apply_tool_cancellation(session, pending_tool, state).await;
                    },
                    chunk = stream.chunks.recv() => chunk,
                },
                None => stream.chunks.recv().await,
            };
            let Some(chunk) = chunk else { break };
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

        let stream_end = match cancellation.as_ref() {
            Some(cancellation) => tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    return self.apply_tool_cancellation(session, pending_tool, state).await;
                },
                result = stream.end => result,
            },
            None => stream.end.await,
        };
        let execution = match stream_end {
            Ok(Ok(execution)) => execution,
            Ok(Err(ToolError::PolicyDenied(denial))) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                return self
                    .apply_tool_policy_denied(session, pending_tool, *denial, state)
                    .await;
            }
            Ok(Err(ToolError::UserDeclined(decline))) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                return self
                    .apply_tool_user_declined(session, pending_tool, *decline, Vec::new(), state)
                    .await;
            }
            Ok(Err(ToolError::CapabilityUnavailable(unavailable))) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                return self
                    .apply_tool_capability_unavailable(session, pending_tool, *unavailable, state)
                    .await;
            }
            Ok(Err(ToolError::ToolUnavailable(unavailable))) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                return self
                    .apply_tool_unavailable(session, pending_tool, *unavailable, state)
                    .await;
            }
            Ok(Err(err)) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                return self
                    .apply_tool_error(session, pending_tool, err, None, state)
                    .await;
            }
            Err(_) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                return self
                    .apply_tool_error(
                        session,
                        pending_tool,
                        ToolError::plugin(format!(
                            "tool stream ended without terminal result: {stream_id}"
                        )),
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

    pub(in crate::session::manager) async fn apply_tool_cancellation(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let assistant_message = update_resolved_tool_message(&mut session, &resolved, |part| {
            if let Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(
                operation,
            ))) = part.content.as_ref()
            {
                let mut operation = operation.clone();
                operation.set_summary("Execution cancelled");
                operation.lifecycle = completed_lifecycle(&resolved.lifecycle);
                part.set_content(PartContent::operation(operation));
            }
            part.status = ExecutionStatus::Cancelled;
            part.summary = Some("Execution cancelled".to_string());
        })?;

        self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            Vec::new(),
            Vec::new(),
            state,
        )
        .await
    }

    /// Persist one text chunk for a pending tool operation. This is shared by
    /// ordinary direct streaming invocations and streaming targets executed
    /// through Tool API function `tools_call`.
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

        self.persist_session_changes(
            session,
            vec![MessageCheckpoint::part(
                pending_tool.part.message_id,
                pending_tool.part.part_id,
            )],
            Vec::new(),
            None,
            state,
        )
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
        Box::pin(self.apply_tool_success_with_rules(
            session,
            pending_tool,
            execution,
            persisted_rule.into_iter().collect(),
            state,
        ))
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
        let authorization = operation_authorization(&session, &resolved);
        let tool_output = execution.output.clone();
        let mut summary = execution.summary();
        let attributed_usage = summary
            .metadata
            .remove(agena_provider::PROVIDER_TOOL_USAGE_METADATA_KEY)
            .map(|value| {
                serde_json::from_str::<agena_provider::AttributedCompletionUsage>(&value).map_err(
                    |error| {
                        AppError::Internal(format!(
                            "provider-backed tool returned invalid nested usage metadata: {error}"
                        ))
                    },
                )
            })
            .transpose()?;
        let output_text = summary.output_text.clone();
        let presentation_summary = summary.summary.clone();
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let blocks = operation_blocks_from_tool_output(
            &resolved.invocation,
            &tool_output,
            execution.view.attachments.as_slice(),
            output_text.as_str(),
        );
        let completion_title = {
            let execution_title = summary.title.trim();
            if !execution_title.is_empty() && !is_authorization_phase_title(execution_title) {
                execution_title.to_string()
            } else {
                terminal_operation_title(&resolved.invocation)
            }
        };
        self.apply_tool_success_execution_context(&mut session, &resolved.invocation, &execution);

        update_resolved_tool_message(&mut session, &resolved, |tool_part| {
            let mut operation = OperationPart::completed(
                resolved.call_id,
                resolved.invocation.clone(),
                crate::message::OperationCompletion::new(
                    completion_title.clone(),
                    presentation_summary.clone(),
                    output_text.clone(),
                    blocks.clone(),
                    execution.view.attachments.clone(),
                    tool_output.clone(),
                ),
                lifecycle.clone(),
            );
            operation.authorization = authorization.clone();
            operation.set_presentation_sections(summary.sections.clone());
            operation.result.metadata.extend(
                summary
                    .metadata
                    .iter()
                    .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone()))),
            );
            tool_part.set_content(PartContent::operation(operation));
            tool_part.status = ExecutionStatus::Completed;
        })?;
        if let Some(attributed_usage) = attributed_usage {
            let message = session
                .messages
                .get_mut(resolved.pending.part.message_index)
                .ok_or_else(|| pending_tool_part_not_found_error(&resolved.pending.part))?;
            message
                .usage
                .get_or_insert_with(agena_provider::CompletionUsage::default)
                .attributed_usage
                .push(attributed_usage);
        }
        let assistant_message = assistant_message_for_part(&session, &resolved.pending.part)?;

        Box::pin(self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            persisted_rules,
            Vec::new(),
            state,
        ))
        .await
    }
}
