use std::{collections::HashSet, sync::Arc, time::Duration};

use arc_swap::ArcSwap;
use chrono::Utc;

use crate::AppError;
use crate::event::{ErrorInfo, RunFailedEvent, RunStartedEvent, SessionEvent};
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
use super::model::{
    MESSAGE_TAG_PROMPT_COMPACTED, MESSAGE_TAG_PROMPT_SUMMARY, ProviderPromptAnchor,
    SessionListRequest, SessionPendingTool, SessionStatus, SessionSummary,
};
use super::processor::SessionRunRequest;
use super::prompt_window::{self, PromptRequestOptions};
use super::store::{ReservedMessageIds, SessionCommit, SessionStore};
use super::{Session, SessionEventRecord, SessionProcessor};

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
    execution: ArcSwap<SessionManagerState>,
}

impl SessionManager {
    pub fn new(
        db: sea_orm::DatabaseConnection,
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
    ) -> Self {
        let store = Arc::new(SessionStore::new(db, tool_executor.workspace_root()));
        let state =
            SessionManagerState::new(processor, tool_executor, SessionManagerConfig::default());
        Self {
            store,
            execution: ArcSwap::from_pointee(state),
        }
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

    pub async fn list_session_summaries(
        &self,
        request: SessionListRequest,
    ) -> Result<Vec<SessionSummary>, AppError> {
        self.store.list_session_summaries(request).await
    }

    pub async fn list_session_events(
        &self,
        session_id: i64,
    ) -> Result<Vec<SessionEventRecord>, AppError> {
        self.store.list_session_events(session_id).await
    }

    pub async fn submit_user_turn(
        &self,
        request: SessionUserTurnRequest,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
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
            .persist_session_changes(session, vec![user_message], Vec::new(), None, state.clone())
            .await?;

        self.run_until_stable(session, &request.options, state)
            .await
    }

    pub async fn continue_session(
        &self,
        request: SessionContinueRequest,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let session = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        self.run_until_stable(session, &request.options, state)
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

        self.run_until_stable(session, &request.options, state)
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

        self.run_until_stable(session, &request.options, state)
            .await
    }

    async fn run_until_stable(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        for _ in 0..state.config.max_turn_loops {
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
                SessionStatus::Idle => return Ok(session),
                SessionStatus::AwaitingModel => {}
            }

            session = self.run_model_turn(session, options, state.clone()).await?;
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
    ) -> Result<Session, AppError> {
        let mut compacted_rounds = 0_u8;

        loop {
            let active_messages = prompt_window::active_prompt_messages(&session);
            let tools = state
                .tool_executor
                .available_tools_for_messages(session.messages.as_slice());
            let prompt_budget =
                self.prompt_budget_for_turn(&session, options, tools.as_slice(), state.as_ref());
            let prompt_request_options = PromptRequestOptions {
                provider_id: options.model.provider_id.as_str(),
                model_id: options.model.model_id.as_str(),
                system: options.system.as_deref(),
                temperature: options.temperature,
                max_output_tokens: options.max_output_tokens,
                tools: tools.as_slice(),
                continuation_supported: state
                    .processor
                    .supports_prompt_continuation(&options.model),
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
                next_part_id: processor_ids.first_part_id,
                next_call_id: session.next_call_id(),
            };

            match state.processor.run_turn(run).await {
                Ok(result) => {
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
                    if let Some(usage) = assistant_message.usage.as_ref() {
                        session.runtime.record_prompt_tokens(
                            assistant_message.id,
                            usage,
                            prepared.prompt_window_generation,
                            prompt_budget.model_context_window_tokens,
                            prepared.system_fingerprint.clone(),
                            prepared.request_options_fingerprint.clone(),
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
                            system_fingerprint: prepared.system_fingerprint,
                            request_options_fingerprint: prepared.request_options_fingerprint,
                            transcript_digest,
                        });
                    } else {
                        session.runtime.clear_provider_anchor(
                            options.model.provider_id.as_str(),
                            options.model.model_id.as_str(),
                        );
                    }

                    let mut client_events = vec![SessionEvent::RunStarted(RunStartedEvent {
                        session_id: session.id,
                        ts_ms: Utc::now().timestamp_millis(),
                    })];
                    client_events.extend(result.client_events);
                    session.messages.push(assistant_message.clone());
                    return self
                        .persist_session_changes(
                            session,
                            vec![assistant_message],
                            client_events,
                            None,
                            state,
                        )
                        .await;
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
        let mut touched_messages = Vec::new();

        for message in &mut session.messages {
            if pruned_ids.contains(&message.id) && prompt_window::prune_tool_result_message(message)
            {
                touched_messages.push(message.clone());
            }
        }

        if touched_messages.is_empty() {
            return Ok(session);
        }

        self.invalidate_prompt_window_runtime(&mut session);
        self.persist_session_changes(session, touched_messages, Vec::new(), None, state)
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
        let mut touched_messages = Vec::new();

        for message in &mut session.messages {
            if stripped_ids.contains(&message.id)
                && prompt_window::strip_attachment_payloads(message)
            {
                touched_messages.push(message.clone());
            }
        }

        if touched_messages.is_empty() {
            return Ok(session);
        }

        self.invalidate_prompt_window_runtime(&mut session);
        self.persist_session_changes(session, touched_messages, Vec::new(), None, state)
            .await
    }

    async fn compact_prompt_window(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        active_messages: &[Message],
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let tools = state
            .tool_executor
            .available_tools_for_messages(session.messages.as_slice());
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

        let compacted_ids = plan
            .compacted_message_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>();
        let mut touched_messages = Vec::new();
        for message in &mut session.messages {
            if compacted_ids.contains(&message.id) {
                message.metadata.add_tag(MESSAGE_TAG_PROMPT_COMPACTED);
                touched_messages.push(message.clone());
            }
        }

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
        touched_messages.push(summary_message);

        self.persist_session_changes(session, touched_messages, Vec::new(), None, state)
            .await
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
        self.persist_session_changes(session, vec![assistant_message], Vec::new(), None, state)
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
            UserInputRequestPart::pending(request),
        );
        session.messages[pending_tool.part.message_index]
            .parts
            .push(input_part.clone());

        let assistant_message = session.messages[pending_tool.part.message_index].clone();
        self.persist_session_changes(session, vec![assistant_message], Vec::new(), None, state)
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
        self.persist_session_changes(
            session,
            vec![assistant_message, tool_message],
            Vec::new(),
            persisted_rule_update(persisted_action_key, persisted_mode),
            state,
        )
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
            Some(reason),
            Vec::new(),
        );
        session.messages.push(tool_message.clone());

        let assistant_message = session.messages[pending_tool.part.message_index].clone();
        self.persist_session_changes(
            session,
            vec![assistant_message, tool_message],
            Vec::new(),
            persisted_rule_update(persisted_action_key, persisted_mode),
            state,
        )
        .await
    }

    async fn persist_session_changes(
        &self,
        session: Session,
        touched_messages: Vec<Message>,
        client_events: Vec<SessionEvent>,
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
        let event = SessionEvent::RunFailed(RunFailedEvent {
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
    match details {
        ToolOutput::Builtin {
            output: BuiltinToolOutput::ApplyPatch { changes, .. },
        } if !changes.is_empty() => Some(FileChangePart {
            changes: changes.clone(),
        }),
        _ => None,
    }
}

fn todo_part_from_tool_output(details: &ToolOutput) -> Option<TodoListPart> {
    match details {
        ToolOutput::Builtin {
            output: BuiltinToolOutput::TodoWrite { items },
        } => Some(TodoListPart {
            items: items.clone(),
        }),
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
        ToolInvocation::Builtin { input } => input.to_string(),
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
        ToolOutput::Builtin {
            output: BuiltinToolOutput::AskUser { answers },
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
    use crate::session::{ContextGovernor, ContextPolicy};

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
                            details:
                                ToolOutput::Builtin {
                                    output: BuiltinToolOutput::AskUser { answers },
                                },
                            ..
                        })) => answers
                            .get("model_choice")
                            .and_then(|values| values.first().cloned())
                            .map(Ok),
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
                    matches!(
                        part.content.as_ref(),
                        Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                            details:
                                ToolOutput::Builtin {
                                output: BuiltinToolOutput::ToolSearch { loaded_tools, .. },
                                },
                            ..
                        })) if loaded_tools.iter().any(|tool| tool == "apply_patch")
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

            Ok(CompletionResponse {
                provider_id: recording_provider_id(),
                model: recording_model_id(),
                text: "recorded".to_string(),
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

    #[tokio::test]
    async fn permission_allow_reply_resumes_and_executes_tool() {
        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Ask),
            SessionManagerConfig::default(),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "allow".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let blocked = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: run_options(),
                parts: vec![PartContent::text("please patch a file")],
            })
            .await
            .expect("submit turn");
        assert!(blocked.blocked());
        assert!(blocked.messages.iter().any(|message| {
            message.parts.iter().any(|part| {
                matches!(
                    part.content.as_ref(),
                    Some(PartContent::PermissionRequest(permission))
                        if permission.reply.is_none()
                            && permission.request.request_id == "call_apply_patch_1"
                )
            })
        }));

        let resumed = service
            .reply_permission(SessionPermissionReplyRequest {
                session_id: created.id,
                options: run_options(),
                reply: PermissionReply {
                    request_id: "call_apply_patch_1".to_string(),
                    kind: PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
            })
            .await
            .expect("reply permission");

        assert!(!resumed.blocked());
        let file_text = fs::read_to_string(workspace.root.join("result.txt"))
            .expect("tool should create result file");
        assert_eq!(file_text, "approved\n");
        assert!(resumed.messages.iter().any(|message| {
            message.parts.iter().any(|part| {
                matches!(
                    part.content.as_ref(),
                    Some(PartContent::PermissionRequest(permission))
                        if matches!(
                            permission.reply.as_ref().map(|reply| reply.kind),
                            Some(PermissionReplyKind::AllowOnce)
                        )
                )
            })
        }));
        assert!(resumed.messages.iter().any(|message| {
            message.parts.iter().any(|part| {
                matches!(
                    part.content.as_ref(),
                    Some(PartContent::FileChange(change))
                        if change.changes.iter().any(|entry| {
                            entry.path == "result.txt" && entry.kind == FileChangeKind::Added
                        })
                )
            })
        }));
        assert_eq!(
            resumed
                .messages
                .last()
                .expect("assistant message should exist")
                .as_text_lossy(),
            "patch done"
        );
    }

    #[tokio::test]
    async fn permission_deny_reply_marks_tool_failed_and_continues() {
        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Ask),
            SessionManagerConfig::default(),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "deny".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let blocked = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: run_options(),
                parts: vec![PartContent::text("please patch a file")],
            })
            .await
            .expect("submit turn");
        assert!(blocked.blocked());

        let resumed = service
            .reply_permission(SessionPermissionReplyRequest {
                session_id: created.id,
                options: run_options(),
                reply: PermissionReply {
                    request_id: "call_apply_patch_1".to_string(),
                    kind: PermissionReplyKind::DenyOnce,
                    reason: Some("operator denied".to_string()),
                    scope: None,
                },
            })
            .await
            .expect("reply permission");

        assert!(!resumed.blocked());
        assert!(!workspace.root.join("result.txt").exists());
        assert_eq!(
            resumed
                .messages
                .last()
                .expect("assistant message should exist")
                .as_text_lossy(),
            "patch denied"
        );
        assert!(
            resumed
                .messages
                .iter()
                .filter(|message| message.role == Role::Tool)
                .any(|message| message.as_text_lossy().contains("operator denied"))
        );
    }

    #[tokio::test]
    async fn user_input_reply_completes_tool_and_resumes_turn() {
        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "user input".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let blocked = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: run_options(),
                parts: vec![PartContent::text("please choose model")],
            })
            .await
            .expect("submit turn");

        assert!(blocked.blocked());
        assert!(blocked.messages.iter().any(|message| {
            message.parts.iter().any(|part| {
                matches!(
                    part.content.as_ref(),
                    Some(PartContent::UserInputRequest(request))
                        if request.reply.is_none()
                            && request.request.request_id == "call_ask_user_1"
                )
            })
        }));

        let resumed = service
            .reply_user_input(SessionUserInputReplyRequest {
                session_id: created.id,
                options: run_options(),
                reply: UserInputReply {
                    request_id: "call_ask_user_1".to_string(),
                    kind: UserInputReplyKind::Submit,
                    answers: BTreeMap::from([(
                        "model_choice".to_string(),
                        vec!["gpt-5".to_string()],
                    )]),
                    reason: None,
                },
            })
            .await
            .expect("reply user input");

        assert!(!resumed.blocked());
        assert!(resumed.messages.iter().any(|message| {
            message.parts.iter().any(|part| {
                matches!(
                    part.content.as_ref(),
                    Some(PartContent::UserInputRequest(request))
                        if matches!(
                            request.reply.as_ref().map(|reply| reply.kind),
                            Some(UserInputReplyKind::Submit)
                        )
                )
            })
        }));
        assert!(
            resumed
                .messages
                .iter()
                .filter(|message| message.role == Role::Tool)
                .any(|message| {
                    message.parts.iter().any(|part| {
                        matches!(
                            part.content.as_ref(),
                            Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                                details:
                                    ToolOutput::Builtin {
                                        output: BuiltinToolOutput::AskUser { answers },
                                    },
                                ..
                            })) if answers
                                .get("model_choice")
                                .is_some_and(|values| values == &vec!["gpt-5".to_string()])
                        )
                    })
                })
        );
        assert_eq!(
            resumed
                .messages
                .last()
                .expect("assistant message should exist")
                .as_text_lossy(),
            "selected model: gpt-5"
        );
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
        let expected_cache_key = created.id.to_string();

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
        let expected_cache_key = first.id.to_string();

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

    #[tokio::test]
    async fn persisted_prompt_token_runtime_survives_cache_eviction_and_drives_compaction() {
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
            ContextPolicy {
                max_messages: 64,
                max_prompt_chars: 96_000,
                keep_tail_messages: 1,
                max_compaction_rounds: 2,
            },
            RecordingProvider::new(requests.clone())
                .with_metadata(
                    crate::provider::ModelMetadata::default()
                        .with_context_window_tokens(4_096)
                        .with_max_output_tokens(512),
                )
                .with_usage(high_recording_usage()),
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

        let first_turn = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: first.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("tiny")],
            })
            .await
            .expect("submit first turn");
        assert_eq!(
            first_turn.runtime.prompt_tokens.total_tokens(),
            Some(4_000),
            "successful turn should persist usage-backed prompt token runtime"
        );

        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: second.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("evict me")],
            })
            .await
            .expect("submit second session turn");
        let compacted = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: first.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("follow up")],
            })
            .await
            .expect("submit reloaded follow-up turn");

        let recorded = requests
            .lock()
            .expect("recording provider request lock should succeed")
            .clone();

        assert_eq!(recorded.len(), 3);
        assert_eq!(recorded[2].prompt_window_generation, Some(1));
        assert_eq!(recorded[2].previous_response_id, None);
        assert!(
            recorded[2]
                .messages
                .iter()
                .any(|message| message.metadata.has_tag(super::MESSAGE_TAG_PROMPT_SUMMARY))
        );
        assert!(
            compacted
                .messages
                .iter()
                .any(|message| message.metadata.has_tag(super::MESSAGE_TAG_PROMPT_SUMMARY))
        );
        assert_eq!(
            compacted.runtime.prompt_tokens.total_tokens(),
            Some(4_000),
            "a fresh successful turn should rebuild prompt token runtime after compaction"
        );
    }

    #[tokio::test]
    async fn tool_result_pruning_preserves_persisted_history_and_projects_placeholder() {
        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "tool prune".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");
        let state = service.execution_state();
        let mut session = service
            .store
            .load_session(created.id, state.cache_policy())
            .await
            .expect("load session");
        session.runtime.set_provider_anchor(ProviderPromptAnchor {
            provider_id: "recording".to_string(),
            model_id: "recording-model".to_string(),
            previous_response_id: "resp_prev".to_string(),
            assistant_message_id: 999,
            prompt_window_generation: 0,
            system_fingerprint: "system".to_string(),
            request_options_fingerprint: "request".to_string(),
            transcript_digest: String::new(),
        });
        session.runtime.record_prompt_tokens(
            999,
            &crate::message::MessageUsage {
                input_tokens: 1_000,
                output_tokens: 100,
                reasoning_tokens: 0,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                total_cost: 0.0,
            },
            0,
            Some(4_096),
            "system".to_string(),
            "request".to_string(),
            String::new(),
        );

        let old_output = "x".repeat(13_000);
        let mid_output = "y".repeat(13_000);
        let latest_output = "z".repeat(13_000);

        let user_one = build_message(
            service
                .store
                .reserve_message_ids(1)
                .await
                .expect("reserve ids"),
            Role::User,
            MessageStatus::Completed,
            vec![PartContent::text("first turn")],
            MessageMetadata {
                source: MessageSource::User,
                parent_message_id: None,
                generated_by_call_id: None,
                model_provider_id: "recording".to_string(),
                model_id: "recording-model".to_string(),
                tags: Vec::new(),
            },
        );
        let old_tool = build_message(
            service
                .store
                .reserve_message_ids(1)
                .await
                .expect("reserve ids"),
            Role::Tool,
            MessageStatus::Completed,
            vec![PartContent::ToolExecution(ToolExecutionPart::Completed {
                call_id: 1,
                invocation: ToolInvocation::Custom {
                    name: "tool".to_string(),
                    input: crate::message::StructuredObject::default(),
                },
                output_text: old_output.clone(),
                blocks: Vec::new(),
                attachments: Vec::new(),
                details: ToolOutput::None,
                lifecycle: TimeRange::default(),
            })],
            MessageMetadata {
                source: MessageSource::Tool,
                parent_message_id: Some(user_one.id),
                generated_by_call_id: None,
                model_provider_id: "recording".to_string(),
                model_id: "recording-model".to_string(),
                tags: Vec::new(),
            },
        );
        let user_two = build_message(
            service
                .store
                .reserve_message_ids(1)
                .await
                .expect("reserve ids"),
            Role::User,
            MessageStatus::Completed,
            vec![PartContent::text("second turn")],
            MessageMetadata {
                source: MessageSource::User,
                parent_message_id: Some(old_tool.id),
                generated_by_call_id: None,
                model_provider_id: "recording".to_string(),
                model_id: "recording-model".to_string(),
                tags: Vec::new(),
            },
        );
        let mid_tool = build_message(
            service
                .store
                .reserve_message_ids(1)
                .await
                .expect("reserve ids"),
            Role::Tool,
            MessageStatus::Completed,
            vec![PartContent::ToolExecution(ToolExecutionPart::Completed {
                call_id: 2,
                invocation: ToolInvocation::Custom {
                    name: "tool".to_string(),
                    input: crate::message::StructuredObject::default(),
                },
                output_text: mid_output,
                blocks: Vec::new(),
                attachments: Vec::new(),
                details: ToolOutput::None,
                lifecycle: TimeRange::default(),
            })],
            MessageMetadata {
                source: MessageSource::Tool,
                parent_message_id: Some(user_two.id),
                generated_by_call_id: None,
                model_provider_id: "recording".to_string(),
                model_id: "recording-model".to_string(),
                tags: Vec::new(),
            },
        );
        let user_three = build_message(
            service
                .store
                .reserve_message_ids(1)
                .await
                .expect("reserve ids"),
            Role::User,
            MessageStatus::Completed,
            vec![PartContent::text("third turn")],
            MessageMetadata {
                source: MessageSource::User,
                parent_message_id: Some(mid_tool.id),
                generated_by_call_id: None,
                model_provider_id: "recording".to_string(),
                model_id: "recording-model".to_string(),
                tags: Vec::new(),
            },
        );
        let latest_tool = build_message(
            service
                .store
                .reserve_message_ids(1)
                .await
                .expect("reserve ids"),
            Role::Tool,
            MessageStatus::Completed,
            vec![PartContent::ToolExecution(ToolExecutionPart::Completed {
                call_id: 3,
                invocation: ToolInvocation::Custom {
                    name: "tool".to_string(),
                    input: crate::message::StructuredObject::default(),
                },
                output_text: latest_output,
                blocks: Vec::new(),
                attachments: Vec::new(),
                details: ToolOutput::None,
                lifecycle: TimeRange::default(),
            })],
            MessageMetadata {
                source: MessageSource::Tool,
                parent_message_id: Some(user_three.id),
                generated_by_call_id: None,
                model_provider_id: "recording".to_string(),
                model_id: "recording-model".to_string(),
                tags: Vec::new(),
            },
        );
        let latest_user = build_message(
            service
                .store
                .reserve_message_ids(1)
                .await
                .expect("reserve ids"),
            Role::User,
            MessageStatus::Completed,
            vec![PartContent::text("latest turn")],
            MessageMetadata {
                source: MessageSource::User,
                parent_message_id: Some(latest_tool.id),
                generated_by_call_id: None,
                model_provider_id: "recording".to_string(),
                model_id: "recording-model".to_string(),
                tags: Vec::new(),
            },
        );

        let seed_messages = vec![
            user_one.clone(),
            old_tool.clone(),
            user_two,
            mid_tool,
            user_three,
            latest_tool,
            latest_user,
        ];
        session.messages.extend(seed_messages.clone());
        session = service
            .persist_session_changes(session, seed_messages, Vec::new(), None, state.clone())
            .await
            .expect("persist seed messages");

        let active_messages = prompt_window::active_prompt_messages(&session);
        let plan = prompt_window::plan_tool_result_pruning(active_messages.as_slice())
            .expect("tool prune plan should exist");
        assert_eq!(plan.pruned_message_ids, vec![old_tool.id]);

        let pruned = service
            .prune_tool_result_history(session, plan, state.clone())
            .await
            .expect("prune tool history");
        assert_eq!(pruned.runtime.prompt_window.generation, 1);
        assert!(pruned.runtime.provider_anchors.is_empty());
        assert!(pruned.runtime.prompt_tokens.is_empty());

        let reloaded = service
            .store
            .load_session(created.id, state.cache_policy())
            .await
            .expect("reload session");
        assert!(reloaded.runtime.provider_anchors.is_empty());
        assert!(reloaded.runtime.prompt_tokens.is_empty());
        let pruned_message = reloaded
            .messages
            .iter()
            .find(|message| message.id == old_tool.id)
            .expect("pruned tool message should exist");

        assert!(
            pruned_message
                .metadata
                .has_tag(crate::session::MESSAGE_TAG_TOOL_RESULT_PRUNED)
        );
        assert_eq!(pruned_message.as_text_lossy(), old_output);
        assert_eq!(
            crate::provider::project_session_text_lossy(pruned_message),
            "[tool_result:1]".to_string()
        );
    }

    #[tokio::test]
    async fn attachment_payload_stripping_preserves_history_and_projects_hints() {
        let workspace = TempWorkspace::new();
        let service = build_manager(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "attachment strip".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");
        let state = service.execution_state();
        let mut session = service
            .store
            .load_session(created.id, state.cache_policy())
            .await
            .expect("load session");

        let old_user = build_message(
            service
                .store
                .reserve_message_ids(2)
                .await
                .expect("reserve ids"),
            Role::User,
            MessageStatus::Completed,
            vec![
                PartContent::text("old image"),
                PartContent::attachments(vec![AttachmentItem {
                    kind: crate::message::AttachmentKind::Image,
                    mime: "image/png".to_string(),
                    source: AttachmentSource::DataUrl {
                        url: format!("data:image/png;base64,{}", "A".repeat(700_000)),
                    },
                    filename: Some("old.png".to_string()),
                    title: None,
                    size_bytes: None,
                    sha256: None,
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: None,
                }]),
            ],
            MessageMetadata {
                source: MessageSource::User,
                parent_message_id: None,
                generated_by_call_id: None,
                model_provider_id: "recording".to_string(),
                model_id: "recording-model".to_string(),
                tags: Vec::new(),
            },
        );
        let old_assistant = build_message(
            service
                .store
                .reserve_message_ids(1)
                .await
                .expect("reserve ids"),
            Role::Assistant,
            MessageStatus::Completed,
            vec![PartContent::text("acknowledged")],
            MessageMetadata {
                source: MessageSource::Assistant,
                parent_message_id: Some(old_user.id),
                generated_by_call_id: None,
                model_provider_id: "recording".to_string(),
                model_id: "recording-model".to_string(),
                tags: Vec::new(),
            },
        );
        let recent_user = build_message(
            service
                .store
                .reserve_message_ids(1)
                .await
                .expect("reserve ids"),
            Role::User,
            MessageStatus::Completed,
            vec![PartContent::text("recent turn")],
            MessageMetadata {
                source: MessageSource::User,
                parent_message_id: Some(old_assistant.id),
                generated_by_call_id: None,
                model_provider_id: "recording".to_string(),
                model_id: "recording-model".to_string(),
                tags: Vec::new(),
            },
        );
        let latest_user = build_message(
            service
                .store
                .reserve_message_ids(1)
                .await
                .expect("reserve ids"),
            Role::User,
            MessageStatus::Completed,
            vec![PartContent::text("latest turn")],
            MessageMetadata {
                source: MessageSource::User,
                parent_message_id: Some(recent_user.id),
                generated_by_call_id: None,
                model_provider_id: "recording".to_string(),
                model_id: "recording-model".to_string(),
                tags: Vec::new(),
            },
        );

        let seed_messages = vec![old_user.clone(), old_assistant, recent_user, latest_user];
        session.messages.extend(seed_messages.clone());
        session = service
            .persist_session_changes(session, seed_messages, Vec::new(), None, state.clone())
            .await
            .expect("persist seed messages");

        let active_messages = prompt_window::active_prompt_messages(&session);
        let plan = prompt_window::plan_attachment_payload_stripping(active_messages.as_slice())
            .expect("attachment strip plan should exist");
        assert_eq!(plan.stripped_message_ids, vec![old_user.id]);

        let stripped = service
            .strip_prompt_attachment_payloads(session, plan, state.clone())
            .await
            .expect("strip attachment payloads");
        assert_eq!(stripped.runtime.prompt_window.generation, 1);

        let reloaded = service
            .store
            .load_session(created.id, state.cache_policy())
            .await
            .expect("reload session");
        let stripped_message = reloaded
            .messages
            .iter()
            .find(|message| message.id == old_user.id)
            .expect("stripped message should exist");

        assert!(
            stripped_message
                .metadata
                .has_tag(crate::session::MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED)
        );
        assert!(stripped_message.as_text_lossy().contains("old.png"));
        assert_eq!(
            crate::provider::project_session_text_lossy(stripped_message),
            "old image[image:old.png]".to_string()
        );
    }

    #[tokio::test]
    async fn compaction_persists_summary_and_bumps_prompt_window_generation() {
        let workspace = TempWorkspace::new();
        let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
        let service = build_manager_with_provider(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy {
                max_messages: 2,
                max_prompt_chars: 96_000,
                keep_tail_messages: 1,
                max_compaction_rounds: 2,
            },
            RecordingProvider::new(requests.clone()),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "compact".to_string(),
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
        let compacted = service
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
        assert_eq!(recorded[1].prompt_window_generation, Some(1));
        assert_eq!(recorded[1].previous_response_id, None);
        assert_eq!(recorded[1].messages.len(), 2);
        assert!(
            recorded[1].messages[0]
                .metadata
                .has_tag(super::MESSAGE_TAG_PROMPT_SUMMARY)
        );
        assert_eq!(compacted.runtime.prompt_window.generation, 1);
        assert!(
            compacted
                .messages
                .iter()
                .any(|message| message.metadata.has_tag(super::MESSAGE_TAG_PROMPT_SUMMARY))
        );
        assert!(compacted.messages.iter().any(|message| {
            message
                .metadata
                .has_tag(super::MESSAGE_TAG_PROMPT_COMPACTED)
        }));
    }

    #[tokio::test]
    async fn compaction_uses_model_context_budget_even_when_message_count_is_small() {
        let workspace = TempWorkspace::new();
        let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
        let service = build_manager_with_provider(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy {
                max_messages: 64,
                max_prompt_chars: 96_000,
                keep_tail_messages: 1,
                max_compaction_rounds: 2,
            },
            RecordingProvider::new(requests.clone()).with_metadata(
                crate::provider::ModelMetadata::default()
                    .with_context_window_tokens(4_096)
                    .with_max_output_tokens(512),
            ),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "budget-compact".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("A".repeat(9_000))],
            })
            .await
            .expect("submit oversized first turn");
        let compacted = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: recording_run_options(),
                parts: vec![PartContent::text("follow up")],
            })
            .await
            .expect("submit follow-up turn");

        let recorded = requests
            .lock()
            .expect("recording provider request lock should succeed")
            .clone();

        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[1].prompt_window_generation, Some(1));
        assert!(
            recorded[1]
                .messages
                .iter()
                .any(|message| { message.metadata.has_tag(super::MESSAGE_TAG_PROMPT_SUMMARY) })
        );
        assert!(
            compacted
                .messages
                .iter()
                .any(|message| { message.metadata.has_tag(super::MESSAGE_TAG_PROMPT_SUMMARY) })
        );
    }
}
