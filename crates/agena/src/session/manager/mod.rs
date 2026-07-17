use std::{
    collections::{HashMap, HashSet},
    future::Future,
    net::IpAddr,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use arc_swap::ArcSwap;
use chrono::Utc;
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};

use crate::AppError;
use crate::config::ProviderNativeToolsConfig;
use crate::event::{
    EventKind, ExecutionFinishedEvent, ExecutionStartedEvent, PermissionRepliedEvent,
    PermissionRequestedEvent,
};
use crate::message::{
    AttachmentItem, ExecutionStatus, InteractiveRequestPart, Message, MessageMetadata, MessagePart,
    MessageSource, MessageStatus, OperationBlock, OperationPart, PartContent, RequestPart,
    TimeRange, ToolInvocation, ToolOutput, UserInputReply, UserInputReplyKind, UserInputRequest,
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
use super::cost::{UsageStats, UsageStatsQuery};
use super::execution_registry::{ExecutionControl, ExecutionControlError, ExecutionRegistry};
use super::history::{
    FinishReason, MessageId as HistoryMessageId, RunAbortReason, RunAborted, RunCompleted,
    RunId as HistoryRunId, RunStarted, ToolCallCompleted, ToolCallId as HistoryToolCallId,
    TranscriptContent, UserMessageAppended,
};
use super::model::{
    PromptCompactionRuntime, PromptCompactionStrategy, ProviderPromptAnchor,
    SessionExecutionContext, SessionListRequest, SessionPendingTool, SessionSummary, WorkflowState,
};
use super::processor::{SessionRunRequest, SessionRunTermination};
use super::prompt_window::PromptRequestOptions;
use super::store::{ReservedMessageIds, SessionCommit, SessionStore};
use crate::session::{
    ExecutionFailureKind, ExecutionOutcome, ExecutionSource, Session, SessionProcessor,
};

pub const DEFAULT_SESSION_CACHE_MAX_SESSIONS: usize = 128;
pub const DEFAULT_SESSION_CACHE_TTL_SECS: u64 = 15 * 60;
pub const DEFAULT_SESSION_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_MAX_CONCURRENT_TOOLS: usize = 32;

#[derive(Debug, Clone, Default)]
pub struct SessionManagerConfig {
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

impl SessionManagerConfig {
    fn cache_policy(&self) -> SessionCachePolicy {
        SessionCachePolicy {
            max_sessions: DEFAULT_SESSION_CACHE_MAX_SESSIONS,
            ttl: Duration::from_secs(DEFAULT_SESSION_CACHE_TTL_SECS),
            max_bytes: DEFAULT_SESSION_CACHE_MAX_BYTES,
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
    fn completion_request(
        &self,
        system: Option<String>,
        messages: Vec<Message>,
        tool_api_functions: Vec<crate::tool::ToolApiBinding>,
        provider_native_tools: ProviderNativeToolsConfig,
        prompt_cache_key: Option<String>,
        previous_response_id: Option<String>,
        prompt_window_generation: Option<u64>,
    ) -> crate::provider::CompletionRequest {
        crate::provider::CompletionRequest {
            model: self.model.model_id.clone(),
            system,
            messages,
            tool_api_functions,
            provider_native_tools,
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
            responses_api_metadata: None,
            request_override: self.request_override.clone(),
        }
    }
}

pub(super) fn merge_system_prompts(
    primary: Option<&str>,
    secondary: Option<&str>,
) -> Option<String> {
    match (
        primary.map(str::trim).filter(|value| !value.is_empty()),
        secondary.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Some(primary), Some(secondary))
            if secondary == primary
                || secondary
                    .strip_prefix(primary)
                    .is_some_and(|suffix| suffix.starts_with("\n\n")) =>
        {
            Some(secondary.to_string())
        }
        (Some(primary), Some(secondary)) => Some(format!("{primary}\n\n{secondary}")),
        (Some(primary), None) => Some(primary.to_string()),
        (None, Some(secondary)) => Some(secondary.to_string()),
        (None, None) => None,
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
    pub profile_name: String,
    pub task_id: Option<String>,
    pub requested_selection: crate::agents::AgentSelectionConfig,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SessionSubtaskResponse {
    pub session: Session,
    pub task_id: String,
    pub parent_session_id: i64,
    pub profile_name: String,
    pub status: crate::session::SubtaskStatus,
    pub resumed: bool,
    pub final_text: Option<String>,
    pub error: Option<String>,
    pub usage: crate::message::MessageUsage,
    pub model_provider_id: Option<String>,
    pub model_adapter_id: Option<String>,
    pub model_id: Option<String>,
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
    advertised_tool_identity: Option<String>,
    prepared_shell_command: Option<PreparedShellCommand>,
    lifecycle: TimeRange,
    session_runtime: crate::session::SessionRuntimeState,
}

struct PendingHostUserInput {
    session_id: i64,
    response: oneshot::Sender<crate::plugin::sdk::host_api::AskUserResponse>,
}

struct PendingHostPermission {
    session_id: i64,
    response: oneshot::Sender<PermissionReply>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HostPermissionGrantKey {
    session_id: i64,
    call_id: i64,
    plugin_id: String,
    tool_name: String,
}

/// A short-lived, exact-action grant for permission checks made by an
/// execution tool after a user approves an inner Tool API call. The tool
/// itself executes with an executor-level bypass; this map covers permission
/// checks that flow back through the plugin host during that same execution.
struct HostPermissionGrantGuard {
    grants: Arc<StdMutex<HashMap<HostPermissionGrantKey, Vec<PermissionAction>>>>,
    key: HostPermissionGrantKey,
}

impl HostPermissionGrantGuard {
    fn install(
        grants: Arc<StdMutex<HashMap<HostPermissionGrantKey, Vec<PermissionAction>>>>,
        session_id: i64,
        call_id: i64,
        plugin_id: String,
        tool_name: String,
        actions: Vec<PermissionAction>,
    ) -> Self {
        let key = HostPermissionGrantKey {
            session_id,
            call_id,
            plugin_id,
            tool_name,
        };
        let mut guard = grants.lock().expect("host permission grant lock poisoned");
        guard.insert(key.clone(), actions);
        drop(guard);
        Self { grants, key }
    }
}

impl Drop for HostPermissionGrantGuard {
    fn drop(&mut self) {
        if let Ok(mut grants) = self.grants.lock() {
            grants.remove(&self.key);
        }
    }
}

static HOST_PERMISSION_REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

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
    tool_execution_semaphore: Arc<Semaphore>,
}

mod compact;
mod helpers;
mod history;
mod replies;
mod runs;
mod sessions;
mod stats;

use self::helpers::*;
use self::replies::{AggregatedPermissionOutcome, AggregatedPermissionRequest};

impl SessionManagerState {
    fn new(
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
        config: SessionManagerConfig,
    ) -> Self {
        let tool_execution_semaphore = Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENT_TOOLS));
        Self {
            processor,
            tool_executor,
            config,
            tool_execution_semaphore,
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
    execution_registry: Arc<ExecutionRegistry>,
    reply_session_locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
    host_user_input_waiters: Arc<Mutex<HashMap<String, PendingHostUserInput>>>,
    host_user_input_sequences: Arc<StdMutex<HashMap<HostUserInputSequenceKey, usize>>>,
    host_permission_waiters: Arc<Mutex<HashMap<String, PendingHostPermission>>>,
    host_permission_grants: Arc<StdMutex<HashMap<HostPermissionGrantKey, Vec<PermissionAction>>>>,
}

/// A one-shot execution capability created only after every permission check
/// for a concrete session tool invocation has resolved to `Allow`.
///
/// Its fields are intentionally private so API surfaces cannot manufacture a
/// post-authorization executor or invoke a generic permission bypass.
pub struct AuthorizedToolInvocation {
    executor: ToolExecutor,
    invocation: ToolInvocation,
    session_id: i64,
}

impl AuthorizedToolInvocation {
    pub fn execute(self, call_id: i64) -> Result<ToolInvocationExecution, ToolError> {
        self.executor
            .execute_invocation_detailed_bypassing_permissions(
                &self.invocation,
                self.session_id,
                call_id,
            )
    }
}

pub enum ToolInvocationAuthorization {
    Allowed(Box<AuthorizedToolInvocation>),
    Ask { reason: String },
    Deny { reason: String },
}

impl SessionManager {
    async fn begin_execution(
        &self,
        session_id: i64,
        control: &ExecutionControl,
        source: ExecutionSource,
    ) -> Result<(), AppError> {
        let event = EventKind::ExecutionStarted(ExecutionStartedEvent {
            session_id,
            execution_id: control.execution_id(),
            source,
            ts_ms: Utc::now().timestamp_millis(),
        });
        self.store
            .append_lifecycle_events(session_id, vec![event])
            .await
    }

    async fn finish_execution(
        &self,
        session_id: i64,
        control: &ExecutionControl,
        outcome: ExecutionOutcome,
    ) -> Result<(), AppError> {
        let event = EventKind::ExecutionFinished(ExecutionFinishedEvent {
            session_id,
            execution_id: control.execution_id(),
            outcome: outcome.clone(),
            ts_ms: Utc::now().timestamp_millis(),
        });
        self.store
            .append_lifecycle_events(session_id, vec![event])
            .await?;
        control
            .finish(outcome)
            .await
            .map_err(execution_control_to_app_error)
    }

    fn execution_outcome<T>(
        control: &ExecutionControl,
        result: &Result<T, AppError>,
    ) -> ExecutionOutcome {
        if control.cancel.is_cancelled() {
            ExecutionOutcome::Cancelled
        } else {
            match result {
                Ok(_) => ExecutionOutcome::Completed,
                Err(error) => ExecutionOutcome::Failed {
                    failure_kind: execution_failure_kind(error),
                    message: error.to_string(),
                },
            }
        }
    }

    /// Own one complete execution lifecycle, including panic-safe task joining,
    /// durable terminal publication, and registry cleanup.
    ///
    /// Every public command that can run model or compaction work must enter
    /// through this boundary. Keeping acquisition and finalization in one
    /// function prevents early returns from leaking an active registry entry or
    /// an unmatched `ExecutionStarted` event.
    async fn execute_registered<T, F, Fut>(
        &self,
        session_id: i64,
        source: ExecutionSource,
        task_name: &'static str,
        operation: F,
    ) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(
                SessionManager,
                Arc<ExecutionControl>,
                mpsc::UnboundedReceiver<Vec<PartContent>>,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<T, AppError>> + Send + 'static,
    {
        let (control, steer_rx) = self
            .execution_registry
            .register(session_id)
            .await
            .map_err(execution_control_to_app_error)?;
        if let Err(error) = self
            .begin_execution(session_id, control.as_ref(), source)
            .await
        {
            self.execution_registry
                .unregister_if_matches(session_id, &control)
                .await;
            return Err(error);
        }

        crate::metrics::session_started();
        let manager = self.background_handle();
        let task_control = Arc::clone(&control);
        let task = tokio::task::spawn(operation(manager, task_control, steer_rx));
        control.attach_operation_abort(task.abort_handle()).await;
        let result = match task.await {
            Ok(result) => result,
            Err(error) if error.is_cancelled() && control.cancel.is_cancelled() => {
                Err(AppError::Cancelled)
            }
            Err(error) => Err(AppError::Internal(format!(
                "{task_name} task failed: {error}"
            ))),
        };
        control.clear_operation_abort().await;
        crate::metrics::session_finished();

        let unmatched_run_reason = result
            .as_ref()
            .err()
            .map(run_abort_reason)
            .unwrap_or(RunAbortReason::Internal);
        let reconciliation_result = self
            .store
            .reconcile_unmatched_runs(
                session_id,
                unmatched_run_reason,
                "execution ended without a terminal run event".to_string(),
            )
            .await;
        let outcome = Self::execution_outcome(control.as_ref(), &result);
        let terminal_result = self
            .finish_execution(session_id, control.as_ref(), outcome)
            .await;
        self.execution_registry
            .unregister_if_matches(session_id, &control)
            .await;
        terminal_result?;
        reconciliation_result?;
        result
    }

    fn background_handle(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            publisher: Arc::clone(&self.publisher),
            bus: Arc::clone(&self.bus),
            execution: ArcSwap::from(self.execution.load_full()),
            execution_registry: Arc::clone(&self.execution_registry),
            reply_session_locks: Arc::clone(&self.reply_session_locks),
            host_user_input_waiters: Arc::clone(&self.host_user_input_waiters),
            host_user_input_sequences: Arc::clone(&self.host_user_input_sequences),
            host_permission_waiters: Arc::clone(&self.host_permission_waiters),
            host_permission_grants: Arc::clone(&self.host_permission_grants),
        }
    }

    pub fn new(
        db: sea_orm::DatabaseConnection,
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
        config: SessionManagerConfig,
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
        let state = SessionManagerState::new(processor, tool_executor, config);
        Self {
            store,
            publisher,
            bus,
            execution: ArcSwap::from_pointee(state),
            execution_registry: Arc::new(ExecutionRegistry::new()),
            reply_session_locks: Arc::new(Mutex::new(HashMap::new())),
            host_user_input_waiters: Arc::new(Mutex::new(HashMap::new())),
            host_user_input_sequences: Arc::new(StdMutex::new(HashMap::new())),
            host_permission_waiters: Arc::new(Mutex::new(HashMap::new())),
            host_permission_grants: Arc::new(StdMutex::new(HashMap::new())),
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

    /// Resolve all static, persisted, and plugin-provided permission decisions
    /// for an externally initiated session tool call without creating a user
    /// approval request. Callers may execute only the returned opaque
    /// capability, never a generic bypass.
    pub async fn authorize_session_tool_invocation(
        &self,
        session_id: i64,
        invocation: ToolInvocation,
    ) -> Result<ToolInvocationAuthorization, AppError> {
        let session = self.get_session(session_id).await?;
        let state = self.execution_state();
        let executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution);
        let checks = executor
            .collect_permission_checks_for_invocation_in_session(&invocation, Some(session.id))
            .map_err(tool_error_to_app_error)?;

        for check in checks {
            match self
                .resolve_tool_permission_check(Some(session.id), &check)
                .await?
                .decision
            {
                PermissionDecision::Allow => {}
                PermissionDecision::Ask { reason } => {
                    return Ok(ToolInvocationAuthorization::Ask { reason });
                }
                PermissionDecision::Deny { reason } => {
                    return Ok(ToolInvocationAuthorization::Deny { reason });
                }
            }
        }

        Ok(ToolInvocationAuthorization::Allowed(Box::new(
            AuthorizedToolInvocation {
                executor,
                invocation,
                session_id: session.id,
            },
        )))
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
                .install_host_user_input_waiter(session_id, existing.request.request_id.clone())
                .await;
            return self
                .await_host_user_input_reply(
                    session_id,
                    existing.request.request_id.as_str(),
                    existing.request.auto_resolution_ms,
                    existing.request.created_at,
                    response_rx,
                )
                .await;
        }

        let request_id = host_user_input_request_id(session_id, call_id, sequence_index);
        let auto_resolution_ms = request.auto_resolution_ms;
        let created_at = Utc::now();
        let response_rx = self
            .install_host_user_input_waiter(session_id, request_id.clone())
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
        self.await_host_user_input_reply(
            session_id,
            request_id.as_str(),
            auto_resolution_ms,
            created_at,
            response_rx,
        )
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
        let cancellation = self.execution_registry.cancellation_token(session_id).await;
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution)
            .with_cancellation_token(cancellation.clone());
        let prepared = scoped_executor
            .prepare_invocation(&invocation, session.id, call_id)
            .map_err(tool_error_to_app_error)?;
        let (invocation, prepared_shell_command) = scoped_executor
            .prepare_shell_invocation(&prepared.invocation, session.id, call_id)
            .map_err(tool_error_to_app_error)?;
        let target = scoped_executor
            .plugin_manager()
            .lookup_tool(invocation.name.as_str())
            .ok_or_else(|| {
                AppError::Internal(format!("target tool `{}` not found", invocation.name))
            })?;
        let target_plugin_id = target.plugin_full_name();
        let target_tool_name = target.tool_name().to_string();

        let permission_checks = scoped_executor
            .collect_permission_checks_for_invocation_in_session(&invocation, Some(session.id))
            .map_err(tool_error_to_app_error)?;
        let granted_actions = match self
            .aggregate_permission_outcome(Some(session.id), permission_checks.as_slice())
            .await?
        {
            AggregatedPermissionOutcome::Allow => None,
            AggregatedPermissionOutcome::Deny { reason } => {
                return Err(AppError::Internal(format!("permission denied: {reason}")));
            }
            AggregatedPermissionOutcome::Request(request) => {
                let request = *request;
                let reply = self
                    .request_host_invoked_tool_permission(
                        session_id,
                        call_id,
                        request.clone(),
                        state.clone(),
                    )
                    .await?;
                match reply.kind {
                    PermissionReplyKind::AllowOnce | PermissionReplyKind::AllowAlways => {
                        Some(if request.requested_actions.is_empty() {
                            vec![request.action]
                        } else {
                            request.requested_actions
                        })
                    }
                    PermissionReplyKind::DenyOnce | PermissionReplyKind::DenyAlways => {
                        let reason = reply
                            .reason
                            .unwrap_or_else(|| "permission denied by user".to_string());
                        return Err(AppError::Internal(format!("permission denied: {reason}")));
                    }
                }
            }
        };

        let _permission_grant = granted_actions.map(|actions| {
            HostPermissionGrantGuard::install(
                Arc::clone(&self.host_permission_grants),
                session_id,
                call_id,
                target_plugin_id,
                target_tool_name,
                actions,
            )
        });

        // The model-visible operation is the outer `tools.call`, while the
        // target reuses its call id through the host callback context. Keep
        // that outer pending part up to date when the target is streaming.
        let outer_pending_tool = session.pending_tools().into_iter().find(|tool| {
            session
                .pending_tool_execution(tool)
                .is_some_and(|(pending_call_id, _, _)| pending_call_id == call_id)
        });
        if let Some(mut stream) = scoped_executor
            .execute_invocation_streaming_after_authorization(&invocation, session_id, call_id)
            .await
            .map_err(tool_error_to_app_error)?
        {
            let stream_id = stream.stream_id.clone();
            loop {
                let chunk = match cancellation.as_ref() {
                    Some(cancellation) => tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => return Err(AppError::Cancelled),
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
                if let Some(pending_tool) = outer_pending_tool.as_ref() {
                    self.append_streaming_tool_output_delta(
                        session_id,
                        pending_tool,
                        delta,
                        state.clone(),
                    )
                    .await?;
                }
            }
            let end = match cancellation.as_ref() {
                Some(cancellation) => tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Err(AppError::Cancelled),
                    end = stream.end => end,
                },
                None => stream.end.await,
            };
            return end
                .map_err(|_| {
                    AppError::Internal(format!(
                        "host-invoked tool stream ended without a terminal result: {stream_id}"
                    ))
                })?
                .map_err(tool_error_to_app_error);
        }

        tokio::task::spawn_blocking(move || {
            scoped_executor.execute_invocation_detailed_bypassing_permissions_with_prepared_shell(
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

    async fn request_host_invoked_tool_permission(
        &self,
        session_id: i64,
        call_id: i64,
        request: AggregatedPermissionRequest,
        state: Arc<SessionManagerState>,
    ) -> Result<PermissionReply, AppError> {
        // Parallel gateway calls may discover permissions at the same time.
        // Serialize only the database projection update so each request is
        // based on the latest session and remains attached to its own call.
        // The lock is deliberately released before waiting for the user.
        let request_lock = self.reply_session_lock(session_id).await;
        let request_guard = request_lock.lock().await;
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
                    "pending tool not found for host-invoked permission: session={session_id}, call={call_id}"
                ))
            })?;
        let resolved = resolve_pending_tool(&session, &pending_tool)?;
        let request_id = host_permission_request_id(session.id, resolved.call_id);
        let response_rx = self
            .install_host_permission_waiter(session.id, request_id.clone())
            .await;
        if let Err(err) = self
            .apply_permission_request_with_id(
                session,
                &pending_tool,
                request_id.clone(),
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
            )
            .await
        {
            self.host_permission_waiters
                .lock()
                .await
                .remove(request_id.as_str());
            return Err(err);
        }
        drop(request_guard);
        self.await_host_permission_reply(request_id.as_str(), response_rx)
            .await
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
        session_id: i64,
        request_id: String,
    ) -> oneshot::Receiver<crate::plugin::sdk::host_api::AskUserResponse> {
        let (response_tx, response_rx) = oneshot::channel();
        self.host_user_input_waiters.lock().await.insert(
            request_id,
            PendingHostUserInput {
                session_id,
                response: response_tx,
            },
        );
        response_rx
    }

    async fn await_host_user_input_reply(
        &self,
        session_id: i64,
        request_id: &str,
        auto_resolution_ms: Option<u64>,
        created_at: chrono::DateTime<Utc>,
        mut response_rx: oneshot::Receiver<crate::plugin::sdk::host_api::AskUserResponse>,
    ) -> Result<crate::plugin::sdk::host_api::AskUserResponse, AppError> {
        let receive = |result: Result<
            crate::plugin::sdk::host_api::AskUserResponse,
            oneshot::error::RecvError,
        >| {
            result.map_err(|_| {
                AppError::Internal(format!(
                    "host user input waiter closed before reply: {request_id}"
                ))
            })
        };
        let Some(timeout_ms) = auto_resolution_ms else {
            return receive(response_rx.await);
        };
        let elapsed_ms = Utc::now()
            .signed_duration_since(created_at)
            .num_milliseconds()
            .max(0) as u64;
        let remaining = Duration::from_millis(timeout_ms.saturating_sub(elapsed_ms));
        tokio::select! {
            biased;
            result = &mut response_rx => receive(result),
            _ = tokio::time::sleep(remaining) => {
                let state = self.execution_state();
                let session = self.store.load_session(session_id, state.cache_policy()).await?;
                let options = self.run_options_from_session(&session, state)?;
                self.reply_user_input(SessionExecutionReplyRequest::new(
                    session_id,
                    options,
                    UserInputReply {
                        request_id: request_id.to_string(),
                        kind: UserInputReplyKind::Timeout,
                        answers: Default::default(),
                        reason: Some("auto-resolution deadline elapsed".to_string()),
                    },
                )).await?;
                receive(response_rx.await)
            }
        }
    }

    async fn install_host_permission_waiter(
        &self,
        session_id: i64,
        request_id: String,
    ) -> oneshot::Receiver<PermissionReply> {
        let (response_tx, response_rx) = oneshot::channel();
        self.host_permission_waiters.lock().await.insert(
            request_id,
            PendingHostPermission {
                session_id,
                response: response_tx,
            },
        );
        response_rx
    }

    async fn cancel_host_interactive_waiters(&self, session_id: i64) {
        let permission_waiters = {
            let mut waiters = self.host_permission_waiters.lock().await;
            let request_ids = waiters
                .iter()
                .filter_map(|(request_id, waiter)| {
                    (waiter.session_id == session_id).then_some(request_id.clone())
                })
                .collect::<Vec<_>>();
            request_ids
                .into_iter()
                .filter_map(|request_id| {
                    waiters
                        .remove(request_id.as_str())
                        .map(|waiter| (request_id, waiter))
                })
                .collect::<Vec<_>>()
        };
        for (request_id, waiter) in permission_waiters {
            let _ = waiter.response.send(PermissionReply {
                request_id,
                kind: PermissionReplyKind::DenyOnce,
                reason: Some("run cancelled by user".to_string()),
                scope: None,
            });
        }

        let input_waiters = {
            let mut waiters = self.host_user_input_waiters.lock().await;
            let request_ids = waiters
                .iter()
                .filter_map(|(request_id, waiter)| {
                    (waiter.session_id == session_id).then_some(request_id.clone())
                })
                .collect::<Vec<_>>();
            request_ids
                .into_iter()
                .filter_map(|request_id| waiters.remove(request_id.as_str()))
                .collect::<Vec<_>>()
        };
        for waiter in input_waiters {
            let _ = waiter
                .response
                .send(crate::plugin::sdk::host_api::AskUserResponse {
                    cancelled: true,
                    ..Default::default()
                });
        }
    }

    async fn await_host_permission_reply(
        &self,
        request_id: &str,
        response_rx: oneshot::Receiver<PermissionReply>,
    ) -> Result<PermissionReply, AppError> {
        response_rx.await.map_err(|_| {
            AppError::Internal(format!(
                "host-invoked permission waiter closed before reply: {request_id}"
            ))
        })
    }

    pub(crate) fn has_host_permission_grant(
        &self,
        session_id: i64,
        call_id: i64,
        plugin_id: &str,
        tool_name: &str,
        action: &PermissionAction,
    ) -> bool {
        let key = HostPermissionGrantKey {
            session_id,
            call_id,
            plugin_id: plugin_id.to_string(),
            tool_name: tool_name.to_string(),
        };
        self.host_permission_grants
            .lock()
            .ok()
            .and_then(|grants| grants.get(&key).cloned())
            .is_some_and(|actions| host_permission_grant_matches_action(&actions, action))
    }

    fn install_host_permission_grant_for_pending_tool(
        &self,
        state: &SessionManagerState,
        session_id: i64,
        pending_tool: &ResolvedPendingTool,
        actions: Vec<PermissionAction>,
    ) -> Option<HostPermissionGrantGuard> {
        let scoped_executor = state
            .tool_executor
            .for_session_context(&pending_tool.session_runtime.execution);
        let target = scoped_executor
            .plugin_manager()
            .lookup_tool(pending_tool.invocation.name.as_str())?;
        Some(HostPermissionGrantGuard::install(
            Arc::clone(&self.host_permission_grants),
            session_id,
            pending_tool.call_id,
            target.plugin_full_name(),
            target.tool_name().to_string(),
            actions,
        ))
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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use sea_orm::Database;

    use super::{
        SessionManager, SessionManagerConfig, build_message, host_permission_grant_matches_action,
        merge_system_prompts,
    };
    use crate::model::ModelRef;
    use crate::plugin::sdk::ToolStreamSink;
    use crate::{
        agent::Agent,
        agents::SubagentRegistry,
        db,
        message::{
            ExecutionStatus, MessageMetadata, OperationPart, PartContent, StructuredObject,
            TimeRange, ToolInvocation,
        },
        permission::{PermissionAction, PermissionPolicy, ToolPermissionPolicy},
        plugin::{
            ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig,
            StaticPluginRegistration, ToolPresentationConfig,
        },
        provider::ProviderRegistry,
        role::Role,
        session::{
            ContextGovernor, ContextPolicy, Session, SessionCreateRequest, SessionProcessor,
            SessionRunOptions,
        },
        tool::ToolExecutor,
    };

    #[test]
    fn system_prompt_merge_is_idempotent_for_an_already_applied_agent_prompt() {
        assert_eq!(
            merge_system_prompts(Some("agent"), Some("agent\n\ncustom")),
            Some("agent\n\ncustom".to_string())
        );
        assert_eq!(
            merge_system_prompts(Some("agent"), Some("custom")),
            Some("agent\n\ncustom".to_string())
        );
    }

    #[derive(Default)]
    struct StreamingExecutionTool;

    #[crate::plugin::sdk::agena_plugin(
        namespace = "test",
        name = "stream",
        version = "0.1.0",
        summary = "Streaming execution-tool regression fixture."
    )]
    impl StreamingExecutionTool {
        #[tool(
            name = "emit",
            summary = "Emit streaming chunks.",
            read_only,
            stream = emit_stream
        )]
        async fn emit(&self) -> String {
            "buffered-handler".to_string()
        }

        async fn emit_stream(&self, sink: ToolStreamSink) -> String {
            sink.text("stream-").await;
            sink.text("handler").await;
            "stream-terminal".to_string()
        }
    }

    async fn test_manager() -> SessionManager {
        let workspace_root = std::env::current_dir().expect("resolve test workspace");
        let mut plugins_config = PluginsConfig::default();
        plugins_config.list.insert(
            "test.stream".to_string(),
            ConfiguredPlugin::static_default(),
        );
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![StaticPluginRegistration::new(
                "test.stream".parse().expect("valid test plugin key"),
                StreamingExecutionTool,
            )],
            config: plugins_config,
            workspace_root: workspace_root.clone(),
            agena_version: "test".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: HashMap::new(),
        })
        .await
        .expect("build test plugin host");

        let executor = ToolExecutor::new(
            workspace_root.clone(),
            Agent::new(
                "test",
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            SubagentRegistry::default(),
            Arc::clone(&plugins),
            None,
            None,
            None,
            ToolPresentationConfig::default(),
        );
        let processor = SessionProcessor::new(
            Arc::new(ProviderRegistry::new()),
            ContextGovernor::new(ContextPolicy::default()),
            plugins,
            workspace_root,
        );
        let database = Database::connect("sqlite::memory:")
            .await
            .expect("open in-memory database");
        db::init_schema(&database)
            .await
            .expect("migrate in-memory database");
        SessionManager::new(
            database,
            processor,
            executor,
            SessionManagerConfig::default(),
        )
    }

    async fn install_pending_tool_api_operation(
        manager: &SessionManager,
        mut session: Session,
        call_id: i64,
    ) -> Session {
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("reserve Tool API operation message ids");
        let invocation = ToolInvocation::new(
            "agena.tools.call",
            StructuredObject::try_from(serde_json::json!({
                "tool": "stream.emit",
                "input": {}
            }))
            .expect("structured Tool API input"),
        );
        let operation =
            OperationPart::pending(call_id, invocation, "Tool tools.call", TimeRange::default());
        let mut message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::InProgress,
            vec![PartContent::Operation(operation)],
            MessageMetadata::default(),
        );
        message.parts[0].operation_id = Some("tool-api-stream-test".to_string());
        session.messages.push(message.clone());
        manager
            .persist_session_changes(
                session,
                vec![message],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist pending Tool API operation")
    }

    #[tokio::test]
    async fn updating_model_selection_is_immediate_and_session_local() {
        let manager = test_manager().await;
        let first = manager
            .create_session(SessionCreateRequest {
                title: "first session".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create first session");
        let selected_model =
            ModelRef::new_with_adapter("selected-provider", "selected-adapter", "selected-model");

        manager
            .update_session_selection(
                first.id,
                SessionRunOptions {
                    model: selected_model.clone(),
                    thinking_mode: Some("high".to_owned()),
                    speed_mode: Some("fast".to_owned()),
                    verbosity: Some("high".to_owned()),
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                    agent_profile: None,
                },
            )
            .await
            .expect("update first session model");

        let reloaded = manager
            .get_session(first.id)
            .await
            .expect("reload first session");
        assert_eq!(
            reloaded
                .runtime()
                .effective_model_ref()
                .expect("valid model reference"),
            Some(selected_model)
        );
        assert_eq!(
            reloaded.runtime().model_thinking_mode_override(),
            Some("high")
        );
        assert_eq!(reloaded.runtime().model_speed_mode_override(), Some("fast"));
        assert_eq!(reloaded.runtime().model_verbosity_override(), Some("high"));

        let second = manager
            .create_session(SessionCreateRequest {
                title: "second session".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create second session");
        assert_eq!(
            second
                .runtime()
                .effective_model_ref()
                .expect("valid empty model selection"),
            None
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_invoked_streaming_tool_updates_outer_tool_api_operation() {
        let manager = test_manager().await;
        let call_id = 73;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "Tool API stream regression".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create test session");
        let session = install_pending_tool_api_operation(&manager, session, call_id).await;

        let execution = manager
            .execute_host_invoked_tool(
                session.id,
                call_id,
                ToolInvocation::new("test.stream.emit", StructuredObject::default()),
            )
            .await
            .expect("execute streaming tool");

        // The ordinary handler deliberately returns a different value. This
        // proves the host path called `tool_invoke_stream`, not `tool_invoke`.
        assert_eq!(execution.view.output_text, "stream-terminal");

        let session = manager
            .get_session(session.id)
            .await
            .expect("reload streamed Tool API session");
        let part = session
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .find(|part| part.operation_id.as_deref() == Some("tool-api-stream-test"))
            .expect("outer Tool API operation remains present");
        assert_eq!(part.status, ExecutionStatus::InProgress);
        let PartContent::Operation(operation) = part.content.as_ref().expect("operation content")
        else {
            panic!("Tool API stream test part is not an operation");
        };
        assert_eq!(operation.model_output.text, "stream-handler");
    }

    #[test]
    fn host_permission_grant_covers_only_public_dns_resolution() {
        let granted = vec![PermissionAction::NetworkAccess {
            target: "https://openai.com/".to_string(),
            host: "openai.com".to_string(),
            port: Some(443),
        }];
        let public_address = PermissionAction::NetworkAccess {
            target: "104.18.33.45:443".to_string(),
            host: "104.18.33.45".to_string(),
            port: Some(443),
        };
        let private_address = PermissionAction::NetworkAccess {
            target: "10.0.0.1:443".to_string(),
            host: "10.0.0.1".to_string(),
            port: Some(443),
        };

        assert!(host_permission_grant_matches_action(
            &granted,
            &public_address
        ));
        assert!(!host_permission_grant_matches_action(
            &granted,
            &private_address
        ));
    }
}
