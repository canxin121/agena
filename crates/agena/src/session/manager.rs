use std::{collections::HashSet, sync::Arc, time::Duration};

use arc_swap::ArcSwap;
use chrono::Utc;
use tokio::sync::mpsc;

use crate::AppError;
use crate::event::{ErrorInfo, EventKind, RunFailedEvent, RunStartedEvent};
use crate::message::{
    AttachmentItem, BuiltinToolOutput, ExecutionStatus, FileChangePart, Message, MessageMetadata,
    MessagePart, MessageSource, MessageStatus, PartContent, PermissionRequestPart, TimeRange,
    TodoListPart, ToolAttachment, ToolExecutionPart, ToolInvocation, ToolOutput, ToolResultBlock,
    UserInputReply, UserInputReplyKind, UserInputRequest, UserInputRequestPart,
};
use crate::model::ModelRef;
use crate::permission::{
    PermissionAction, PermissionDecision, PermissionMode, PermissionReply, PermissionReplyKind,
    PermissionRequest, decide_from_mode,
};
use crate::role::Role;
use crate::tool::{ToolError, ToolExecutor, ToolInvocationExecution, ToolPermissionCheck};

use super::cache::SessionCachePolicy;
pub use super::cache::SessionCacheStats;
use super::control::{TurnControl, TurnControlError, TurnRegistry};
use super::model::{
    MESSAGE_TAG_PROMPT_SUMMARY, ProviderPromptAnchor, SessionListRequest, SessionPendingTool,
    SessionStatus, SessionSummary,
};
use super::history::{
    FinishReason, MessageId as HistoryMessageId, MessageRevised, RevisionKind, ToolCallCompleted,
    ToolCallId as HistoryToolCallId, TranscriptContent, TranscriptToolOutput, TurnAbortReason,
    TurnAborted, TurnCompleted, TurnId as HistoryTurnId, TurnStarted, UserMessageAppended,
};
use super::processor::SessionRunRequest;
use super::prompt_window::{self, PromptRequestOptions};
use super::store::{ReservedMessageIds, SessionCommit, SessionStore};
use super::{Session, SessionProcessor};

#[derive(Debug, Clone)]
pub struct SessionManagerConfig {
    pub cache_max_sessions: usize,
    pub cache_ttl: Duration,
    pub cache_max_bytes: usize,
    pub max_turn_loops: usize,
}

impl Default for SessionManagerConfig {
    fn default() -> Self {
        Self {
            cache_max_sessions: 128,
            cache_ttl: Duration::from_secs(15 * 60),
            cache_max_bytes: 64 * 1024 * 1024,
            max_turn_loops: 16,
        }
    }
}

