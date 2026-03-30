use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use sea_orm::{DatabaseConnection, DbErr, EntityTrait, QueryOrder};

use crate::AppError;
use crate::db::crud::{message, permission_rule, session, session_runtime, workspace};
use crate::db::entities;
use crate::db::tx::with_transaction_and_effects;
use crate::event::{
    AgentEvent, ErrorInfo, MessagePartUpdatedEvent, ThreadFailedEvent, ThreadStartedEvent,
};
use crate::message::{
    BuiltinToolInput, ErrorPart, ExecutionStatus, Message, MessageMetadata, MessagePart,
    MessageSource, MessageStatus, PartContent, TimeRange, ToolExecutionPart, ToolInvocation,
    ToolOutput, ToolResultBlock,
};
use crate::permission::{
    PermissionAction, PermissionDecision, PermissionMode, PermissionReply, PermissionReplyKind,
    PermissionRequest, decide_from_mode,
};
use crate::role::Role;
use crate::tool::{BuiltinExecution, ToolError, ToolExecutor, ToolPermissionCheck};

use super::{Session, SessionProcessor, SessionRunRequest, SessionSnapshot};

const PERMISSION_REQUIRED_CODE: &str = "permission_required";
const PERMISSION_APPROVED_CODE: &str = "permission_approved";
const PERMISSION_DENIED_CODE: &str = "permission_denied";
const PROCESSOR_PART_ID_BLOCK: i64 = 1024;

#[derive(Debug, Clone)]
pub struct SessionServiceConfig {
    pub cache_max_sessions: usize,
    pub cache_ttl: Duration,
    pub max_turn_loops: usize,
}

