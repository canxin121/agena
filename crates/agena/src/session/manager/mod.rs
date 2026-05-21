use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use arc_swap::ArcSwap;
use chrono::Utc;
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};
use tracing::Instrument;

use crate::AppError;
use crate::db::crud::session_goal::GoalUpdate;
use crate::event::{
    ErrorInfo, EventKind, PermissionRepliedEvent, PermissionRequestedEvent, RunFailedEvent,
    RunStartedEvent, SessionGoalEvent,
};
use crate::message::{
    ExecutionStatus, Message, MessageMetadata, MessagePart, MessageSource, MessageStatus,
    OperationBlock, OperationPart, PartContent, PermissionRequestPart, TaskSubagentType, TimeRange,
    ToolAttachment, ToolInvocation, ToolOutput, UserInputReply, UserInputReplyKind,
    UserInputRequest, UserInputRequestPart,
};
use crate::model::ModelRef;
use crate::model::ModelSpeedModeRequestOverride;
use crate::permission::{
    DecisionTraceStep, PermissionAction, PermissionDecision, PermissionMode, PermissionReply,
    PermissionReplyKind, PermissionRequest, PermissionRiskLevel, PermissionScope,
    PersistedPermissionRule, PolicySourceKind, resolve_permission_with_persisted_rule,
};
use crate::provider::ThinkingRequest;
use crate::role::Role;
use crate::tool::{
    PreparedShellCommand, StreamingToolExecution, ToolError, ToolExecutor, ToolInvocationExecution,
    ToolPermissionCheck,
};
use std::path::PathBuf;