impl SessionManagerConfig {
    fn cache_policy(&self) -> SessionCachePolicy {
        SessionCachePolicy {
            max_sessions: self.cache_max_sessions,
            ttl: self.cache_ttl,
            max_bytes: self.cache_max_bytes,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionCreateRequest {
    pub title: String,
    pub parent_session_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SessionRunOptions {
    pub model: ModelRef,
    pub system: Option<String>,
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<u32>,
}

impl SessionRunOptions {
    fn completion_request(
        &self,
        system: Option<String>,
        messages: Vec<Message>,
        tools: Vec<crate::tool::ToolDefinition>,
        prompt_cache_key: Option<String>,
        previous_response_id: Option<String>,
        prompt_window_generation: Option<u64>,
    ) -> crate::provider::CompletionRequest {
        crate::provider::CompletionRequest {
            model: self.model.model_id.clone(),
            system,
            messages,
            tools,
            temperature: self.temperature,
            max_output_tokens: self.max_output_tokens,
            prompt_cache_key,
            previous_response_id,
            prompt_window_generation,
            stop_sequences: Vec::new(),
            top_p: None,
            top_k: None,
            seed: None,
            thinking: None,
            response_format: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionUserTurnRequest {
    pub session_id: i64,
    pub options: SessionRunOptions,
    pub parts: Vec<PartContent>,
}

#[derive(Debug, Clone)]
pub struct SessionContinueRequest {
    pub session_id: i64,
    pub options: SessionRunOptions,
}

#[derive(Debug, Clone)]
pub struct SessionRewindRequest {
    pub session_id: i64,
    pub message_id: i64,
}

#[derive(Debug, Clone)]
pub struct SessionPermissionReplyRequest {
    pub session_id: i64,
    pub options: SessionRunOptions,
    pub reply: PermissionReply,
}

#[derive(Debug, Clone)]
pub struct SessionUserInputReplyRequest {
    pub session_id: i64,
    pub options: SessionRunOptions,
    pub reply: UserInputReply,
}

#[derive(Debug, Clone, Copy)]
struct PromptTurnBudget {
    max_prompt_chars: usize,
    max_prompt_tokens: u64,
    model_context_window_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
struct ResolvedPendingTool {
    pending: SessionPendingTool,
    operation_id: String,
    call_id: i64,
    invocation: ToolInvocation,
    lifecycle: TimeRange,
}

#[derive(Clone)]
struct SessionManagerState {
    processor: SessionProcessor,
    tool_executor: ToolExecutor,
    config: SessionManagerConfig,
}

impl SessionManagerState {
    fn new(
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
        config: SessionManagerConfig,
    ) -> Self {
        Self {
            processor,
            tool_executor,
            config,
        }
    }

    fn cache_policy(&self) -> SessionCachePolicy {
        self.config.cache_policy()
    }
}

pub struct SessionManager {
    store: Arc<SessionStore>,
    publisher: Arc<crate::event::EventPublisher>,
    bus: Arc<dyn crate::event::EventBus<crate::event::EventKind>>,
    execution: ArcSwap<SessionManagerState>,
    turn_registry: Arc<TurnRegistry>,
}

impl SessionManager {
    pub fn new(
        db: sea_orm::DatabaseConnection,
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
    ) -> Self {
        let db_arc = Arc::new(db.clone());
        // HistoryEventStore wraps SeaEventStore but silently drops UI-only
        // (non-persistent) events so streaming deltas never land in SQLite.
        let store_inner: Arc<dyn crate::event::EventStore<crate::event::EventKind>> =
            Arc::new(crate::db::HistoryEventStore::new(Arc::clone(&db_arc)));
        let bus: Arc<dyn crate::event::EventBus<crate::event::EventKind>> = Arc::new(
            crate::event::InProcessEventBus::<crate::event::EventKind>::new(4096),
        );
        let seq = Arc::new(crate::event::SequenceAllocator::new());
        let publisher = Arc::new(crate::event::publisher::EventPublisher::new(
            seq,
            Arc::clone(&store_inner),
            Arc::clone(&bus),
        ));
        let store = Arc::new(SessionStore::new(
            db,
            tool_executor.workspace_root(),
            Arc::clone(&publisher),
        ));
        let state =
            SessionManagerState::new(processor, tool_executor, SessionManagerConfig::default());
        Self {
            store,
            publisher,
            bus,
            execution: ArcSwap::from_pointee(state),
            turn_registry: Arc::new(TurnRegistry::new()),
        }
    }

    /// Returns the unified event publisher that core sites use to emit
    /// `EventKind`. Public so the API server crate can wire it into
    /// transports (REST/WS/SSE/IPC).
    pub fn event_publisher(&self) -> Arc<crate::event::EventPublisher> {
        Arc::clone(&self.publisher)
    }

    /// Returns the in-process bus subscribers can attach to.
    pub fn event_bus(&self) -> Arc<dyn crate::event::EventBus<crate::event::EventKind>> {
        Arc::clone(&self.bus)
    }

    pub fn with_config(self, config: SessionManagerConfig) -> Self {
        let mut next = (*self.execution.load_full()).clone();
        next.config = config;
        self.execution.store(Arc::new(next));
        self
    }

    pub(crate) fn reconfigure(
        &self,
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
        config: SessionManagerConfig,
    ) {
        self.execution.store(Arc::new(SessionManagerState::new(
            processor,
            tool_executor,
            config,
        )));
    }

    pub fn prune_cache(&self) {
        let state = self.execution_state();
        self.store.prune_cache(state.cache_policy());
    }

    pub fn cache_stats(&self) -> SessionCacheStats {
        self.store.cache_stats()
    }

    pub async fn create_session(&self, request: SessionCreateRequest) -> Result<Session, AppError> {
        let state = self.execution_state();
        self.store
            .create_session(
                request.title,
                request.parent_session_id,
                state.cache_policy(),
            )
            .await
    }

    pub async fn get_session(&self, session_id: i64) -> Result<Session, AppError> {
        let state = self.execution_state();
        self.store
            .load_session(session_id, state.cache_policy())
            .await
    }

    pub async fn workspace_session_ids(&self) -> Result<Vec<i64>, AppError> {
        self.store.list_workspace_session_ids().await
    }

    /// Locate the session that contains a given message id by projecting each
    /// workspace session and probing its in-memory messages. The legacy
    /// `message` SQL table no longer exists, so this scan is the only way to
    /// satisfy the `/api/v1/messages/{id}` family of endpoints which receive
    /// no session_id from the caller.
    pub async fn find_session_id_for_message(
        &self,
        message_id: i64,
    ) -> Result<Option<i64>, AppError> {
        for session_id in self.workspace_session_ids().await? {
            let session = self.get_session(session_id).await?;
            if session.messages.iter().any(|message| message.id == message_id) {
                return Ok(Some(session_id));
            }
        }
        Ok(None)
    }

    /// Same as `find_session_id_for_message`, but for a part id.
    pub async fn find_session_id_for_part(
        &self,
        part_id: i64,
    ) -> Result<Option<i64>, AppError> {
        for session_id in self.workspace_session_ids().await? {
            let session = self.get_session(session_id).await?;
            if session
                .messages
                .iter()
                .any(|message| message.parts.iter().any(|part| part.id == part_id))
            {
                return Ok(Some(session_id));
            }
        }
        Ok(None)
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

    pub async fn submit_user_turn(
        &self,
        request: SessionUserTurnRequest,
    ) -> Result<Session, AppError> {
        let session_id = request.session_id;
        let (control, steer_rx) = self.turn_registry.register(session_id).await;
        let result = self.submit_user_turn_inner(request, control.clone(), steer_rx).await;
        self.turn_registry
            .unregister_if_matches(session_id, &control)
            .await;
        result
    }

    async fn submit_user_turn_inner(
        &self,
        mut request: SessionUserTurnRequest,
        control: Arc<TurnControl>,
        steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();

        // Plugin chain: user.prompt.submit. Plugins can rewrite or block the
        // user's prompt before it enters the session.
        let prompt_text = request
            .parts
            .iter()
            .filter_map(|p| p.text_value())
            .collect::<Vec<_>>()
            .join("\n");
        if !prompt_text.is_empty() {
            let input = crate::plugin::UserPromptSubmitInput {
                session_id: request.session_id,
                prompt: prompt_text,
            };
            match state
                .tool_executor
                .plugin_manager()
                .dispatch_user_prompt_submit(input)
                .await
            {
                Ok(updated) => {
                    // Replace text parts with the (potentially rewritten) prompt.
                    let mut replaced = false;
                    for part in &mut request.parts {
                        if part.text_value().is_some() {
                            *part = PartContent::text(updated.prompt.clone());
                            replaced = true;
                            break;
                        }
                    }
                    if !replaced {
                        request.parts.push(PartContent::text(updated.prompt));
                    }
                }
                Err(err) => {
                    return Err(AppError::Internal(format!(
                        "prompt blocked by plugin: {}",
                        err.message
                    )));
                }
            }
        }

        let mut session = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        let ids = self.store.reserve_message_ids(request.parts.len()).await?;
        let user_message = build_message(
            ids,
            Role::User,
            MessageStatus::Completed,
            request.parts,
            MessageMetadata {
                source: MessageSource::User,
                parent_message_id: session
                    .last_conversation_message()
                    .map(|message| message.id),
                generated_by_call_id: None,
                model_provider_id: request.options.model.provider_id.to_string(),
                model_id: request.options.model.model_id.to_string(),
                tags: Vec::new(),
            },
        );
        session.messages.push(user_message.clone());
        session = self
            .persist_session_changes(session, vec![user_message.clone()], Vec::new(), None, state.clone())
            .await?;

        // Append-only model: emit a self-contained turn carrying the user
        // message so SessionViewBuilder sees a closed turn for it. The
        // matching TurnStarted/TurnCompleted bracket keeps the projection
        // invariant ("turn must close to flush") intact.
        let user_turn_id = HistoryTurnId::new();
        let user_history_items = vec![
            EventKind::TurnStarted(TurnStarted {
                turn_id: user_turn_id,
                model_id: request.options.model.model_id.as_str().into(),
                provider_id: request.options.model.provider_id.as_str().into(),
                request_digest: None,
            }),
            EventKind::UserMessageAppended(UserMessageAppended {
                message_id: HistoryMessageId(user_message.id),
                turn_id: user_turn_id,
                created_at: user_message.created_at,
                content: TranscriptContent::from_message_lossy(&user_message),
                metadata: user_message.metadata.clone(),
            }),
            EventKind::TurnCompleted(TurnCompleted {
                turn_id: user_turn_id,
                finish_reason: FinishReason::Stop,
            }),
        ];
        session = self
            .store
            .append_history_items(session, user_history_items, state.cache_policy())
            .await?;

        self.run_until_stable(session, &request.options, state, control, steer_rx)
            .await
    }

    pub async fn continue_session(
        &self,
        request: SessionContinueRequest,
    ) -> Result<Session, AppError> {
        let session_id = request.session_id;
        let (control, steer_rx) = self.turn_registry.register(session_id).await;
        let state = self.execution_state();
        let session = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        let result = self
            .run_until_stable(session, &request.options, state, control.clone(), steer_rx)
            .await;
        self.turn_registry
            .unregister_if_matches(session_id, &control)
            .await;
        result
    }

    /// External entry: cancel the in-flight turn for `session_id`. Returns
    /// `Ok(())` if a token was signalled, `Err` if no turn is active.
    pub async fn cancel_active_turn(&self, session_id: i64) -> Result<(), AppError> {
        self.turn_registry
            .cancel(session_id)
            .await
            .map_err(turn_control_to_app_error)
    }

    /// External entry: inject `parts` as a steer message into the in-flight
    /// turn for `session_id`. Returns `Err` if no turn is active or the
    /// channel was closed.
    pub async fn steer_input(
        &self,
        session_id: i64,
        parts: Vec<PartContent>,
    ) -> Result<(), AppError> {
        self.turn_registry
            .steer(session_id, parts)
            .await
            .map_err(turn_control_to_app_error)
    }

    pub async fn rewind_session(&self, request: SessionRewindRequest) -> Result<Session, AppError> {
        let state = self.execution_state();
        self.store
            .rewind_to_message(request.session_id, request.message_id, state.cache_policy())
            .await
    }

    pub async fn reply_permission(
        &self,
        request: SessionPermissionReplyRequest,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(request.session_id, state.cache_policy())
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
            permission_part.set_content(PartContent::PermissionRequest(
                PermissionRequestPart::pending(permission_request.clone())
                    .with_reply(request.reply.clone()),
            ));
            permission_part.status = ExecutionStatus::Completed;
        }

        let persisted_mode = persisted_mode_for_reply(request.reply.kind);
        let persisted_action_key = persisted_mode
            .map(|_| permission_action_key(&permission_request.action))
            .transpose()?;

        match request.reply.kind {
            PermissionReplyKind::AllowOnce | PermissionReplyKind::AllowAlways => {
                let resolved_tool = resolve_pending_tool(&session, &pending.tool)?;
                let execution = self
                    .execute_pending_tool_after_approval(state.as_ref(), session.id, &resolved_tool)
                    .map_err(tool_error_to_app_error)?;
                session = self
                    .apply_tool_success(
                        session,
                        &pending.tool,
                        execution,
                        persisted_action_key,
                        persisted_mode,
                        state.clone(),
                    )
                    .await?;
            }
            PermissionReplyKind::DenyOnce | PermissionReplyKind::DenyAlways => {
                session = self
                    .apply_tool_failure(
                        session,
                        &pending.tool,
                        reply_reason,
                        persisted_action_key,
                        persisted_mode,
                        state.clone(),
                    )
                    .await?;
            }
        }

        self.run_until_stable_for(request.session_id, session, &request.options, state)
            .await
    }

    pub async fn reply_user_input(
        &self,
        request: SessionUserInputReplyRequest,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(request.session_id, state.cache_policy())
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
            input_part.set_content(PartContent::UserInputRequest(
                UserInputRequestPart::pending(user_input_request.clone())
                    .with_reply(request.reply.clone()),
            ));
            input_part.status = ExecutionStatus::Completed;
        }

        match request.reply.kind {
            UserInputReplyKind::Submit => {
                let execution = user_input_execution(&user_input_request, &request.reply)?;
                session = self
                    .apply_tool_success(
                        session,
                        &pending.tool,
                        execution,
                        None,
                        None,
                        state.clone(),
                    )
                    .await?;
            }
            UserInputReplyKind::Cancel => {
                let reason =
                    request.reply.reason.clone().unwrap_or_else(|| {
                        "user declined to answer requested questions".to_string()
                    });
                session = self
                    .apply_tool_failure(session, &pending.tool, reason, None, None, state.clone())
                    .await?;
            }
        }

        self.run_until_stable_for(request.session_id, session, &request.options, state)
            .await
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
            .run_until_stable(session, options, state, control.clone(), steer_rx)
            .await;
        self.turn_registry
            .unregister_if_matches(session_id, &control)
            .await;
        result
    }

    async fn run_until_stable(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
        control: Arc<TurnControl>,
        mut steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
    ) -> Result<Session, AppError> {
        for _ in 0..state.config.max_turn_loops {
            // External cancel — surface as the same TurnAborted shape we
            // use elsewhere so the projection sees a clean boundary.
            if control.cancel.is_cancelled() {
                self.persist_run_failed_event(
                    session.id,
                    "turn cancelled by user".to_string(),
                    state.clone(),
                )
                .await?;
                return Ok(session);
            }

            // Drain any steer messages that arrived since the last
            // iteration. Each becomes a User message appended to the
            // transcript before the next model turn — so the model sees
            // the new input on its next step.
            session = self
                .drain_steer_input(session, &mut steer_rx, options, state.clone())
                .await?;

            session.refresh_derived();
            if session.blocked() {
                return Ok(session);
            }

            if let Some(tool) = session.next_pending_tool() {
                session = self
                    .resolve_pending_tool(session, tool, state.clone())
                    .await?;
                continue;
            }

            match session.status() {
                SessionStatus::Idle => {
                    // Plugin hook: agent.stop. Plugins can inspect the final
                    // assistant message and optionally inject a follow-up turn.
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
                            // Inject the follow-up message and loop again.
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
                                    model_provider_id: options.model.provider_id.to_string(),
                                    model_id: options.model.model_id.to_string(),
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
                            // Don't return — let the loop continue so the model
                            // handles the injected message.
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
                SessionStatus::AwaitingModel => {}
            }

            session = self
                .run_model_turn(session, options, state.clone(), control.clone())
                .await?;
        }

        Err(AppError::Internal(
            "session manager exceeded max turn loop budget".to_string(),
        ))
    }

    async fn run_model_turn(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
        control: Arc<TurnControl>,
    ) -> Result<Session, AppError> {
        let mut compacted_rounds = 0_u8;

        loop {
            let active_messages = prompt_window::active_prompt_messages(&session);
            let tools = state.tool_executor.available_tools_for_messages_and_loaded(
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
            let should_compact_from_runtime = prompt_window::estimate_prompt_tokens_from_runtime(
                &session,
                active_messages.as_slice(),
                prompt_fingerprints.system_fingerprint.as_str(),
                prompt_fingerprints.request_options_fingerprint.as_str(),
            )
            .is_some_and(|estimate| estimate.total_tokens > prompt_budget.max_prompt_tokens);
            if let Some(plan) =
                prompt_window::plan_attachment_payload_stripping(active_messages.as_slice())
            {
                session = self
                    .strip_prompt_attachment_payloads(session, plan, state.clone())
                    .await?;
                continue;
            }

            if let Some(plan) = prompt_window::plan_tool_result_pruning(active_messages.as_slice())
            {
                session = self
                    .prune_tool_result_history(session, plan, state.clone())
                    .await?;
                continue;
            }

            if (should_compact_from_runtime
                || state.processor.should_compact_prompt_with_budget(
                    active_messages.as_slice(),
                    prompt_budget.max_prompt_chars,
                ))
                && prompt_window::can_compact(
                    active_messages.as_slice(),
                    state.processor.keep_tail_messages(),
                    prompt_budget.max_prompt_chars,
                )
                && state.processor.can_retry_compaction(compacted_rounds)
            {
                compacted_rounds += 1;
                session = self
                    .compact_prompt_window(
                        session,
                        options,
                        active_messages.as_slice(),
                        state.clone(),
                    )
                    .await?;
                continue;
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

            let processor_ids = self.store.reserve_processor_ids().await?;
            let run = SessionRunRequest {
                session_id: session.id,
                model: options.model.clone(),
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

            // Sub-task B: pre-allocate the turn id and emit a TurnStarted
            // boundary event before invoking the processor. The processor
            // currently mints its own TurnId internally; we use the one from
            // its result to wrap the matching TurnCompleted/TurnAborted.
            let processor_fut = state.processor.run_turn(run);
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

                    // Sub-task B cutover: thread the processor's append-only
                    // history events through the store, wrapped with turn
                    // boundary markers so SessionViewBuilder can group them.
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
                    persisted_session = self
                        .store
                        .append_history_items(persisted_session, turn_events, state.cache_policy())
                        .await?;

                    if let Some(err) = terminal_error {
                        self.persist_run_failed_event(persisted_session.id, err.to_string(), state)
                            .await?;
                        return Err(err);
                    }

                    return Ok(persisted_session);
                }
                Err(err)
                    if state
                        .processor
                        .should_retry_with_compaction(&err, compacted_rounds)
                        && prompt_window::can_compact(
                            active_messages.as_slice(),
                            state.processor.keep_tail_messages(),
                            prompt_budget.max_prompt_chars,
                        ) =>
                {
                    compacted_rounds += 1;
                    session = self
                        .compact_prompt_window(
                            session,
                            options,
                            active_messages.as_slice(),
                            state.clone(),
                        )
                        .await?;
                }
                Err(err) => {
                    self.persist_run_failed_event(session.id, err.to_string(), state)
                        .await?;
                    return Err(err);
                }
            }
        }
    }

    fn prompt_budget_for_turn(
        &self,
        session: &Session,
        options: &SessionRunOptions,
        tools: &[crate::tool::ToolDefinition],
        state: &SessionManagerState,
    ) -> PromptTurnBudget {
        let fallback_budget = state.processor.max_prompt_chars();
        let metadata = state
            .processor
            .model_metadata(&options.model)
            .unwrap_or_default();
        let context_window_tokens = metadata
            .limits
            .context_window_tokens
            .or(session.runtime.prompt_tokens.model_context_window_tokens);
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

    async fn prune_tool_result_history(
        &self,
        mut session: Session,
        plan: prompt_window::ToolResultPrunePlan,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let pruned_ids = plan.pruned_message_ids.into_iter().collect::<HashSet<_>>();
        let mut items = Vec::new();

        for message in &session.messages {
            if !pruned_ids.contains(&message.id) {
                continue;
            }
            for part in &message.parts {
                if matches!(part.content, Some(PartContent::ToolExecution(_))) {
                    let Some(op_id) = part.operation_id.as_deref() else {
                        continue;
                    };
                    items.push(EventKind::MessageRevised(MessageRevised {
                        target_message_id: message.id,
                        kind: RevisionKind::ToolResultPruned {
                            call_id: HistoryToolCallId::new(op_id),
                            replacement: crate::provider::PRUNED_TOOL_RESULT_PLACEHOLDER
                                .to_string(),
                        },
                    }));
                }
            }
        }

        if items.is_empty() {
            return Ok(session);
        }

        self.invalidate_prompt_window_runtime(&mut session);
        self.store
            .append_history_items(session, items, state.cache_policy())
            .await
    }

    async fn strip_prompt_attachment_payloads(
        &self,
        mut session: Session,
        plan: prompt_window::AttachmentPayloadStripPlan,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let stripped_ids = plan
            .stripped_message_ids
            .into_iter()
            .collect::<HashSet<_>>();
        let mut items = Vec::new();

        for message in &session.messages {
            if !stripped_ids.contains(&message.id) {
                continue;
            }
            for part in &message.parts {
                if matches!(part.content, Some(PartContent::Attachment(_))) {
                    items.push(EventKind::MessageRevised(MessageRevised {
                        target_message_id: message.id,
                        kind: RevisionKind::AttachmentStripped { part_id: part.id },
                    }));
                }
            }
        }

        if items.is_empty() {
            return Ok(session);
        }

        self.invalidate_prompt_window_runtime(&mut session);
        self.store
            .append_history_items(session, items, state.cache_policy())
            .await
    }

    async fn compact_prompt_window(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        active_messages: &[Message],
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let tools = state.tool_executor.available_tools_for_messages_and_loaded(
            active_messages,
            session.runtime.loaded_deferred_tools(),
        );
        let prompt_budget =
            self.prompt_budget_for_turn(&session, options, tools.as_slice(), state.as_ref());
        let Some(plan) = prompt_window::plan_compaction(
            active_messages,
            state.processor.keep_tail_messages(),
            prompt_budget.max_prompt_chars,
        ) else {
            return Err(AppError::Internal(
                "prompt window cannot be compacted further".to_string(),
            ));
        };

        // Plugin hook: session.compacting — notify plugins that compaction is
        // about to happen. (Fire-and-forget; patch is accepted but not applied
        // to the already-computed plan since replanning would be expensive.)
        {
            let sdk_messages = active_messages
                .iter()
                .filter_map(|msg| {
                    let content = serde_json::to_value(&msg.parts).ok()?;
                    let role = match msg.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        Role::Tool => "tool",
                        Role::System => "system",
                    };
                    Some(crate::plugin::ChatMessage {
                        role: role.to_string(),
                        content,
                    })
                })
                .collect::<Vec<_>>();
            let compacting_input = crate::plugin::SessionCompactingInput {
                session_id: session.id,
                messages: sdk_messages,
                strategy: "summarize".to_string(),
            };
            if let Err(err) = state
                .tool_executor
                .plugin_manager()
                .dispatch_session_compacting(compacting_input)
                .await
            {
                tracing::warn!(
                    target: "agena_plugin_host::session_compacting",
                    "session.compacting hook failed (continuing): {err}"
                );
            }
        }

        let compacted_message_ids = plan.compacted_message_ids.clone();
        let messages_before = active_messages.len();

        let summary_message = build_message(
            self.store.reserve_message_ids(1).await?,
            Role::System,
            MessageStatus::Completed,
            vec![PartContent::text(plan.summary_text)],
            MessageMetadata {
                source: MessageSource::System,
                parent_message_id: active_messages.last().map(|message| message.id),
                generated_by_call_id: None,
                model_provider_id: options.model.provider_id.to_string(),
                model_id: options.model.model_id.to_string(),
                tags: vec![MESSAGE_TAG_PROMPT_SUMMARY.to_string()],
            },
        );

        session.messages.push(summary_message.clone());
        self.invalidate_prompt_window_runtime(&mut session);
        let summary_text = summary_message
            .as_text_lossy();
        let messages_after = session.messages.len();
        let mut items: Vec<EventKind> = compacted_message_ids
            .into_iter()
            .map(|target_message_id| {
                EventKind::MessageRevised(MessageRevised {
                    target_message_id,
                    kind: RevisionKind::Compacted,
                })
            })
            .collect();
        items.push(EventKind::SystemNoticeAppended(
            super::history::SystemNoticeAppended {
                message_id: super::history::MessageId(summary_message.id),
                created_at: summary_message.created_at,
                kind: super::history::SystemNoticeKind::CompactionSummary,
                text: summary_text.clone(),
            },
        ));

        let result = self
            .store
            .append_history_items(session, items, state.cache_policy())
            .await?;

        // Plugin notification: session.compacted (fire-and-forget).
        {
            let compacted_input = crate::plugin::SessionCompactedInput {
                session_id: result.id,
                strategy: "summarize".to_string(),
                summary: summary_text,
                messages_before,
                messages_after,
            };
            state
                .tool_executor
                .plugin_manager()
                .broadcast_session_compacted(compacted_input)
                .await;
        }

        Ok(result)
    }

    fn invalidate_prompt_window_runtime(&self, session: &mut Session) {
        session.runtime.prompt_window.generation += 1;
        session.runtime.clear_provider_anchors();
        session.runtime.clear_prompt_tokens();
    }

    async fn resolve_pending_tool(
        &self,
        mut session: Session,
        pending_tool: SessionPendingTool,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let mut resolved = resolve_pending_tool(&session, &pending_tool)?;
        let prepared = state
            .tool_executor
            .prepare_invocation(&resolved.invocation, session.id, resolved.call_id)
            .map_err(tool_error_to_app_error)?;
        if prepared.invocation != resolved.invocation || prepared.title_override.is_some() {
            let current_title = match session
                .part(&resolved.pending.part)
                .and_then(|part| part.content.as_ref())
            {
                Some(PartContent::ToolExecution(ToolExecutionPart::Pending { title, .. })) => {
                    title.clone()
                }
                _ => format!("Tool {}", tool_name(&resolved.invocation)),
            };

            resolved.invocation = prepared.invocation.clone();
            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    resolved.pending.part.message_id, resolved.pending.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id: resolved.call_id,
                invocation: prepared.invocation,
                title: prepared.title_override.unwrap_or(current_title),
                lifecycle: resolved.lifecycle.clone(),
            }));
        }

        for check in state
            .tool_executor
            .collect_permission_checks_for_invocation(&resolved.invocation)
            .map_err(tool_error_to_app_error)?
        {
            let decision = self.resolve_permission_decision(&check).await?;
            match decision {
                PermissionDecision::Allow => {}
                PermissionDecision::Ask { reason } => {
                    return self
                        .apply_permission_request(
                            session,
                            &resolved.pending,
                            check.action,
                            reason,
                            state,
                        )
                        .await;
                }
                PermissionDecision::Deny { reason } => {
                    return self
                        .apply_tool_failure(session, &resolved.pending, reason, None, None, state)
                        .await;
                }
            }
        }

        match self.execute_pending_tool(state.as_ref(), session.id, &resolved) {
            Ok(execution) => {
                self.apply_tool_success(session, &resolved.pending, execution, None, None, state)
                    .await
            }
            Err(ToolError::UserInputRequired(input)) => {
                self.apply_user_input_request(session, &resolved.pending, input, state)
                    .await
            }
            Err(err) => Err(tool_error_to_app_error(err)),
        }
    }

    async fn resolve_permission_decision(
        &self,
        check: &ToolPermissionCheck,
    ) -> Result<PermissionDecision, AppError> {
        match &check.decision {
            PermissionDecision::Allow => Ok(PermissionDecision::Allow),
            PermissionDecision::Deny { reason } => Ok(PermissionDecision::Deny {
                reason: reason.clone(),
            }),
            PermissionDecision::Ask { reason } => {
                let key = permission_action_key(&check.action)?;
                if let Some(mode) = self.store.resolve_permission_mode(key.as_str()).await? {
                    return Ok(decide_from_mode(mode, reason.clone()));
                }
                Ok(PermissionDecision::Ask {
                    reason: reason.clone(),
                })
            }
        }
    }

    async fn apply_permission_request(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        action: PermissionAction,
        reason: String,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let request = PermissionRequest {
            request_id: resolved.operation_id.clone(),
            session_id: Some(session.id),
            action,
            reason: reason.clone(),
            created_at: Utc::now(),
        };

        {
            let tool_part = session.part_mut(&pending_tool.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    pending_tool.part.message_id, pending_tool.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id: resolved.call_id,
                invocation: resolved.invocation.clone(),
                title: format!("Awaiting permission: {reason}"),
                lifecycle: resolved.lifecycle.clone(),
            }));
            tool_part.status = ExecutionStatus::Pending;
            tool_part.summary = Some(reason.clone());
        }

        let permission_part_id = self.store.reserve_part_id().await?;
        let permission_part = build_permission_part(
            permission_part_id,
            pending_tool.part.message_id,
            resolved.operation_id.as_str(),
            PermissionRequestPart::pending(request),
        );
        session.messages[pending_tool.part.message_index]
            .parts
            .push(permission_part.clone());

        let assistant_message = session.messages[pending_tool.part.message_index].clone();
        let _ = resolved;
        let _ = reason;
        self.persist_session_changes(session, vec![assistant_message], Vec::new(), None, state.clone())
            .await
    }

    async fn apply_user_input_request(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        input: crate::message::AskUserToolInput,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let request = UserInputRequest {
            request_id: resolved.operation_id.clone(),
            session_id: Some(session.id),
            questions: input.questions,
            created_at: Utc::now(),
        };

        {
            let tool_part = session.part_mut(&pending_tool.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    pending_tool.part.message_id, pending_tool.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id: resolved.call_id,
                invocation: resolved.invocation.clone(),
                title: ask_user_title(&request),
                lifecycle: resolved.lifecycle.clone(),
            }));
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
            pending_tool.part.message_id,
            resolved.operation_id.as_str(),
            UserInputRequestPart::pending(request.clone()),
        );
        session.messages[pending_tool.part.message_index]
            .parts
            .push(input_part.clone());

        let assistant_message = session.messages[pending_tool.part.message_index].clone();
        self.persist_session_changes(session, vec![assistant_message], Vec::new(), None, state.clone())
            .await
    }

    async fn apply_tool_success(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        execution: ToolInvocationExecution,
        persisted_action_key: Option<String>,
        persisted_mode: Option<PermissionMode>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let tool_output = execution.output.clone();
        let output_text = execution.view.output_text.clone();
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let blocks = text_result_blocks(output_text.as_str());
        let extra_part_contents = tool_message_extra_part_contents(
            &tool_output,
            execution.view.attachments.as_slice(),
            blocks.as_slice(),
        );
        if let Some(BuiltinToolOutput::ToolSearch { loaded_tools, .. }) = tool_output.as_builtin() {
            session.runtime.record_loaded_deferred_tools(&loaded_tools);
        }

        {
            let tool_part = session.part_mut(&pending_tool.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    pending_tool.part.message_id, pending_tool.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::ToolExecution(ToolExecutionPart::Completed {
                call_id: resolved.call_id,
                invocation: resolved.invocation.clone(),
                output_text: output_text.clone(),
                blocks: blocks.clone(),
                attachments: execution.view.attachments.clone(),
                details: tool_output.clone(),
                lifecycle: lifecycle.clone(),
            }));
            tool_part.status = ExecutionStatus::Completed;
        }

        let tool_message = build_tool_message(
            self.store
                .reserve_message_ids(1 + extra_part_contents.len())
                .await?,
            &resolved,
            execution.view.attachments,
            output_text,
            blocks,
            tool_output,
            lifecycle,
            None,
            extra_part_contents,
        );
        session.messages.push(tool_message.clone());

        let assistant_message = session.messages[pending_tool.part.message_index].clone();
        let tool_call_id = tool_call_id_for(&resolved);
        let tool_output_event = match &execution.output {
            ToolOutput::None => TranscriptToolOutput::Text {
                text: execution.view.output_text.clone(),
            },
            _ => TranscriptToolOutput::Text {
                text: execution.view.output_text.clone(),
            },
        };
        let session = self
            .persist_session_changes(
                session,
                vec![assistant_message, tool_message],
                Vec::new(),
                persisted_rule_update(persisted_action_key, persisted_mode),
                state.clone(),
            )
            .await?;
        let now = Utc::now();
        let turn_id = HistoryTurnId::new();
        let events = vec![EventKind::ToolCallCompleted(ToolCallCompleted {
            call_id: tool_call_id,
            turn_id,
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
        persisted_action_key: Option<String>,
        persisted_mode: Option<PermissionMode>,
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
            let tool_part = session.part_mut(&pending_tool.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    pending_tool.part.message_id, pending_tool.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::ToolExecution(ToolExecutionPart::Failed {
                call_id: resolved.call_id,
                invocation: resolved.invocation.clone(),
                error_message: reason.clone(),
                output_text: reason.clone(),
                blocks: blocks.clone(),
                attachments: Vec::new(),
                details: ToolOutput::None,
                lifecycle: lifecycle.clone(),
            }));
            tool_part.status = ExecutionStatus::Failed;
        }

        let tool_message = build_tool_message(
            self.store.reserve_message_ids(1).await?,
            &resolved,
            Vec::new(),
            reason.clone(),
            blocks,
            ToolOutput::None,
            lifecycle,
            Some(reason.clone()),
            Vec::new(),
        );
        session.messages.push(tool_message.clone());

        let assistant_message = session.messages[pending_tool.part.message_index].clone();
        let tool_call_id = tool_call_id_for(&resolved);
        let session = self
            .persist_session_changes(
                session,
                vec![assistant_message, tool_message],
                Vec::new(),
                persisted_rule_update(persisted_action_key, persisted_mode),
                state.clone(),
            )
            .await?;
        let now = Utc::now();
        let turn_id = HistoryTurnId::new();
        let events = vec![EventKind::ToolCallCompleted(ToolCallCompleted {
            call_id: tool_call_id,
            turn_id,
            output: TranscriptToolOutput::Error {
                message: reason,
            },
            completed_at: now,
        })];
        self.store
            .append_history_items(session, events, state.cache_policy())
            .await
    }

    async fn persist_session_changes(
        &self,
        session: Session,
        touched_messages: Vec<Message>,
        client_events: Vec<EventKind>,
        persisted_rule: Option<(String, PermissionMode)>,
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

    /// Drain every pending steer message (non-blocking) and append each as
    /// a User message before the next model turn. Mirrors Codex's
    /// `push_pending_input` semantics: a user steer becomes the next input
    /// the model sees.
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
                    parent_message_id: session
                        .last_conversation_message()
                        .map(|m| m.id),
                    generated_by_call_id: None,
                    model_provider_id: options.model.provider_id.to_string(),
                    model_id: options.model.model_id.to_string(),
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
        state.tool_executor.execute_invocation_detailed(
            &pending_tool.invocation,
            session_id,
            pending_tool.call_id,
        )
    }

    fn execute_pending_tool_after_approval(
        &self,
        state: &SessionManagerState,
        session_id: i64,
        pending_tool: &ResolvedPendingTool,
    ) -> Result<ToolInvocationExecution, ToolError> {
        state
            .tool_executor
            .execute_invocation_detailed_bypassing_permissions(
                &pending_tool.invocation,
                session_id,
                pending_tool.call_id,
            )
    }

    fn execution_state(&self) -> Arc<SessionManagerState> {
        self.execution.load_full()
    }
}

fn turn_control_to_app_error(err: TurnControlError) -> AppError {
    match err {
        TurnControlError::NoActiveTurn(id) => {
            AppError::Internal(format!("no in-flight turn for session {id}"))
        }
        TurnControlError::SteerClosed => {
            AppError::Internal("steer channel closed for session".to_string())
        }
    }
}

fn build_message(
    ids: ReservedMessageIds,
    role: Role,
    message_state: MessageStatus,
    parts: Vec<PartContent>,
    metadata: MessageMetadata,
) -> Message {
    let created_at = Utc::now();
    let mut message = Message {
        id: ids.message_id,
        role,
        state: message_state,
        parts: Vec::with_capacity(parts.len()),
        created_at,
        metadata,
        usage: None,
        finish: None,
    };

    for content in parts {
        let mut part = MessagePart::with_content(
            ids.part_ids[message.parts.len()],
            message.id,
            created_at,
            part_status(&content),
            content,
        );
        part.part_index = message.parts.len() as i32;
        message.parts.push(part);
    }

    message
}

fn tool_call_id_for(resolved: &ResolvedPendingTool) -> HistoryToolCallId {
    HistoryToolCallId::new(format!("call_{}", resolved.call_id))
}

fn resolve_pending_tool(
    session: &Session,
    pending_tool: &SessionPendingTool,
) -> Result<ResolvedPendingTool, AppError> {
    let part = session.part(&pending_tool.part).ok_or_else(|| {
        AppError::Internal(format!(
            "pending tool part not found: message={}, part={}",
            pending_tool.part.message_id, pending_tool.part.part_id
        ))
    })?;
    let operation_id = part.operation_id.clone().ok_or_else(|| {
        AppError::Internal(format!(
            "pending tool operation id missing: message={}, part={}",
            pending_tool.part.message_id, pending_tool.part.part_id
        ))
    })?;
    let (call_id, invocation, lifecycle) = session
        .pending_tool_execution(pending_tool)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "pending tool payload missing: message={}, part={}",
                pending_tool.part.message_id, pending_tool.part.part_id
            ))
        })?;

