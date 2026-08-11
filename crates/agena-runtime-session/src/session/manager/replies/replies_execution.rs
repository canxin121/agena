use super::{
    AggregatedPermissionOutcome, AggregatedPermissionRequest, AppError, Arc, ExecutionControl,
    OperationPart, PersistedPermissionRule, PromptRequestOptions, PromptTurnBudget,
    ProviderPromptAnchor, ResolvedPendingTool, SessionManager, SessionManagerState,
    SessionPendingTool, SessionRunOptions, SessionRunRequest, SessionRunTermination,
    StreamingToolExecution, ToolError, ToolInvocationExecution, ToolPermissionCheck, Utc,
    ask_user_title, assistant_message_id, completed_lifecycle, execution_control_to_app_error,
    is_authorization_phase_title, operation_authorization, operation_blocks_from_tool_output,
    operation_permission_approved_actions, pending_operation_for_resolved,
    pending_tool_part_not_found_error, permission_action_key, push_unique_permission_action,
    resolve_pending_tool, responses_api_request_metadata, run_abort_reason,
    should_execute_pending_tools_concurrently, terminal_operation_title, tool_name,
    update_resolved_tool_message,
};
use crate::part::{InteractiveRequestPart, RequestPart};
use crate::session::Session;
use crate::session::prompt_window;
use crate::session::store::{
    interaction_from_request, new_part_from_content, run_marker_content, text_content,
    tool_call_from_operation, typed_content_from_value, typed_content_to_value,
};
use crate::tool::ToolExecutor;
use agena_domain::{UserInputKind, UserInputRequest};
use agena_domain::{
    DecisionTraceStep, ExecutionPhase, ExecutionSource, FinishReason, PermissionAction,
    PermissionDecision, PermissionRequest, PermissionScope, PolicySourceKind,
    PromptCompactionTrigger, RunAbortReason,
};
use agena_runtime_contracts::part_content::{InteractionContent, TypedContent, operation_from_tool_call};
use agena_storage::store::{Part, PartRole, PartState};
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

