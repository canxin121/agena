use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use arc_swap::ArcSwap;
use chrono::Utc;
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};
use tracing::Instrument;

use crate::AppError;
use crate::config::ProviderNativeToolsConfig;
use crate::db::crud::session_goal::GoalUpdate;
use crate::event::{
    ErrorInfo, EventKind, ExecutionFailedEvent, ExecutionStartedEvent, PermissionRepliedEvent,
    PermissionRequestedEvent, SessionGoalEvent,
};
use crate::message::{
    AttachmentItem, ExecutionStatus, InteractiveRequestPart, Message, MessageMetadata, MessagePart,
    MessageSource, MessageStatus, OperationBlock, OperationPart, PartContent, RequestPart,
    TaskSubagentType, TimeRange, ToolInvocation, ToolOutput, UserInputReply, UserInputReplyKind,
    UserInputRequest,
};
use crate::model::ModelRef;
use crate::model::ModelSpeedModeRequestOverride;
use crate::permission::{
    DecisionTraceStep, PermissionAction, PermissionDecision, PermissionMode, PermissionReply,
    PermissionReplyKind, PermissionRequest, PermissionRiskLevel, PermissionScope,
    PersistedPermissionRule, PolicySourceKind, resolve_permission_with_persisted_rules,
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
use super::control::{RunControl, RunControlError, RunRegistry};
use super::cost::{SessionCostSummary, UsageStats, UsageStatsQuery};
use super::history::{
    FinishReason, MessageId as HistoryMessageId, RunAbortReason, RunAborted, RunCompleted,
    RunId as HistoryRunId, RunSource, RunStarted, ToolCallCompleted,
    ToolCallId as HistoryToolCallId, TranscriptContent, TranscriptToolOutput, UserMessageAppended,
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
    pub doom_loop: crate::session::DoomLoopPolicy,
    pub default_selection: crate::execution_prefs::ExecutionSelection,
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
            doom_loop: crate::session::DoomLoopPolicy::default(),
            default_selection: crate::execution_prefs::ExecutionSelection::default(),
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
    pub fn new(model: ModelRef) -> Self {
        Self {
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
        }
    }

    pub fn with_thinking_mode(mut self, thinking_mode: Option<String>) -> Self {
        self.thinking_mode = thinking_mode;
        self
    }

    pub fn with_speed_mode(mut self, speed_mode: Option<String>) -> Self {
        self.speed_mode = speed_mode;
        self
    }

    pub fn with_verbosity(mut self, verbosity: Option<String>) -> Self {
        self.verbosity = verbosity;
        self
    }

    pub fn with_thinking(mut self, thinking: Option<ThinkingRequest>) -> Self {
        self.thinking = thinking;
        self
    }

    pub fn with_request_override(
        mut self,
        request_override: ModelSpeedModeRequestOverride,
    ) -> Self {
        self.request_override = request_override;
        self
    }

    pub fn with_system(mut self, system: Option<String>) -> Self {
        self.system = system;
        self
    }

    pub fn with_temperature(mut self, temperature: Option<f32>) -> Self {
        self.temperature = temperature;
        self
    }

    pub fn with_max_output_tokens(mut self, max_output_tokens: Option<u32>) -> Self {
        self.max_output_tokens = max_output_tokens;
        self
    }

    pub fn with_agent_profile(mut self, agent_profile: Option<String>) -> Self {
        self.agent_profile = agent_profile;
        self
    }

    fn completion_request(
        &self,
        system: Option<String>,
        messages: Vec<Message>,
        tools: Vec<crate::plugin::registry::RegisteredTool>,
        native_tools: ProviderNativeToolsConfig,
        prompt_cache_key: Option<String>,
        previous_response_id: Option<String>,
        prompt_window_generation: Option<u64>,
    ) -> crate::provider::CompletionRequest {
        crate::provider::CompletionRequest {
            model: self.model.model_id.clone(),
            system,
            messages,
            tools,
            native_tools,
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
pub struct SessionExecutionRequest {
    pub session_id: i64,
    pub options: SessionRunOptions,
}

impl SessionExecutionRequest {
    pub fn new(session_id: i64, options: SessionRunOptions) -> Self {
        Self {
            session_id,
            options,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionUserMessageRequest {
    pub run: SessionExecutionRequest,
    pub parts: Vec<PartContent>,
}

impl SessionUserMessageRequest {
    pub fn new(session_id: i64, options: SessionRunOptions, parts: Vec<PartContent>) -> Self {
        Self {
            run: SessionExecutionRequest::new(session_id, options),
            parts,
        }
    }
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
pub struct SessionExecutionReplyRequest<T> {
    pub session_id: i64,
    pub options: SessionRunOptions,
    pub reply: T,
}

impl<T> SessionExecutionReplyRequest<T> {
    pub fn new(session_id: i64, options: SessionRunOptions, reply: T) -> Self {
        Self {
            session_id,
            options,
            reply,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionPermissionReplyRequest {
    pub request: SessionExecutionReplyRequest<PermissionReply>,
    pub operator: Option<String>,
}

impl SessionPermissionReplyRequest {
    pub fn new(
        session_id: i64,
        options: SessionRunOptions,
        reply: PermissionReply,
        operator: Option<String>,
    ) -> Self {
        Self {
            request: SessionExecutionReplyRequest::new(session_id, options, reply),
            operator,
        }
    }
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
enum GoalRunDirectiveKind {
    ObjectiveUpdated,
    Continuation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GoalRunDirective {
    goal_id: i64,
    kind: GoalRunDirectiveKind,
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

type HostUserInputSequenceKey = (i64, i64);

struct HostUserInputSequenceGuard {
    sequences: Arc<StdMutex<HashMap<HostUserInputSequenceKey, usize>>>,
    key: HostUserInputSequenceKey,
}

impl HostUserInputSequenceGuard {
    fn new(
        sequences: Arc<StdMutex<HashMap<HostUserInputSequenceKey, usize>>>,
        session_id: i64,
        call_id: i64,
    ) -> Self {
        let key = (session_id, call_id);
        if let Ok(mut guard) = sequences.lock() {
            guard.remove(&key);
        }
        Self { sequences, key }
    }
}

impl Drop for HostUserInputSequenceGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.sequences.lock() {
            guard.remove(&self.key);
        }
    }
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
mod runs;
mod sessions;

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
    run_registry: Arc<RunRegistry>,
    reply_session_locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
    host_user_input_waiters: Arc<Mutex<HashMap<String, PendingHostUserInput>>>,
    host_user_input_sequences: Arc<StdMutex<HashMap<HostUserInputSequenceKey, usize>>>,
}

impl SessionManager {
    fn background_handle(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            publisher: Arc::clone(&self.publisher),
            bus: Arc::clone(&self.bus),
            execution: ArcSwap::from(self.execution.load_full()),
            run_registry: Arc::clone(&self.run_registry),
            reply_session_locks: Arc::clone(&self.reply_session_locks),
            host_user_input_waiters: Arc::clone(&self.host_user_input_waiters),
            host_user_input_sequences: Arc::clone(&self.host_user_input_sequences),
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
            run_registry: Arc::new(RunRegistry::new()),
            reply_session_locks: Arc::new(Mutex::new(HashMap::new())),
            host_user_input_waiters: Arc::new(Mutex::new(HashMap::new())),
            host_user_input_sequences: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    /// Returns the unified event publisher that core sites use to emit
    /// `EventKind`. Public so the API server crate can wire it into
    /// transports (REST/WS/SSE/IPC).
    pub fn event_publisher(&self) -> Arc<crate::event::EventPublisher> {
        Arc::clone(&self.publisher)
    }

    async fn reply_session_lock(&self, session_id: i64) -> Arc<Mutex<()>> {
        let mut guard = self.reply_session_locks.lock().await;
        Arc::clone(
            guard
                .entry(session_id)
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
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
        let resolved_pending = resolve_pending_tool(&session, &pending_tool)?;
        let sequence_index = self.next_host_user_input_sequence(session_id, call_id);
        if let Some(existing) = session.user_input_request_for_operation(
            resolved_pending.operation_id.as_str(),
            sequence_index,
        ) {
            if existing.request.questions != request.questions {
                return Err(AppError::Internal(format!(
                    "host user input request mismatch for operation {} at step {}",
                    resolved_pending.operation_id, sequence_index
                )));
            }
            if let Some(reply) = existing.reply.as_ref() {
                return host_user_input_response(&existing.request, reply);
            }
            let response_rx = self
                .install_host_user_input_waiter(existing.request.request_id.clone())
                .await;
            return self
                .await_host_user_input_reply(existing.request.request_id.as_str(), response_rx)
                .await;
        }

        let request_id = host_user_input_request_id(session_id, call_id, sequence_index);
        let response_rx = self
            .install_host_user_input_waiter(request_id.clone())
            .await;
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
        self.await_host_user_input_reply(request_id.as_str(), response_rx)
            .await
    }

    pub async fn execute_host_invoked_tool(
        &self,
        session_id: i64,
        call_id: i64,
        invocation: ToolInvocation,
    ) -> Result<ToolInvocationExecution, AppError> {
        let _host_user_input_sequence = self.host_user_input_sequence_guard(session_id, call_id);
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

    fn next_host_user_input_sequence(&self, session_id: i64, call_id: i64) -> usize {
        let mut guard = self
            .host_user_input_sequences
            .lock()
            .expect("host user input sequence lock poisoned");
        let sequence = guard.entry((session_id, call_id)).or_insert(0);
        let next = *sequence;
        *sequence += 1;
        next
    }

    fn host_user_input_sequence_guard(
        &self,
        session_id: i64,
        call_id: i64,
    ) -> HostUserInputSequenceGuard {
        HostUserInputSequenceGuard::new(
            Arc::clone(&self.host_user_input_sequences),
            session_id,
            call_id,
        )
    }

    async fn install_host_user_input_waiter(
        &self,
        request_id: String,
    ) -> oneshot::Receiver<crate::plugin::sdk::host_api::AskUserResponse> {
        let (response_tx, response_rx) = oneshot::channel();
        self.host_user_input_waiters.lock().await.insert(
            request_id,
            PendingHostUserInput {
                response: response_tx,
            },
        );
        response_rx
    }

    async fn await_host_user_input_reply(
        &self,
        request_id: &str,
        response_rx: oneshot::Receiver<crate::plugin::sdk::host_api::AskUserResponse>,
    ) -> Result<crate::plugin::sdk::host_api::AskUserResponse, AppError> {
        response_rx.await.map_err(|_| {
            AppError::Internal(format!(
                "host user input waiter closed before reply: {request_id}"
            ))
        })
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

fn run_control_to_app_error(err: RunControlError) -> AppError {
    match err {
        RunControlError::NoActiveRun(id) => {
            AppError::Internal(format!("no in-flight run for session {id}"))
        }
        RunControlError::SteerClosed => {
            AppError::Internal("steer channel closed for session".to_string())
        }
    }
}

fn is_user_cancelled_error(err: &AppError) -> bool {
    matches!(err, AppError::Internal(message) if message == "run cancelled by user")
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
        provider_state: None,
        usage: None,
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
    let normalized_part = session
        .resolve_part_ref(&pending_tool.part)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "pending tool part not found: message={}, part={}",
                pending_tool.part.message_id, pending_tool.part.part_id
            ))
        })?;
    let normalized_pending = SessionPendingTool {
        part: normalized_part,
    };
    let part = session.part(&normalized_pending.part).ok_or_else(|| {
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
        .pending_tool_execution(&normalized_pending)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "pending tool payload missing: message={}, part={}",
                pending_tool.part.message_id, pending_tool.part.part_id
            ))
        })?;

    Ok(ResolvedPendingTool {
        pending: normalized_pending,
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
    attachments: &[AttachmentItem],
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
        PartContent::Request(RequestPart::Permission(permission)) => permission.status(),
        PartContent::Request(RequestPart::UserInput(request)) => request.status(),
        _ => ExecutionStatus::Completed,
    }
}

fn build_request_part(
    part_id: i64,
    message_id: i64,
    operation_id: &str,
    request: RequestPart,
) -> MessagePart {
    let mut part = MessagePart::with_content(
        part_id,
        message_id,
        Utc::now(),
        request.status(),
        PartContent::Request(request),
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

async fn persisted_rules_for_reply(
    store: &SessionStore,
    session_id: i64,
    actions: &[PermissionAction],
    reply: &PermissionReply,
    operator: Option<&str>,
) -> Result<Vec<PersistedPermissionRule>, AppError> {
    let Some(mode) = persisted_mode_for_reply(reply.kind) else {
        return Ok(Vec::new());
    };
    let scope = reply.scope.unwrap_or(PermissionScope::Session);
    let workspace_id = match scope {
        PermissionScope::Session | PermissionScope::Global => None,
        PermissionScope::Workspace => Some(store.current_workspace_id().await?),
    };
    let session_rule_id = match scope {
        PermissionScope::Session => Some(session_id),
        PermissionScope::Workspace | PermissionScope::Global => None,
    };
    let mut seen = HashSet::new();
    let mut rules = Vec::new();
    for action in actions {
        let action_key = permission_action_key(action)?;
        if !seen.insert(action_key.clone()) {
            continue;
        }
        rules.push(PersistedPermissionRule {
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
        });
    }
    Ok(rules)
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

fn host_user_input_request_id(session_id: i64, call_id: i64, sequence_index: usize) -> String {
    format!("host-input:{session_id}:{call_id}:{sequence_index}")
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