use super::cache::SessionCachePolicy;
pub use super::cache::SessionCacheStats;
use super::control::{TurnControl, TurnControlError, TurnRegistry};
use super::cost::{SessionCostSummary, UsageStats, UsageStatsQuery};
use super::history::{
    FinishReason, MessageId as HistoryMessageId, ToolCallCompleted,
    ToolCallId as HistoryToolCallId, TranscriptContent, TranscriptToolOutput, TurnAbortReason,
    TurnAborted, TurnCompleted, TurnId as HistoryTurnId, TurnStarted, UserMessageAppended,
};
use super::model::{
    GoalStatus, GoalSteeringKind, PromptCompactionRuntime, PromptCompactionStrategy,
    ProviderPromptAnchor, SessionExecutionContext, SessionGoal, SessionListRequest,
    SessionPendingTool, SessionStatus, SessionSummary, validate_session_goal_objective,
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
    pub doom_loop: crate::session::DoomLoopPolicy,
    pub default_agent: Option<String>,
    pub permission: crate::agent::PermissionConfig,
    pub auto_compaction: SessionAutoCompactionConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionAutoCompactionConfig {
    pub enabled: bool,
    pub reserved_tokens: Option<u32>,
}

impl Default for SessionAutoCompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            reserved_tokens: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionUsageLimitBasis {
    ContextWindow,
    PromptThreshold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionUsage {
    pub measured_prompt_tokens: Option<u64>,
    pub current_tokens: u64,
    pub projected_tokens: Option<u64>,
    pub limit_tokens: Option<u64>,
    pub limit_basis: Option<SessionUsageLimitBasis>,
    pub reserved_tokens: Option<u32>,
    pub model_context_window_tokens: Option<u32>,
    pub model_max_input_tokens: Option<u32>,
    pub model_max_output_tokens: Option<u32>,
}

impl Default for SessionManagerConfig {
    fn default() -> Self {
        Self {
            cache_max_sessions: 128,
            cache_ttl: Duration::from_secs(15 * 60),
            cache_max_bytes: 64 * 1024 * 1024,
            max_turn_loops: 16,
            doom_loop: crate::session::DoomLoopPolicy::default(),
            default_agent: None,
            permission: crate::agent::PermissionConfig::default(),
            auto_compaction: SessionAutoCompactionConfig::default(),
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
    pub thinking_mode: Option<String>,
    pub speed_mode: Option<String>,
    pub verbosity: Option<String>,
    pub thinking: Option<ThinkingRequest>,
    pub request_override: ModelSpeedModeRequestOverride,
    pub system: Option<String>,
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<u32>,
    pub agent_profile: Option<String>,
    pub max_turn_loops: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAgentSwitchOutcome {
    pub session_id: i64,
    pub previous_agent: Option<String>,
    pub current_agent: Option<String>,
    pub stack_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAgentRestoreOutcome {
    pub session_id: i64,
    pub restored: bool,
    pub previous_agent: Option<String>,
    pub current_agent: Option<String>,
    pub stack_depth: usize,
}

impl SessionRunOptions {
    fn completion_request(
        &self,
        system: Option<String>,
        messages: Vec<Message>,
        tools: Vec<crate::plugin::registry::PluginEntry>,
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
            thinking: self.thinking.clone(),
            verbosity: self.verbosity.clone(),
            response_format: None,
            request_override: self.request_override.clone(),
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
pub struct SessionCompactRequest {
    pub session_id: i64,
    pub options: SessionRunOptions,
}

#[derive(Debug, Clone)]
pub struct SessionForkRequest {
    pub session_id: i64,
    /// Fork point. `None` clones the entire history; `Some(id)` clones every
    /// event up to and including the last event tied to that message.
    pub at_message_id: Option<i64>,
    pub title: Option<String>,
    /// Optimistic-lock check. When `Some(v)` and the source session's
    /// `version` no longer matches, the call returns `AppError::Conflict`.
    #[doc(hidden)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SessionRewindRequest {
    pub session_id: i64,
    pub message_id: i64,
    #[doc(hidden)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SessionPermissionReplyRequest {
    pub session_id: i64,
    pub options: SessionRunOptions,
    pub reply: PermissionReply,
    pub operator: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionUserInputReplyRequest {
    pub session_id: i64,
    pub options: SessionRunOptions,
    pub reply: UserInputReply,
}

#[derive(Debug, Clone)]
pub struct SessionSubtaskRequest {
    pub parent_session_id: i64,
    pub description: String,
    pub prompt: String,
    pub subagent_type: TaskSubagentType,
    pub profile_name: Option<String>,
    pub task_id: Option<String>,
    pub command: Option<String>,
    pub requested_model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionSubtaskResponse {
    pub session: Session,
    pub profile_name: Option<String>,
    pub model_provider_id: Option<String>,
    pub model_adapter_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionGoalCreateRequest {
    pub session_id: i64,
    pub objective: String,
}

#[derive(Debug, Clone, Default)]
pub struct SessionGoalUpdateRequest {
    pub session_id: i64,
    pub objective: Option<String>,
    pub status: Option<GoalStatus>,
    pub expected_goal_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GoalTurnDirectiveKind {
    ObjectiveUpdated,
    Continuation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GoalTurnDirective {
    goal_id: i64,
    kind: GoalTurnDirectiveKind,
    prompt: String,
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
    prepared_shell_command: Option<PreparedShellCommand>,
    lifecycle: TimeRange,
    session_runtime: crate::session::SessionRuntimeState,
}

struct PendingHostUserInput {
    response: oneshot::Sender<crate::plugin::sdk::host_api::AskUserResponse>,
}

#[derive(Clone)]
struct SessionManagerState {
    processor: SessionProcessor,
    tool_executor: ToolExecutor,
    config: SessionManagerConfig,
}

mod compact;
mod goals;
mod history;
mod replies;
mod sessions;
mod turns;

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
    host_user_input_waiters: Arc<Mutex<HashMap<String, PendingHostUserInput>>>,
}

impl SessionManager {
    fn background_handle(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            publisher: Arc::clone(&self.publisher),
            bus: Arc::clone(&self.bus),
            execution: ArcSwap::from(self.execution.load_full()),
            turn_registry: Arc::clone(&self.turn_registry),
            host_user_input_waiters: Arc::clone(&self.host_user_input_waiters),
        }
    }

    pub fn new(
        db: sea_orm::DatabaseConnection,
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
    ) -> Self {
        let db_arc = Arc::new(db.clone());
        // The publisher (not the store) consults `EventKind::is_persistent`
        // to decide which events land in SQLite, so the store stays a single
        // generic type.
        let store_inner: Arc<dyn crate::event::EventStore<crate::event::EventKind>> = Arc::new(
            crate::db::SeaEventStore::<crate::event::EventKind>::new(Arc::clone(&db_arc)),
        );
        let bus: Arc<dyn crate::event::EventBus<crate::event::EventKind>> =
            Arc::new(crate::event::InProcessEventBus::<crate::event::EventKind>::new(4096));
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
            host_user_input_waiters: Arc::new(Mutex::new(HashMap::new())),
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

    pub fn tool_executor(&self) -> ToolExecutor {
        self.execution_state().tool_executor.clone()
    }

    pub async fn request_host_user_input(
        &self,
        session_id: i64,
        call_id: i64,
        request: crate::message::AskUserToolInput,
    ) -> Result<crate::plugin::sdk::host_api::AskUserResponse, AppError> {
        let state = self.execution_state();
        let session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        let pending_tool = session
            .pending_tools()
            .into_iter()
            .find(|tool| {
                session
                    .pending_tool_execution(tool)
                    .is_some_and(|(pending_call_id, _, _)| pending_call_id == call_id)
            })
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool not found for host user input: session={session_id}, call={call_id}"
                ))
            })?;
        let request_id = format!("host-input:{call_id}:{}", uuid::Uuid::new_v4().simple());
        let (response_tx, response_rx) = oneshot::channel();
        self.host_user_input_waiters.lock().await.insert(
            request_id.clone(),
            PendingHostUserInput {
                response: response_tx,
            },
        );
        if let Err(err) = self
            .apply_user_input_request_with_id(
                session,
                &pending_tool,
                request,
                request_id.clone(),
                state.clone(),
            )
            .await
        {
            self.host_user_input_waiters
                .lock()
                .await
                .remove(&request_id);
            return Err(err);
        }
        response_rx.await.map_err(|_| {
            AppError::Internal(format!(
                "host user input waiter closed before reply: {request_id}"
            ))
        })
    }

    pub async fn execute_host_invoked_tool(
        &self,
        session_id: i64,
        call_id: i64,
        invocation: ToolInvocation,
    ) -> Result<ToolInvocationExecution, AppError> {
        let session = self.get_session(session_id).await?;
        let state = self.execution_state();
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution);
        let prepared = scoped_executor
            .prepare_invocation(&invocation, session.id, call_id)
            .map_err(tool_error_to_app_error)?;
        let (invocation, prepared_shell_command) = scoped_executor
            .prepare_bash_invocation(&prepared.invocation, session.id, call_id)
            .map_err(tool_error_to_app_error)?;
        scoped_executor
            .enforce_plan_mode_for(&invocation, session.id)
            .map_err(tool_error_to_app_error)?;
        self.require_immediate_tool_permissions(session.id, &scoped_executor, &invocation)
            .await?;
        tokio::task::spawn_blocking(move || {
            scoped_executor.execute_invocation_detailed_with_prepared_shell(
                &invocation,
                session_id,
                call_id,
                prepared_shell_command,
            )
        })
        .await
        .map_err(|err| AppError::Internal(format!("host-invoked tool task failed: {err}")))?
        .map_err(tool_error_to_app_error)
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
}

fn permission_subject(action: &PermissionAction) -> serde_json::Value {
    match action {
        PermissionAction::Tool { tool_name, .. } => {
            serde_json::json!({
                "kind": "tool",
                "tool_name": tool_name,
            })
        }
        PermissionAction::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        } => serde_json::json!({
            "kind": "path_access",
            "access_kind": access_kind,
            "workspace_root": workspace_root,
            "target_path": target_path,
        }),
        PermissionAction::NetworkAccess { target, host, port } => serde_json::json!({
            "kind": "network_access",
            "target": target,
            "host": host,
            "port": port,
        }),
    }
}

fn infer_session_model(session: &Session) -> Result<Option<ModelRef>, AppError> {
    let mut sorted: Vec<&Message> = session.messages.iter().collect();
    sorted.sort_by(|a, b| {
        (b.created_at.timestamp_millis(), b.id).cmp(&(a.created_at.timestamp_millis(), a.id))
    });
    for message in sorted {
        let provider_id = message.metadata.model_provider_id.trim();
        let adapter_id = message.metadata.model_adapter_id.as_deref().map(str::trim);
        let model_id = message.metadata.model_id.trim();
        if provider_id.is_empty() || model_id.is_empty() {
            continue;
        }
        let model = match adapter_id.filter(|value| !value.is_empty()) {
            Some(adapter_id) => ModelRef::try_new_with_adapter(provider_id, adapter_id, model_id),
            None => ModelRef::try_new(provider_id, model_id),
        };
        return model.map(Some).map_err(|error| {
            AppError::Internal(format!(
                "session {} contains invalid persisted model metadata: {error}",
                session.id
            ))
        });
    }
    Ok(None)
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

fn is_user_cancelled_error(err: &AppError) -> bool {
    matches!(err, AppError::Internal(message) if message == "turn cancelled by user")
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
    HistoryToolCallId::new(resolved.operation_id.clone())
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
        prepared_shell_command: None,
        lifecycle: lifecycle.clone(),
        session_runtime: session.runtime.clone(),
    })
}

fn operation_blocks_from_tool_output(
    invocation: &ToolInvocation,
    details: &ToolOutput,
    attachments: &[ToolAttachment],
    output_text: &str,
) -> Vec<OperationBlock> {
    let mut blocks = text_result_blocks(output_text);

    let payload_tool_name = payload_tool_name_for_invocation(invocation);
    match crate::tool::ToolPayloadOutput::from_tool_output(payload_tool_name.as_str(), details) {
        Some(crate::tool::ToolPayloadOutput::ApplyPatch { changes, .. }) if !changes.is_empty() => {
            blocks.push(OperationBlock::FileChanges { changes });
        }
        Some(crate::tool::ToolPayloadOutput::TodoWrite { items }) if !items.is_empty() => {
            blocks.push(OperationBlock::Checklist { items });
        }
        _ => {}
    }

    for block in details.content_blocks() {
        blocks.push(block);
    }

    for attachment in attachments {
        blocks.push(OperationBlock::Media {
            mime_type: attachment.mime.clone(),
            artifact: crate::message::ArtifactRef {
                uri: attachment_source_uri(&attachment.source),
                mime: attachment.mime.clone(),
                name: attachment
                    .filename
                    .clone()
                    .or_else(|| attachment.title.clone()),
                size_bytes: attachment.size_bytes,
                sha256: attachment.sha256.clone(),
            },
        });
    }

    dedupe_operation_blocks(blocks)
}

fn payload_tool_name_for_invocation(invocation: &ToolInvocation) -> String {
    crate::tool::ToolPayloadInput::from_invocation(invocation)
        .map(|payload| payload.tool_name().to_string())
        .unwrap_or_else(|| invocation.name.clone())
}

fn loaded_tools_from_tool_output(details: &ToolOutput) -> Option<Vec<String>> {
    let loaded_tools =
        serde_json::from_value(custom_payload_value(details)?.get("loaded_tools")?.clone()).ok()?;
    Some(loaded_tools)
}

#[cfg(test)]
fn answers_from_tool_output(
    details: &ToolOutput,
) -> Option<std::collections::BTreeMap<String, Vec<String>>> {
    serde_json::from_value(custom_payload_value(details)?.get("answers")?.clone()).ok()
}

fn custom_payload_value(details: &ToolOutput) -> Option<serde_json::Value> {
    details.to_json_payload()
}

fn attachment_source_uri(source: &crate::message::AttachmentSource) -> String {
    match source {
        crate::message::AttachmentSource::Url { url }
        | crate::message::AttachmentSource::DataUrl { url } => url.clone(),
        crate::message::AttachmentSource::LocalPath { path } => path.clone(),
        crate::message::AttachmentSource::Base64 { .. } => {
            "data:application/octet-stream;base64".to_string()
        }
        crate::message::AttachmentSource::FileId { file_id } => format!("file:{file_id}"),
    }
}

fn dedupe_operation_blocks(blocks: Vec<OperationBlock>) -> Vec<OperationBlock> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::with_capacity(blocks.len());
    for block in blocks {
        let key = serde_json::to_string(&block).unwrap_or_else(|_| format!("{:?}", block));
        if seen.insert(key) {
            deduped.push(block);
        }
    }
    deduped
}

fn part_status(content: &PartContent) -> ExecutionStatus {
    match content {
        PartContent::Operation(tool) => tool.status(),
        PartContent::Request(crate::message::RequestPart::Permission(permission)) => {
            permission.status()
        }
        PartContent::Request(crate::message::RequestPart::UserInput(request)) => request.status(),
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
        PartContent::permission_request(permission),
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
        PartContent::user_input_request(request),
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
    let ToolInvocation { name, .. } = invocation;
    name.clone()
}

fn text_result_blocks(output_text: &str) -> Vec<OperationBlock> {
    if output_text.trim().is_empty() {
        Vec::new()
    } else {
        vec![OperationBlock::Text {
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

async fn persisted_rule_for_reply(
    store: &SessionStore,
    session_id: i64,
    action: &PermissionAction,
    reply: &PermissionReply,
    operator: Option<&str>,
) -> Result<Option<PersistedPermissionRule>, AppError> {
    let Some(mode) = persisted_mode_for_reply(reply.kind) else {
        return Ok(None);
    };
    let scope = reply.scope.unwrap_or(PermissionScope::Session);
    let action_key = permission_action_key(action)?;
    let workspace_id = match scope {
        PermissionScope::Session | PermissionScope::Global => None,
        PermissionScope::Workspace => Some(store.current_workspace_id().await?),
    };
    let session_rule_id = match scope {
        PermissionScope::Session => Some(session_id),
        PermissionScope::Workspace | PermissionScope::Global => None,
    };
    Ok(Some(PersistedPermissionRule {
        action_key,
        mode,
        scope,
        session_id: session_rule_id,
        workspace_id,
        source: "permission_reply".to_string(),
        reason: reply.reason.clone(),
        operator: operator.map(str::to_string),
        revoked_at_ms: None,
        revoked_reason: None,
        revoked_by: None,
    }))
}

fn permission_scope_label(scope: PermissionScope) -> String {
    match scope {
        PermissionScope::Session => "session".to_string(),
        PermissionScope::Workspace => "workspace".to_string(),
        PermissionScope::Global => "global".to_string(),
    }
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

fn apply_advisory_permission_decision(
    base: PermissionDecision,
    advice: crate::plugin::PermissionDecision,
    explanation: &str,
) -> PermissionDecision {
    match (base, advice) {
        (PermissionDecision::Deny { reason }, _) => PermissionDecision::Deny { reason },
        (_, crate::plugin::PermissionDecision::Deny) => PermissionDecision::Deny {
            reason: if explanation.trim().is_empty() {
                "denied by plugin advice".to_string()
            } else {
                explanation.to_string()
            },
        },
        (PermissionDecision::Ask { reason }, _) => PermissionDecision::Ask { reason },
        (PermissionDecision::Allow, crate::plugin::PermissionDecision::Prompt) => {
            PermissionDecision::Ask {
                reason: if explanation.trim().is_empty() {
                    "permission requires confirmation".to_string()
                } else {
                    explanation.to_string()
                },
            }
        }
        (PermissionDecision::Allow, crate::plugin::PermissionDecision::Allow) => {
            PermissionDecision::Allow
        }
    }
}

fn risk_for_permission_decision(decision: &PermissionDecision) -> PermissionRiskLevel {
    match decision {
        PermissionDecision::Allow => PermissionRiskLevel::Low,
        PermissionDecision::Ask { .. } => PermissionRiskLevel::Medium,
        PermissionDecision::Deny { .. } => PermissionRiskLevel::High,
    }
}

fn plugin_risk_to_core(risk: crate::plugin::sdk::PermissionRiskLevel) -> PermissionRiskLevel {
    match risk {
        crate::plugin::sdk::PermissionRiskLevel::Low => PermissionRiskLevel::Low,
        crate::plugin::sdk::PermissionRiskLevel::Medium => PermissionRiskLevel::Medium,
        crate::plugin::sdk::PermissionRiskLevel::High => PermissionRiskLevel::High,
        crate::plugin::sdk::PermissionRiskLevel::Critical => PermissionRiskLevel::Critical,
    }
}

fn max_permission_risk(
    left: PermissionRiskLevel,
    right: PermissionRiskLevel,
) -> PermissionRiskLevel {
    if permission_risk_rank(left) >= permission_risk_rank(right) {
        left
    } else {
        right
    }
}

fn permission_risk_rank(risk: PermissionRiskLevel) -> u8 {
    match risk {
        PermissionRiskLevel::Low => 0,
        PermissionRiskLevel::Medium => 1,
        PermissionRiskLevel::High => 2,
        PermissionRiskLevel::Critical => 3,
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
        crate::tool::ToolPayloadOutput::AskUser { answers }.into_tool_output(),
        view,
    ))
}

fn host_user_input_response(
    request: &UserInputRequest,
    reply: &UserInputReply,
) -> Result<crate::plugin::sdk::host_api::AskUserResponse, AppError> {
    match reply.kind {
        UserInputReplyKind::Cancel => Ok(crate::plugin::sdk::host_api::AskUserResponse {
            reply: reply.reason.clone().unwrap_or_default(),
            cancelled: true,
            answers: Default::default(),
        }),
        UserInputReplyKind::Submit => {
            let answers = validate_user_input_reply(request, reply)?;
            let question = request.questions.first().ok_or_else(|| {
                AppError::Internal("host user input request is missing its question".to_string())
            })?;
            let answer = answers
                .get(question.id.as_str())
                .and_then(|values| values.first())
                .cloned()
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "host user input reply missing answer for question {}",
                        question.id
                    ))
                })?;
            Ok(crate::plugin::sdk::host_api::AskUserResponse {
                reply: answer,
                cancelled: false,
                answers,
            })
        }
    }
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
        if !question.allow_custom
            && let Some(answer) = normalized
                .iter()
                .find(|value| !allowed.contains(value.as_str()))
        {
            return Err(AppError::Internal(format!(
                "unsupported answer '{}' for question {}",
                answer, question.id
            )));
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
mod tests;