impl Default for SessionServiceConfig {
    fn default() -> Self {
        Self {
            cache_max_sessions: 128,
            cache_ttl: Duration::from_secs(15 * 60),
            max_turn_loops: 16,
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
    pub provider_id: String,
    pub model: String,
    pub system: Option<String>,
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<u32>,
}

impl SessionRunOptions {
    fn completion_request(&self, messages: Vec<Message>) -> crate::provider::CompletionRequest {
        crate::provider::CompletionRequest {
            model: self.model.clone(),
            system: self.system.clone(),
            messages,
            temperature: self.temperature,
            max_output_tokens: self.max_output_tokens,
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
pub struct SessionServiceResponse {
    pub session: Session,
    pub messages: Vec<Message>,
    pub blocked: bool,
}

pub struct SessionService {
    db: DatabaseConnection,
    processor: SessionProcessor,
    tool_executor: ToolExecutor,
    cache: Arc<RwLock<SessionCache>>,
    id_allocator: Arc<RwLock<GlobalIdAllocator>>,
    config: SessionServiceConfig,
}

impl SessionService {
    pub fn new(
        db: DatabaseConnection,
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
    ) -> Self {
        Self {
            db,
            processor,
            tool_executor,
            cache: Arc::new(RwLock::new(SessionCache::default())),
            id_allocator: Arc::new(RwLock::new(GlobalIdAllocator::default())),
            config: SessionServiceConfig::default(),
        }
    }

    pub fn with_config(mut self, config: SessionServiceConfig) -> Self {
        self.config = config;
        self
    }

    pub fn prune_cache(&self) {
        if let Ok(mut cache) = self.cache.write() {
            cache.prune(self.config.cache_ttl);
            cache.enforce_limit(self.config.cache_max_sessions);
        }
    }

    pub async fn create_session(
        &self,
        request: SessionCreateRequest,
    ) -> Result<SessionServiceResponse, AppError> {
        let workspace_path = self.workspace_path_string();
        let cache = Arc::clone(&self.cache);
        let config = self.config.clone();
        let title = request.title;
        let parent_session_id = request.parent_session_id;

        let state = with_transaction_and_effects(&self.db, move |txn, effects| {
            let workspace_path = workspace_path.clone();
            let cache = Arc::clone(&cache);
            let config = config.clone();
            let title = title.clone();
            Box::pin(async move {
                let workspace_id =
                    workspace::ensure_workspace_id(txn, workspace_path.as_str()).await?;
                let created =
                    session::create_session(txn, workspace_id, parent_session_id, title).await?;
                let state = LoadedSessionState {
                    session: session_from_model_db(created)?,
                    messages: Vec::new(),
                };
                session_runtime::save_checkpoint(
                    txn,
                    state.session.id,
                    0,
                    SessionSnapshot {
                        session: state.session.clone(),
                    },
                    None,
                    Utc::now(),
                )
                .await?;

                let state_for_cache = state.clone();
                effects.push(async move {
                    if let Ok(mut guard) = cache.write() {
                        guard.insert(state_for_cache, config.cache_max_sessions, config.cache_ttl);
                    }
                });

                Ok(state)
            })
        })
        .await?;

        self.build_response(state)
    }

    pub async fn get_session(&self, session_id: i64) -> Result<SessionServiceResponse, AppError> {
        self.build_response(self.load_state(session_id).await?)
    }

    pub async fn submit_user_turn(
        &self,
        request: SessionUserTurnRequest,
    ) -> Result<SessionServiceResponse, AppError> {
        let mut state = self.load_state(request.session_id).await?;
        let ids = self.reserve_message_ids(request.parts.len()).await?;
        let user_message = build_message(
            ids,
            Role::User,
            MessageStatus::Completed,
            request.parts,
            MessageMetadata {
                source: MessageSource::User,
                parent_message_id: state.messages.last().map(|message| message.id),
                generated_by_call_id: None,
                model_provider_id: request.options.provider_id.clone(),
                model_id: request.options.model.clone(),
                tags: Vec::new(),
            },
        );
        state.messages.push(user_message.clone());
        state = self
            .persist_state_changes(state, vec![user_message], Vec::new(), None)
            .await?;

        self.run_until_stable(state, &request.options).await
    }

    pub async fn continue_session(
        &self,
        request: SessionContinueRequest,
    ) -> Result<SessionServiceResponse, AppError> {
        let state = self.load_state(request.session_id).await?;
        self.run_until_stable(state, &request.options).await
    }

    pub async fn reply_permission(
        &self,
        request: SessionPermissionReplyRequest,
    ) -> Result<SessionServiceResponse, AppError> {
        let mut state = self.load_state(request.session_id).await?;
        let pending = state
            .find_pending_permission_by_request_id(request.reply.request_id.as_str())?
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "pending permission request not found: {}",
                    request.reply.request_id
                ))
            })?;

        let reply_json = serde_json::to_string(&request.reply)?;
        let reply_reason = request
            .reply
            .reason
            .clone()
            .unwrap_or_else(|| pending.request.reason.clone());

        {
            let permission_part = &mut state.messages[pending.permission_message_index].parts
                [pending.permission_part_index];
            permission_part.set_content(PartContent::Error(ErrorPart {
                code: if matches!(
                    request.reply.kind,
                    PermissionReplyKind::AllowOnce | PermissionReplyKind::AllowAlways
                ) {
                    PERMISSION_APPROVED_CODE.to_string()
                } else {
                    PERMISSION_DENIED_CODE.to_string()
                },
                message: reply_json,
            }));
            permission_part.status = ExecutionStatus::Completed;
            permission_part.summary = Some(reply_reason.clone());
        }

        let persisted_mode = persisted_mode_for_reply(request.reply.kind);
        let persisted_action_key = persisted_mode
            .map(|_| permission_action_key(&pending.request.action))
            .transpose()?;

        match request.reply.kind {
            PermissionReplyKind::AllowOnce | PermissionReplyKind::AllowAlways => {
                let execution = self.execute_pending_tool(&pending.tool)?;
                state = self
                    .apply_tool_success(
                        state,
                        &pending.tool,
                        execution,
                        persisted_action_key,
                        persisted_mode,
                    )
                    .await?;
            }
            PermissionReplyKind::DenyOnce | PermissionReplyKind::DenyAlways => {
                state = self
                    .apply_tool_failure(
                        state,
                        &pending.tool,
                        reply_reason,
                        persisted_action_key,
                        persisted_mode,
                    )
                    .await?;
            }
        }

        self.run_until_stable(state, &request.options).await
    }

    async fn run_until_stable(
        &self,
        mut state: LoadedSessionState,
        options: &SessionRunOptions,
    ) -> Result<SessionServiceResponse, AppError> {
        for _ in 0..self.config.max_turn_loops {
            if state.find_any_pending_permission()?.is_some() {
                return self.build_response(state);
            }

            if let Some(pending_tool) = state.find_next_pending_tool()? {
                state = self.resolve_pending_tool(state, pending_tool).await?;
                continue;
            }

            if !state.should_run_model() {
                return self.build_response(state);
            }

            state = self.run_model_turn(state, options).await?;
        }

        Err(AppError::Internal(
            "session service exceeded max turn loop budget".to_string(),
        ))
    }

    async fn run_model_turn(
        &self,
        mut state: LoadedSessionState,
        options: &SessionRunOptions,
    ) -> Result<LoadedSessionState, AppError> {
        let processor_ids = self.reserve_processor_ids().await?;
        let run = SessionRunRequest {
            session_id: state.session.id,
            provider_id: options.provider_id.clone(),
            completion: options.completion_request(state.messages.clone()),
            next_message_id: processor_ids.message_id,
            next_part_id: processor_ids.first_part_id,
            next_call_id: state.next_call_id(),
        };

        match self.processor.run_turn(run).await {
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
                let mut events = vec![AgentEvent::ThreadStarted(ThreadStartedEvent {
                    thread_id: state.session.id,
                    ts_ms: Utc::now().timestamp_millis(),
                })];
                events.extend(result.events);
                state.messages.push(assistant_message.clone());
                self.persist_state_changes(state, vec![assistant_message], events, None)
                    .await
            }
            Err(err) => {
                self.persist_thread_failed(state.session.id, err.to_string())
                    .await?;
                Err(err)
            }
        }
    }

