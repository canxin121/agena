use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use sea_orm::{DatabaseConnection, DbErr, EntityTrait, QueryOrder};

use crate::AppError;
use crate::checkpoint::{
    CheckpointBlob, FilesystemCheckpointCapture, SessionRestoreMode, SessionRestorePointSnapshot,
    SessionRestoreRequest, restore_filesystem,
};
use crate::db::crud::{
    message, permission_rule, session, session_restore, session_runtime, workspace,
};
use crate::db::entities;
use crate::db::tx::with_transaction_and_effects;
use crate::event::{
    ErrorInfo, MessagePartUpdatedEvent, RunFailedEvent, RunStartedEvent, SessionEvent,
    SessionRestoredEvent,
};
use crate::message::{
    AttachmentItem, BuiltinToolOutput, ExecutionStatus, FileChangePart, Message, MessageMetadata,
    MessagePart, MessageSource, MessageStatus, PartContent, PermissionRequestPart, TimeRange,
    TodoListPart, ToolAttachment, ToolExecutionPart, ToolInvocation, ToolOutput, ToolResultBlock,
    UserInputReply, UserInputReplyKind, UserInputRequest, UserInputRequestPart,
};
use crate::permission::{
    PermissionAction, PermissionDecision, PermissionMode, PermissionReply, PermissionReplyKind,
    PermissionRequest, decide_from_mode,
};
use crate::role::Role;
use crate::tool::{ToolError, ToolExecutor, ToolInvocationExecution, ToolPermissionCheck};

use super::model::{SessionCacheSource, SessionPendingTool, SessionStatus};
use super::processor::SessionRunRequest;
use super::{Session, SessionEventRecord, SessionProcessor};

const PROCESSOR_PART_ID_BLOCK: i64 = 1024;

#[derive(Debug, Clone)]
pub struct SessionServiceConfig {
    pub cache_max_sessions: usize,
    pub cache_ttl: Duration,
    pub cache_max_bytes: usize,
    pub max_turn_loops: usize,
}

impl Default for SessionServiceConfig {
    fn default() -> Self {
        Self {
            cache_max_sessions: 128,
            cache_ttl: Duration::from_secs(15 * 60),
            cache_max_bytes: 64 * 1024 * 1024,
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
    fn completion_request(
        &self,
        messages: Vec<Message>,
        tools: Vec<crate::tool::ToolDefinition>,
    ) -> crate::provider::CompletionRequest {
        crate::provider::CompletionRequest {
            model: self.model.clone(),
            system: self.system.clone(),
            messages,
            tools,
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
pub struct SessionUserInputReplyRequest {
    pub session_id: i64,
    pub options: SessionRunOptions,
    pub reply: UserInputReply,
}

#[derive(Debug, Clone)]
struct PendingRestorePointWrite {
    call_id: i64,
    message_id: i64,
    operation_id: String,
    snapshot: SessionRestorePointSnapshot,
    blobs: Vec<CheckpointBlob>,
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
            cache.enforce_limit(self.config.cache_max_sessions, self.config.cache_max_bytes);
        }
    }

    pub async fn create_session(&self, request: SessionCreateRequest) -> Result<Session, AppError> {
        let workspace_path = self.workspace_path_string();
        let cache = Arc::clone(&self.cache);
        let config = self.config.clone();
        let title = request.title;
        let parent_session_id = request.parent_session_id;

        let session = with_transaction_and_effects(&self.db, move |txn, effects| {
            let workspace_path = workspace_path.clone();
            let cache = Arc::clone(&cache);
            let config = config.clone();
            let title = title.clone();
            Box::pin(async move {
                let workspace_id =
                    workspace::ensure_workspace_id(txn, workspace_path.as_str()).await?;
                let created =
                    session::create_session(txn, workspace_id, parent_session_id, title).await?;
                let created_session_id = created.id;
                let mut session = session_from_model_db(created)?;
                session.set_cache_source(SessionCacheSource::Fresh);
                session.refresh_derived();
                session_runtime::save_checkpoint(
                    txn,
                    session.id,
                    0,
                    session.clone(),
                    None,
                    Utc::now(),
                )
                .await?;

                let session_for_cache = session.clone();
                effects.push(async move {
                    if let Ok(mut guard) = cache.write() {
                        guard.insert(
                            session_for_cache,
                            config.cache_max_sessions,
                            config.cache_max_bytes,
                            config.cache_ttl,
                        );
                        if let Some(parent_session_id) = parent_session_id {
                            guard.append_child_session(parent_session_id, created_session_id);
                        }
                    }
                });

                Ok(session)
            })
        })
        .await?;

        Ok(session)
    }

    pub async fn get_session(&self, session_id: i64) -> Result<Session, AppError> {
        self.load_session(session_id).await
    }

    pub async fn list_session_events(
        &self,
        session_id: i64,
    ) -> Result<Vec<SessionEventRecord>, AppError> {
        Ok(session_runtime::list_session_events(&self.db, session_id).await?)
    }

    pub async fn restore_session(
        &self,
        request: SessionRestoreRequest,
    ) -> Result<Session, AppError> {
        let restore_point = match request.restore_point_id {
            Some(id) => {
                session_restore::find_restore_point(&self.db, request.session_id, id).await?
            }
            None => session_restore::latest_restore_point(&self.db, request.session_id).await?,
        }
        .ok_or_else(|| {
            AppError::Internal(format!(
                "restore point not found for session {}",
                request.session_id
            ))
        })?;

        let restored_paths = if request.mode.restores_filesystem() {
            let blobs = self
                .load_restore_blobs(restore_point.snapshot.filesystem.journal())
                .await?;
            restore_filesystem(
                self.tool_executor.workspace_root(),
                &restore_point.snapshot.filesystem,
                |hash| Ok(blobs.get(hash).cloned()),
            )
            .map_err(|err| AppError::Internal(err.to_string()))?
            .restored_paths
        } else {
            Vec::new()
        };

        let state = if request.mode.restores_conversation() {
            self.restore_conversation_state(&restore_point, request.mode, restored_paths.clone())
                .await?
        } else {
            let session = self.load_session(request.session_id).await?;
            let event = SessionEvent::SessionRestored(SessionRestoredEvent {
                session_id: request.session_id,
                restore_point_id: restore_point.id,
                mode: request.mode,
                restored_paths,
                ts_ms: Utc::now().timestamp_millis(),
            });
            self.persist_session_changes(session, Vec::new(), vec![event], None, None)
                .await?
        };

        Ok(state)
    }

    pub async fn submit_user_turn(
        &self,
        request: SessionUserTurnRequest,
    ) -> Result<Session, AppError> {
        let mut session = self.load_session(request.session_id).await?;
        let ids = self.reserve_message_ids(request.parts.len()).await?;
        let user_message = build_message(
            ids,
            Role::User,
            MessageStatus::Completed,
            request.parts,
            MessageMetadata {
                source: MessageSource::User,
                parent_message_id: session.messages.last().map(|message| message.id),
                generated_by_call_id: None,
                model_provider_id: request.options.provider_id.clone(),
                model_id: request.options.model.clone(),
                tags: Vec::new(),
            },
        );
        session.messages.push(user_message.clone());
        session = self
            .persist_session_changes(session, vec![user_message], Vec::new(), None, None)
            .await?;

        self.run_until_stable(session, &request.options).await
    }

    pub async fn continue_session(
        &self,
        request: SessionContinueRequest,
    ) -> Result<Session, AppError> {
        let session = self.load_session(request.session_id).await?;
        self.run_until_stable(session, &request.options).await
    }

    pub async fn reply_permission(
        &self,
        request: SessionPermissionReplyRequest,
    ) -> Result<Session, AppError> {
        let mut session = self.load_session(request.session_id).await?;
        let pending = session
            .find_pending_permission_by_request_id(request.reply.request_id.as_str())
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "pending permission request not found: {}",
                    request.reply.request_id
                ))
            })?;