/// The id (run-marker part_id) of the most recent user-authored message in
/// the session, if any. Used to detect new steer input across model turns.
fn last_user_message_id(parts: &[Part]) -> Option<i64> {
    parts
        .iter()
        .rev()
        .find(|part| part.is_run_marker() && part.role == PartRole::User)
        .map(|marker| marker.part_id)
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
    scoped_executor: ToolExecutor,
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

/// Outcome of the shared preflight chain for one pending tool.
struct PreparedToolPreflight {
    resolved: ResolvedPendingTool,
    /// Shell-prepared invocation used for the concurrency-safety check and
    /// invocation-changed detection.
    prepared_invocation: agena_domain::ToolInvocation,
    permission_checks: Vec<ToolPermissionCheck>,
    session_changed: bool,
}

/// Error split so callers keep their own preflight semantics: batch defers
/// non-cancelled tool errors to the sequential path, while the sequential
/// path wraps them in `PendingToolPreparationError`.
enum PendingToolPreflightError {
    Session(AppError),
    Tool(ToolError),
}

impl From<ToolError> for PendingToolPreflightError {
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
        _session_id: i64,
        _pending: &ResolvedPendingTool,
    ) -> agena_tool::ToolRuntimeEventSink {
        // v2 (design 14): the event bus and its `EventKind::CommandBegin/Delta/
        // End` envelopes are gone. Command progress is carried by the streamed
        // tool part's own deltas and the live ActivityV2 bridge; there is
        // nothing durable to publish here. The sink is kept as a no-op so the
        // tool executor's `with_command_event_sink` contract still type-checks.
        Arc::new(|_event| {})
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
        // Backoff before retrying a failed model turn through an agent.stop
        // continuation. Grows geometrically up to a cap so a persistently
        // failing run does not hammer the provider with immediate retries.
        let mut failure_retry_backoff_ms: u64 = 250;
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
        let mut observed_user_message_id = last_user_message_id(session.parts());
        // The assistant run marker for the current turn (one user message ==
        // one run marker). Created on the turn's first model turn and reused by
        // every subsequent model turn (tool results, follow-ups, truncations)
        // so all of one reply's parts persist under a single run marker.
        let mut turn_run_id: Option<i64> = None;

        // Turn-scoped lease heartbeat. A stable run can spend many seconds
        // between database commits — a slow reasoning stream, a multi-second
        // tool execution, a long permission wait — far past
        // `LEASE_STALENESS_MS`. Without a heartbeat the next commit treats the
        // run as stale, steals the lease, and aborts the in-flight run
        // mid-stream (`lease_stolen`). The heartbeat extends the run's
        // ownership every half-window and is aborted on every exit path via
        // its Drop guard. If the lease was already stolen by another owner the
        // heartbeat stops and the next commit surfaces the conflict
        // authoritatively.
        struct LeaseHeartbeatGuard {
            task: tokio::task::JoinHandle<()>,
        }
        impl Drop for LeaseHeartbeatGuard {
            fn drop(&mut self) {
                self.task.abort();
            }
        }
        let heartbeat_store = Arc::clone(&self.store);
        let heartbeat_session_id = session.id;
        let heartbeat_task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(
                agena_storage::store::LEASE_STALENESS_MS as u64 / 2,
            ));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // Skip the immediate first tick: the run just committed and the
            // lease is fresh.
            tick.tick().await;
            loop {
                tick.tick().await;
                if !heartbeat_store.heartbeat_lease(heartbeat_session_id).await {
                    tracing::warn!(
                        session_id = heartbeat_session_id,
                        "stable-run lease heartbeat stopped: no lease held by this owner"
                    );
                    break;
                }
                tracing::debug!(
                    target: "agena::session::lease",
                    session_id = heartbeat_session_id,
                    "stable-run lease heartbeat extended"
                );
            }
        });
        let _heartbeat_guard = LeaseHeartbeatGuard {
            task: heartbeat_task,
        };
        loop {
            let current_options = self
                .apply_execution_context_to_run_options_async(&session, options.clone())
                .await?;
            if control.cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }

            session = self
                .drain_steer_input(session, &mut steer_rx, &current_options, state.clone())
                .await?;

            let latest_user = last_user_message_id(session.parts());
            if latest_user != observed_user_message_id {
                active_model_turn_id = latest_user;
                observed_user_message_id = latest_user;
                model_requested = true;
                // New steer input starts a new turn: the next model turn opens a
                // fresh assistant run marker instead of continuing the previous
                // turn's marker.
                turn_run_id = None;
            }

            let mut current_options = self
                .apply_execution_context_to_run_options_async(&session, options.clone())
                .await?;
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
                session = self.store.load_session(session.id).await?;
                session.refresh_derived();
                if !session.blocked() {
                    continue;
                }

                tokio::select! {
                    biased;
                    _ = control.cancel.cancelled() => return Err(AppError::Cancelled),
                    _ = control.wait_for_interaction_after(interaction_epoch) => {}
                }
                session = self.store.load_session(session.id).await?;
                continue;
            }

            if let Some(hit) = crate::session::doom_loop::detect(
                session.active_window_parts(),
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
                let pending_before = pending_tools
                    .iter()
                    .map(|pending| pending.part.part_id)
                    .collect::<Vec<_>>();
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
                // Drain and record hook runs observed while the tool batch
                // executed (command.before/after, and any hook the tool itself
                // triggered) before the next model turn, so they land at the
                // tool-call position instead of piling up at the end.
                let hook_runs = state
                    .tool_executor
                    .plugin_manager()
                    .drain_hook_runs(session.id);
                if !hook_runs.is_empty() {
                    session = self
                        .record_hook_runs(session, hook_runs, state.clone())
                        .await?;
                }
                let pending_after = session
                    .pending_tools()
                    .into_iter()
                    .map(|pending| pending.part.part_id)
                    .collect::<Vec<_>>();
                if !session.blocked() && pending_after == pending_before {
                    // A pending-tool pass must either complete at least one
                    // operation or install a blocking interaction. Returning
                    // to the top with the identical pending set creates a
                    // ready-future busy loop that can pin a Tokio worker at
                    // 100% CPU while the lease heartbeat makes the run look
                    // healthy forever. Fail closed instead of spinning even
                    // if a future lifecycle regression misclassifies a part.
                    return Err(AppError::Internal(format!(
                        "tool resolution made no progress for session {} (pending call ids: {:?})",
                        session.id, pending_after
                    )));
                }
                model_requested = !session.blocked() && pending_after.is_empty();
                continue;
            }

            if !model_requested && !force_model_retry {
                let last_assistant_text = crate::session::store::parts_into_runs(session.parts())
                    .into_iter()
                    .rev()
                    // Only real assistant-authored runs count as the last
                    // assistant text: hook-only System-originated runs carry no
                    // body and must not feed the stop input.
                    .find(|run| {
                        run.first().is_some_and(|marker| {
                            marker.is_run_marker()
                                && marker.role == PartRole::Assistant
                                && marker.content.get("source").and_then(serde_json::Value::as_str)
                                    == Some("tool")
                        })
                    })
                    .map(|run| crate::provider::project_session_text_lossy(&run));
                let stop_input = agena_plugin_host::AgentStopInput {
                    session_id: session.id,
                    stop_hook_active: false,
                    last_assistant_message: last_assistant_text.clone(),
                    run_error: None,
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
                        // continuation) actually fired. The dispatch pushed
                        // HookRunRecords for agent.stop (plus any lingering
                        // command.before/after runs from the final tool
                        // batch); drain and record them here.
                        let hook_runs = state
                            .tool_executor
                            .plugin_manager()
                            .drain_hook_runs(session.id);
                        if !hook_runs.is_empty() {
                            session = self
                                .record_hook_runs(session, hook_runs, state.clone())
                                .await?;
                        }
                        if dispatch.patch.continue_with_message.is_some() {
                            let follow_up =
                                dispatch.patch.continue_with_message.unwrap_or_default();
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
                        // The dispatch pushed a HookRunRecord for the failed
                        // transport; drain and record it so the stop failure is
                        // visible instead of being misattributed to the next run.
                        let hook_runs = state
                            .tool_executor
                            .plugin_manager()
                            .drain_hook_runs(session.id);
                        if !hook_runs.is_empty() {
                            session = self
                                .record_hook_runs(session, hook_runs, state.clone())
                                .await?;
                        }
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
                let budget_error = AppError::ModelTurnBudgetExhausted { max_turns };
                // Budget exhaustion is a run failure, not a silent soft stop.
                // Surface it to agent.stop hooks via `run_error` so the plan
                // autorun (or another hook) can decide to continue. When a
                // hook continues, the run is granted a fresh turn budget
                // (`model_turns_taken` resets) so it keeps working instead of
                // immediately re-triggering the budget check.
                if let Some(continued) = self
                    .dispatch_run_failure_continuation(
                        session,
                        &current_options,
                        state.clone(),
                        control.clone(),
                        &budget_error,
                        &mut failure_retry_backoff_ms,
                    )
                    .await?
                {
                    session = continued;
                    model_turns_taken = 0;
                    model_requested = true;
                    continue;
                }
                return Err(budget_error);
            }
            model_turns_taken += 1;

            control
                .transition(ExecutionPhase::PreparingModel)
                .await
                .map_err(execution_control_to_app_error)?;

            // The compaction marker is appended after the boundary run, so an
            // already-compacted boundary is exactly a trailing compaction
            // checkpoint part (the window it closes is empty of new input).
            let already_auto_compacted_at_boundary = session.parts().last().is_some_and(|part| {
                part.kind == "run"
                    && part
                        .content
                        .get("run_kind")
                        .and_then(serde_json::Value::as_str)
                        == Some("compaction")
            });
            let session_usage = self.session_usage_async(&session).await?;
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
                session = Box::pin(self.automatic_compact_session(
                    session,
                    &current_options,
                    PromptCompactionTrigger::Auto,
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
            let message_count = crate::session::store::parts_into_runs(session.parts()).len();
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

            // One user message == one run marker (turn-scoped runs). The first
            // model turn of a stable run starts the marker (durable before the
            // provider call, design 17.4); every later model turn of the same
            // turn reuses it. The marker's current content (accumulated round
            // records, usage) is threaded into the processor so it can extend
            // the round list.
            let marker_run_id = match turn_run_id {
                Some(run_id) => run_id,
                None => {
                    let run_id = self
                        .store
                        .start_run(
                            session.id,
                            "continue",
                            crate::session::store::run_marker_content(
                                "continue",
                                Some(current_options.model.provider_id.as_ref()),
                                Some(current_options.model.model_id.as_ref()),
                                Some(control.turn_id()),
                                Some(control.reply_id()),
                            ),
                        )
                        .await?;
                    turn_run_id = Some(run_id);
                    run_id
                }
            };
            let marker_content = session
                .parts()
                .iter()
                .find(|part| part.part_id == marker_run_id)
                .map(|marker| marker.content.clone());

            match Box::pin(self.run_model_turn(
                session,
                &current_options,
                base_run_source,
                marker_run_id,
                marker_content,
                state.clone(),
                control.clone(),
            ))
            .await
            {
                Ok((next_session, outcome, marker_run_id)) => {
                    session = next_session;
                    failure_retry_backoff_ms = 250;
                    model_requested = false;
                    turn_run_id = Some(marker_run_id);
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
                            .parts()
                            .iter()
                            .rev()
                            .find(|part| part.kind == "run" && part.role == PartRole::Assistant)
                            .map(|marker| marker.part_id);
                    }
                    reactive_compaction_attempted = false;
                    let post_run_input = agena_plugin_host::PostRunInput {
                        session_id: session.id,
                        model,
                        status: format!("{:?}", session.workflow_state()),
                        message_count: crate::session::store::parts_into_runs(session.parts())
                            .len(),
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
                        let reloaded = self.store.load_session(session_id).await?;
                        let generation = reloaded.runtime.prompt_window.generation;
                        let compacted = Box::pin(self.automatic_compact_session(
                            reloaded,
                            &current_options,
                            PromptCompactionTrigger::Reactive,
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
                    // A failed model turn is retryable when an agent.stop
                    // hook (for example the workflow plan autorun) asks to
                    // continue. Surface the error to the hook so it can
                    // decide, and rate-limit retries with backoff.
                    if control.cancel.is_cancelled() {
                        return Err(AppError::Cancelled);
                    }
                    session = self.store.load_session(session_id).await?;
                    if let Some(continued) = self
                        .dispatch_run_failure_continuation(
                            session,
                            &current_options,
                            state.clone(),
                            control.clone(),
                            &err,
                            &mut failure_retry_backoff_ms,
                        )
                        .await?
                    {
                        session = continued;
                        model_requested = true;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }

    /// Surface a run failure to agent.stop hooks and, if a hook asks to
    /// continue (for example the workflow plan autorun), inject the
    /// continuation message. Returns `Some(session)` when the run should
    /// continue after backoff; `None` when the run should fail with `error`.
    ///
    /// Shared by the model-turn error path and the model-turn budget
    /// exhaustion path so both treat run errors uniformly: the error is
    /// surfaced to hooks via `run_error`, and only an explicit hook
    /// continuation keeps the run alive.
    async fn dispatch_run_failure_continuation(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
        control: Arc<ExecutionControl>,
        error: &AppError,
        retry_backoff_ms: &mut u64,
    ) -> Result<Option<Session>, AppError> {
        let run_error = error.public_message();
        let stop_input = agena_plugin_host::AgentStopInput {
            session_id: session.id,
            stop_hook_active: false,
            last_assistant_message: None,
            run_error: Some(run_error.to_string()),
        };
        session = self.store.load_session(session.id).await?;
        match state
            .tool_executor
            .plugin_manager()
            .dispatch_agent_stop_cancellable(stop_input, Some(control.cancel.clone()))
            .await
        {
            Ok(dispatch) => {
                let hook_runs = state
                    .tool_executor
                    .plugin_manager()
                    .drain_hook_runs(session.id);
                if !hook_runs.is_empty() {
                    session = self
                        .record_hook_runs(session, hook_runs, state.clone())
                        .await?;
                }
                if let Some(follow_up) = dispatch.patch.continue_with_message {
                    session = self
                        .inject_continuation_message(session, options, follow_up, state.clone())
                        .await?;
                    let delay = *retry_backoff_ms;
                    *retry_backoff_ms = (*retry_backoff_ms * 2).min(5_000);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    Ok(Some(session))
                } else {
                    Ok(None)
                }
            }
            Err(dispatch_err) => {
                if control.cancel.is_cancelled() {
                    return Err(AppError::Cancelled);
                }
                tracing::warn!(
                    target: "agena_plugin_host::agent_stop",
                    "agent.stop hook failed after run error: {dispatch_err}"
                );
                let hook_runs = state
                    .tool_executor
                    .plugin_manager()
                    .drain_hook_runs(session.id);
                if !hook_runs.is_empty() {
                    let _ = self
                        .record_hook_runs(session, hook_runs, state.clone())
                        .await?;
                }
                Ok(None)
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
        _options: &SessionRunOptions,
        text: String,
        _state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        // v2 (design 4.1): a system-originated continuation is a User run
        // (marker + text part) submitted through the facade, exactly like a
        // steer input. v1 carried no idempotency key, so none is used here.
        let new_parts = vec![new_part_from_content(
            "text",
            PartRole::User,
            &TypedContent::Text(text_content(text)),
            PartState::Completed,
        )?];
        let outcome = self
            .store
            .submit_user_run(session.id, new_parts, None)
            .await?;
        if outcome.created {
            let mut projected = session.parts().to_vec();
            projected.extend(outcome.parts);
            session.install_projected_parts(projected);
        }
        Ok(session)
    }

    /// Record one System-originated Assistant message with a `Hook` part per
    /// observed hook run. This makes hook execution visible in the same
    /// transcript activity pipeline as tool calls and is persisted so a
    /// restart keeps the record. Every session operation that drains
    /// `PluginHost::drain_hook_runs` (session.start, user.prompt.submit,
    /// chat.params, command.before/after, agent.stop) records through here.
    ///
    /// v2 (design 4.1): hook parts are ordinary parts under an assistant run
    /// marker (kind `hook`, run_kind `execution`), persisted through the
    /// facade like any other message; there is no separate content-node
    /// projection to mirror into.
    pub(in crate::session::manager) async fn record_hook_runs(
        &self,
        mut session: Session,
        runs: Vec<agena_plugin_host::HookRunRecord>,
        _state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        // v2 (design 4.1): start an assistant-side run marker (`run_kind`
        // `execution`) and append one `hook` part per observed run through the
        // facade, then extend the projection with the marker + created parts.
        let run_marker_content_value = run_marker_content("execution", None, None, None, None);
        let run_id = self
            .store
            .start_run(session.id, "execution", run_marker_content_value.clone())
            .await?;
        let new_parts = runs
            .iter()
            .map(|run| {
                new_part_from_content(
                    "hook",
                    PartRole::Assistant,
                    &TypedContent::Hook(agena_runtime_contracts::part_content::HookContent {
                        hook: run.hook.clone(),
                        plugin_id: Some(run.plugin_id.clone()),
                        summary: run.summary.clone(),
                        detail: run.detail.clone(),
                        extra: Default::default(),
                    }),
                    PartState::Completed,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let created = self
            .store
            .append_parts(session.id, run_id, new_parts)
            .await?;
        // `start_run` returns only the marker id; rebuild the marker in the
        // projection exactly as the engine created it (facade `start_run`
        // maps `run_kind == "execution"` to the Runtime role, pending state).
        let now_ms = Utc::now().timestamp_millis();
        let marker = Part {
            part_id: run_id,
            kind: "run".to_owned(),
            role: PartRole::Runtime,
            state: PartState::Pending,
            content: run_marker_content_value,
            summary: None,
            visibility: agena_storage::store::PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: None,
            origin_session_id: session.id,
            revision: 1,
            started_at_ms: now_ms,
            finished_at_ms: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            provider_state: None,
        };
        let mut projected = session.parts().to_vec();
        projected.push(marker);
        projected.extend(created);
        session.install_projected_parts(projected);
        Ok(session)
    }

    /// Run one provider model turn. `marker_run_id` is the assistant run marker
    /// this turn appends its parts under — the durable message id for the whole
    /// turn. On the first model turn of a stable run the caller creates the
    /// marker (`start_run`); every subsequent model turn of the same run (tool
    /// results, follow-ups, truncations) reuses it so one user message == one
    /// run marker == one turn (design: turn-scoped runs). `marker_content` is
    /// the marker's current content (carrying any accumulated `rounds` and
    /// `usage`), which the processor extends with this round's record.
    pub(in crate::session::manager) async fn run_model_turn(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        _run_source: ExecutionSource,
        marker_run_id: i64,
        marker_content: Option<serde_json::Value>,
        state: Arc<SessionManagerState>,
        control: Arc<ExecutionControl>,
    ) -> Result<(Session, ModelTurnOutcome, i64), AppError> {
        let run_span = tracing::info_span!(
            "session.run",
            session_id = session.id,
            provider_id = %options.model.provider_id,
            model_id = %options.model.model_id,
        );
        {
            let provider_registry = &state.provider_registry;
            let native_compaction_enabled =
                provider_registry.native_compaction_enabled(&options.model)?;
            let scoped_executor = state
                .tool_executor
                .for_session_context_async(&session.runtime.execution)
                .await;
            let agena_tool_mode = provider_registry.agena_tool_mode(&options.model)?;
            let tool_api_functions = if agena_tool_mode.is_disabled() {
                Vec::new()
            } else {
                scoped_executor.available_tool_api_bindings_async().await
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
            let provider_request_shape = state.provider_registry.prompt_cache_shape(&options.model)?;
            let continuation_supported = state
                .provider_registry
                .supports_prompt_continuation(&options.model)
                .unwrap_or(false);
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
            let active_window_parts = session.active_window_parts();
            let prompt_exceeds_runtime_budget = prompt_window::estimate_prompt_tokens_from_runtime(
                &session,
                active_window_parts,
                prompt_fingerprints.system_fingerprint.as_str(),
                prompt_fingerprints.request_options_fingerprint.as_str(),
            )
            .is_some_and(|estimate| estimate.total_tokens > prompt_budget.max_prompt_tokens);
            if prompt_exceeds_runtime_budget
                || state.context_governor.prompt_exceeds_budget(
                    prompt_window::approximate_prompt_payload_chars(active_window_parts),
                    prompt_budget.max_prompt_chars,
                )
            {
                tracing::warn!(
                    session_id = session.id,
                    prompt_message_count = active_window_parts.len(),
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
                prompt_message_count = prepared.turns.len(),
                system_included = prepared.system.is_some(),
                "prepared prompt for session run"
            );

            // The assistant message's durable id is the run marker's part id
            // (started below). v2 has no placeholder part allocator: the
            // processor appends parts with placeholder ids of its own, which
            // the adapter remaps to engine ids on append.
            let run_id = agena_domain::RunId::new();
            let turn_started_at_unix_ms = Utc::now().timestamp_millis();
            let mut completion = super::super::completion_request(
                options,
                prepared.system.clone(),
                prepared.turns.clone(),
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
            // The run marker (started by the caller for the turn's first model
            // turn) is the assistant message's durable id (design 4.1: a
            // message == one run). Every model turn of one stable run appends
            // its parts under the same marker, so a tool-calling reply persists
            // as exactly one assistant run holding all its parts.
            let run = SessionRunRequest {
                session_id: session.id,
                model: options.model.clone(),
                completion,
                next_message_id: marker_run_id,
                marker_content,
                part_ids: Default::default(),
                next_call_id: session.next_call_id(),
                // R2: the processor persists this turn's parts itself through
                // the facade-backed store — the only durable write source.
                store: self.store.clone(),
                cancel: Some(control.cancel.clone()),
            };

            // `SessionProcessor` is the sole owner of cooperative model-stream
            // cancellation. Never race it with an outer select: dropping this
            // future would skip message and part terminalization.
            control
                .transition(ExecutionPhase::StreamingModel)
                .await
                .map_err(execution_control_to_app_error)?;
            let run_outcome = state
                .processor
                .run_turn(run, &state.provider_registry)
                .instrument(run_span.clone())
                .await;
            match run_outcome {
                Ok(result) => {
                    // Drain hook runs collected during this model turn
                    // (chat.params fired at the turn start, plus any lingering
                    // command.before/after runs from the previous tool batch)
                    // and record them before this turn's assistant message.
                    let hook_runs = state
                        .tool_executor
                        .plugin_manager()
                        .drain_hook_runs(session.id);
                    if !hook_runs.is_empty() {
                        session = self
                            .record_hook_runs(session, hook_runs, state.clone())
                            .await?;
                    }
                    let termination = result.termination;
                    // R2/T6: the processor persisted this turn entirely through
                    // parts. The transcript digest covers the active window plus
                    // this turn's freshly persisted run (marker + child parts).
                    // Nothing here re-persists the turn — parts are the only
                    // durable write source.
                    let transcript_digest = {
                        let mut transcript_parts = session.active_window_parts().to_vec();
                        transcript_parts.push(result.run_marker.clone());
                        transcript_parts.extend(result.parts.iter().cloned());
                        prompt_window::prompt_transcript_digest(transcript_parts.as_slice())
                    };
                    let anchored_provider_request_shape = match state
                        .provider_registry
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
                    if let Some(usage) = result.usage.as_ref() {
                        session.runtime.record_prompt_tokens(
                            result.assistant_message_id,
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
                            assistant_message_id: result.assistant_message_id,
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

                    // R2: the processor already persisted this turn through
                    // parts, so this is projection installation only — never
                    // a second write. The stable-run loop operates on this
                    // in-memory Session immediately after `run_model_turn`;
                    // omitting the freshly persisted marker/children makes it
                    // believe a tool-calling turn has no pending tools and
                    // finish the execution while the database is left with a
                    // pending `tool_call`. Upsert the authoritative engine
                    // rows returned by the processor, preserving hook parts
                    // that may have been recorded above, then restore the same
                    // ordering used by the facade load path.
                    let mut projected = session.parts().to_vec();
                    for updated in std::iter::once(&result.run_marker).chain(result.parts.iter()) {
                        if let Some(existing) = projected
                            .iter_mut()
                            .find(|part| part.part_id == updated.part_id)
                        {
                            *existing = updated.clone();
                        } else {
                            projected.push(updated.clone());
                        }
                    }
                    projected.sort_by_key(|part| (part.created_at_ms, part.part_id));
                    session.install_projected_parts(projected);

                    match termination {
                        SessionRunTermination::Completed => Ok((
                            session,
                            ModelTurnOutcome {
                                follow_up_requested: result.follow_up_requested,
                                finish_reason: result.finish_reason,
                            },
                            marker_run_id,
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
                    // Drain hook runs observed while the provider call failed
                    // (for example chat.params that fired before the transport
                    // error) and record them before the run marker is
                    // terminalized below. Recording failures are swallowed so
                    // the original run error is preserved.
                    let session_id = session.id;
                    let hook_runs = state
                        .tool_executor
                        .plugin_manager()
                        .drain_hook_runs(session_id);
                    if !hook_runs.is_empty() {
                        match self
                            .record_hook_runs(session, hook_runs, state.clone())
                            .await
                        {
                            Ok(_recorded) => {}
                            Err(record_err) => {
                                tracing::warn!(
                                    target: "agena::session::hook_runs",
                                    session_id,
                                    "failed to record hook runs after failed model turn: {record_err}"
                                );
                            }
                        }
                    }
                    // Terminalize the run marker started above (design 17.4).
                    // A provider can fail before yielding a stream, so the
                    // processor has no opportunity to build content. Persist a
                    // safe Error part first; otherwise the TUI can only render
                    // an empty, non-expandable "Response failed" lifecycle row.
                    // A user-cancelled run remains cancellation-only.
                    if matches!(reason, RunAbortReason::UserCancelled) {
                        self.store
                            .cancel_run(session_id, marker_run_id)
                            .await
                            .or_else(|error| {
                                tracing::warn!(
                                    target: "agena::session::run",
                                    session_id,
                                    "failed to cancel the interrupted run marker: {error}"
                                );
                                Ok::<(), AppError>(())
                            })?;
                    } else {
                        if let Err(error) = self
                            .store
                            .append_failure_part(session_id, marker_run_id, &failure)
                            .await
                        {
                            tracing::warn!(
                                target: "agena::session::run",
                                session_id,
                                "failed to persist interrupted run detail: {error}"
                            );
                        }
                        self.store
                            .complete_run(
                                session_id,
                                marker_run_id,
                                agena_storage::store::RunOutcome {
                                    status: agena_storage::store::PartState::Failed,
                                    abort_reason: Some(reason.to_string()),
                                    content: None,
                                    provider_state: None,
                                },
                            )
                            .await
                            .or_else(|error| {
                                tracing::warn!(
                                    target: "agena::session::run",
                                    session_id,
                                    "failed to terminalize the interrupted run marker: {error}"
                                );
                                Ok::<(), AppError>(())
                            })?;
                    }
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
        let fallback_budget = state.context_governor.max_prompt_chars();
        let metadata = state
            .provider_registry
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
        let batch_executor = state
            .tool_executor
            .for_session_context_async(&session.runtime.execution)
            .await;
        for pending_tool in pending_tools {
            match Box::pin(self.prepare_pending_tool_batch_member(
                &mut session,
                &pending_tool,
                state.as_ref(),
                &batch_executor,
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
            // parallel execution instead of pending placeholders. Each changed
            // part id is one `update_part` in the batch transition checkpoint.
            let mut changed_part_ids = Vec::new();
            for resolved in &ready_tools {
                let Some(part) = session.part_mut(&resolved.pending.part) else {
                    continue;
                };
                if matches!(part.state, PartState::Pending | PartState::InProgress) {
                    part.state = PartState::InProgress;
                    changed_part_ids.push(part.part_id);
                }
            }
            if !changed_part_ids.is_empty() {
                session = Box::pin(self.persist_session_changes(
                    session,
                    changed_part_ids,
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
            session = self.store.load_session(session.id).await?;
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
                    .is_some_and(|part| part.state.is_in_flight())
                {
                    session =
                        Box::pin(self.resolve_pending_tool(session, pending_tool, state.clone()))
                            .await?;
                }
            }
        }

        Ok(session)
    }

    /// Shared single-tool preflight: validate the advertised identity, prepare
    /// the invocation and shell command, collect permission checks, and
    /// rewrite the persisted operation preview when the invocation changed.
    /// Callers keep their own error semantics (batch defers non-cancelled tool
    /// failures to the sequential path; the sequential path routes them
    /// through `PendingToolPreparationError`).
    async fn prepare_pending_tool_preflight(
        &self,
        session: &mut Session,
        mut resolved: ResolvedPendingTool,
        scoped_executor: &ToolExecutor,
    ) -> Result<PreparedToolPreflight, PendingToolPreflightError> {
        let invocation = resolved.invocation.clone();
        let original_invocation = invocation.clone();
        let advertised_identity = resolved.advertised_tool_identity.clone();
        let call_id = resolved.call_id;
        let session_id = session.id;
        let (prepared, prepared_invocation, prepared_shell_command, permission_checks) = {
            scoped_executor
                .validate_advertised_tool_identity(&invocation, advertised_identity.as_deref())?;
            let prepared = scoped_executor
                .prepare_invocation(&invocation, session_id, call_id)
                .await?;
            let (prepared_invocation, prepared_shell_command) = scoped_executor
                .prepare_shell_invocation(&prepared.invocation, session_id, call_id)
                .await?;
            let permission_checks = scoped_executor
                .collect_permission_checks_for_invocation_in_session(
                    &prepared_invocation,
                    Some(session_id),
                )
                .await?;
            (prepared, prepared_invocation, prepared_shell_command, permission_checks)
        };
        let invocation_changed = prepared_invocation != original_invocation;
        resolved.prepared_shell_command = prepared_shell_command;
        // The rewritten invocation only feeds the persisted operation preview;
        // execution itself runs against `prepared_shell_command`. Set the
        // pre-rewrite form once — the shell-rewritten value would otherwise be
        // overwritten inside the block below.
        resolved.invocation = prepared.invocation.clone();
        let mut session_changed = false;
        if invocation_changed || prepared.title_override.is_some() {
            let authorization = operation_authorization(session, &resolved);
            let current_title = match session
                .part(&resolved.pending.part)
                .and_then(|part| typed_content_from_value(&part.kind, &part.content).ok())
            {
                Some(TypedContent::ToolCall(tool_call)) => {
                    operation_from_tool_call(&tool_call).title.clone()
                }
                _ => format!("Tool {}", tool_name(&resolved.invocation)),
            };

            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                PendingToolPreflightError::Session(AppError::Internal(format!(
                    "pending tool part not found: part={}",
                    resolved.pending.part.part_id
                )))
            })?;
            tool_part.content = typed_content_to_value(&TypedContent::ToolCall(
                tool_call_from_operation(&pending_operation_for_resolved(
                    &resolved,
                    prepared.invocation,
                    prepared.title_override.unwrap_or(current_title),
                    resolved.lifecycle.clone(),
                    authorization,
                )),
            ))
            .expect("operation content is always JSON serializable");
            session_changed = true;
        }
        Ok(PreparedToolPreflight {
            resolved,
            prepared_invocation,
            permission_checks,
            session_changed,
        })
    }

    async fn prepare_pending_tool_batch_member(
        &self,
        session: &mut Session,
        pending_tool: &SessionPendingTool,
        state: &SessionManagerState,
        batch_executor: &ToolExecutor,
    ) -> Result<PendingToolBatchMember, AppError> {
        self.refresh_execution_policy(session, state);
        let before_prepare = session.clone();
        let resolved = resolve_pending_tool(session, pending_tool)?;
        let cancellation = self.execution_registry.cancellation_token(session.id).await;
        let session_id = session.id;
        let call_id = resolved.call_id;
        let scoped_executor = batch_executor.clone().with_cancellation_token(cancellation);
        let PreparedToolPreflight {
            resolved,
            prepared_invocation,
            permission_checks,
            session_changed: _,
        } = match self
            .prepare_pending_tool_preflight(session, resolved, &scoped_executor)
            .await
        {
            Ok(prepared) => prepared,
            Err(PendingToolPreflightError::Tool(ToolError::Cancelled)) => {
                return Err(AppError::Cancelled);
            }
            Err(PendingToolPreflightError::Tool(err)) => {
                tracing::debug!(
                    target: "agena::session::tools",
                    session_id,
                    call_id,
                    error = %err,
                    "deferring tool preflight error to sequential failure handling"
                );
                *session = before_prepare;
                return Ok(PendingToolBatchMember::Sequential(pending_tool.clone()));
            }
            Err(PendingToolPreflightError::Session(error)) => return Err(error),
        };
        let concurrency_safe = scoped_executor.is_concurrency_safe_invocation(&prepared_invocation);

        let approved_actions = operation_permission_approved_actions(
            session,
            assistant_message_id(session, &resolved.pending.part)?,
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

        if !concurrency_safe {
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
        let batch_executor = match pending_tools.first() {
            Some(pending_tool) => {
                state
                    .tool_executor
                    .for_session_context_async(&pending_tool.session_runtime.execution)
                    .await
            }
            None => return Ok(Vec::new()),
        };
        for pending_tool in pending_tools {
            let command_event_sink =
                self.command_event_sink_for_pending_if_needed(session_id, &pending_tool);
            let scoped_executor = batch_executor
                .clone()
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
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                scoped_executor.validate_advertised_tool_identity(
                    &pending_tool.invocation,
                    pending_tool.advertised_tool_identity.as_deref(),
                )?;
                scoped_executor
                    .execute_invocation_detailed_with_prepared_shell(
                        &pending_tool.invocation,
                        session_id,
                        pending_tool.call_id,
                        pending_tool.prepared_shell_command.clone(),
                    )
                    .await
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

    async fn prepare_pending_tool_execution(
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
        let execution_context = session.runtime.execution.clone();
        let executor = state.tool_executor.clone();
        let scoped_executor = executor
            .for_session_context_async(&execution_context)
            .await
            .with_cancellation_token(cancellation);
        let PreparedToolPreflight {
            resolved,
            prepared_invocation: _,
            permission_checks,
            session_changed,
        } = self
            .prepare_pending_tool_preflight(session, resolved, &scoped_executor)
            .await
            .map_err(|error| match error {
                PendingToolPreflightError::Session(error) => {
                    PendingToolPreparationError::Session(error)
                }
                PendingToolPreflightError::Tool(error) => PendingToolPreparationError::Tool(error),
            })?;

        Ok(PreparedPendingToolExecution {
            resolved,
            scoped_executor,
            permission_checks,
            session_changed,
        })
    }

    /// Route a terminal tool error to its canonical handler. Every execution
    /// path (sequential, parallel, and streaming) must converge here so an
    /// error can never leave the original operation pending.
    async fn route_tool_error(
        &self,
        session: Session,
        pending: &SessionPendingTool,
        error: ToolError,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
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
            session = self.store.load_session(session.id).await?;
        }

        self.route_tool_error(session, pending, error, state).await
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
            scoped_executor,
            permission_checks,
            session_changed,
        } = match self
            .prepare_pending_tool_execution(
                &mut session,
                resolved,
                state.as_ref(),
                cancellation.clone(),
            )
            .await
        {
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

        let approved_actions = operation_permission_approved_actions(
            &session,
            assistant_message_id(&session, &resolved.pending.part)?,
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
                vec![resolved.pending.part.part_id],
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
                let was_pending = matches!(tool_part.state, PartState::Pending);
                if matches!(tool_part.state, PartState::Pending | PartState::InProgress) {
                    tool_part.state = PartState::InProgress;
                }
                was_pending
            })
            .unwrap_or(false);
        if part_was_pending {
            session = Box::pin(self.persist_session_changes(
                session,
                vec![resolved.pending.part.part_id],
                None,
                state.clone(),
            ))
            .await?;
        }

        let scoped_executor = scoped_executor.with_command_event_sink(
            self.command_event_sink_for_pending_if_needed(session.id, &resolved),
        );
        let streaming_tool = match scoped_executor
            .execute_invocation_streaming(&resolved.invocation, session.id, resolved.call_id)
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
        let execution_resolved = resolved.clone();
        let execution_session_id = session.id;
        let _host_user_input_sequence = manager
            .host_user_input_sequence_guard(execution_session_id, execution_resolved.call_id);
        let execution = scoped_executor
            .execute_invocation_detailed_with_prepared_shell(
                &execution_resolved.invocation,
                execution_session_id,
                execution_resolved.call_id,
                execution_resolved.prepared_shell_command.clone(),
            )
            .await;

        let session = self.store.load_session(session.id).await?;
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
                Box::pin(self.apply_tool_success_with_rules(
                    session,
                    pending_tool,
                    execution,
                    Vec::new(),
                    state,
                ))
                .await
            }
            Err(ToolError::UserInputRequired(input)) => {
                let request_id = resolve_pending_tool(&session, pending_tool)?.operation_id;
                Box::pin(self.apply_user_input_request_with_id(
                    session,
                    pending_tool,
                    *input,
                    request_id,
                    state,
                ))
                .await
            }
            Err(error) => self.route_tool_error(session, pending_tool, error, state).await,
        }
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
        mut session: Session,
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
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let request_id = resolved.operation_id.clone();
        let existing_permission_replied = session
            .part(&resolved.pending.part)
            .and_then(|part| typed_content_from_value(&part.kind, &part.content).ok())
            .and_then(|content| match content {
                TypedContent::ToolCall(tool_call) => {
                    let operation = operation_from_tool_call(&tool_call);
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

        update_resolved_tool_message(&mut session, &resolved, |tool_part| {
            let Ok(TypedContent::ToolCall(tool_call)) =
                typed_content_from_value(&tool_part.kind, &tool_part.content)
            else {
                return;
            };
            let mut operation = operation_from_tool_call(&tool_call);
            operation.authorization.push_pending(request.clone());
            operation.set_summary(format!("Awaiting approval · {reason}"));
            tool_part.summary = Some(operation.summary.clone());
            tool_part.content = typed_content_to_value(&TypedContent::ToolCall(
                tool_call_from_operation(&operation),
            ))
            .expect("operation content is always JSON serializable");
            // The part lifecycle is forward-only (17.2): a tool that has
            // already started executing is InProgress and must stay that way
            // while it waits on the host — the store rejects `in_progress ->
            // pending`. The awaiting state is carried by the operation summary
            // (and any interaction part), not by the tool part's lifecycle
            // state, so a not-yet-started Pending tool is left Pending and an
            // executing InProgress tool is not downgraded.
            if !matches!(tool_part.state, PartState::InProgress) {
                tool_part.state = PartState::Pending;
            }
        })?;
        self.persist_session_changes(
            session,
            vec![resolved.pending.part.part_id],
            None,
            state.clone(),
        )
        .await
    }

    pub(in crate::session::manager) async fn apply_user_input_request_with_id(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        input: crate::part::AskUserToolInput,
        request_id: String,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let request = UserInputRequest {
            request_id,
            session_id: Some(session.id),
            title: input.title,
            kind: UserInputKind::from(input.kind),
            auto_resolution_ms: super::super::helpers::effective_user_input_timeout_ms(
                input.auto_resolution_ms,
            ),
            presented_at: None,
            questions: input.questions,
            created_at: Utc::now(),
        };
        let authorization = operation_authorization(&session, &resolved);

        update_resolved_tool_message(&mut session, &resolved, |tool_part| {
            tool_part.content = typed_content_to_value(&TypedContent::ToolCall(
                tool_call_from_operation(&pending_operation_for_resolved(
                    &resolved,
                    resolved.invocation.clone(),
                    ask_user_title(&request),
                    resolved.lifecycle.clone(),
                    authorization.clone(),
                )),
            ))
            .expect("operation content is always JSON serializable");
            // Same forward-only lifecycle rule as permission requests: an
            // executing tool (InProgress) that suspends on a host ask_user
            // stays InProgress — the store rejects `in_progress -> pending`.
            if !matches!(tool_part.state, PartState::InProgress) {
                tool_part.state = PartState::Pending;
            }
            tool_part.summary = Some(match request.questions.len() {
                0 => "Ask user".to_string(),
                1 => "Waiting for answer".to_string(),
                count => format!("Waiting for {count} answers"),
            });
        })?;

        // Present the request as an `interaction` part under the owning run
        // marker (v2): update an already-pending request for the same id in
        // place, otherwise append a fresh part through the facade. The part
        // content is the canonical `interaction` shape (`type`/`prompt`/
        // `options`/`request`). The reply machinery (`find_pending_user_
        // input_by_request_id`, `matching_request_part_refs`) correlates on
        // the flat `request_id`/`tool_part_id`/`operation_id` keys, so mirror
        // them at the top level alongside the canonical keys; the nested
        // request id stays authoritative for the TUI and re-request dedup.
        let mut interaction = interaction_from_request(&RequestPart::UserInput(
            InteractiveRequestPart::pending(request.clone()),
        ));
        interaction.extra.insert(
            "request_id".to_owned(),
            serde_json::to_value(request.request_id.as_str())
                .expect("request id is always JSON serializable"),
        );
        interaction.extra.insert(
            "tool_part_id".to_owned(),
            serde_json::to_value(resolved.pending.part.part_id)
                .expect("part id is always JSON serializable"),
        );
        interaction.extra.insert(
            "operation_id".to_owned(),
            serde_json::to_value(resolved.operation_id.as_str())
                .expect("operation id is always JSON serializable"),
        );
        let interaction_content = interaction.as_value();
        let existing_request_part = session
            .parts()
            .iter()
            .enumerate()
            .find(|(_, part)| {
                part.kind == "interaction"
                    && part.state.is_in_flight()
                    && InteractionContent::try_from(&part.content)
                        .ok()
                        .and_then(|interaction| interaction.request_id())
                        .as_deref()
                        == Some(request.request_id.as_str())
            })
            .map(|(part_index, part)| (part_index, part.part_id));
        let mut changed_part_ids = vec![resolved.pending.part.part_id];
        if let Some((part_index, part_id)) = existing_request_part {
            let part_ref = crate::session::model::SessionPartRef {
                part_index,
                part_id,
            };
            let part = session.part_mut(&part_ref).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending interaction part not found while requesting user input: part={}",
                    part_id
                ))
            })?;
            part.content = interaction_content;
            // Same forward-only lifecycle rule as the tool downgrade above: a
            // re-request must not downgrade an already-in-flight (InProgress)
            // interaction part — the store rejects `in_progress -> pending`.
            if !matches!(part.state, PartState::InProgress) {
                part.state = PartState::Pending;
            }
            changed_part_ids.push(part_id);
        } else {
            let run_marker_id = assistant_message_id(&session, &resolved.pending.part)?;
            // A live awaiting-user part is by definition in flight: born
            // InProgress so the reply can complete via the legal
            // `in_progress -> completed` edge (the store forbids
            // `pending -> completed`).
            let new_part = new_part_from_content(
                "interaction",
                PartRole::Assistant,
                &TypedContent::Interaction(interaction),
                PartState::InProgress,
            )?;
            let created = self
                .store
                .append_parts(session.id, run_marker_id, vec![new_part])
                .await?;
            let mut projected = session.parts().to_vec();
            projected.extend(created);
            session.install_projected_parts(projected);
        }
        self.persist_session_changes(session, changed_part_ids, None, state)
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
        let mut last_title_refresh = std::time::Instant::now();
        let mut streamed_output = String::new();
        // Resolve the Activity id once so live detail broadcasts never need a
        // per-tick session load. v2 parts carry no activity id; the live
        // handler is purely in-memory (broadcast is a no-op bridge), so a
        // fresh id per stream is sufficient.
        let streaming_activity_id = Some(agena_domain::ActivityId::new());
        // Activity v2 live bridge (07 §5.2, §6.1): one in-memory handler feeds
        // the unified wire events from the same text deltas that drive the
        // streamed tool part. Events are broadcast live (a no-op bridge in v2;
        // P5 re-homes onto the facade notification bus).
        let initial_title = session
            .part(&pending_tool.part)
            .and_then(|part| typed_content_from_value(&part.kind, &part.content).ok())
            .and_then(|content| match content {
                TypedContent::ToolCall(tool_call) => {
                    let operation = operation_from_tool_call(&tool_call);
                    (!operation.title.is_empty()).then_some(operation.title.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| "tool".to_owned());
        let mut activity_handler = streaming_activity_id.map(|activity_id| {
            // The built-in human renderer maps the tool's structured output to
            // ViewBlocks (v2). Shell/process executions carry the command line
            // so the human view renders a `$ command` card.
            let invocation = session
                .part(&pending_tool.part)
                .and_then(|part| typed_content_from_value(&part.kind, &part.content).ok())
                .and_then(|content| match content {
                    TypedContent::ToolCall(tool_call) => {
                        Some(operation_from_tool_call(&tool_call).invocation)
                    }
                    _ => None,
                });
            let invocation = invocation.as_ref();
            let command = invocation.and_then(|invocation| {
                invocation
                    .input
                    .get("command")
                    .and_then(|value| value.as_text())
                    .map(ToOwned::to_owned)
            });
            let cwd = invocation.and_then(|invocation| {
                invocation
                    .input
                    .get("workdir")
                    .and_then(|value| value.as_text())
                    .map(ToOwned::to_owned)
            });
            let mut renderer = crate::tool::human_view::BuiltinHumanRenderer::new(
                invocation
                    .map(|invocation| invocation.name.as_str())
                    .unwrap_or("tool"),
            );
            if let Some(command) = command {
                renderer = renderer.with_command(command);
            }
            if let Some(cwd) = cwd {
                renderer = renderer.with_cwd(cwd);
            }
            crate::activity::ActivityHandler::begin(
                activity_id,
                crate::activity::ActivityKind::Operation,
                initial_title,
            )
            .with_renderer(renderer)
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
            // Live streaming detail is carried by ActivityV2 DetailDelta
            // broadcasts above; the legacy CommandOutputDelta detail path is
            // removed.
            if last_title_refresh.elapsed() >= std::time::Duration::from_millis(TITLE_REFRESH_MS) {
                last_title_refresh = std::time::Instant::now();
                session = self
                    .refresh_streaming_title(session.id, pending_tool, state.clone())
                    .await?;
                if let Some(handler) = &mut activity_handler
                    && let Some(event) =
                        handler.refresh_elapsed_title(stream_started.elapsed().as_secs())
                {
                    self.broadcast_activity_v2(session.id, event)?;
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
                let session = self.store.load_session(session.id).await?;
                return self
                    .apply_tool_policy_denied(session, pending_tool, *denial, state)
                    .await;
            }
            Ok(Err(ToolError::UserDeclined(decline))) => {
                let session = self.store.load_session(session.id).await?;
                return self
                    .apply_tool_user_declined(session, pending_tool, *decline, Vec::new(), state)
                    .await;
            }
            Ok(Err(ToolError::CapabilityUnavailable(unavailable))) => {
                let session = self.store.load_session(session.id).await?;
                return self
                    .apply_tool_capability_unavailable(session, pending_tool, *unavailable, state)
                    .await;
            }
            Ok(Err(ToolError::ToolUnavailable(unavailable))) => {
                let session = self.store.load_session(session.id).await?;
                return self
                    .apply_tool_unavailable(session, pending_tool, *unavailable, state)
                    .await;
            }
            Ok(Err(err)) => {
                let session = self.store.load_session(session.id).await?;
                return self
                    .apply_tool_error(session, pending_tool, err, None, state)
                    .await;
            }
            Err(_) => {
                let session = self.store.load_session(session.id).await?;
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

        // Assemble the terminal activity node once the stream finished
        // successfully and broadcast it live (a no-op bridge in v2; P5
        // re-homes onto the notification bus). The durable payload is the
        // streamed output the tool part checkpoint carries — the v1
        // The former duplicate transcript write path is deleted.
        if let Some(mut handler) = activity_handler.take() {
            let node = handler.finish(
                agena_tool::ToolActivityResult::raw(agena_domain::RawOutput::text(streamed_output)),
                agena_domain::ActivityState::Completed,
            );
            self.broadcast_activity_v2(
                session.id,
                crate::activity::ActivityLiveEvent::Upserted {
                    node: Box::new(node),
                },
            )?;
        }

        let session = self.store.load_session(session.id).await?;
        self.apply_tool_success_with_rules(session, pending_tool, execution, Vec::new(), state)
            .await
    }

    pub(in crate::session::manager) async fn apply_tool_cancellation(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let _run_marker_id = update_resolved_tool_message(&mut session, &resolved, |part| {
            if let Ok(TypedContent::ToolCall(tool_call)) =
                typed_content_from_value(&part.kind, &part.content)
            {
                let mut operation = operation_from_tool_call(&tool_call);
                operation.set_summary("Execution cancelled");
                operation.lifecycle = completed_lifecycle(&resolved.lifecycle);
                part.content = typed_content_to_value(&TypedContent::ToolCall(
                    tool_call_from_operation(&operation),
                ))
                .expect("operation content is always JSON serializable");
            }
            part.state = PartState::Cancelled;
            part.summary = Some("Execution cancelled".to_string());
        })?;

        self.persist_tool_completion(session, &resolved, Vec::new(), state)
            .await
    }

    /// Broadcast a slice of freshly streamed output to live presentation
    /// consumers as a non-persistent `CommandOutputDelta`. Expanded terminals
    /// render this delta into the Activity's detail in real time; collapsed
    /// terminals drop it. Nothing is written to disk.
    /// Publish one activity v2 live wire event (07 §5.2). In-memory,
    /// non-persistent, fire-and-forget like the legacy detail broadcasts.
    ///
    /// v2 (design 14): the event bus is gone; live streaming progress is
    /// delivered by the facade's `NotificationBus` as part-patch
    /// [`SessionChange`](agena_storage::store::SessionChange) events (D10),
    /// and this call is a no-op bridge kept for the streaming-tool path. P5
    /// re-homes any remaining consumers onto the notification bus.
    fn broadcast_activity_v2(
        &self,
        _session_id: i64,
        _event: crate::activity::ActivityLiveEvent,
    ) -> Result<(), AppError> {
        Ok(())
    }

    /// Refresh the running title of a streaming tool and emit a header-only
    /// checkpoint. This is the only durable write during a stream: it updates
    /// the compact title (a tiny UPDATE), never the cumulative output, so a
    /// long stream costs O(1) writes rather than re-persisting the growing
    /// text every 2s.
    pub(in crate::session::manager) async fn refresh_streaming_title(
        &self,
        session_id: i64,
        pending_tool: &SessionPendingTool,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let mut session = self.store.load_session(session_id).await?;
        let tool_part_ref = session
            .resolve_part_ref(&pending_tool.part)
            .ok_or_else(|| pending_tool_part_not_found_error(&pending_tool.part))?;
        {
            let tool_part = session
                .part_mut(&tool_part_ref)
                .ok_or_else(|| pending_tool_part_not_found_error(&pending_tool.part))?;
            if matches!(tool_part.state, PartState::Pending | PartState::InProgress) {
                tool_part.state = PartState::InProgress;
            }
            if let Ok(TypedContent::ToolCall(tool_call)) =
                typed_content_from_value(&tool_part.kind, &tool_part.content)
            {
                let mut operation = operation_from_tool_call(&tool_call);
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
                }
                tool_part.content = typed_content_to_value(&TypedContent::ToolCall(
                    tool_call_from_operation(&operation),
                ))
                .expect("operation content is always JSON serializable");
            }
        };
        // Persist the refreshed title as a part delta checkpoint (v2 D10):
        // the in-memory title change is written through the facade, which is
        // the single write path for streamed content. There is no separate
        // content-node title column to target.
        self.persist_session_changes(session, vec![pending_tool.part.part_id], None, state)
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
            return self.store.load_session(session_id).await;
        }
        // Bound the intermediate preview so a giant stream is not written in
        // full even once; the terminal frame carries the real truncated output.
        let preview = agena_runtime_tools::truncate_tool_output_text(streamed_output, 16 * 1024);
        let mut session = self.store.load_session(session_id).await?;
        let tool_part_ref = session
            .resolve_part_ref(&pending_tool.part)
            .ok_or_else(|| pending_tool_part_not_found_error(&pending_tool.part))?;
        {
            let tool_part = session
                .part_mut(&tool_part_ref)
                .ok_or_else(|| pending_tool_part_not_found_error(&pending_tool.part))?;
            // `append_tool_output_delta` becomes a decode → mutate → re-encode
            // cycle on the part's canonical operation content.
            let Ok(TypedContent::ToolCall(tool_call)) =
                typed_content_from_value(&tool_part.kind, &tool_part.content)
            else {
                return Err(AppError::Internal(format!(
                    "streaming tool part refused terminal output: part={}",
                    pending_tool.part.part_id
                )));
            };
            let mut operation = operation_from_tool_call(&tool_call);
            operation.append_output_delta(preview.as_str());
            tool_part.content = typed_content_to_value(&TypedContent::ToolCall(
                tool_call_from_operation(&operation),
            ))
            .expect("operation content is always JSON serializable");
        }
        self.persist_session_changes(session, vec![pending_tool.part.part_id], None, state)
            .await
    }

    pub(in crate::session::manager) async fn apply_tool_success_with_rules(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        mut execution: ToolInvocationExecution,
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
        let attributed_usage = execution
            .view
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
        let output_text = execution.view.output_text.clone();
        let presentation_summary = execution.view.summary.clone();
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let blocks = operation_blocks_from_tool_output(
            &resolved.invocation,
            &tool_output,
            execution.view.attachments.as_slice(),
            output_text.as_str(),
        );
        let completion_title = {
            let execution_title = execution.view.title.trim();
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
                crate::part::OperationCompletion::new(
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
            operation.set_presentation_sections(execution.view.sections.clone());
            operation.result.metadata.extend(
                execution
                    .view
                    .metadata
                    .iter()
                    .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone()))),
            );
            tool_part.content = typed_content_to_value(&TypedContent::ToolCall(
                tool_call_from_operation(&operation),
            ))
            .expect("operation content is always JSON serializable");
            tool_part.state = PartState::Completed;
        })?;
        // Mirror the v1 message usage attribution into the owning run marker's
        // `content["usage"]` (the v2 projection `aggregate_usage()` sums it).
        // Flush the marker content directly: `persist_tool_completion` only
        // persists the tool part and the cancelled request parts.
        if let Some(attributed_usage) = attributed_usage {
            let run_marker_id = assistant_message_id(&session, &resolved.pending.part)?;
            let marker_index = session
                .parts()
                .iter()
                .position(|part| part.part_id == run_marker_id)
                .ok_or_else(|| pending_tool_part_not_found_error(&resolved.pending.part))?;
            let marker_ref = crate::session::model::SessionPartRef {
                part_index: marker_index,
                part_id: run_marker_id,
            };
            let marker_content = session
                .part(&marker_ref)
                .map(|marker| {
                    let mut merged = marker
                        .content
                        .get("usage")
                        .cloned()
                        .and_then(|usage| {
                            serde_json::from_value::<agena_provider::CompletionUsage>(usage).ok()
                        })
                        .unwrap_or_default();
                    merged.add_assign(&agena_provider::CompletionUsage {
                        attributed_usage: vec![attributed_usage],
                        ..Default::default()
                    });
                    let mut content = marker.content.clone();
                    content["usage"] =
                        serde_json::to_value(&merged).expect("usage is always JSON serializable");
                    content
                })
                .ok_or_else(|| pending_tool_part_not_found_error(&resolved.pending.part))?;
            self.store
                .update_part(
                    session.id,
                    run_marker_id,
                    agena_storage::store::PartDelta {
                        content: Some(marker_content),
                        ..Default::default()
                    },
                )
                .await?;
        }

        Box::pin(self.persist_tool_completion(session, &resolved, persisted_rules, state)).await
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