    async fn resolve_pending_tool(
        &self,
        state: LoadedSessionState,
        pending_tool: PendingToolTarget,
    ) -> Result<LoadedSessionState, AppError> {
        let ToolInvocation::Builtin { input } = pending_tool.invocation.clone() else {
            return self
                .apply_tool_failure(
                    state,
                    &pending_tool,
                    "unsupported non-builtin tool invocation".to_string(),
                    None,
                    None,
                )
                .await;
        };

        for check in self
            .tool_executor
            .collect_permission_checks(&input)
            .map_err(tool_error_to_app_error)?
        {
            let decision = self.resolve_permission_decision(&check).await?;
            match decision {
                PermissionDecision::Allow => {}
                PermissionDecision::Ask { reason } => {
                    return self
                        .apply_permission_request(state, &pending_tool, check.action, reason)
                        .await;
                }
                PermissionDecision::Deny { reason } => {
                    return self
                        .apply_tool_failure(state, &pending_tool, reason, None, None)
                        .await;
                }
            }
        }

        let execution = self.execute_builtin_unchecked(&input)?;
        self.apply_tool_success(state, &pending_tool, execution, None, None)
            .await
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
                if let Some(mode) = permission_rule::resolve_rule(&self.db, key.as_str()).await? {
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
        mut state: LoadedSessionState,
        pending_tool: &PendingToolTarget,
        action: PermissionAction,
        reason: String,
    ) -> Result<LoadedSessionState, AppError> {
        let request = PermissionRequest {
            request_id: pending_tool.operation_id.clone(),
            session_id: Some(state.session.id),
            action,
            reason: reason.clone(),
            created_at: Utc::now(),
        };

        {
            let tool_part =
                &mut state.messages[pending_tool.message_index].parts[pending_tool.part_index];
            tool_part.set_content(PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id: pending_tool.call_id,
                invocation: pending_tool.invocation.clone(),
                title: format!("Awaiting permission: {reason}"),
                lifecycle: pending_tool.lifecycle.clone(),
            }));
            tool_part.status = ExecutionStatus::Pending;
            tool_part.summary = Some(reason.clone());
        }

        let permission_part_id = self.reserve_part_id().await?;
        let permission_part = build_permission_part(
            permission_part_id,
            state.messages[pending_tool.message_index].id,
            pending_tool.operation_id.as_str(),
            PERMISSION_REQUIRED_CODE,
            serde_json::to_string(&request)?,
            reason,
            ExecutionStatus::Pending,
        );
        state.messages[pending_tool.message_index]
            .parts
            .push(permission_part.clone());