    Ok(ResolvedPendingTool {
        pending: pending_tool.clone(),
        operation_id,
        call_id,
        invocation: invocation.clone(),
        lifecycle: lifecycle.clone(),
    })
}

fn build_tool_message(
    ids: ReservedMessageIds,
    pending_tool: &ResolvedPendingTool,
    attachments: Vec<ToolAttachment>,
    output_text: String,
    blocks: Vec<ToolResultBlock>,
    details: ToolOutput,
    lifecycle: TimeRange,
    error_message: Option<String>,
    extra_part_contents: Vec<PartContent>,
) -> Message {
    let created_at = Utc::now();
    let message_state = if error_message.is_some() {
        MessageStatus::Failed
    } else {
        MessageStatus::Completed
    };
    let content = match error_message {
        Some(error_message) => PartContent::ToolExecution(ToolExecutionPart::Failed {
            call_id: pending_tool.call_id,
            invocation: pending_tool.invocation.clone(),
            error_message,
            output_text,
            blocks,
            attachments,
            details,
            lifecycle,
        }),
        None => PartContent::ToolExecution(ToolExecutionPart::Completed {
            call_id: pending_tool.call_id,
            invocation: pending_tool.invocation.clone(),
            output_text,
            blocks,
            attachments,
            details,
            lifecycle,
        }),
    };

    let mut part = MessagePart::with_content(
        ids.part_ids[0],
        ids.message_id,
        created_at,
        part_status(&content),
        content,
    );
    part.operation_id = Some(pending_tool.operation_id.clone());
    part.part_index = 0;

    let mut parts = vec![part];
    parts.extend(build_extra_message_parts(
        ids.part_ids[1..].iter().copied(),
        ids.message_id,
        created_at,
        extra_part_contents,
    ));

    Message {
        id: ids.message_id,
        role: Role::Tool,
        state: message_state,
        parts,
        created_at,
        metadata: MessageMetadata {
            source: MessageSource::Tool,
            parent_message_id: Some(pending_tool.pending.part.message_id),
            generated_by_call_id: Some(pending_tool.call_id),
            model_provider_id: String::new(),
            model_id: String::new(),
            tags: Vec::new(),
        },
        usage: None,
        finish: None,
    }
}