        let reply_reason = request
            .reply
            .reason
            .clone()
            .unwrap_or_else(|| pending.request.reason.clone());

        {
            let permission_part = &mut session.messages[pending.permission_message_index].parts
                [pending.permission_part_index];
            permission_part.set_content(PartContent::PermissionRequest(
                PermissionRequestPart::pending(pending.request.clone())
                    .with_reply(request.reply.clone()),
            ));
            permission_part.status = ExecutionStatus::Completed;
        }

        let persisted_mode = persisted_mode_for_reply(request.reply.kind);
        let persisted_action_key = persisted_mode
            .map(|_| permission_action_key(&pending.request.action))
            .transpose()?;

        match request.reply.kind {
            PermissionReplyKind::AllowOnce | PermissionReplyKind::AllowAlways => {
                let execution = self
                    .execute_pending_tool_after_approval(session.id, &pending.tool)
                    .map_err(tool_error_to_app_error)?;
                session = self
                    .apply_tool_success(
                        session,
                        &pending.tool,
                        execution,
                        persisted_action_key,
                        persisted_mode,
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
                    )
                    .await?;
            }
        }

        self.run_until_stable(session, &request.options).await
    }

    pub async fn reply_user_input(
        &self,
        request: SessionUserInputReplyRequest,
    ) -> Result<Session, AppError> {
        let mut session = self.load_session(request.session_id).await?;
        let pending = session
            .find_pending_user_input_by_request_id(request.reply.request_id.as_str())
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "pending user input request not found: {}",
                    request.reply.request_id
                ))
            })?;

        {
            let input_part =
                &mut session.messages[pending.input_message_index].parts[pending.input_part_index];
            input_part.set_content(PartContent::UserInputRequest(
                UserInputRequestPart::pending(pending.request.clone())
                    .with_reply(request.reply.clone()),
            ));
            input_part.status = ExecutionStatus::Completed;
        }

        match request.reply.kind {
            UserInputReplyKind::Submit => {
                let execution = user_input_execution(&pending.request, &request.reply)?;
                session = self
                    .apply_tool_success(session, &pending.tool, execution, None, None)
                    .await?;
            }
            UserInputReplyKind::Cancel => {
                let reason =
                    request.reply.reason.clone().unwrap_or_else(|| {
                        "user declined to answer requested questions".to_string()
                    });
                session = self
                    .apply_tool_failure(session, &pending.tool, reason, None, None)
                    .await?;
            }
        }

        self.run_until_stable(session, &request.options).await
    }

    async fn run_until_stable(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
    ) -> Result<Session, AppError> {
        for _ in 0..self.config.max_turn_loops {
            session.refresh_derived();
            match session.status().clone() {
                SessionStatus::AwaitingPermission { .. }
                | SessionStatus::AwaitingUserInput { .. }
                | SessionStatus::Idle => return Ok(session),
                SessionStatus::AwaitingTool { tool } => {
                    session = self.resolve_pending_tool(session, tool).await?;
                    continue;
                }
                SessionStatus::AwaitingModel => {}
            }

            session = self.run_model_turn(session, options).await?;
        }

        Err(AppError::Internal(
            "session service exceeded max turn loop budget".to_string(),
        ))
    }

    async fn run_model_turn(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
    ) -> Result<Session, AppError> {
        let processor_ids = self.reserve_processor_ids().await?;
        let run = SessionRunRequest {
            session_id: session.id,
            provider_id: options.provider_id.clone(),
            completion: options.completion_request(
                session.messages.clone(),
                self.tool_executor
                    .available_tools_for_messages(session.messages.as_slice()),
            ),
            next_message_id: processor_ids.message_id,
            next_part_id: processor_ids.first_part_id,
            next_call_id: session.next_call_id(),
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
                let mut client_events = vec![SessionEvent::RunStarted(RunStartedEvent {
                    session_id: session.id,
                    ts_ms: Utc::now().timestamp_millis(),
                })];
                client_events.extend(result.client_events);
                session.messages.push(assistant_message.clone());
                self.persist_session_changes(
                    session,
                    vec![assistant_message],
                    client_events,
                    None,
                    None,
                )
                .await
            }
            Err(err) => {
                self.persist_run_failed_event(session.id, err.to_string())
                    .await?;
                Err(err)
            }
        }
    }

    async fn resolve_pending_tool(
        &self,
        mut session: Session,
        mut pending_tool: SessionPendingTool,
    ) -> Result<Session, AppError> {
        let prepared = self
            .tool_executor
            .prepare_invocation(&pending_tool.invocation, session.id, pending_tool.call_id)
            .map_err(tool_error_to_app_error)?;
        if prepared.invocation != pending_tool.invocation || prepared.title_override.is_some() {
            let current_title = match session.messages[pending_tool.message_index].parts
                [pending_tool.part_index]
                .content
                .as_ref()
            {
                Some(PartContent::ToolExecution(ToolExecutionPart::Pending { title, .. })) => {
                    title.clone()
                }
                _ => format!("Tool {}", tool_name(&pending_tool.invocation)),
            };

            pending_tool.invocation = prepared.invocation.clone();
            let tool_part =
                &mut session.messages[pending_tool.message_index].parts[pending_tool.part_index];
            tool_part.set_content(PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id: pending_tool.call_id,
                invocation: prepared.invocation,
                title: prepared.title_override.unwrap_or(current_title),
                lifecycle: pending_tool.lifecycle.clone(),
            }));
        }

        for check in self
            .tool_executor
            .collect_permission_checks_for_invocation(&pending_tool.invocation)
            .map_err(tool_error_to_app_error)?
        {
            let decision = self.resolve_permission_decision(&check).await?;
            match decision {
                PermissionDecision::Allow => {}
                PermissionDecision::Ask { reason } => {
                    return self
                        .apply_permission_request(session, &pending_tool, check.action, reason)
                        .await;
                }
                PermissionDecision::Deny { reason } => {
                    return self
                        .apply_tool_failure(session, &pending_tool, reason, None, None)
                        .await;
                }
            }
        }

        match self.execute_pending_tool(session.id, &pending_tool) {
            Ok(execution) => {
                self.apply_tool_success(session, &pending_tool, execution, None, None)
                    .await
            }
            Err(ToolError::UserInputRequired(input)) => {
                self.apply_user_input_request(session, &pending_tool, input)
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
        mut session: Session,
        pending_tool: &SessionPendingTool,
        action: PermissionAction,
        reason: String,
    ) -> Result<Session, AppError> {
        let request = PermissionRequest {
            request_id: pending_tool.operation_id.clone(),
            session_id: Some(session.id),
            action,
            reason: reason.clone(),
            created_at: Utc::now(),
        };

        {
            let tool_part =
                &mut session.messages[pending_tool.message_index].parts[pending_tool.part_index];
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
            session.messages[pending_tool.message_index].id,
            pending_tool.operation_id.as_str(),
            PermissionRequestPart::pending(request),
        );
        session.messages[pending_tool.message_index]
            .parts
            .push(permission_part.clone());

        let assistant_message = session.messages[pending_tool.message_index].clone();
        self.persist_session_changes(session, vec![assistant_message], Vec::new(), None, None)
            .await
    }

    async fn apply_user_input_request(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        input: crate::message::RequestUserInputToolInput,
    ) -> Result<Session, AppError> {
        let request = UserInputRequest {
            request_id: pending_tool.operation_id.clone(),
            session_id: Some(session.id),
            questions: input.questions,
            created_at: Utc::now(),
        };

        {
            let tool_part =
                &mut session.messages[pending_tool.message_index].parts[pending_tool.part_index];
            tool_part.set_content(PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id: pending_tool.call_id,
                invocation: pending_tool.invocation.clone(),
                title: request_user_input_title(&request),
                lifecycle: pending_tool.lifecycle.clone(),
            }));
            tool_part.status = ExecutionStatus::Pending;
            tool_part.summary = Some(format!(
                "Awaiting user input for {} question(s)",
                request.questions.len()
            ));
        }

        let input_part_id = self.reserve_part_id().await?;
        let input_part = build_user_input_part(
            input_part_id,
            session.messages[pending_tool.message_index].id,
            pending_tool.operation_id.as_str(),
            UserInputRequestPart::pending(request),
        );
        session.messages[pending_tool.message_index]
            .parts
            .push(input_part.clone());

        let assistant_message = session.messages[pending_tool.message_index].clone();
        self.persist_session_changes(session, vec![assistant_message], Vec::new(), None, None)
            .await
    }

    async fn apply_tool_success(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        execution: ToolInvocationExecution,
        persisted_action_key: Option<String>,
        persisted_mode: Option<PermissionMode>,
    ) -> Result<Session, AppError> {
        let restore_point = pending_restore_point_write(
            &session,
            pending_tool,
            execution.filesystem_checkpoint.clone(),
        );
        let tool_output = execution.output.clone();
        let output_text = execution.view.output_text.clone();
        let lifecycle = completed_lifecycle(&pending_tool.lifecycle);
        let blocks = text_result_blocks(output_text.as_str());
        let extra_part_contents = tool_message_extra_part_contents(
            &tool_output,
            execution.view.attachments.as_slice(),
            blocks.as_slice(),
        );

        {
            let tool_part =
                &mut session.messages[pending_tool.message_index].parts[pending_tool.part_index];
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
            self.reserve_message_ids(1 + extra_part_contents.len())
                .await?,
            pending_tool,
            execution.view.attachments,
            output_text,
            blocks,
            tool_output,
            lifecycle,
            None,
            extra_part_contents,
        );
        session.messages.push(tool_message.clone());

        let assistant_message = session.messages[pending_tool.message_index].clone();
        self.persist_session_changes(
            session,
            vec![assistant_message, tool_message],
            Vec::new(),
            persisted_rule_update(persisted_action_key, persisted_mode),
            restore_point,
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
    ) -> Result<Session, AppError> {
        let lifecycle = completed_lifecycle(&pending_tool.lifecycle);
        let blocks = text_result_blocks(reason.as_str());

        {
            let tool_part =
                &mut session.messages[pending_tool.message_index].parts[pending_tool.part_index];
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
            Vec::new(),
        );
        session.messages.push(tool_message.clone());

        let assistant_message = session.messages[pending_tool.message_index].clone();
        self.persist_session_changes(
            session,
            vec![assistant_message, tool_message],
            Vec::new(),
            persisted_rule_update(persisted_action_key, persisted_mode),
            None,
        )
        .await
    }

    async fn persist_session_changes(
        &self,
        mut session: Session,
        touched_messages: Vec<Message>,
        mut client_events: Vec<SessionEvent>,
        persisted_rule: Option<(String, PermissionMode)>,
        restore_point: Option<PendingRestorePointWrite>,
    ) -> Result<Session, AppError> {
        let session_id = session.id;
        let mut unique_messages = HashMap::new();
        for message in touched_messages {
            unique_messages.insert(message.id, message);
        }
        let touched_messages = unique_messages.into_values().collect::<Vec<_>>();
        let now = Utc::now();
        let ts_ms = now.timestamp_millis();
        for message in &touched_messages {
            for part in &message.parts {
                client_events.push(SessionEvent::MessagePartUpdated(MessagePartUpdatedEvent {
                    session_id,
                    message_id: message.id,
                    part: part.clone(),
                    ts_ms,
                }));
            }
        }

        let cache = Arc::clone(&self.cache);
        let config = self.config.clone();
        let session_for_cache = session.clone();
        let updated_session = with_transaction_and_effects(&self.db, move |txn, effects| {
            let touched_messages = touched_messages.clone();
            let client_events = client_events.clone();
            let persisted_rule = persisted_rule.clone();
            let restore_point = restore_point.clone();
            let cache = Arc::clone(&cache);
            let config = config.clone();
            let session_for_cache = session_for_cache.clone();
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
                for event in client_events {
                    next_seq += 1;
                    session_runtime::append_session_event(txn, session_id, next_seq, event, now)
                        .await?;
                }

                if let Some(restore_point) = restore_point {
                    for blob in &restore_point.blobs {
                        session_restore::upsert_blob(txn, blob, now).await?;
                    }
                    session_restore::create_restore_point(
                        txn,
                        session_id,
                        next_seq,
                        Some(restore_point.call_id),
                        Some(restore_point.message_id),
                        Some(restore_point.operation_id.as_str()),
                        restore_point.snapshot,
                        now,
                    )
                    .await?;
                }

                let mut checkpoint_session = session_for_cache.clone();
                checkpoint_session.apply_persisted_metadata(&updated_session);
                checkpoint_session.refresh_derived();
                session_runtime::save_checkpoint(
                    txn,
                    session_id,
                    next_seq,
                    checkpoint_session.clone(),
                    None,
                    now,
                )
                .await?;

                effects.push(async move {
                    if let Ok(mut guard) = cache.write() {
                        guard.insert(
                            checkpoint_session,
                            config.cache_max_sessions,
                            config.cache_max_bytes,
                            config.cache_ttl,
                        );
                    }
                });

                Ok(updated_session)
            })
        })
        .await?;

        session.apply_persisted_metadata(&updated_session);
        session.refresh_derived();
        Ok(session)
    }

    async fn persist_run_failed_event(
        &self,
        session_id: i64,
        reason: String,
    ) -> Result<(), AppError> {
        let event = SessionEvent::RunFailed(RunFailedEvent {
            session_id,
            error: ErrorInfo {
                code: "session_run_failed".to_string(),
                message: reason,
            },
            ts_ms: Utc::now().timestamp_millis(),
        });
        let session = self.load_session(session_id).await?;
        let _ = self
            .persist_session_changes(session, Vec::new(), vec![event], None, None)
            .await?;
        Ok(())
    }

    async fn load_restore_blobs(
        &self,
        journal: &crate::checkpoint::FileJournalCheckpoint,
    ) -> Result<HashMap<String, Vec<u8>>, AppError> {
        let mut blobs = HashMap::new();
        for entry in &journal.entries {
            let crate::checkpoint::JournalFileState::RegularFile { blob_hash, .. } =
                &entry.prior_state
            else {
                continue;
            };
            if blobs.contains_key(blob_hash.as_str()) {
                continue;
            }

            let bytes = session_restore::load_blob(&self.db, blob_hash.as_str())
                .await?
                .ok_or_else(|| {
                    AppError::Internal(format!("missing checkpoint blob: {blob_hash}"))
                })?;
            blobs.insert(blob_hash.clone(), bytes);
        }

        Ok(blobs)
    }

    async fn restore_conversation_state(
        &self,
        restore_point: &crate::checkpoint::SessionRestorePoint,
        mode: SessionRestoreMode,
        restored_paths: Vec<String>,
    ) -> Result<Session, AppError> {
        let session_id = restore_point.session_id;
        let restore_point_id = restore_point.id;
        let restored_session = restore_point.snapshot.conversation.clone().into_session();
        let cache = Arc::clone(&self.cache);
        let config = self.config.clone();
        let restored = with_transaction_and_effects(&self.db, move |txn, effects| {
            let restored_session = restored_session.clone();
            let restored_paths = restored_paths.clone();
            let cache = Arc::clone(&cache);
            let config = config.clone();
            Box::pin(async move {
                message::delete_messages_by_session_id(txn, session_id).await?;
                for message in &restored_session.messages {
                    message::insert_message_with_parts(txn, session_id, message).await?;
                }

                let updated_session = session::touch_session_updated_at(txn, session_id)
                    .await?
                    .ok_or_else(|| DbErr::Custom(format!("session not found: {session_id}")))?;
                let updated_session = session_from_model_db(updated_session)?;

                let mut next_seq = session_runtime::latest_checkpoint(txn, session_id)
                    .await?
                    .map(|checkpoint| checkpoint.upto_seq)
                    .unwrap_or(0);
                next_seq += 1;
                session_runtime::append_session_event(
                    txn,
                    session_id,
                    next_seq,
                    SessionEvent::SessionRestored(SessionRestoredEvent {
                        session_id,
                        restore_point_id,
                        mode,
                        restored_paths: restored_paths.clone(),
                        ts_ms: Utc::now().timestamp_millis(),
                    }),
                    Utc::now(),
                )
                .await?;
                session_runtime::save_checkpoint(
                    txn,
                    session_id,
                    next_seq,
                    {
                        let mut checkpoint_session = restored_session.clone();
                        checkpoint_session.apply_persisted_metadata(&updated_session);
                        checkpoint_session.replace_child_session_ids(
                            session::list_child_session_ids(txn, session_id).await?,
                        );
                        checkpoint_session.set_cache_source(SessionCacheSource::Restored);
                        checkpoint_session
                    },
                    None,
                    Utc::now(),
                )
                .await?;

                let mut session = restored_session.clone();
                session.apply_persisted_metadata(&updated_session);
                session.replace_child_session_ids(
                    session::list_child_session_ids(txn, session_id).await?,
                );
                session.set_cache_source(SessionCacheSource::Restored);
                let session_for_cache = session.clone();
                effects.push(async move {
                    if let Ok(mut guard) = cache.write() {
                        guard.insert(
                            session_for_cache,
                            config.cache_max_sessions,
                            config.cache_max_bytes,
                            config.cache_ttl,
                        );
                    }
                });

                Ok(session)
            })
        })
        .await?;

        Ok(restored)
    }

    async fn load_session(&self, session_id: i64) -> Result<Session, AppError> {
        if let Ok(mut cache) = self.cache.write()
            && let Some(session) = cache.get(session_id, self.config.cache_ttl)
        {
            return Ok(session);
        }

        let session_model = session::get_session_by_id(&self.db, session_id)
            .await?
            .ok_or_else(|| AppError::Internal(format!("session not found: {session_id}")))?;
        let mut session = session_from_model(session_model)?;
        session.replace_child_session_ids(
            session::list_child_session_ids(&self.db, session_id).await?,
        );
        session.replace_messages(message::list_messages_with_parts(&self.db, session_id).await?);
        session.set_cache_source(SessionCacheSource::Database);
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(
                session.clone(),
                self.config.cache_max_sessions,
                self.config.cache_max_bytes,
                self.config.cache_ttl,
            );
        }
        Ok(session)
    }

    fn execute_pending_tool(
        &self,
        session_id: i64,
        pending_tool: &SessionPendingTool,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.tool_executor.execute_invocation_detailed(
            &pending_tool.invocation,
            session_id,
            pending_tool.call_id,
        )
    }

    fn execute_pending_tool_after_approval(
        &self,
        session_id: i64,
        pending_tool: &SessionPendingTool,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.tool_executor
            .execute_invocation_detailed_bypassing_permissions(
                &pending_tool.invocation,
                session_id,
                pending_tool.call_id,
            )
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
struct CachedSessionEntry {
    session: Session,
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
    total_bytes: usize,
}

impl SessionCache {
    fn get(&mut self, session_id: i64, ttl: Duration) -> Option<Session> {
        self.prune(ttl);
        let mut session = {
            let entry = self.entries.get_mut(&session_id)?;
            entry.last_accessed = Instant::now();
            entry.session.clone()
        };
        self.bump(session_id);
        session.set_cache_source(SessionCacheSource::Memory);
        session.refresh_derived();
        Some(session)
    }

    fn insert(
        &mut self,
        mut session: Session,
        max_sessions: usize,
        max_bytes: usize,
        ttl: Duration,
    ) {
        self.prune(ttl);
        session.refresh_derived();
        let session_id = session.id;
        self.remove(session_id);
        let approx_bytes = session.approx_bytes();
        if approx_bytes > max_bytes.max(1) {
            return;
        }

        self.entries.insert(
            session_id,
            CachedSessionEntry {
                session,
                last_accessed: Instant::now(),
            },
        );
        self.total_bytes = self.total_bytes.saturating_add(approx_bytes);
        self.bump(session_id);
        self.enforce_limit(max_sessions, max_bytes);
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
            self.remove(session_id);
        }
    }

    fn enforce_limit(&mut self, max_sessions: usize, max_bytes: usize) {
        while self.entries.len() > max_sessions.max(1) || self.total_bytes > max_bytes.max(1) {
            let Some(session_id) = self.access_order.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&session_id) {
                self.total_bytes = self
                    .total_bytes
                    .saturating_sub(entry.session.approx_bytes());
            }
        }
    }

    fn bump(&mut self, session_id: i64) {
        self.access_order.retain(|item| *item != session_id);
        self.access_order.push_back(session_id);
    }

    fn remove(&mut self, session_id: i64) {
        if let Some(entry) = self.entries.remove(&session_id) {
            self.total_bytes = self
                .total_bytes
                .saturating_sub(entry.session.approx_bytes());
        }
        self.access_order.retain(|item| *item != session_id);
    }

    fn append_child_session(&mut self, parent_session_id: i64, child_session_id: i64) {
        let Some(entry) = self.entries.get_mut(&parent_session_id) else {
            return;
        };

        let before = entry.session.approx_bytes();
        entry.session.append_child_session_id(child_session_id);
        let after = entry.session.approx_bytes();
        self.total_bytes = self
            .total_bytes
            .saturating_sub(before)
            .saturating_add(after);
        self.bump(parent_session_id);
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

fn build_tool_message(
    ids: ReservedMessageIds,
    pending_tool: &SessionPendingTool,
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

fn pending_restore_point_write(
    session: &Session,
    pending_tool: &SessionPendingTool,
    filesystem_checkpoint: Option<FilesystemCheckpointCapture>,
) -> Option<PendingRestorePointWrite> {
    let filesystem_checkpoint = filesystem_checkpoint?;
    Some(PendingRestorePointWrite {
        call_id: pending_tool.call_id,
        message_id: pending_tool.message_id,
        operation_id: pending_tool.operation_id.clone(),
        snapshot: SessionRestorePointSnapshot::new(session.clone(), filesystem_checkpoint.snapshot),
        blobs: filesystem_checkpoint.blobs,
    })
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

fn session_from_model(model: crate::db::entities::session::Model) -> Result<Session, AppError> {
    let created_at = timestamp_millis_to_utc(model.created_at_ms)?;
    let updated_at = timestamp_millis_to_utc(model.updated_at_ms)?;
    let mut session = Session::new(model.id, model.workspace_id, model.title, created_at);
    session.parent_id = model.parent_id;
    session.updated_at = updated_at;
    Ok(session)
}

fn session_from_model_db(model: crate::db::entities::session::Model) -> Result<Session, DbErr> {
    let created_at = timestamp_millis_to_utc_db(model.created_at_ms)?;
    let updated_at = timestamp_millis_to_utc_db(model.updated_at_ms)?;
    let mut session = Session::new(model.id, model.workspace_id, model.title, created_at);
    session.parent_id = model.parent_id;
    session.updated_at = updated_at;
    Ok(session)
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<DateTime<Utc>, AppError> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| AppError::Internal(format!("invalid timestamp millis: {timestamp_ms}")))
}

fn timestamp_millis_to_utc_db(timestamp_ms: i64) -> Result<DateTime<Utc>, DbErr> {
    DateTime::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| DbErr::Custom(format!("invalid timestamp millis: {timestamp_ms}")))
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

fn request_user_input_title(request: &UserInputRequest) -> String {
    match request.questions.len() {
        0 => "Awaiting user input".to_string(),
        1 => format!("Awaiting user input: {}", request.questions[0].header),
        count => format!("Awaiting user input for {count} questions"),
    }
}

fn user_input_execution(
    request: &UserInputRequest,
    reply: &UserInputReply,
) -> Result<ToolInvocationExecution, AppError> {
    let answers = validate_user_input_reply(request, reply)?;
    let mut lines = vec!["Collected user input:".to_string()];
    for question in &request.questions {
        if let Some(answer) = answers.get(question.id.as_str()) {
            lines.push(format!("- {}: {}", question.id, answer));
        }
    }

    let mut view = crate::tool::ToolExecutionView::simple("User input", lines.join("\n"));
    view.metadata
        .insert("answer_count".to_string(), answers.len().to_string());
    view.metadata.insert(
        "question_count".to_string(),
        request.questions.len().to_string(),
    );

    Ok(ToolInvocationExecution::new(
        ToolOutput::Builtin {
            output: BuiltinToolOutput::RequestUserInput { answers },
        },
        view,
    ))
}

fn validate_user_input_reply(
    request: &UserInputRequest,
    reply: &UserInputReply,
) -> Result<std::collections::BTreeMap<String, String>, AppError> {
    let mut answers = std::collections::BTreeMap::new();

    for question in &request.questions {
        let answer = reply
            .answers
            .get(question.id.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "missing answer for user input question {}",
                    question.id
                ))
            })?;
        answers.insert(question.id.clone(), answer.to_string());
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
    use std::process::Command;
    use std::sync::Arc;

    use async_trait::async_trait;
    use futures_core::Stream;
    use futures_util::stream;
    use sea_orm::Database;
    use uuid::Uuid;

    use crate::agent::Agent;
    use crate::checkpoint::{FilesystemCheckpoint, SessionRestoreMode, SessionRestoreRequest};
    use crate::db::crud::session_restore;
    use crate::db::init_schema;
    use crate::message::{
        ApplyPatchToolInput, AttachmentSource, BuiltinToolInput, BuiltinToolOutput, FileChangeKind,
        McpToolOutput, RequestUserInputToolInput, ToolAttachment, ToolExecutionPart, ToolOutput,
        ToolResultBlock, ToolSearchToolInput, UserInputOption, UserInputQuestion, UserInputReply,
        UserInputReplyKind,
    };
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
            Ok(vec![
                ProviderModel::new("scripted", "scripted-model").with_display_name("Scripted"),
            ])
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
                    if part.operation_id.as_deref() != Some("call_request_user_input_1") {
                        return None;
                    }
                    match part.content.as_ref() {
                        Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                            details:
                                ToolOutput::Builtin {
                                    output: BuiltinToolOutput::RequestUserInput { answers },
                                },
                            ..
                        })) => answers.get("model_choice").cloned().map(Ok),
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
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
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
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if last_user_text.contains("choose model") && user_input_result.is_none() {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
                        stream_key: "call_request_user_input_1".to_string(),
                        id: Some("call_request_user_input_1".to_string()),
                        name: Some("request_user_input".to_string()),
                        arguments_delta: serde_json::to_string(&RequestUserInputToolInput {
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
                            }],
                        })
                        .expect("serialize request_user_input input"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
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
            } else if last_user_text.contains("patch") && tool_result.is_none() {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
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
                        provider_id: "scripted".to_string(),
                        model: "scripted-model".to_string(),
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

    fn ensure_git_repo(path: &std::path::Path) -> bool {
        if Command::new("git").arg("--version").output().is_err() {
            return false;
        }

        let status = Command::new("git")
            .arg("init")
            .arg(path)
            .status()
            .expect("git init should run");
        assert!(status.success(), "git init should succeed");

        fs::write(path.join("seed.txt"), "seed\n").expect("seed file should be written");
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["add", "seed.txt"])
            .status()
            .expect("git add should run");
        assert!(status.success(), "git add should succeed");

        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args([
                "-c",
                "user.name=agena",
                "-c",
                "user.email=agena@example.invalid",
                "commit",
                "-m",
                "seed",
            ])
            .status()
            .expect("git commit should run");
        assert!(status.success(), "git commit should succeed");
        true
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
        let service = build_service(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionServiceConfig::default(),
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
                            && request.request.request_id == "call_request_user_input_1"
                )
            })
        }));

        let resumed = service
            .reply_user_input(SessionUserInputReplyRequest {
                session_id: created.id,
                options: run_options(),
                reply: UserInputReply {
                    request_id: "call_request_user_input_1".to_string(),
                    kind: UserInputReplyKind::Submit,
                    answers: BTreeMap::from([("model_choice".to_string(), "gpt-5".to_string())]),
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
                                        output: BuiltinToolOutput::RequestUserInput { answers },
                                    },
                                ..
                            })) if answers.get("model_choice").map(String::as_str) == Some("gpt-5")
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

    #[tokio::test]
    async fn restore_session_rewinds_non_git_apply_patch_filesystem_and_conversation() {
        let workspace = TempWorkspace::new();
        let service = build_service(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionServiceConfig::default(),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "restore-non-git".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let completed = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: run_options(),
                parts: vec![PartContent::text("please patch a file")],
            })
            .await
            .expect("submit turn");

        assert!(!completed.blocked());
        assert_eq!(
            fs::read_to_string(workspace.root.join("result.txt"))
                .expect("patched file should exist"),
            "approved\n"
        );

        let restored = service
            .restore_session(SessionRestoreRequest {
                session_id: created.id,
                restore_point_id: None,
                mode: SessionRestoreMode::Both,
            })
            .await
            .expect("restore session");

        assert!(!workspace.root.join("result.txt").exists());
        assert!(
            restored.messages.iter().all(|message| {
                message.role != Role::Tool
                    || !message.as_text_lossy().contains("Applied 1 file changes")
            }),
            "tool result message should be removed after conversation restore"
        );
        assert!(restored.messages.iter().any(|message| {
            message.parts.iter().any(|part| {
                matches!(
                    part.content.as_ref(),
                    Some(PartContent::ToolExecution(ToolExecutionPart::Pending { invocation, .. }))
                        if matches!(
                            invocation,
                            ToolInvocation::Builtin {
                                input: BuiltinToolInput::ApplyPatch(_)
                            }
                        )
                )
            })
        }));
    }

    #[tokio::test]
    async fn restore_session_captures_git_snapshot_when_workspace_is_repo() {
        let workspace = TempWorkspace::new();
        if !ensure_git_repo(&workspace.root) {
            return;
        }
        let service = build_service(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionServiceConfig::default(),
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "restore-git".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let completed = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: run_options(),
                parts: vec![PartContent::text("please patch a file")],
            })
            .await
            .expect("submit turn");

        assert!(!completed.blocked());
        let restore_point = session_restore::latest_restore_point(&service.db, created.id)
            .await
            .expect("load restore point")
            .expect("restore point should exist");
        assert!(matches!(
            restore_point.snapshot.filesystem,
            FilesystemCheckpoint::Composite { .. }
        ));

        let restored = service
            .restore_session(SessionRestoreRequest {
                session_id: created.id,
                restore_point_id: Some(restore_point.id),
                mode: SessionRestoreMode::Filesystem,
            })
            .await
            .expect("restore filesystem");

        assert!(!workspace.root.join("result.txt").exists());
        assert!(
            restored
                .messages
                .iter()
                .any(|message| message.as_text_lossy() == "patch done"),
            "filesystem-only restore should preserve current conversation state"
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

    #[test]
    fn cache_skips_entries_larger_than_byte_budget() {
        let state = cache_state(1, "x".repeat(256));
        let mut cache = SessionCache::default();
        let max_bytes = state.approx_bytes().saturating_sub(1).max(1);

        cache.insert(state.clone(), 8, max_bytes, Duration::from_secs(60));

        assert!(cache.get(state.id, Duration::from_secs(60)).is_none());
        assert_eq!(cache.total_bytes, 0);
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

        cache.insert(first.clone(), 8, max_bytes, Duration::from_secs(60));
        cache.insert(second.clone(), 8, max_bytes, Duration::from_secs(60));

        assert!(cache.get(first.id, Duration::from_secs(60)).is_none());
        assert!(cache.get(second.id, Duration::from_secs(60)).is_some());
        assert!(cache.total_bytes <= max_bytes);
    }
}