        let assistant_message = state.messages[pending_tool.message_index].clone();
        self.persist_state_changes(state, vec![assistant_message], Vec::new(), None)
            .await
    }

    async fn apply_tool_success(
        &self,
        mut state: LoadedSessionState,
        pending_tool: &PendingToolTarget,
        execution: BuiltinExecution,
        persisted_action_key: Option<String>,
        persisted_mode: Option<PermissionMode>,
    ) -> Result<LoadedSessionState, AppError> {
        let tool_output = ToolOutput::Builtin {
            output: execution.output.clone(),
        };
        let output_text = execution.view.output_text.clone();
        let lifecycle = completed_lifecycle(&pending_tool.lifecycle);
        let blocks = text_result_blocks(output_text.as_str());

        {
            let tool_part =
                &mut state.messages[pending_tool.message_index].parts[pending_tool.part_index];
            tool_part.set_content(PartContent::ToolExecution(ToolExecutionPart::Completed {
                call_id: pending_tool.call_id,
                invocation: pending_tool.invocation.clone(),
                output_text: output_text.clone(),
                blocks: blocks.clone(),
                attachments: execution.view.attachments.clone(),
                details: tool_output.clone(),
                lifecycle: lifecycle.clone(),
            }));
            tool_part.status = ExecutionStatus::Completed;
        }

        let tool_message = build_tool_message(
            self.reserve_message_ids(1).await?,
            pending_tool,
            execution.view.attachments,
            output_text,
            blocks,
            tool_output,
            lifecycle,
            None,
        );
        state.messages.push(tool_message.clone());

        let assistant_message = state.messages[pending_tool.message_index].clone();
        self.persist_state_changes(
            state,
            vec![assistant_message, tool_message],
            Vec::new(),
            persisted_rule_update(persisted_action_key, persisted_mode),
        )
        .await
    }

    async fn apply_tool_failure(
        &self,
        mut state: LoadedSessionState,
        pending_tool: &PendingToolTarget,
        reason: String,
        persisted_action_key: Option<String>,
        persisted_mode: Option<PermissionMode>,
    ) -> Result<LoadedSessionState, AppError> {
        let lifecycle = completed_lifecycle(&pending_tool.lifecycle);
        let blocks = text_result_blocks(reason.as_str());

        {
            let tool_part =
                &mut state.messages[pending_tool.message_index].parts[pending_tool.part_index];
            tool_part.set_content(PartContent::ToolExecution(ToolExecutionPart::Failed {
                call_id: pending_tool.call_id,
                invocation: pending_tool.invocation.clone(),
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
            self.reserve_message_ids(1).await?,
            pending_tool,
            Vec::new(),
            reason.clone(),
            blocks,
            ToolOutput::None,
            lifecycle,
            Some(reason),
        );
        state.messages.push(tool_message.clone());

        let assistant_message = state.messages[pending_tool.message_index].clone();
        self.persist_state_changes(
            state,
            vec![assistant_message, tool_message],
            Vec::new(),
            persisted_rule_update(persisted_action_key, persisted_mode),
        )
        .await
    }

    async fn persist_state_changes(
        &self,
        mut state: LoadedSessionState,
        touched_messages: Vec<Message>,
        mut extra_events: Vec<AgentEvent>,
        persisted_rule: Option<(String, PermissionMode)>,
    ) -> Result<LoadedSessionState, AppError> {
        let session_id = state.session.id;
        let mut unique_messages = HashMap::new();
        for message in touched_messages {
            unique_messages.insert(message.id, message);
        }
        let touched_messages = unique_messages.into_values().collect::<Vec<_>>();
        let now = Utc::now();
        let ts_ms = now.timestamp_millis();
        for message in &touched_messages {
            for part in &message.parts {
                extra_events.push(AgentEvent::MessagePartUpdated(MessagePartUpdatedEvent {
                    thread_id: session_id,
                    message_id: message.id,
                    part: part.clone(),
                    ts_ms,
                }));
            }
        }

        let cache = Arc::clone(&self.cache);
        let config = self.config.clone();
        let state_for_cache = state.clone();
        let updated_session = with_transaction_and_effects(&self.db, move |txn, effects| {
            let touched_messages = touched_messages.clone();
            let extra_events = extra_events.clone();
            let persisted_rule = persisted_rule.clone();
            let cache = Arc::clone(&cache);
            let config = config.clone();
            let state_for_cache = state_for_cache.clone();
            Box::pin(async move {
                for message in &touched_messages {
                    message::upsert_message_with_parts(txn, session_id, message).await?;
                }

                if let Some((action_key, mode)) = persisted_rule {
                    permission_rule::upsert_rule(txn, action_key.as_str(), mode).await?;
                }

                let updated_session = session::touch_session_updated_at(txn, session_id)
                    .await?
                    .ok_or_else(|| DbErr::Custom(format!("session not found: {session_id}")))?;
                let updated_session = session_from_model_db(updated_session)?;

                let mut next_seq = session_runtime::latest_checkpoint(txn, session_id)
                    .await?
                    .map(|checkpoint| checkpoint.upto_seq)
                    .unwrap_or(0);
                for event in extra_events {
                    next_seq += 1;
                    session_runtime::append_session_event(txn, session_id, next_seq, event, now)
                        .await?;
                }
                session_runtime::save_checkpoint(
                    txn,
                    session_id,
                    next_seq,
                    SessionSnapshot {
                        session: updated_session.clone(),
                    },
                    None,
                    now,
                )
                .await?;

                let mut state = state_for_cache.clone();
                state.session = updated_session.clone();
                effects.push(async move {
                    if let Ok(mut guard) = cache.write() {
                        guard.insert(state, config.cache_max_sessions, config.cache_ttl);
                    }
                });

                Ok(updated_session)
            })
        })
        .await?;

        state.session = updated_session;
        Ok(state)
    }

    async fn persist_thread_failed(&self, session_id: i64, reason: String) -> Result<(), AppError> {
        let event = AgentEvent::ThreadFailed(ThreadFailedEvent {
            thread_id: session_id,
            error: ErrorInfo {
                code: "session_run_failed".to_string(),
                message: reason,
            },
            ts_ms: Utc::now().timestamp_millis(),
        });
        let state = self.load_state(session_id).await?;
        let _ = self
            .persist_state_changes(state, Vec::new(), vec![event], None)
            .await?;
        Ok(())
    }

    async fn load_state(&self, session_id: i64) -> Result<LoadedSessionState, AppError> {
        if let Ok(mut cache) = self.cache.write()
            && let Some(state) = cache.get(session_id, self.config.cache_ttl)
        {
            return Ok(state);
        }

        let session_model = session::get_session_by_id(&self.db, session_id)
            .await?
            .ok_or_else(|| AppError::Internal(format!("session not found: {session_id}")))?;
        let state = LoadedSessionState {
            session: session_from_model(session_model)?,
            messages: message::list_messages_with_parts(&self.db, session_id).await?,
        };
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(
                state.clone(),
                self.config.cache_max_sessions,
                self.config.cache_ttl,
            );
        }
        Ok(state)
    }

    fn build_response(
        &self,
        state: LoadedSessionState,
    ) -> Result<SessionServiceResponse, AppError> {
        Ok(SessionServiceResponse {
            blocked: state.find_any_pending_permission()?.is_some(),
            session: state.session,
            messages: state.messages,
        })
    }

    fn execute_pending_tool(
        &self,
        pending_tool: &PendingToolTarget,
    ) -> Result<BuiltinExecution, AppError> {
        let ToolInvocation::Builtin { input } = pending_tool.invocation.clone() else {
            return Err(AppError::Internal(
                "permission reply attempted on non-builtin invocation".to_string(),
            ));
        };
        self.execute_builtin_unchecked(&input)
    }

    fn execute_builtin_unchecked(
        &self,
        input: &BuiltinToolInput,
    ) -> Result<BuiltinExecution, AppError> {
        self.tool_executor
            .execute_builtin_unchecked(input)
            .map_err(tool_error_to_app_error)
    }

    fn workspace_path_string(&self) -> String {
        self.tool_executor
            .workspace_root()
            .to_string_lossy()
            .replace('\\', "/")
    }

    async fn reserve_message_ids(&self, part_count: usize) -> Result<ReservedMessageIds, AppError> {
        self.ensure_id_allocator().await?;
        let mut allocator = self
            .id_allocator
            .write()
            .map_err(|_| AppError::Internal("session id allocator lock poisoned".to_string()))?;
        let message_id = allocator.next_message_id;
        allocator.next_message_id += 1;

        let first_part_id = allocator.next_part_id;
        allocator.next_part_id += part_count as i64;
        let part_ids = (0..part_count)
            .map(|index| first_part_id + index as i64)
            .collect::<Vec<_>>();

        Ok(ReservedMessageIds {
            message_id,
            part_ids,
        })
    }

    async fn reserve_part_id(&self) -> Result<i64, AppError> {
        self.ensure_id_allocator().await?;
        let mut allocator = self
            .id_allocator
            .write()
            .map_err(|_| AppError::Internal("session id allocator lock poisoned".to_string()))?;
        let part_id = allocator.next_part_id;
        allocator.next_part_id += 1;
        Ok(part_id)
    }

    async fn reserve_processor_ids(&self) -> Result<ReservedProcessorIds, AppError> {
        self.ensure_id_allocator().await?;
        let mut allocator = self
            .id_allocator
            .write()
            .map_err(|_| AppError::Internal("session id allocator lock poisoned".to_string()))?;
        let ids = ReservedProcessorIds {
            message_id: allocator.next_message_id,
            first_part_id: allocator.next_part_id,
        };
        allocator.next_message_id += 1;
        allocator.next_part_id += PROCESSOR_PART_ID_BLOCK;
        Ok(ids)
    }

    async fn ensure_id_allocator(&self) -> Result<(), AppError> {
        let initialized = self
            .id_allocator
            .read()
            .map_err(|_| AppError::Internal("session id allocator lock poisoned".to_string()))?
            .initialized;
        if initialized {
            return Ok(());
        }

        let next_message_id = entities::message::Entity::find()
            .order_by_desc(entities::message::Column::Id)
            .one(&self.db)
            .await?
            .map(|model| model.id + 1)
            .unwrap_or(1);
        let next_part_id = entities::message_part::Entity::find()
            .order_by_desc(entities::message_part::Column::Id)
            .one(&self.db)
            .await?
            .map(|model| model.id + 1)
            .unwrap_or(1);

        let mut allocator = self
            .id_allocator
            .write()
            .map_err(|_| AppError::Internal("session id allocator lock poisoned".to_string()))?;
        if !allocator.initialized {
            allocator.initialized = true;
            allocator.next_message_id = next_message_id;
            allocator.next_part_id = next_part_id;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct LoadedSessionState {
    session: Session,
    messages: Vec<Message>,
}

impl LoadedSessionState {
    fn next_call_id(&self) -> i64 {
        self.messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .filter_map(extract_call_id)
            .max()
            .unwrap_or(0)
            + 1
    }

    fn should_run_model(&self) -> bool {
        matches!(
            self.messages.last().map(|message| message.role),
            Some(Role::User | Role::Tool)
        )
    }

    fn find_any_pending_permission(&self) -> Result<Option<PendingPermissionTarget>, AppError> {
        for message_index in 0..self.messages.len() {
            let message = &self.messages[message_index];
            if message.role != Role::Assistant {
                continue;
            }

            for part_index in 0..message.parts.len() {
                let part = &message.parts[part_index];
                if part.status != ExecutionStatus::Pending {
                    continue;
                }
                let Some(operation_id) = part.operation_id.as_deref() else {
                    continue;
                };
                let Some(PartContent::ToolExecution(ToolExecutionPart::Pending {
                    call_id,
                    invocation,
                    title: _,
                    lifecycle,
                })) = part.content.as_ref()
                else {
                    continue;
                };

                if let Some((permission_part_index, request)) = self.find_permission_part(
                    message_index,
                    operation_id,
                    PERMISSION_REQUIRED_CODE,
                )? {
                    return Ok(Some(PendingPermissionTarget {
                        permission_message_index: message_index,
                        permission_part_index,
                        request,
                        tool: PendingToolTarget {
                            message_index,
                            part_index,
                            message_id: message.id,
                            operation_id: operation_id.to_string(),
                            call_id: *call_id,
                            invocation: invocation.clone(),
                            lifecycle: lifecycle.clone(),
                        },
                    }));
                }
            }
        }

        Ok(None)
    }

    fn find_pending_permission_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Option<PendingPermissionTarget>, AppError> {
        Ok(self
            .find_any_pending_permission()?
            .filter(|pending| pending.request.request_id == request_id))
    }

    fn find_next_pending_tool(&self) -> Result<Option<PendingToolTarget>, AppError> {
        for (message_index, message) in self.messages.iter().enumerate() {
            if message.role != Role::Assistant {
                continue;
            }

            for (part_index, part) in message.parts.iter().enumerate() {
                if part.status != ExecutionStatus::Pending {
                    continue;
                }
                let Some(operation_id) = part.operation_id.as_deref() else {
                    continue;
                };
                let Some(PartContent::ToolExecution(ToolExecutionPart::Pending {
                    call_id,
                    invocation,
                    title: _,
                    lifecycle,
                })) = part.content.as_ref()
                else {
                    continue;
                };

                if self
                    .find_permission_part(message_index, operation_id, PERMISSION_REQUIRED_CODE)?
                    .is_some()
                {
                    continue;
                }
                if self.has_tool_result(operation_id) {
                    continue;
                }

                return Ok(Some(PendingToolTarget {
                    message_index,
                    part_index,
                    message_id: message.id,
                    operation_id: operation_id.to_string(),
                    call_id: *call_id,
                    invocation: invocation.clone(),
                    lifecycle: lifecycle.clone(),
                }));
            }
        }

        Ok(None)
    }

    fn has_tool_result(&self, operation_id: &str) -> bool {
        self.messages
            .iter()
            .filter(|message| message.role == Role::Tool)
            .any(|message| {
                message
                    .parts
                    .iter()
                    .any(|part| part.operation_id.as_deref() == Some(operation_id))
            })
    }

    fn find_permission_part(
        &self,
        message_index: usize,
        operation_id: &str,
        code: &str,
    ) -> Result<Option<(usize, PermissionRequest)>, AppError> {
        let message = &self.messages[message_index];
        for (part_index, part) in message.parts.iter().enumerate() {
            if part.operation_id.as_deref() != Some(operation_id)
                || part.status != ExecutionStatus::Pending
            {
                continue;
            }
            let Some(PartContent::Error(error)) = part.content.as_ref() else {
                continue;
            };
            if error.code != code {
                continue;
            }
            let request = serde_json::from_str::<PermissionRequest>(error.message.as_str())?;
            return Ok(Some((part_index, request)));
        }
        Ok(None)
    }
}

#[derive(Debug, Clone)]
struct PendingToolTarget {
    message_index: usize,
    part_index: usize,
    message_id: i64,
    operation_id: String,
    call_id: i64,
    invocation: ToolInvocation,
    lifecycle: TimeRange,
}

#[derive(Debug, Clone)]
struct PendingPermissionTarget {
    permission_message_index: usize,
    permission_part_index: usize,
    request: PermissionRequest,
    tool: PendingToolTarget,
}

#[derive(Debug, Clone)]
struct CachedSessionEntry {
    state: LoadedSessionState,
    last_accessed: Instant,
}

#[derive(Debug, Default)]
struct GlobalIdAllocator {
    initialized: bool,
    next_message_id: i64,
    next_part_id: i64,
}

#[derive(Debug, Clone)]
struct ReservedMessageIds {
    message_id: i64,
    part_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy)]
struct ReservedProcessorIds {
    message_id: i64,
    first_part_id: i64,
}

#[derive(Debug, Default)]
struct SessionCache {
    entries: HashMap<i64, CachedSessionEntry>,
    access_order: VecDeque<i64>,
}

impl SessionCache {
    fn get(&mut self, session_id: i64, ttl: Duration) -> Option<LoadedSessionState> {
        self.prune(ttl);
        let state = {
            let entry = self.entries.get_mut(&session_id)?;
            entry.last_accessed = Instant::now();
            entry.state.clone()
        };
        self.bump(session_id);
        Some(state)
    }

    fn insert(&mut self, state: LoadedSessionState, max_sessions: usize, ttl: Duration) {
        self.prune(ttl);
        let session_id = state.session.id;
        self.entries.insert(
            session_id,
            CachedSessionEntry {
                state,
                last_accessed: Instant::now(),
            },
        );
        self.bump(session_id);
        self.enforce_limit(max_sessions);
    }

    fn prune(&mut self, ttl: Duration) {
        let now = Instant::now();
        let expired = self
            .entries
            .iter()
            .filter_map(|(session_id, entry)| {
                (now.saturating_duration_since(entry.last_accessed) > ttl).then_some(*session_id)
            })
            .collect::<Vec<_>>();
        for session_id in expired {
            self.entries.remove(&session_id);
            self.access_order.retain(|item| *item != session_id);
        }
    }

    fn enforce_limit(&mut self, max_sessions: usize) {
        while self.entries.len() > max_sessions.max(1) {
            let Some(session_id) = self.access_order.pop_front() else {
                break;
            };
            self.entries.remove(&session_id);
        }
    }

    fn bump(&mut self, session_id: i64) {
        self.access_order.retain(|item| *item != session_id);
        self.access_order.push_back(session_id);
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

    for (index, content) in parts.into_iter().enumerate() {
        let status = match &content {
            PartContent::ToolExecution(tool) => tool.status(),
            _ => ExecutionStatus::Completed,
        };
        let mut part =
            MessagePart::with_content(ids.part_ids[index], message.id, created_at, status, content);
        part.part_index = message.parts.len() as i32;
        message.parts.push(part);
    }

    message
}

fn build_tool_message(
    ids: ReservedMessageIds,
    pending_tool: &PendingToolTarget,
    attachments: Vec<crate::message::ToolAttachment>,
    output_text: String,
    blocks: Vec<ToolResultBlock>,
    details: ToolOutput,
    lifecycle: TimeRange,
    error_message: Option<String>,
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
        match &content {
            PartContent::ToolExecution(tool) => tool.status(),
            _ => ExecutionStatus::Completed,
        },
        content,
    );
    part.operation_id = Some(pending_tool.operation_id.clone());
    part.part_index = 0;

    Message {
        id: ids.message_id,
        role: Role::Tool,
        state: message_state,
        parts: vec![part],
        created_at,
        metadata: MessageMetadata {
            source: MessageSource::Tool,
            parent_message_id: Some(pending_tool.message_id),
            generated_by_call_id: Some(pending_tool.call_id),
            model_provider_id: String::new(),
            model_id: String::new(),
            tags: Vec::new(),
        },
        usage: None,
        finish: None,
    }
}

fn build_permission_part(
    part_id: i64,
    message_id: i64,
    operation_id: &str,
    code: &str,
    payload: String,
    summary: String,
    status: ExecutionStatus,
) -> MessagePart {
    let mut part = MessagePart::with_content(
        part_id,
        message_id,
        Utc::now(),
        status,
        PartContent::Error(ErrorPart {
            code: code.to_string(),
            message: payload,
        }),
    );
    part.operation_id = Some(operation_id.to_string());
    part.summary = Some(summary);
    part
}

fn completed_lifecycle(lifecycle: &TimeRange) -> TimeRange {
    TimeRange {
        start_ms: lifecycle.start_ms,
        end_ms: Some(Utc::now().timestamp_millis()),
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

fn session_from_model(model: crate::db::entities::session::Model) -> Result<Session, AppError> {
    Ok(Session {
        id: model.id,
        parent_id: model.parent_id,
        workspace_id: model.workspace_id,
        title: model.title,
        version: 1,
        created_at: timestamp_millis_to_utc(model.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(model.updated_at_ms)?,
    })
}

fn session_from_model_db(model: crate::db::entities::session::Model) -> Result<Session, DbErr> {
    Ok(Session {
        id: model.id,
        parent_id: model.parent_id,
        workspace_id: model.workspace_id,
        title: model.title,
        version: 1,
        created_at: timestamp_millis_to_utc_db(model.created_at_ms)?,
        updated_at: timestamp_millis_to_utc_db(model.updated_at_ms)?,
    })
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<DateTime<Utc>, AppError> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| AppError::Internal(format!("invalid timestamp millis: {timestamp_ms}")))
}

fn timestamp_millis_to_utc_db(timestamp_ms: i64) -> Result<DateTime<Utc>, DbErr> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| DbErr::Custom(format!("invalid timestamp millis: {timestamp_ms}")))
}

fn extract_call_id(part: &MessagePart) -> Option<i64> {
    part.content.as_ref().and_then(|content| match content {
        PartContent::ToolExecution(tool) => match tool {
            ToolExecutionPart::Pending { call_id, .. }
            | ToolExecutionPart::InProgress { call_id, .. }
            | ToolExecutionPart::Completed { call_id, .. }
            | ToolExecutionPart::Failed { call_id, .. } => Some(*call_id),
        },
        _ => None,
    })
}

fn tool_error_to_app_error(err: ToolError) -> AppError {
    match err {
        ToolError::PermissionDenied(reason) | ToolError::PermissionAsk(reason) => {
            AppError::Internal(reason)
        }
        other => AppError::Internal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use async_trait::async_trait;
    use futures_core::Stream;
    use futures_util::stream;
    use sea_orm::Database;
    use uuid::Uuid;

    use crate::agent::Agent;
    use crate::db::init_schema;
    use crate::message::{ToolExecutionPart, WriteToolInput};
    use crate::permission::{PermissionMode, PermissionPolicy};
    use crate::provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        ModelProvider, ProviderModel, ProviderRegistry,
    };
    use crate::session::{ContextGovernor, ContextPolicy};

    use super::*;

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

    #[async_trait]
    impl ModelProvider for ScriptedProvider {
        fn id(&self) -> &str {
            "scripted"
        }

        fn default_model(&self) -> &str {
            "scripted-model"
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel {
                provider_id: "scripted".to_string(),
                id: "scripted-model".to_string(),
                display_name: Some("Scripted".to_string()),
            }])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Ok(CompletionResponse {
                provider_id: "scripted".to_string(),
                model: "scripted-model".to_string(),
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
                    if part.operation_id.as_deref() != Some("call_write_1") {
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

            let events = if last_user_text.contains("write") && tool_result.is_none() {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
                        stream_key: "call_write_1".to_string(),
                        id: Some("call_write_1".to_string()),
                        name: Some("write".to_string()),
                        arguments_delta: serde_json::to_string(&WriteToolInput {
                            file_path: "result.txt".to_string(),
                            content: "approved\n".to_string(),
                        })
                        .expect("serialize tool input"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if let Some(tool_result) = tool_result {
                let delta = match tool_result {
                    Ok(_) => "write done".to_string(),
                    Err(_) => "write denied".to_string(),
                };
                vec![
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
                        delta,
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
                        finish_reason: Some(CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else {
                vec![
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
                        delta: format!("echo:{last_user_text}"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
                        finish_reason: Some(CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            };

            Ok(Box::pin(stream::iter(events)))
        }
    }

    async fn build_service(
        root: &std::path::Path,
        permission_policy: PermissionPolicy,
        config: SessionServiceConfig,
    ) -> SessionService {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("failed to create sqlite db");
        init_schema(&db).await.expect("failed to init schema");

        let mut registry = ProviderRegistry::new();
        registry.register(ScriptedProvider);
        let processor = SessionProcessor::new(
            Arc::new(registry),
            ContextGovernor::new(ContextPolicy::default()),
        );
        let executor = ToolExecutor::new(root, Agent::new("build", permission_policy));

        SessionService::new(db, processor, executor).with_config(config)
    }

    fn run_options() -> SessionRunOptions {
        SessionRunOptions {
            provider_id: "scripted".to_string(),
            model: "scripted-model".to_string(),
            system: None,
            temperature: None,
            max_output_tokens: Some(128),
        }
    }

    #[tokio::test]
    async fn permission_allow_reply_resumes_and_executes_tool() {
        let workspace = TempWorkspace::new();
        let service = build_service(
            &workspace.root,
            PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Ask),
            SessionServiceConfig::default(),
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
                session_id: created.session.id,
                options: run_options(),
                parts: vec![PartContent::text("please write a file")],
            })
            .await
            .expect("submit turn");
        assert!(blocked.blocked);

        let resumed = service
            .reply_permission(SessionPermissionReplyRequest {
                session_id: created.session.id,
                options: run_options(),
                reply: PermissionReply {
                    request_id: "call_write_1".to_string(),
                    kind: PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
            })
            .await
            .expect("reply permission");

        assert!(!resumed.blocked);
        let file_text = fs::read_to_string(workspace.root.join("result.txt"))
            .expect("tool should create result file");
        assert_eq!(file_text, "approved\n");
        assert_eq!(
            resumed
                .messages
                .last()
                .expect("assistant message should exist")
                .as_text_lossy(),
            "write done"
        );
    }

    #[tokio::test]
    async fn permission_deny_reply_marks_tool_failed_and_continues() {
        let workspace = TempWorkspace::new();
        let service = build_service(
            &workspace.root,
            PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Ask),
            SessionServiceConfig::default(),
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
                session_id: created.session.id,
                options: run_options(),
                parts: vec![PartContent::text("please write a file")],
            })
            .await
            .expect("submit turn");
        assert!(blocked.blocked);

        let resumed = service
            .reply_permission(SessionPermissionReplyRequest {
                session_id: created.session.id,
                options: run_options(),
                reply: PermissionReply {
                    request_id: "call_write_1".to_string(),
                    kind: PermissionReplyKind::DenyOnce,
                    reason: Some("operator denied".to_string()),
                    scope: None,
                },
            })
            .await
            .expect("reply permission");

        assert!(!resumed.blocked);
        assert!(!workspace.root.join("result.txt").exists());
        assert_eq!(
            resumed
                .messages
                .last()
                .expect("assistant message should exist")
                .as_text_lossy(),
            "write denied"
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
    async fn cache_eviction_falls_back_to_db_reload() {
        let workspace = TempWorkspace::new();
        let service = build_service(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionServiceConfig {
                cache_max_sessions: 1,
                cache_ttl: Duration::from_secs(60),
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
                session_id: first.session.id,
                options: run_options(),
                parts: vec![PartContent::text("hello one")],
            })
            .await
            .expect("submit first turn");
        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: second.session.id,
                options: run_options(),
                parts: vec![PartContent::text("hello two")],
            })
            .await
            .expect("submit second turn");

        let reloaded = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: first.session.id,
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
}