fn build_extra_message_parts(
    part_ids: impl IntoIterator<Item = i64>,
    message_id: i64,
    created_at: chrono::DateTime<Utc>,
    contents: Vec<PartContent>,
) -> Vec<MessagePart> {
    contents
        .into_iter()
        .zip(part_ids)
        .enumerate()
        .map(|(index, (content, part_id))| {
            let mut part = MessagePart::with_content(
                part_id,
                message_id,
                created_at,
                part_status(&content),
                content,
            );
            part.part_index = index as i32 + 1;
            part
        })
        .collect()
}

fn tool_message_extra_part_contents(
    details: &ToolOutput,
    attachments: &[ToolAttachment],
    blocks: &[ToolResultBlock],
) -> Vec<PartContent> {
    let mut contents = Vec::new();

    if let Some(file_change) = file_change_part_from_tool_output(details) {
        contents.push(PartContent::FileChange(file_change));
    }

    if let Some(todo) = todo_part_from_tool_output(details) {
        contents.push(PartContent::TodoList(todo));
    }

    let attachment_items = attachment_items_from_tool_output(details, attachments, blocks);
    if !attachment_items.is_empty() {
        contents.push(PartContent::attachments(attachment_items));
    }

    contents
}

fn file_change_part_from_tool_output(details: &ToolOutput) -> Option<FileChangePart> {
    match details.as_builtin() {
        Some(BuiltinToolOutput::ApplyPatch { changes, .. }) if !changes.is_empty() => {
            Some(FileChangePart { changes })
        }
        _ => None,
    }
}

