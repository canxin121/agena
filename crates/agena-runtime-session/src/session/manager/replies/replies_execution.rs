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
    operation_authorization, operation_blocks_from_tool_output, pending_operation_for_resolved,
    pending_tool_part_not_found_error, permission_action_key, permission_scope_label,
    push_unique_permission_action, resolve_pending_tool, responses_api_request_metadata,
    run_abort_reason, should_execute_pending_tools_concurrently, terminal_operation_title,
    tool_name, update_resolved_tool_message,
};
use crate::session::Session;
use crate::session::prompt_window;
use agena_domain::UserInputRequest;
use agena_domain::{
    DecisionTraceStep, ExecutionPhase, ExecutionSource, FinishReason, PermissionAction,
    PermissionDecision, PermissionRequest, PermissionRequestedEvent, PermissionScope,
    PolicySourceKind, Role, RunAbortReason,
};
use tracing::Instrument;

use super::super::StableRunContext;

/// Outcome of one provider model turn as observed by the stable-run loop.
#[derive(Debug, Clone, Copy)]
pub(in crate::session::manager) struct ModelTurnOutcome {
    /// True when the provider terminal event carried an explicit
    /// `end_turn=false` signal.
    pub follow_up_requested: bool,
    /// Normalized terminal finish reason for the model turn.
    pub finish_reason: FinishReason,
}

/// The stable-run loop's decision after one model turn about whether to
/// request another model turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnContinuation {
    /// The turn is finished and the run should stop. The loop only runs the
    /// `agent.stop` hook before returning.
    Stop,
    /// The model requested tools this turn; the loop's pending-tools branch
    /// executes them, then requests another model turn.
    PendingTools,
    /// The provider explicitly signaled `end_turn=false`: the model asked to
    /// keep working even though it did not request tools. Honor the protocol
    /// signal the way Codex's `needs_follow_up` path does, bounded by a budget.
    FollowUpRequested,
    /// The provider hit the output-token limit (`max_tokens`); the reply was
    /// cut off mid-sentence. Inject a truncation continuation message and
    /// request one more turn, bounded by a budget.
    Truncated,
}

/// The message injected when a model turn was cut off by the output limit.
/// Kept terse and imperative so the model resumes work instead of repeating.
const TRUNCATED_CONTINUATION_PROMPT: &str = "Your previous response was cut off by the output limit. Continue directly from where it stopped; do not repeat what was already written.";

/// Feedback injected when the doom-loop detector fires but the run still has
/// recovery budget. Mirrors gemini's `_recoverFromLoop`: the model is told
/// the identical call cannot make progress and must change approach, and the
/// run continues instead of aborting.
const DOOM_LOOP_RECOVERY_PROMPT: &str = "Trusted Agena runtime note: the previous tool call has already been invoked with the exact same input and did not make progress; repeating it identically is a loop. Do not invoke that tool with the same arguments again. Change approach: inspect the available tool results, adjust the arguments, or answer the user directly.";

/// Number of doom-loop recoveries allowed before a run stops softly instead
/// of looping forever (gemini bounds loop recovery with `boundedTurns - 1`;
/// this is the same idea with an explicit small budget).
const MAX_DOOM_LOOP_RECOVERIES: usize = 2;

/// Fallback for the session agent loop's model-turn cap when no explicit
/// `max_turns` is configured. Originally mirrored gemini's MAX_TURNS=100 and
/// the other reference CLIs' turn budgets; the default has since been raised
/// to 500 to accommodate longer agentic runs. `RuntimeSessionManagerConfig::default`
/// uses the same value.
pub(crate) const DEFAULT_MAX_MODEL_TURNS: usize = 500;

/// Decide whether the stable-run loop should request another model turn after
/// the given outcome.
///
/// The primary signal is the main-stream agent judgment: "did this turn
/// request tools?" — `has_pending_tools` is authoritative and takes priority.
/// The terminal `finish_reason` only classifies abnormal turns: a truncated
/// (`MaxTokens`) reply is almost certainly cut off and continues under budget;
/// content-filter refusals, unknown reasons, or a missing reason stop the run.
/// An explicit `end_turn=false` protocol signal (`follow_up_requested`) also
/// continues, bounded by its own budget so a misbehaving provider cannot loop
/// forever.
fn should_continue_turn(
    outcome: &ModelTurnOutcome,
    has_pending_tools: bool,
    truncation_budget: &mut usize,
    follow_up_budget: &mut usize,
) -> TurnContinuation {
    if has_pending_tools {
        return TurnContinuation::PendingTools;
    }
    if outcome.follow_up_requested {
        if *follow_up_budget > 0 {
            *follow_up_budget -= 1;
            return TurnContinuation::FollowUpRequested;
        }
        tracing::warn!(
            target: "agena::session::run_until_stable",
            "provider keeps signaling `end_turn=false`; stopping after follow-up budget exhausted"
        );
        return TurnContinuation::Stop;
    }
    if outcome.finish_reason == FinishReason::MaxTokens {
        if *truncation_budget > 0 {
            *truncation_budget -= 1;
            return TurnContinuation::Truncated;
        }
        tracing::warn!(
            target: "agena::session::run_until_stable",
            "provider keeps truncating responses; stopping after truncation budget exhausted"
        );
        return TurnContinuation::Stop;
    }
    TurnContinuation::Stop
}