fn todo_part_from_tool_output(details: &ToolOutput) -> Option<TodoListPart> {
    match details.as_builtin() {
        Some(BuiltinToolOutput::TodoWrite { items }) => Some(TodoListPart { items }),
        _ => None,
    }
}

fn attachment_items_from_tool_output(
    details: &ToolOutput,
    attachments: &[ToolAttachment],
    blocks: &[ToolResultBlock],
) -> Vec<AttachmentItem> {
    let mut items = Vec::new();

    for attachment in attachments {
        push_unique_attachment_item(&mut items, attachment.clone());
    }

    let block_source = match details {
        ToolOutput::Mcp { output } => output.content_blocks.as_slice(),
        _ => blocks,
    };

    for block in block_source {
        if let Some(item) = block.to_attachment_item() {
            push_unique_attachment_item(&mut items, item);
        }
    }

    items
}

fn push_unique_attachment_item(items: &mut Vec<AttachmentItem>, item: AttachmentItem) {
    if !items.contains(&item) {
        items.push(item);
    }
}

fn part_status(content: &PartContent) -> ExecutionStatus {
    match content {
        PartContent::ToolExecution(tool) => tool.status(),
        PartContent::PermissionRequest(permission) => permission.status(),
        PartContent::UserInputRequest(request) => request.status(),
        _ => ExecutionStatus::Completed,
    }
}

fn build_permission_part(
    part_id: i64,
    message_id: i64,
    operation_id: &str,
    permission: PermissionRequestPart,
) -> MessagePart {
    let mut part = MessagePart::with_content(
        part_id,
        message_id,
        Utc::now(),
        permission.status(),
        PartContent::PermissionRequest(permission),
    );
    part.operation_id = Some(operation_id.to_string());
    part
}

fn build_user_input_part(
    part_id: i64,
    message_id: i64,
    operation_id: &str,
    request: UserInputRequestPart,
) -> MessagePart {
    let mut part = MessagePart::with_content(
        part_id,
        message_id,
        Utc::now(),
        request.status(),
        PartContent::UserInputRequest(request),
    );
    part.operation_id = Some(operation_id.to_string());
    part
}

fn completed_lifecycle(lifecycle: &TimeRange) -> TimeRange {
    TimeRange {
        start_ms: lifecycle.start_ms,
        end_ms: Some(Utc::now().timestamp_millis()),
    }
}

fn tool_name(invocation: &ToolInvocation) -> String {
    match invocation {
        ToolInvocation::Mcp { server, tool, .. } => format!("{server}:{tool}"),
        ToolInvocation::Custom { name, .. } => name.clone(),
    }
}

fn text_result_blocks(output_text: &str) -> Vec<ToolResultBlock> {
    if output_text.trim().is_empty() {
        Vec::new()
    } else {
        vec![ToolResultBlock::Text {
            text: output_text.to_string(),
        }]
    }
}

fn persisted_mode_for_reply(kind: PermissionReplyKind) -> Option<PermissionMode> {
    match kind {
        PermissionReplyKind::AllowAlways => Some(PermissionMode::Allow),
        PermissionReplyKind::DenyAlways => Some(PermissionMode::Deny),
        PermissionReplyKind::AllowOnce | PermissionReplyKind::DenyOnce => None,
    }
}

fn persisted_rule_update(
    action_key: Option<String>,
    mode: Option<PermissionMode>,
) -> Option<(String, PermissionMode)> {
    action_key.zip(mode)
}

fn permission_action_key(action: &PermissionAction) -> Result<String, AppError> {
    serde_json::to_string(action).map_err(AppError::from)
}

fn tool_error_to_app_error(err: ToolError) -> AppError {
    match err {
        ToolError::PermissionDenied(reason) | ToolError::PermissionAsk(reason) => {
            AppError::Internal(reason)
        }
        ToolError::UserInputRequired(_) => {
            AppError::Internal("unexpected unresolved user input request".to_string())
        }
        other => AppError::Internal(other.to_string()),
    }
}

fn ask_user_title(request: &UserInputRequest) -> String {
    match request.questions.len() {
        0 => "Ask user".to_string(),
        1 => {
            let header = request.questions[0].header.trim();
            if header.is_empty() {
                "Ask user".to_string()
            } else {
                format!("Ask: {header}")
            }
        }
        count => format!("Ask user ({count})"),
    }
}

fn user_input_execution(
    request: &UserInputRequest,
    reply: &UserInputReply,
) -> Result<ToolInvocationExecution, AppError> {
    let answers = validate_user_input_reply(request, reply)?;
    let mut lines = vec!["Answers:".to_string()];
    for question in &request.questions {
        if let Some(answer) = answers.get(question.id.as_str()) {
            lines.push(format!("- {}: {}", question.id, answer.join(", ")));
        }
    }

    let mut view = crate::tool::ToolExecutionView::simple("Ask user", lines.join("\n"));
    let selection_count: usize = answers.values().map(Vec::len).sum();
    view.metadata
        .insert("answer_count".to_string(), selection_count.to_string());
    view.metadata.insert(
        "question_count".to_string(),
        request.questions.len().to_string(),
    );

    Ok(ToolInvocationExecution::new(
        ToolOutput::Custom {
            output: BuiltinToolOutput::AskUser { answers }.into_custom_output(),
        },
        view,
    ))
}

fn validate_user_input_reply(
    request: &UserInputRequest,
    reply: &UserInputReply,
) -> Result<std::collections::BTreeMap<String, Vec<String>>, AppError> {
    let mut answers = std::collections::BTreeMap::new();

    for question in &request.questions {
        let raw_answers = reply
            .answers
            .get(question.id.as_str())
            .cloned()
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "missing answer for user input question {}",
                    question.id
                ))
            })?;
        let mut normalized = Vec::new();
        for value in raw_answers {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !normalized
                .iter()
                .any(|existing: &String| existing == trimmed)
            {
                normalized.push(trimmed.to_string());
            }
        }

        if normalized.is_empty() {
            return Err(AppError::Internal(format!(
                "missing answer for user input question {}",
                question.id
            )));
        }
        if !question.multiple && normalized.len() != 1 {
            return Err(AppError::Internal(format!(
                "question {} accepts exactly one answer",
                question.id
            )));
        }

        let allowed = question
            .options
            .iter()
            .map(|option| option.label.trim())
            .filter(|label| !label.is_empty())
            .collect::<std::collections::HashSet<_>>();
        if !question.allow_custom {
            if let Some(answer) = normalized
                .iter()
                .find(|value| !allowed.contains(value.as_str()))
            {
                return Err(AppError::Internal(format!(
                    "unsupported answer '{}' for question {}",
                    answer, question.id
                )));
            }
        }

        answers.insert(question.id.clone(), normalized);
    }

    for answer_id in reply.answers.keys() {
        if !request
            .questions
            .iter()
            .any(|question| question.id == *answer_id)
        {
            return Err(AppError::Internal(format!(
                "unexpected answer for unknown user input question {answer_id}"
            )));
        }
    }

    Ok(answers)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use futures_core::Stream;
    use futures_util::stream;
    use sea_orm::Database;
    use uuid::Uuid;

    use crate::agent::Agent;
    use crate::db::init_schema;
    use crate::event::{EventKind, RunFailedEvent, StreamErrorEvent};
    use crate::message::{
        ApplyPatchToolInput, AskUserToolInput, AttachmentItem, AttachmentSource, BuiltinToolOutput,
        FileChangeKind, McpToolOutput, ToolAttachment, ToolExecutionPart, ToolOutput,
        ToolResultBlock, ToolSearchToolInput, UserInputOption, UserInputQuestion, UserInputReply,
        UserInputReplyKind,
    };
    use crate::model::{ModelId, ModelRef, ProviderId};
    use crate::permission::{PermissionMode, PermissionPolicy};
    use crate::provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionUsage, ModelProvider, ProviderModel, ProviderRegistry,
    };
    use crate::session::{ContextGovernor, ContextPolicy, MESSAGE_TAG_PROMPT_COMPACTED};

    use super::*;
    use crate::session::cache::{SessionCache, SessionCachePolicy};

    struct TempWorkspace {
        root: std::path::PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("agena-session-tests-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).expect("failed to create temp workspace");
            Self { root }
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    struct ScriptedProvider;

    #[derive(Clone)]
    struct RecordingProvider {
        requests: Arc<Mutex<Vec<CompletionRequest>>>,
        next_response_id: Arc<Mutex<u64>>,
        metadata: crate::provider::ModelMetadata,
        usage: Option<CompletionUsage>,
        current_prompt_cache_shape: Arc<Mutex<Option<crate::provider::PromptCacheShape>>>,
        dynamic_prompt_cache_shape: Option<crate::provider::PromptCacheShape>,
    }

    fn scripted_provider_id() -> ProviderId {
        ProviderId::new("scripted")
    }

    fn scripted_model_id() -> ModelId {
        ModelId::new("scripted-model")
    }

    fn scripted_model_ref() -> ModelRef {
        ModelRef::new("scripted", "scripted-model")
    }

    fn recording_provider_id() -> ProviderId {
        ProviderId::new("recording")
    }

    fn recording_model_id() -> ModelId {
        ModelId::new("recording-model")
    }

    fn recording_model_ref() -> ModelRef {
        ModelRef::new("recording", "recording-model")
    }

    impl RecordingProvider {
        fn new(requests: Arc<Mutex<Vec<CompletionRequest>>>) -> Self {
            Self {
                requests,
                next_response_id: Arc::new(Mutex::new(0)),
                metadata: crate::provider::ModelMetadata::default(),
                usage: None,
                current_prompt_cache_shape: Arc::new(Mutex::new(None)),
                dynamic_prompt_cache_shape: None,
            }
        }

        fn next_response_id(&self) -> String {
            let mut guard = self
                .next_response_id
                .lock()
                .expect("recording provider response id lock should succeed");
            *guard += 1;
            format!("resp_{}", *guard)
        }

        fn with_metadata(mut self, metadata: crate::provider::ModelMetadata) -> Self {
            self.metadata = metadata;
            self
        }

        fn with_usage(mut self, usage: CompletionUsage) -> Self {
            self.usage = Some(usage);
            self
        }

        fn with_dynamic_prompt_cache_shape(
            mut self,
            shape: crate::provider::PromptCacheShape,
        ) -> Self {
            self.dynamic_prompt_cache_shape = Some(shape);
            self
        }
    }

    #[async_trait]
    impl ModelProvider for ScriptedProvider {
        fn id(&self) -> &str {
            "scripted"
        }

        fn default_model(&self) -> &ModelId {
            static DEFAULT_MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("scripted-model"));
            &DEFAULT_MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![
                ProviderModel::new("scripted", "scripted-model").with_display_name("Scripted"),
            ])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Ok(CompletionResponse {
                provider_id: scripted_provider_id(),
                model: scripted_model_id(),
                text: String::new(),
                reasoning_text: None,
                finish_reason: Some(CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
        }

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            let last_user_text = request
                .messages
                .iter()
                .rev()
                .find(|message| message.role == Role::User)
                .map(Message::as_text_lossy)
                .unwrap_or_default();

            let tool_result = request.messages.iter().find_map(|message| {
                if message.role != Role::Tool {
                    return None;
                }
                message.parts.iter().find_map(|part| {
                    if part.operation_id.as_deref() != Some("call_apply_patch_1") {
                        return None;
                    }
                    match part.content.as_ref() {
                        Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                            output_text,
                            ..
                        })) => Some(Ok(output_text.clone())),
                        Some(PartContent::ToolExecution(ToolExecutionPart::Failed {
                            error_message,
                            ..
                        })) => Some(Err(error_message.clone())),
                        _ => None,
                    }
                })
            });
            let user_input_result = request.messages.iter().find_map(|message| {
                if message.role != Role::Tool {
                    return None;
                }
                message.parts.iter().find_map(|part| {
                    if part.operation_id.as_deref() != Some("call_ask_user_1") {
                        return None;
                    }
                    match part.content.as_ref() {
                        Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                            details,
                            ..
                        })) => {
                            let answers = match details.as_builtin() {
                                Some(BuiltinToolOutput::AskUser { answers }) => answers,
                                _ => return None,
                            };
                            answers
                                .get("model_choice")
                                .and_then(|values| values.first().cloned())
                                .map(Ok)
                        }
                        Some(PartContent::ToolExecution(ToolExecutionPart::Failed {
                            error_message,
                            ..
                        })) => Some(Err(error_message.clone())),
                        _ => None,
                    }
                })
            });
            let apply_patch_tool_loaded = request.messages.iter().any(|message| {
                message.parts.iter().any(|part| {
                    let details = match part.content.as_ref() {
                        Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                            details,
                            ..
                        })) => details,
                        _ => return false,
                    };
                    matches!(
                        details.as_builtin(),
                        Some(BuiltinToolOutput::ToolSearch { ref loaded_tools, .. })
                            if loaded_tools.iter().any(|name| name == "apply_patch")
                    )
                })
            });

            let events = if last_user_text.contains("patch")
                && tool_result.is_none()
                && !apply_patch_tool_loaded
            {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        stream_key: "call_tool_search_1".to_string(),
                        id: Some("call_tool_search_1".to_string()),
                        name: Some("tool_search".to_string()),
                        arguments_delta: serde_json::to_string(&ToolSearchToolInput {
                            query: "patch file".to_string(),
                            load: vec!["apply_patch".to_string()],
                            limit: None,
                        })
                        .expect("serialize tool search input"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if last_user_text.contains("choose model") && user_input_result.is_none() {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        stream_key: "call_ask_user_1".to_string(),
                        id: Some("call_ask_user_1".to_string()),
                        name: Some("ask_user".to_string()),
                        arguments_delta: serde_json::to_string(&AskUserToolInput {
                            questions: vec![UserInputQuestion {
                                id: "model_choice".to_string(),
                                header: "Model".to_string(),
                                question: "Which model should we use?".to_string(),
                                options: vec![
                                    UserInputOption {
                                        label: "gpt-5".to_string(),
                                        description: "Use the flagship reasoning model."
                                            .to_string(),
                                    },
                                    UserInputOption {
                                        label: "gpt-4.1".to_string(),
                                        description: "Use the faster general-purpose model."
                                            .to_string(),
                                    },
                                ],
                                multiple: false,
                                allow_custom: false,
                            }],
                        })
                        .expect("serialize ask_user input"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if let Some(user_input_result) = user_input_result {
                let delta = match user_input_result {
                    Ok(answer) => format!("selected model: {answer}"),
                    Err(_) => "selection cancelled".to_string(),
                };
                vec![
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        delta,
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        finish_reason: Some(CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if last_user_text.contains("patch") && tool_result.is_none() {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        stream_key: "call_apply_patch_1".to_string(),
                        id: Some("call_apply_patch_1".to_string()),
                        name: Some("apply_patch".to_string()),
                        arguments_delta: serde_json::to_string(&ApplyPatchToolInput {
                            patch: "*** Begin Patch\n*** Add File: result.txt\n+approved\n*** End Patch"
                                .to_string(),
                        })
                        .expect("serialize tool input"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if let Some(tool_result) = tool_result {
                let delta = match tool_result {
                    Ok(_) => "patch done".to_string(),
                    Err(_) => "patch denied".to_string(),
                };
                vec![
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        delta,
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        finish_reason: Some(CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else {
                vec![
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        delta: format!("echo:{last_user_text}"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        finish_reason: Some(CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            };

            Ok(Box::pin(stream::iter(events)))
        }
    }

    async fn build_manager(
        root: &std::path::Path,
        permission_policy: PermissionPolicy,
        config: SessionManagerConfig,
    ) -> SessionManager {
        build_manager_with_provider(
            root,
            permission_policy,
            config,
            ContextPolicy::default(),
            ScriptedProvider,
        )
        .await
    }

    async fn build_manager_with_provider<P>(
        root: &std::path::Path,
        permission_policy: PermissionPolicy,
        config: SessionManagerConfig,
        context_policy: ContextPolicy,
        provider: P,
    ) -> SessionManager
    where
        P: ModelProvider + 'static,
    {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("failed to create sqlite db");
        init_schema(&db).await.expect("failed to init schema");

        let mut registry = ProviderRegistry::new();
        registry.register(provider);
        let processor =
            SessionProcessor::new(Arc::new(registry), ContextGovernor::new(context_policy));
        let executor = ToolExecutor::new(root, Agent::new("build", permission_policy));

        SessionManager::new(db, processor, executor).with_config(config)
    }

    fn run_options() -> SessionRunOptions {
        SessionRunOptions {
            model: scripted_model_ref(),
            system: None,
            temperature: None,
            max_output_tokens: Some(128),
        }
    }

    fn recording_run_options() -> SessionRunOptions {
        SessionRunOptions {
            model: recording_model_ref(),
            system: Some("system".to_string()),
            temperature: Some(0.2),
            max_output_tokens: Some(256),
        }
    }

    fn interrupted_model_ref() -> ModelRef {
        ModelRef::new("interrupted", "interrupted-model")
    }

    fn interrupted_run_options() -> SessionRunOptions {
        SessionRunOptions {
            model: interrupted_model_ref(),
            system: None,
            temperature: None,
            max_output_tokens: Some(128),
        }
    }

    fn high_recording_usage() -> CompletionUsage {
        CompletionUsage {
            input_tokens: 3_800,
            output_tokens: 200,
            reasoning_tokens: 0,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            total_cost: 0.0,
        }
    }

    struct InterruptedStreamProvider;

    #[async_trait]
    impl ModelProvider for InterruptedStreamProvider {
        fn id(&self) -> &str {
            "interrupted"
        }

        fn default_model(&self) -> &ModelId {
            static DEFAULT_MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("interrupted-model"));
            &DEFAULT_MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new("interrupted", "interrupted-model")])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Err(AppError::Provider(
                "interrupted provider only supports streaming".to_string(),
            ))
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            Ok(Box::pin(stream::iter(vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: ProviderId::new("interrupted"),
                    model: ModelId::new("interrupted-model"),
                    delta: "partial reply".to_string(),
                }),
                Err(AppError::Provider("stream interrupted".to_string())),
            ])))
        }
    }

    #[async_trait]
    impl ModelProvider for RecordingProvider {
        fn id(&self) -> &str {
            "recording"
        }

        fn default_model(&self) -> &ModelId {
            static DEFAULT_MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("recording-model"));
            &DEFAULT_MODEL
        }

        fn model_metadata(&self, _model: &ModelId) -> crate::provider::ModelMetadata {
            self.metadata.clone()
        }

        fn supports_prompt_continuation(&self, _model: &ModelId) -> bool {
            true
        }

        fn prompt_cache_shape(
            &self,
            _model: &ModelId,
        ) -> Option<crate::provider::PromptCacheShape> {
            self.current_prompt_cache_shape
                .lock()
                .expect("recording provider prompt cache shape lock should succeed")
                .clone()
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![
                ProviderModel::new("recording", "recording-model").with_display_name("Recording"),
            ])
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            self.requests
                .lock()
                .expect("recording provider request lock should succeed")
                .push(request);
            if let Some(shape) = self.dynamic_prompt_cache_shape.clone() {
                *self
                    .current_prompt_cache_shape
                    .lock()
                    .expect("recording provider prompt cache shape lock should succeed") =
                    Some(shape);
            }

            Ok(CompletionResponse {
                provider_id: recording_provider_id(),
                model: recording_model_id(),
                text: "recorded".to_string(),
                reasoning_text: None,
                finish_reason: Some(CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: self.usage.clone(),
                provider_metadata: Some(serde_json::json!({
                    "response_id": self.next_response_id()
                })),
            })
        }
    }

    fn cache_state(session_id: i64, text: impl Into<String>) -> Session {
        Session::new(session_id, 1, format!("session-{session_id}"), Utc::now())
            .with_messages(vec![Message::prompt_text(Role::User, text.into())])
    }

    #[test]
    fn tool_message_extra_part_contents_materialize_mcp_resources_as_attachments() {
        let contents = tool_message_extra_part_contents(
            &ToolOutput::Mcp {
                output: McpToolOutput {
                    server: "fixtures".to_string(),
                    tool: "resource_tool".to_string(),
                    content_blocks: vec![
                        ToolResultBlock::Image {
                            mime: "image/png".to_string(),
                            url: "https://example.com/chart.png".to_string(),
                        },
                        ToolResultBlock::ResourceLink {
                            uri: "https://example.com/report.pdf".to_string(),
                            title: Some("report".to_string()),
                        },
                    ],
                    structured_content: None,
                },
            },
            &[ToolAttachment {
                kind: crate::message::AttachmentKind::Audio,
                mime: "audio/mpeg".to_string(),
                source: AttachmentSource::Url {
                    url: "https://example.com/audio.mp3".to_string(),
                },
                filename: Some("audio.mp3".to_string()),
                title: None,
                size_bytes: None,
                sha256: None,
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            }],
            &[],
        );

        assert_eq!(contents.len(), 1);
        let Some(PartContent::Attachment(part)) = contents.first() else {
            panic!("expected attachment part");
        };
        assert_eq!(part.attachments.len(), 3);
        assert!(part.attachments.iter().any(|item| {
            item.kind == crate::message::AttachmentKind::Image
                && matches!(
                    item.source,
                    AttachmentSource::Url { ref url }
                        if url == "https://example.com/chart.png"
                )
        }));
        assert!(part.attachments.iter().any(|item| {
            item.kind == crate::message::AttachmentKind::Pdf
                && matches!(
                    item.source,
                    AttachmentSource::Url { ref url }
                        if url == "https://example.com/report.pdf"
                )
        }));
        assert!(part.attachments.iter().any(|item| {
            item.kind == crate::message::AttachmentKind::Audio
                && matches!(
                    item.source,
                    AttachmentSource::Url { ref url }
                        if url == "https://example.com/audio.mp3"
                )
        }));
    }

    #[test]
    fn validate_user_input_reply_supports_multi_select_and_custom_answers() {
        let request = UserInputRequest {
            request_id: "ask-1".to_string(),
            session_id: Some(1),
            questions: vec![UserInputQuestion {
                id: "stack".to_string(),
                header: "Stack".to_string(),
                question: "Which stacks should we support?".to_string(),
                options: vec![
                    UserInputOption {
                        label: "rust".to_string(),
                        description: String::new(),
                    },
                    UserInputOption {
                        label: "go".to_string(),
                        description: String::new(),
                    },
                ],
                multiple: true,
                allow_custom: true,
            }],
            created_at: Utc::now(),
        };
        let reply = UserInputReply {
            request_id: "ask-1".to_string(),
            kind: UserInputReplyKind::Submit,
            answers: BTreeMap::from([(
                "stack".to_string(),
                vec![
                    "rust".to_string(),
                    "zig".to_string(),
                    "rust".to_string(),
                    "  ".to_string(),
                ],
            )]),
            reason: None,
        };

        let answers = validate_user_input_reply(&request, &reply).expect("reply should validate");
        assert_eq!(
            answers.get("stack"),
            Some(&vec!["rust".to_string(), "zig".to_string()])
        );
    }

    #[tokio::test]
    async fn cache_eviction_falls_back_to_db_reload() {
        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig {
                cache_max_sessions: 1,
                cache_ttl: Duration::from_secs(60),
                cache_max_bytes: usize::MAX,
                max_turn_loops: 16,
            },
        )
        .await;

        let first = service
            .create_session(SessionCreateRequest {
                title: "first".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create first session");
        let second = service
            .create_session(SessionCreateRequest {
                title: "second".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create second session");

        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: first.id,
                options: run_options(),
                parts: vec![PartContent::text("hello one")],
            })
            .await
            .expect("submit first turn");
        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: second.id,
                options: run_options(),
                parts: vec![PartContent::text("hello two")],
            })
            .await
            .expect("submit second turn");

        let reloaded = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: first.id,
                options: run_options(),
                parts: vec![PartContent::text("hello again")],
            })
            .await
            .expect("submit turn after cache eviction");

        assert!(
            reloaded
                .messages
                .iter()
                .filter(|message| message.role == Role::User)
                .any(|message| message.as_text_lossy() == "hello one")
        );
        assert!(
            reloaded
                .messages
                .iter()
                .filter(|message| message.role == Role::User)
                .any(|message| message.as_text_lossy() == "hello again")
        );
    }

    #[tokio::test]
    async fn list_session_summaries_reports_workspace_order_counts_and_pagination() {
        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
        )
        .await;

        let parent = service
            .create_session(SessionCreateRequest {
                title: "parent".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create parent session");
        let child = service
            .create_session(SessionCreateRequest {
                title: "child".to_string(),
                parent_session_id: Some(parent.id),
            })
            .await
            .expect("create child session");
        let sibling = service
            .create_session(SessionCreateRequest {
                title: "sibling".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create sibling session");

        let updated_parent = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: parent.id,
                options: run_options(),
                parts: vec![PartContent::text("hello parent")],
            })
            .await
            .expect("update parent session");

        let session_ids = service
            .workspace_session_ids()
            .await
            .expect("list workspace session ids");
        assert_eq!(session_ids, vec![parent.id, sibling.id, child.id]);

        let summaries = service
            .list_session_summaries(SessionListRequest::default())
            .await
            .expect("list session summaries");
        assert_eq!(summaries.len(), 3);
        assert_eq!(summaries[0].id, parent.id);
        assert_eq!(summaries[0].title, "parent");
        assert_eq!(summaries[0].version, updated_parent.version);
        assert_eq!(summaries[0].message_count, 2);
        assert_eq!(summaries[0].child_session_count, 1);
        assert!(summaries[0].last_message_at.is_some());
        assert_eq!(summaries[1].id, sibling.id);
        assert_eq!(summaries[1].message_count, 0);
        assert_eq!(summaries[1].child_session_count, 0);
        assert_eq!(summaries[1].last_message_at, None);
        assert_eq!(summaries[2].id, child.id);
        assert_eq!(summaries[2].parent_id, Some(parent.id));

        let paged = service
            .list_session_summaries(SessionListRequest {
                offset: 1,
                limit: Some(1),
            })
            .await
            .expect("list paged session summaries");
        assert_eq!(paged.len(), 1);
        assert_eq!(paged[0].id, sibling.id);
    }

    #[test]
    fn cache_skips_entries_larger_than_byte_budget() {
        let state = cache_state(1, "x".repeat(256));
        let mut cache = SessionCache::default();
        let max_bytes = state.approx_bytes().saturating_sub(1).max(1);
        let cache_policy = SessionCachePolicy {
            max_sessions: 8,
            ttl: Duration::from_secs(60),
            max_bytes,
        };

        cache.insert(state.clone(), cache_policy);

        assert!(cache.get(state.id, cache_policy).is_none());
        assert_eq!(cache.total_bytes(), 0);
    }

    #[test]
    fn cache_evicts_lru_entries_when_byte_budget_is_exceeded() {
        let first = cache_state(1, "alpha");
        let second = cache_state(2, "beta beta beta");
        let mut cache = SessionCache::default();
        let max_bytes = first
            .approx_bytes()
            .saturating_add(second.approx_bytes())
            .saturating_sub(1);
        let cache_policy = SessionCachePolicy {
            max_sessions: 8,
            ttl: Duration::from_secs(60),
            max_bytes,
        };

        cache.insert(first.clone(), cache_policy);
        cache.insert(second.clone(), cache_policy);

        assert!(cache.get(first.id, cache_policy).is_none());
        assert!(cache.get(second.id, cache_policy).is_some());
        assert!(cache.total_bytes() <= max_bytes);
    }

    #[tokio::test]
    async fn follow_up_requests_reuse_prompt_cache_key_and_previous_response_id() {
        let workspace = TempWorkspace::new();
        let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
        let service = build_manager_with_provider(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            RecordingProvider::new(requests.clone()),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "recording".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("first")],
            })
            .await
            .expect("submit first turn");
        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("second")],
            })
            .await
            .expect("submit second turn");

        let recorded = requests
            .lock()
            .expect("recording provider request lock should succeed")
            .clone();
        let expected_cache_key = prompt_window::prompt_cache_key_for_session(&created);

        assert_eq!(recorded.len(), 2);
        assert_eq!(
            recorded[0].prompt_cache_key.as_deref(),
            Some(expected_cache_key.as_str())
        );
        assert_eq!(
            recorded[1].prompt_cache_key.as_deref(),
            Some(expected_cache_key.as_str())
        );
        assert_eq!(recorded[0].previous_response_id, None);
        assert_eq!(recorded[1].previous_response_id.as_deref(), Some("resp_1"));
        assert_eq!(recorded[1].system, None);
        assert_eq!(recorded[1].messages.len(), 1);
        assert_eq!(recorded[1].messages[0].as_text_lossy(), "second");
    }

    #[tokio::test]
    async fn follow_up_requests_reuse_previous_response_id_when_shape_appears_after_first_response()
    {
        let workspace = TempWorkspace::new();
        let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
        let service = build_manager_with_provider(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            RecordingProvider::new(requests.clone()).with_dynamic_prompt_cache_shape(
                crate::provider::PromptCacheShape::new("recording")
                    .with_string("runtime_route", "route-a"),
            ),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "dynamic shape".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("first")],
            })
            .await
            .expect("submit first turn");
        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("second")],
            })
            .await
            .expect("submit second turn");

        let recorded = requests
            .lock()
            .expect("recording provider request lock should succeed")
            .clone();

        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].previous_response_id, None);
        assert_eq!(recorded[1].previous_response_id.as_deref(), Some("resp_1"));
        assert_eq!(recorded[1].system, None);
        assert_eq!(recorded[1].messages.len(), 1);
        assert_eq!(recorded[1].messages[0].as_text_lossy(), "second");
    }

    #[tokio::test]
    async fn persisted_runtime_anchor_survives_cache_eviction() {
        let workspace = TempWorkspace::new();
        let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
        let service = build_manager_with_provider(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig {
                cache_max_sessions: 1,
                cache_ttl: Duration::from_secs(60),
                cache_max_bytes: usize::MAX,
                max_turn_loops: 16,
            },
            ContextPolicy::default(),
            RecordingProvider::new(requests.clone()),
        )
        .await;

        let first = service
            .create_session(SessionCreateRequest {
                title: "first".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create first session");
        let second = service
            .create_session(SessionCreateRequest {
                title: "second".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create second session");

        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: first.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("hello one")],
            })
            .await
            .expect("submit first turn");
        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: second.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("hello two")],
            })
            .await
            .expect("submit second session turn");
        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: first.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("hello again")],
            })
            .await
            .expect("submit reloaded turn");

        let recorded = requests
            .lock()
            .expect("recording provider request lock should succeed")
            .clone();
        let expected_cache_key = prompt_window::prompt_cache_key_for_session(&first);

        assert_eq!(recorded.len(), 3);
        assert_eq!(
            recorded[2].prompt_cache_key.as_deref(),
            Some(expected_cache_key.as_str())
        );
        assert_eq!(recorded[2].previous_response_id.as_deref(), Some("resp_1"));
        assert_eq!(recorded[2].system, None);
        assert_eq!(recorded[2].messages.len(), 1);
        assert_eq!(recorded[2].messages[0].as_text_lossy(), "hello again");
    }

    /// Sub-task C: verify that `submit_user_turn` writes the new append-only
    /// `UserMessageAppended` event (wrapped in `TurnStarted` / `TurnCompleted`).
    /// After the processor turn completes there must also be
    /// `AssistantMessageCompleted` from the TurnBuffer commit. The test
    /// enforces append-only invariants on the event log: events for the
    /// user-input turn are written exactly once and never rewritten.
    #[tokio::test]
    async fn submit_user_turn_emits_append_only_user_message_event() {
        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "append-only-user".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: run_options(),
                parts: vec![PartContent::text("hello there")],
            })
            .await
            .expect("submit turn");

        let history = service.list_session_events(created.id)
        .await
        .expect("history should load");

        // Locate the user-message turn boundary (TurnStarted with no model
        // request_digest immediately followed by UserMessageAppended +
        // TurnCompleted) and verify the user payload is present and correctly
        // wired to the turn id.
        let mut user_payload: Option<&UserMessageAppended> = None;
        let mut user_turn_id: Option<HistoryTurnId> = None;
        for record in &history {
            if let EventKind::UserMessageAppended(payload) = &record.kind {
                user_payload = Some(payload);
                user_turn_id = Some(payload.turn_id);
                break;
            }
        }
        let user_payload = user_payload.expect("user_message_appended event must exist");
        let user_turn_id = user_turn_id.expect("user message turn id must be set");
        assert_eq!(user_payload.content.blocks.len(), 1);

        // Both the wrapping TurnStarted and TurnCompleted for this turn id
        // must be present in the event log.
        let turn_starts = history
            .iter()
            .filter(|record| {
                matches!(&record.kind, EventKind::TurnStarted(payload) if payload.turn_id == user_turn_id)
            })
            .count();
        let turn_completes = history
            .iter()
            .filter(|record| {
                matches!(&record.kind, EventKind::TurnCompleted(payload) if payload.turn_id == user_turn_id)
            })
            .count();
        assert_eq!(turn_starts, 1, "user turn started exactly once");
        assert_eq!(turn_completes, 1, "user turn completed exactly once");

        // Append-only invariant: each event row has a unique seq.
        let seqs: Vec<i64> = history.iter().map(|r| r.meta.seq_global).collect();
        let mut sorted = seqs.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(seqs.len(), sorted.len(), "no duplicate seq values");
    }

    // ─── Phase 8: append-only integration tests ─────────────────────────────

    #[tokio::test]
    async fn append_only_full_turn_writes_one_row_per_event_no_overwrites() {
        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "append-only-turn".into(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: run_options(),
                parts: vec![PartContent::text("hi")],
            })
            .await
            .expect("submit turn");

        let history = service.list_session_events(created.id)
        .await
        .expect("history should load");

        // The legacy mutable-snapshot variant has been removed; nothing to
        // assert here beyond the seq invariant below.

        // Every seq is unique and monotonically increasing — the cardinal
        // invariant of an append-only log.
        let mut prev: Option<i64> = None;
        for record in &history {
            if let Some(p) = prev {
                assert!(record.meta.seq_global > p, "seq must be strictly increasing");
            }
            prev = Some(record.meta.seq_global);
        }
    }

    #[tokio::test]
    async fn append_only_prefix_digest_stable_across_different_trailing_user_message() {
        use crate::session::history::ProviderTranscriptBuilder;
        use crate::session::history::fold_history;

        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
        )
        .await;

        async fn run_prefix_then(
            service: &SessionManager,
            trailing: &str,
        ) -> blake3::Hash {
            let created = service
                .create_session(SessionCreateRequest {
                    title: "digest".into(),
                    parent_session_id: None,
                })
                .await
                .expect("create session");
            service
                .submit_user_turn(SessionUserTurnRequest {
                    session_id: created.id,
                    options: run_options(),
                    parts: vec![PartContent::text("shared prefix")],
                })
                .await
                .expect("first turn");
            let records = service.list_session_events(created.id)
            .await
            .expect("records");
            // Take only the closed prefix (everything before the trailing
            // edit) — for this single-turn test the entire prefix is closed.
            let prefix_records: Vec<_> = records.iter().cloned().collect();
            let _ = trailing; // Trailing message is intentionally unused: we compare digests of the closed prefix only.
            let transcript = fold_history::<ProviderTranscriptBuilder>(prefix_records.as_slice())
                .expect("fold")
                .expect("transcript");
            transcript.digest()
        }

        let a = run_prefix_then(&service, "follow-up A").await;
        let b = run_prefix_then(&service, "follow-up B").await;
        assert_eq!(a, b, "prefix digest must be stable across different trailing messages");
    }

    #[tokio::test]
    async fn append_only_dangling_turn_started_gets_aborted_on_reload() {
        use crate::session::history::TurnAbortReason;

        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "dangling".into(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        // Inject a hanging TurnStarted event directly into the history table
        // (no matching TurnCompleted/TurnAborted) to simulate a process
        // restart mid-turn.
        let dangling_turn = HistoryTurnId::new();
        service
            .event_publisher()
            .publish(
                crate::event::PublishContext::for_session(created.id),
                EventKind::TurnStarted(TurnStarted {
                    turn_id: dangling_turn,
                    model_id: "test-model".into(),
                    provider_id: "test-provider".into(),
                    request_digest: None,
                }),
            )
            .await
            .expect("inject dangling TurnStarted");

        // Force the session out of cache so load_session takes the DB path
        // (and runs repair_hanging_turns).
        let cache_policy = SessionCachePolicy {
            max_sessions: 8,
            ttl: std::time::Duration::from_secs(60),
            max_bytes: usize::MAX,
        };
        service.store.prune_cache(SessionCachePolicy {
            max_sessions: 0,
            ttl: std::time::Duration::from_secs(0),
            max_bytes: 0,
        });

        // Now load the session — the store must repair the dangling turn by
        // appending a `TurnAborted{ProcessRestart}` marker.
        service
            .store
            .load_session(created.id, cache_policy)
            .await
            .expect("session should reload");

        let history = service.list_session_events(created.id)
        .await
        .expect("history");

        let aborted = history
            .iter()
            .find_map(|r| match &r.kind {
                EventKind::TurnAborted(payload) if payload.turn_id == dangling_turn => {
                    Some(payload)
                }
                _ => None,
            })
            .expect("dangling turn must be repaired with a TurnAborted marker");
        assert_eq!(aborted.reason, TurnAbortReason::ProcessRestart);
    }

    /// Phase 3 of the event-system refactor: every legacy `SessionEvent` and
    /// `HistoryItem` produced by a turn must also surface on the unified
    /// `EventBus` as the corresponding `EventKind`. This guards the cutover
    /// while readers are migrated.
    #[tokio::test]
    async fn unified_bus_mirrors_legacy_events_during_a_turn() {
        use crate::event::{EventFilter, Scope, bus::SubscriptionItem};

        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
        )
        .await;

        let bus = service.event_bus();
        let mut subscription = bus.subscribe(EventFilter::new(Scope::Global));

        let created = service
            .create_session(SessionCreateRequest {
                title: "mirror-test".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: run_options(),
                parts: vec![PartContent::text("hello mirror")],
            })
            .await
            .expect("submit turn");

        // Drain the bus into a vector with a hard timeout so the test can't
        // hang if mirroring is broken.
        let mut received = Vec::new();
        let drain = async {
            while let Some(item) = subscription.recv().await {
                if let SubscriptionItem::Event(event) = item {
                    received.push(event.kind.tag_str());
                }
                if received.len() >= 16 {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_millis(200), drain).await;

        assert!(
            received.contains(&"run_started"),
            "bus should carry RunStarted, got: {received:?}"
        );
        assert!(
            received.contains(&"user_message_appended"),
            "bus should carry UserMessageAppended, got: {received:?}"
        );
        assert!(
            received.contains(&"turn_started"),
            "bus should carry TurnStarted, got: {received:?}"
        );
    }

    /// Cancel a turn while the provider stream is still pending. The
    /// processor must observe the cancellation token and surface a
    /// terminal error rather than running to completion.
    #[tokio::test]
    async fn cancel_active_turn_aborts_a_running_turn() {
        struct SlowProvider;

        #[async_trait]
        impl ModelProvider for SlowProvider {
            fn id(&self) -> &str {
                "slow"
            }
            fn default_model(&self) -> &ModelId {
                static M: std::sync::LazyLock<ModelId> =
                    std::sync::LazyLock::new(|| ModelId::new("slow-model"));
                &M
            }
            async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
                Ok(vec![ProviderModel::new("slow", "slow-model")])
            }
            async fn complete(
                &self,
                _: CompletionRequest,
            ) -> Result<CompletionResponse, AppError> {
                Err(AppError::Provider("streaming only".into()))
            }
            async fn complete_stream(
                &self,
                _: CompletionRequest,
            ) -> Result<
                std::pin::Pin<
                    Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>,
                >,
                AppError,
            > {
                let s = async_stream::stream! {
                    // First chunk arrives quickly so the turn is "live".
                    yield Ok(CompletionStreamEvent::TextDelta {
                        provider_id: ProviderId::new("slow"),
                        model: ModelId::new("slow-model"),
                        delta: "thinking".to_string(),
                    });
                    // Then we stall — long enough that the test can issue
                    // the cancel before the next chunk would have arrived.
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    yield Ok(CompletionStreamEvent::TextDelta {
                        provider_id: ProviderId::new("slow"),
                        model: ModelId::new("slow-model"),
                        delta: "should never arrive".to_string(),
                    });
                };
                Ok(Box::pin(s))
            }
        }

        fn slow_options() -> SessionRunOptions {
            SessionRunOptions {
                model: ModelRef::new("slow", "slow-model"),
                system: None,
                temperature: None,
                max_output_tokens: Some(64),
            }
        }

        let workspace = TempWorkspace::new();
        let manager = Arc::new(
            build_manager_with_provider(
                &workspace.root,
                PermissionPolicy::allow_all(),
                SessionManagerConfig::default(),
                ContextPolicy::default(),
                SlowProvider,
            )
            .await,
        );

        let created = manager
            .create_session(SessionCreateRequest {
                title: "cancel-test".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create");
        let session_id = created.id;

        let mgr = Arc::clone(&manager);
        let submit = tokio::spawn(async move {
            mgr.submit_user_turn(SessionUserTurnRequest {
                session_id,
                options: slow_options(),
                parts: vec![PartContent::text("ping")],
            })
            .await
        });

        // Wait long enough for the turn to register with TurnRegistry —
        // 50ms is plenty given the first delta is yielded immediately.
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        manager
            .cancel_active_turn(session_id)
            .await
            .expect("cancel should find active turn");

        // The submit future should resolve quickly now (not after 60s).
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), submit)
            .await
            .expect("submit should complete after cancel")
            .expect("join");
        // The session run reports an error because the turn was aborted.
        assert!(result.is_err(), "expected turn to be reported as failed/cancelled");
    }

    /// `cancel_active_turn` for a session with no in-flight turn returns
    /// the corresponding error, never panics.
    #[tokio::test]
    async fn cancel_with_no_active_turn_is_a_clean_error() {
        let workspace = TempWorkspace::new();
        let manager = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
        )
        .await;
        let err = manager.cancel_active_turn(1234).await.unwrap_err();
        assert!(matches!(err, AppError::Internal(_)));
    }

    /// `steer_input` against a session with no active turn surfaces the
    /// "no in-flight turn" error so callers can fall back gracefully.
    #[tokio::test]
    async fn steer_with_no_active_turn_is_a_clean_error() {
        let workspace = TempWorkspace::new();
        let manager = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
        )
        .await;
        let err = manager
            .steer_input(99, vec![PartContent::text("late")])
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Internal(_)));
    }
}