/// True for tools whose operation is scoped to concrete paths (filesystem
/// read/write tools such as `fs.write` / `fs.apply_patch`). Arbitrary
/// execution tools (shell/process) are never path-scoped even when they
/// declare filesystem effects, because their declared paths are derived
/// from free-form input, not authoritative: a user who allows writes inside
/// the workspace has not authorized arbitrary command execution.
///
/// Driven by the tool's permission contract, never by tags: tags are
/// metadata and carry no authority.
fn is_path_scoped_tool(contract: &agena_domain::ToolPermissionContract) -> bool {
    let path_scoped = !contract.input_paths.is_empty() || !contract.path_access.is_empty();
    path_scoped && !contract.shell
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
        let activity_id = pending.activity_id.map(|id| id.to_string());
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
                    event.context.activity_id = activity_id.clone();
                    EventKind::CommandBegin(event)
                }
                agena_tool::ToolRuntimeEvent::CommandOutputDelta(mut event) => {
                    event.context.session_id = session_id;
                    event.context.call_id = call_id;
                    event.context.message_id = Some(message_id);
                    event.context.part_id = Some(part_id);
                    event.context.activity_id = activity_id.clone();
                    EventKind::CommandOutputDelta(event)
                }
                agena_tool::ToolRuntimeEvent::CommandEnd(mut event) => {
                    event.context.session_id = session_id;
                    event.context.call_id = call_id;
                    event.context.message_id = Some(message_id);
                    event.context.part_id = Some(part_id);
                    event.context.activity_id = activity_id.clone();
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
            base_run_source,
            mut active_model_turn_id,
            state,
            control,
            mut steer_rx,
            usage_budget,
        } = context;
        let mut reactive_compaction_attempted = false;
        let mut force_model_retry = false;
        // Bound on the total number of model turns in one stable run (see
        // `DEFAULT_MAX_MODEL_TURNS`); the run stops softly when reached.
        let mut model_turns_taken: usize = 0;
        // Doom-loop recoveries already injected (see `MAX_DOOM_LOOP_RECOVERIES`).
        let mut doom_loop_recoveries: usize = 0;
        // Bounded safety net for model turns that were cut off by the output
        // limit (`finish_reason == max_tokens`). Each firing consumes budget;
        // a degenerate model that always truncates cannot loop forever.
        const TRUNCATION_CONTINUATION_LIMIT: usize = 4;
        let mut truncation_continuation_remaining = TRUNCATION_CONTINUATION_LIMIT;
        // Bounded honor of the explicit `end_turn=false` protocol signal. A
        // provider that keeps signaling "keep working" without requesting tools
        // is stopped after this budget rather than looping forever.
        const FOLLOW_UP_CONTINUATION_LIMIT: usize = 4;
        let mut follow_up_continuation_remaining = FOLLOW_UP_CONTINUATION_LIMIT;
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
                if doom_loop_recoveries < MAX_DOOM_LOOP_RECOVERIES {
                    doom_loop_recoveries += 1;
                    tracing::warn!(
                        target: "agena::session::doom_loop",
                        session_id = session.id,
                        tool = %hit.tool_label,
                        repeat = hit.repeat_count,
                        recovery = doom_loop_recoveries,
                        max_recoveries = MAX_DOOM_LOOP_RECOVERIES,
                        "doom-loop detected; injecting recovery feedback and continuing"
                    );
                    session = self
                        .inject_continuation_message(
                            session,
                            &current_options,
                            DOOM_LOOP_RECOVERY_PROMPT.to_owned(),
                            state.clone(),
                        )
                        .await?;
                    model_requested = true;
                    continue;
                }
                tracing::warn!(
                    target: "agena::session::doom_loop",
                    session_id = session.id,
                    tool = %hit.tool_label,
                    repeat = hit.repeat_count,
                    recoveries = doom_loop_recoveries,
                    "doom-loop persisted past the recovery budget; stopping run softly"
                );
                return Ok(session);
            }

            let pending_tools = session.pending_tools();
            if !pending_tools.is_empty() {
                // The model requested tools this turn: we are mid-task. Resolve
                // and execute them, then request another model turn so the
                // tool results are sent back.
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
                    last_assistant_message: last_assistant_text.clone(),
                };
                match state
                    .tool_executor
                    .plugin_manager()
                    .dispatch_agent_stop_cancellable(stop_input, Some(control.cancel.clone()))
                    .await
                {
                    Ok(dispatch) => {
                        // Surface every observed hook run as first-class
                        // transcript activity so users can see whether a stop
                        // hook (for example the workflow plan autorun
                        // continuation) actually fired.
                        if !dispatch.runs.is_empty() {
                            session = self
                                .record_agent_stop_hook_runs(session, dispatch.runs, state.clone())
                                .await?;
                        }
                        if dispatch.patch.continue_with_message.is_some() {
                            let follow_up = dispatch.patch.continue_with_message.unwrap_or_default();
                            session = self
                                .inject_continuation_message(
                                    session,
                                    &current_options,
                                    follow_up,
                                    state.clone(),
                                )
                                .await?;
                            model_requested = true;
                            continue;
                        }
                    }
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

                return Ok(session);
            }
            // Reaching this point consumes the one-shot retry authorization.
            // It must not survive a successful model turn and trigger another
            // model call after the session becomes quiescent.
            force_model_retry = false;

            // Bounded soft stop: a run that keeps requesting model turns
            // (tools, follow-ups, truncation) is stopped after the configured
            // cap instead of looping forever. `Some(0)` means unlimited
            // (quota.rs precedent); `None` falls back to the default cap.
            let max_turns = match state.config.max_turns {
                Some(0) => usize::MAX,
                Some(n) => n,
                None => DEFAULT_MAX_MODEL_TURNS,
            };
            if model_turns_taken >= max_turns {
                tracing::warn!(
                    target: "agena::session::run_until_stable",
                    session_id = session.id,
                    max_turns,
                    "stopping run softly after the model-turn budget was exhausted"
                );
                // Surface a user-facing notice in the transcript before the
                // silent soft stop, so the run does not look like it finished
                // normally when it was actually cut off by the budget cap.
                session = self
                    .record_model_turn_budget_notice(session, max_turns, state.clone())
                    .await?;
                return Ok(session);
            }
            model_turns_taken += 1;

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
                Ok((next_session, outcome)) => {
                    session = next_session;
                    model_requested = false;
                    match should_continue_turn(
                        &outcome,
                        !session.pending_tools().is_empty(),
                        &mut truncation_continuation_remaining,
                        &mut follow_up_continuation_remaining,
                    ) {
                        TurnContinuation::PendingTools | TurnContinuation::FollowUpRequested => {
                            // Tools will be executed by the pending-tools branch,
                            // or the model explicitly asked to keep working.
                            model_requested = true;
                        }
                        TurnContinuation::Truncated => {
                            // The reply was cut off by the output limit; ask the
                            // model to resume from where it stopped.
                            session = self
                                .inject_continuation_message(
                                    session,
                                    &current_options,
                                    TRUNCATED_CONTINUATION_PROMPT.to_owned(),
                                    state.clone(),
                                )
                                .await?;
                            model_requested = true;
                        }
                        TurnContinuation::Stop => {
                            // Plain-text completion (or abnormal non-truncated
                            // finish). The stop path below runs the agent.stop
                            // hook and returns.
                        }
                    }
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

    /// Inject a system-originated user message that asks the model to keep
    /// working. Used by agent.stop continuation patches and by the truncation
    /// continuation path; the message is persisted so a process restart can
    /// resume from the same state.
    async fn inject_continuation_message(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        text: String,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let ids = self.store.reserve_message_ids(1).await?;
        let follow_up_turn_id = ids.message_id;
        let user_message = build_message(
            ids,
            Role::User,
            ExecutionStatus::Completed,
            vec![PartContent::text(text)],
            MessageMetadata {
                source: MessageSource::System,
                idempotency_key: None,
                model_turn_id: Some(follow_up_turn_id),
                parent_message_id: session.last_conversation_message().map(|m| m.id),
                generated_by_call_id: None,
                externally_initiated_tool: false,
                model_provider_id: options.model.provider_id.to_string(),
                model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
                model_id: options.model.model_id.to_string(),
                model_thinking_mode: options.thinking_mode.clone(),
                model_speed_mode: options.speed_mode.clone(),
            },
        )?;
        session.messages.push(user_message.clone());
        let checkpoint = MessageCheckpoint::all(&user_message);
        self.persist_session_changes(session, vec![checkpoint], Vec::new(), None, state)
            .await
    }

    /// Record one System-originated Assistant message with a `Hook` part per
    /// observed `agent.stop` hook run. This makes hook execution visible in
    /// the same transcript activity pipeline as tool calls and is persisted
    /// so a restart keeps the record.
    async fn record_agent_stop_hook_runs(
        &self,
        mut session: Session,
        runs: Vec<agena_plugin_host::AgentStopHookRun>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let ids = self.store.reserve_message_ids(runs.len()).await?;
        let parts = runs
            .into_iter()
            .map(|run| {
                let summary = match (&run.continue_with_message, &run.reason) {
                    (Some(_), Some(reason)) => format!("agent.stop hook blocked stop: {reason}"),
                    (Some(_), None) => "agent.stop hook blocked stop".to_string(),
                    (None, Some(reason)) => format!("agent.stop hook ran: {reason}"),
                    (None, None) => "agent.stop hook ran (no continuation)".to_string(),
                };
                let detail = run.continue_with_message.or(run.reason);
                PartContent::hook(crate::message::HookPart {
                    hook: run.hook,
                    plugin_id: Some(run.plugin_id),
                    summary,
                    detail,
                })
            })
            .collect();
        let message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::Completed,
            parts,
            MessageMetadata {
                source: MessageSource::System,
                idempotency_key: None,
                model_turn_id: None,
                parent_message_id: session.last_conversation_message().map(|m| m.id),
                generated_by_call_id: None,
                externally_initiated_tool: false,
                model_provider_id: String::new(),
                model_adapter_id: None,
                model_id: String::new(),
                model_thinking_mode: None,
                model_speed_mode: None,
            },
        )?;
        session.messages.push(message.clone());
        let checkpoint = MessageCheckpoint::all(&message);
        self.persist_session_changes(session, vec![checkpoint], Vec::new(), None, state)
            .await
    }

    /// Record a System-originated Assistant message with a single `Notice`
    /// part explaining that the run stopped because the model-turn budget was
    /// exhausted. Follows the `record_agent_stop_hook_runs` recipe: persisted
    /// so a restart keeps the record, surfaced as first-class transcript
    /// activity, and (Assistant + System + `model_turn_id: None`) never
    /// triggers another model turn. The Notice is user-facing only — the
    /// provider projection (`wire_message.rs`) skips it, and on a later run
    /// `normalize_prompt_messages` drops it for having no visible payload.
    async fn record_model_turn_budget_notice(
        &self,
        mut session: Session,
        max_turns: usize,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let ids = self.store.reserve_message_ids(1).await?;
        const SUMMARY: &str = "Model-turn budget exhausted; the run stopped.";
        let detail = Some(format!(
            "The run reached the configured model-turn cap (max_turns={max_turns}) and stopped. \
             Send a new message to continue, or raise the cap via `session.max_turns` in the \
             config (`0` means unlimited)."
        ));
        let message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::Completed,
            vec![PartContent::notice(crate::message::NoticePart {
                kind: "max_turns_exhausted".to_string(),
                summary: SUMMARY.to_string(),
                detail,
            })],
            MessageMetadata {
                source: MessageSource::System,
                idempotency_key: None,
                model_turn_id: None,
                parent_message_id: session.last_conversation_message().map(|m| m.id),
                generated_by_call_id: None,
                externally_initiated_tool: false,
                model_provider_id: String::new(),
                model_adapter_id: None,
                model_id: String::new(),
                model_thinking_mode: None,
                model_speed_mode: None,
            },
        )?;
        session.messages.push(message.clone());
        let checkpoint = MessageCheckpoint::all(&message);
        self.persist_session_changes(session, vec![checkpoint], Vec::new(), None, state)
            .await
    }

    pub(in crate::session::manager) async fn run_model_turn(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        run_source: ExecutionSource,
        model_turn_id: Option<i64>,
        state: Arc<SessionManagerState>,
        control: Arc<ExecutionControl>,
    ) -> Result<(Session, ModelTurnOutcome), AppError> {
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
                        SessionRunTermination::Completed => Ok((
                            persisted_session,
                            ModelTurnOutcome {
                                follow_up_requested: result.follow_up_requested,
                                finish_reason: result.finish_reason,
                            },
                        )),
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
            // Mark every ready tool InProgress before fanning out so the live
            // transcript shows in-flight tools during the (potentially long)
            // parallel execution instead of pending placeholders. Group changed
            // parts by their owning message so each checkpoint stays within one
            // message, then persist the batch transitions.
            let mut parts_by_message = std::collections::HashMap::<i64, Vec<i64>>::new();
            for resolved in &ready_tools {
                let Some(part) = session.part_mut(&resolved.pending.part) else {
                    continue;
                };
                if matches!(
                    part.status,
                    ExecutionStatus::Pending | ExecutionStatus::InProgress
                ) {
                    part.status = ExecutionStatus::InProgress;
                    parts_by_message
                        .entry(resolved.pending.part.message_id)
                        .or_default()
                        .push(part.id);
                }
            }
            if !parts_by_message.is_empty() {
                let checkpoints = parts_by_message
                    .into_iter()
                    .map(|(message_id, part_ids)| MessageCheckpoint::parts(message_id, part_ids))
                    .collect::<Vec<_>>();
                session = Box::pin(self.persist_session_changes(
                    session,
                    checkpoints,
                    Vec::new(),
                    None,
                    state.clone(),
                ))
                .await?;
            }
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

        // The tool is now authorized and about to execute. Move the Activity
        // from Pending to InProgress and checkpoint immediately so the live
        // transcript shows an in-flight tool instead of leaving a pending
        // placeholder while execution runs. The checkpoint carries the
        // resolved turn/reply owner (resolved by `SessionStore::persist`), so
        // the terminal sees an incremental update rather than a full refresh.
        // The title itself is produced by the tool during execution; at this
        // point the prepared invocation title is already in place.
        let part_was_pending = session
            .part_mut(&resolved.pending.part)
            .map(|tool_part| {
                let was_pending = matches!(tool_part.status, ExecutionStatus::Pending);
                if matches!(
                    tool_part.status,
                    ExecutionStatus::Pending | ExecutionStatus::InProgress
                ) {
                    tool_part.status = ExecutionStatus::InProgress;
                }
                was_pending
            })
            .unwrap_or(false);
        if part_was_pending {
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
            && is_path_scoped_tool(&tool_check.contract)
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
                if let agena_domain::ActionSpec::Tool { contract, .. } = &mut spec {
                    *contract = check.contract.clone();
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
                    Err(failure) => PermissionDecision::Ask {
                        reason: format!("automatic approval unavailable: {failure}"),
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
        // Streaming output is accumulated in memory only. The durable record
        // never grows per-delta: the streamed text is written once, truncated,
        // at completion. Every 2s we emit a header-only checkpoint that only
        // refreshes the running title (a tiny UPDATE), so a long stream costs
        // O(delta) writes instead of re-persisting the cumulative output.
        const TITLE_REFRESH_MS: u64 = 2_000;
        const DETAIL_BROADCAST_MS: u64 = 200;
        let mut last_title_refresh = std::time::Instant::now();
        let mut last_detail_broadcast = std::time::Instant::now();
        let mut streamed_output = String::new();
        let mut pending_detail_delta = String::new();
        // Resolve the Activity id once so live detail broadcasts never need a
        // per-tick session load.
        let streaming_activity_id = session
            .part(&pending_tool.part)
            .and_then(|part| part.activity_id);
        // Activity v2 live bridge (07 §5.2, §6.1): one in-memory handler feeds
        // the unified wire events from the same text deltas that drive the
        // legacy detail broadcasts. Events are published as
        // `EventKind::ActivityV2` (live, non-persistent) for TUI/Web.
        let initial_title = session
            .part(&pending_tool.part)
            .and_then(|part| match &part.content {
                Some(crate::message::PartContent::Activity(
                    crate::message::RuntimeActivity::Operation(operation),
                )) if !operation.title.is_empty() => Some(operation.title.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "tool".to_owned());
        let mut activity_handler = streaming_activity_id.map(|activity_id| {
            crate::activity::ActivityHandler::begin(
                activity_id,
                crate::activity::ActivityKind::Operation,
                initial_title,
            )
        });
        let stream_started = std::time::Instant::now();
        let mut stream_block_created = false;
        loop {
            let chunk = match cancellation.as_ref() {
                Some(cancellation) => tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        return self.apply_tool_cancellation(session, pending_tool, state).await;
                    },
                    chunk = stream.chunks.recv() => chunk,
                    // Idle heartbeat: refresh the running title so a silent
                    // long-running command still shows live progress.
                    _ = tokio::time::sleep(std::time::Duration::from_millis(TITLE_REFRESH_MS)) => {
                        session = self
                            .refresh_streaming_title(session.id, pending_tool, state.clone())
                            .await?;
                        continue;
                    },
                },
                None => stream.chunks.recv().await,
            };
            let Some(chunk) = chunk else {
                break;
            };
            let Some(delta) = chunk.text_delta.as_deref() else {
                continue;
            };
            if delta.is_empty() {
                continue;
            }
            streamed_output.push_str(delta);
            pending_detail_delta.push_str(delta);
            if let Some(handler) = &mut activity_handler {
                let render_event = if stream_block_created {
                    agena_tool::ToolActivityEvent::Render(agena_domain::RenderDelta::append(
                        "stream",
                        agena_domain::ViewBlock::Log {
                            id: None,
                            stream: agena_domain::CommandOutputStream::Stdout,
                            text: delta.to_string(),
                        },
                    ))
                } else {
                    stream_block_created = true;
                    agena_tool::ToolActivityEvent::Render(agena_domain::RenderDelta::new(
                        agena_domain::ViewBlock::Log {
                            id: Some("stream".to_owned()),
                            stream: agena_domain::CommandOutputStream::Stdout,
                            text: delta.to_string(),
                        },
                    ))
                };
                for event in handler.apply_event(render_event) {
                    self.broadcast_activity_v2(session.id, event)?;
                }
            }
            // Broadcast the new output as a live, non-persisted detail delta so
            // an expanded terminal renders the growing detail in real time.
            if last_detail_broadcast.elapsed()
                >= std::time::Duration::from_millis(DETAIL_BROADCAST_MS)
                && !pending_detail_delta.is_empty()
            {
                last_detail_broadcast = std::time::Instant::now();
                self.broadcast_streaming_detail(
                    session.id,
                    pending_tool,
                    &pending_detail_delta,
                    streaming_activity_id,
                )?;
                pending_detail_delta.clear();
            }
            if last_title_refresh.elapsed() >= std::time::Duration::from_millis(TITLE_REFRESH_MS) {
                last_title_refresh = std::time::Instant::now();
                session = self
                    .refresh_streaming_title(session.id, pending_tool, state.clone())
                    .await?;
                if let Some(handler) = &mut activity_handler {
                    if let Some(event) =
                        handler.refresh_elapsed_title(stream_started.elapsed().as_secs())
                    {
                        self.broadcast_activity_v2(session.id, event)?;
                    }
                }
            }
        }
        session = self
            .apply_streaming_terminal_output(
                session.id,
                pending_tool,
                streamed_output.as_str(),
                state.clone(),
            )
            .await?;

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

        // Publish the terminal v2 activity node once the stream finished
        // successfully. The durable write happens in the legacy success path;
        // the live wire event is broadcast in memory only.
        if let Some(mut handler) = activity_handler.take() {
            let node = handler.finish(
                agena_tool::ToolActivityResult::raw(agena_domain::RawOutput::text(
                    streamed_output,
                )),
                agena_domain::ActivityState::Completed,
            );
            self.broadcast_activity_v2(
                session.id,
                crate::activity::ActivityLiveEvent::Upserted {
                    node: Box::new(node),
                },
            )?;
        }

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

    /// Broadcast a slice of freshly streamed output to live presentation
    /// consumers as a non-persistent `CommandOutputDelta`. Expanded terminals
    /// render this delta into the Activity's detail in real time; collapsed
    /// terminals drop it. Nothing is written to disk.
    fn broadcast_streaming_detail(
        &self,
        session_id: i64,
        pending_tool: &SessionPendingTool,
        delta: &str,
        activity_id: Option<agena_domain::ActivityId>,
    ) -> Result<(), AppError> {
        let event = agena_domain::CommandOutputDeltaEvent {
            context: agena_domain::CommandContext {
                session_id,
                call_id: 0,
                message_id: Some(pending_tool.part.message_id),
                part_id: Some(pending_tool.part.part_id),
                activity_id: activity_id.map(|id| id.to_string()),
            },
            stream: agena_domain::CommandOutputStream::Stdout,
            seq: 0,
            ts_ms: chrono::Utc::now().timestamp_millis(),
            chunk: delta.as_bytes().to_vec(),
            preview_text: delta.to_string(),
            preview_lossy: false,
        };
        let publisher = Arc::clone(&self.publisher);
        let context = crate::event::PublishContext::for_session(session_id);
        let handle = tokio::runtime::Handle::current();
        // Fire-and-forget in-memory broadcast; the bus is in-process and
        // non-persistent for CommandOutputDelta, so this never blocks the
        // streaming loop on disk I/O.
        handle.spawn(async move {
            if let Err(error) = publisher
                .publish(context, crate::event::EventKind::CommandOutputDelta(event))
                .await
            {
                tracing::debug!(
                    target: "agena::session::streaming_detail",
                    session_id,
                    error = %error,
                    "failed to broadcast live streaming detail"
                );
            }
        });
        Ok(())
    }

    /// Refresh the running title of a streaming tool and emit a header-only
    /// checkpoint. This is the only durable write during a stream: it updates
    /// the compact title (a tiny UPDATE), never the cumulative output, so a
    /// long stream costs O(1) writes rather than re-persisting the growing
    /// text every 2s.
    /// Publish one activity v2 live wire event (07 §5.2). In-memory,
    /// non-persistent, fire-and-forget like the legacy detail broadcasts.
    fn broadcast_activity_v2(
        &self,
        session_id: i64,
        event: crate::activity::ActivityLiveEvent,
    ) -> Result<(), AppError> {
        let publisher = Arc::clone(&self.publisher);
        let context = crate::event::PublishContext::for_session(session_id);
        let handle = tokio::runtime::Handle::current();
        handle.spawn(async move {
            if let Err(error) = publisher
                .publish(
                    context,
                    crate::event::EventKind::ActivityV2(Box::new(event)),
                )
                .await
            {
                tracing::debug!(
                    target: "agena::session::activity_v2",
                    session_id,
                    error = %error,
                    "failed to broadcast activity v2 live event"
                );
            }
        });
        Ok(())
    }

    pub(in crate::session::manager) async fn refresh_streaming_title(
        &self,
        session_id: i64,
        pending_tool: &SessionPendingTool,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let mut session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        let tool_part_ref = session
            .resolve_part_ref(&pending_tool.part)
            .ok_or_else(|| pending_tool_part_not_found_error(&pending_tool.part))?;
        let refreshed_title = {
            let tool_part = session
                .part_mut(&tool_part_ref)
                .ok_or_else(|| pending_tool_part_not_found_error(&pending_tool.part))?;
            if matches!(
                tool_part.status,
                ExecutionStatus::Pending | ExecutionStatus::InProgress
            ) {
                tool_part.status = ExecutionStatus::InProgress;
            }
            if let Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(
                operation,
            ))) = tool_part.content.as_mut()
            {
                let elapsed_secs = if operation.lifecycle.start_ms > 0 {
                    (Utc::now().timestamp_millis() - operation.lifecycle.start_ms) / 1000
                } else {
                    0
                };
                if elapsed_secs >= 1 {
                    let base_title = operation
                        .metadata
                        .get("agena_streaming_base_title")
                        .and_then(|value| value.as_str())
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| operation.title.clone());
                    operation.metadata.insert(
                        "agena_streaming_base_title".to_owned(),
                        serde_json::json!(base_title),
                    );
                    operation.set_title(format!("{base_title} · {elapsed_secs}s"));
                    Some(operation.title.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };
        // Persist the title with a targeted column update (no content/payload
        // rewrite), then broadcast a header checkpoint so the terminal sees
        // the live title change. The Activity id lets the tiny UPDATE target
        // the content-node title column directly.
        if let Some(title) = refreshed_title {
            let activity_id = session
                .part(&tool_part_ref)
                .and_then(|part| part.activity_id);
            self.store
                .update_part_title(
                    session_id,
                    pending_tool.part.message_id,
                    pending_tool.part.part_id,
                    activity_id,
                    &title,
                )
                .await?;
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

    /// Persist a bounded preview of the streamed output into the Operation at
    /// stream end. The text was buffered in memory during the stream; this
    /// bounds the model preview (truncated for context economy) and writes a
    /// single checkpoint instead of re-persisting cumulative text. The final
    /// `apply_tool_success` replaces this preview with the tool's own truncated
    /// result, so this is only a crash-recovery / TUI intermediate view.
    pub(in crate::session::manager) async fn apply_streaming_terminal_output(
        &self,
        session_id: i64,
        pending_tool: &SessionPendingTool,
        streamed_output: &str,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        if streamed_output.is_empty() {
            return self
                .store
                .load_session(session_id, state.cache_policy())
                .await;
        }
        // Bound the intermediate preview so a giant stream is not written in
        // full even once; the terminal frame carries the real truncated output.
        let preview = agena_runtime_tools::truncate_tool_output_text(streamed_output, 16 * 1024);
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
            if !tool_part.append_tool_output_delta(preview.as_str()) {
                return Err(AppError::Internal(format!(
                    "streaming tool part refused terminal output: message={}, part={}",
                    pending_tool.part.message_id, pending_tool.part.part_id
                )));
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
        let mut tool_output = execution.output.clone();
        // The executor compacts the model-visible payload when it exceeds the
        // model boundary, but the user-facing Operation must keep the complete
        // result. apply_patch carries the full diff outside the compacted
        // payload; restore it so the terminal renders the real diff instead of
        // the truncated one.
        if let Some(apply_patch) = execution.apply_patch.as_ref()
            && let Some(mut payload) = tool_output.to_json_payload()
            && let Some(object) = payload.as_object_mut()
        {
            object.insert("diff".to_owned(), serde_json::json!(apply_patch.diff));
            object.insert(
                "inverse_patch".to_owned(),
                serde_json::json!(apply_patch.inverse_patch),
            );
            object.insert(
                "progress".to_owned(),
                serde_json::json!(apply_patch.progress),
            );
            if let Ok(restored) = agena_domain::ToolOutput::from_json_payload(Some(&payload)) {
                tool_output = restored;
            }
        }
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
                agena_tool::compose_tool_title(resolved.invocation.name.as_str(), execution_title)
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

#[cfg(test)]
mod should_continue_turn_tests {
    use super::{ModelTurnOutcome, TurnContinuation, should_continue_turn};
    use agena_domain::FinishReason;

    fn outcome(finish_reason: FinishReason, follow_up_requested: bool) -> ModelTurnOutcome {
        ModelTurnOutcome {
            follow_up_requested,
            finish_reason,
        }
    }

    fn decide(
        outcome: &ModelTurnOutcome,
        has_pending_tools: bool,
    ) -> (TurnContinuation, usize, usize) {
        let mut truncation_budget = 4;
        let mut follow_up_budget = 4;
        let continuation = should_continue_turn(
            outcome,
            has_pending_tools,
            &mut truncation_budget,
            &mut follow_up_budget,
        );
        (continuation, truncation_budget, follow_up_budget)
    }

    #[test]
    fn pending_tools_is_the_primary_signal_and_takes_priority() {
        // Pending tools always continue, regardless of finish reason or
        // end_turn, and consume no budget.
        for reason in [
            FinishReason::Stop,
            FinishReason::ToolCalls,
            FinishReason::MaxTokens,
            FinishReason::ContentFilter,
            FinishReason::Error,
            FinishReason::Other,
        ] {
            let (continuation, truncation_budget, follow_up_budget) =
                decide(&outcome(reason, false), true);
            assert_eq!(
                continuation,
                TurnContinuation::PendingTools,
                "reason={reason:?}"
            );
            assert_eq!(truncation_budget, 4);
            assert_eq!(follow_up_budget, 4);
        }
        let (continuation, _, _) = decide(&outcome(FinishReason::Stop, true), true);
        assert_eq!(continuation, TurnContinuation::PendingTools);
    }

    #[test]
    fn plain_text_completion_stops() {
        let (continuation, _, _) = decide(&outcome(FinishReason::Stop, false), false);
        assert_eq!(continuation, TurnContinuation::Stop);
        let (continuation, _, _) = decide(&outcome(FinishReason::ToolCalls, false), false);
        assert_eq!(continuation, TurnContinuation::Stop);
    }

    #[test]
    fn abnormal_finish_reasons_stop_without_tools() {
        for reason in [
            FinishReason::ContentFilter,
            FinishReason::Error,
            FinishReason::Other,
        ] {
            let (continuation, _, _) = decide(&outcome(reason, false), false);
            assert_eq!(continuation, TurnContinuation::Stop, "reason={reason:?}");
        }
    }

    #[test]
    fn end_turn_false_continues_under_budget() {
        let (continuation, _, follow_up_budget) = decide(&outcome(FinishReason::Stop, true), false);
        assert_eq!(continuation, TurnContinuation::FollowUpRequested);
        assert_eq!(follow_up_budget, 3, "follow-up budget decremented");
    }

    #[test]
    fn end_turn_false_budget_exhaustion_stops() {
        let mut truncation_budget = 4;
        let mut follow_up_budget = 0;
        let outcome = outcome(FinishReason::Stop, true);
        let continuation = should_continue_turn(
            &outcome,
            false,
            &mut truncation_budget,
            &mut follow_up_budget,
        );
        assert_eq!(continuation, TurnContinuation::Stop);
    }

    #[test]
    fn truncated_continues_under_budget() {
        let (continuation, truncation_budget, _) =
            decide(&outcome(FinishReason::MaxTokens, false), false);
        assert_eq!(continuation, TurnContinuation::Truncated);
        assert_eq!(truncation_budget, 3, "truncation budget decremented");
    }

    #[test]
    fn truncation_budget_exhaustion_stops() {
        let mut truncation_budget = 0;
        let mut follow_up_budget = 4;
        let outcome = outcome(FinishReason::MaxTokens, false);
        let continuation = should_continue_turn(
            &outcome,
            false,
            &mut truncation_budget,
            &mut follow_up_budget,
        );
        assert_eq!(continuation, TurnContinuation::Stop);
    }

    #[test]
    fn end_turn_false_takes_priority_over_truncation() {
        // An explicit end_turn=false is a stronger "keep working" signal than a
        // max-tokens truncation; it is honored first and consumes follow-up
        // budget rather than truncation budget.
        let (continuation, truncation_budget, follow_up_budget) =
            decide(&outcome(FinishReason::MaxTokens, true), false);
        assert_eq!(continuation, TurnContinuation::FollowUpRequested);
        assert_eq!(truncation_budget, 4, "truncation budget not consumed");
        assert_eq!(follow_up_budget, 3);
    }

    #[test]
    fn truncated_prompt_is_non_empty() {
        assert!(!super::TRUNCATED_CONTINUATION_PROMPT.is_empty());
    }
}

