use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock},
    time::Duration,
};

use arc_swap::ArcSwap;
use chrono::Utc;
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};

use crate::AppError;
use crate::context_governor::ContextGovernor;
use crate::part::{AttachmentItem, AttachmentKind, AttachmentSource, OperationPart};
use crate::provider::ProviderRegistry;
use crate::tool::{StreamingToolExecution, ToolError, ToolExecutor, ToolInvocationExecution};
use agena_domain::ToolInvocation;
use agena_domain::ToolOutput;
use agena_domain::{
    DecisionTraceStep, ExecutionOutcome, ExecutionSource, PermissionAction, PermissionMode,
    PermissionReplyKind, PermissionScope, RunAbortReason, TimeRange, UserInputReplyKind,
};
use agena_domain::{ExecutionStatus, ProcessStatus};
pub(crate) use agena_domain::{ModelRef, ModelSpeedModeRequestOverride};
use agena_domain::{UserInputReply, UserInputSource};
use agena_failure::Failure;
use agena_runtime_contracts::part_content::{SystemNotificationContent, TextContent, TypedContent};
use agena_storage::PersistedPermissionRule;
use agena_storage::store::{
    BackgroundDelivery, BackgroundEventRequest, BackgroundOperationKind, BackgroundOperationPhase,
    BackgroundOperationTransition, NewBackgroundOperation, PartRole, PartState,
};
use agena_tool::PreparedShellCommand;

use super::model::{PromptCompactionRuntime, ProviderPromptAnchor, SessionPendingTool};
use super::processor::{SessionRunRequest, SessionRunTermination};
use super::prompt_window::PromptRequestOptions;
use super::store::{StoreAdapter, new_part_from_content};
use super::{ExecutionControl, ExecutionControlError, ExecutionRegistry};
use crate::session::{Session, SessionProcessor};
use agena_domain::{SessionListRequest, SessionSummary, UsageStats, UsageStatsQuery};

use agena_runtime::RuntimeSessionManagerConfig;

use agena_runtime::{
    SessionCreateRequest, SessionExecutionReplyRequest, SessionExecutionRequest,
    SessionPermissionReplyRequest, SessionRunOptions,
};

fn completion_request(
    options: &SessionRunOptions,
    system: Option<String>,
    turns: Vec<agena_provider::CompletionInputRun>,
    tool_api_functions: Vec<crate::tool::ToolApiBinding>,
    prompt_cache_key: Option<String>,
    previous_response_id: Option<String>,
    prompt_window_generation: Option<u64>,
) -> agena_provider::CompletionRequest {
    agena_provider::CompletionRequest {
        model: options.model.model_id.clone(),
        system,
        turns,
        tool_api_functions: tool_api_functions
            .into_iter()
            .map(|binding| binding.definition())
            .collect(),
        provider_native_tools: Default::default(),
        disable_tools: false,
        temperature: options.temperature,
        max_output_tokens: options.max_output_tokens,
        prompt_cache_key,
        previous_response_id,
        prompt_window_generation,
        provider_compaction: None,
        stop_sequences: Vec::new(),
        top_p: None,
        top_k: None,
        seed: None,
        thinking: options.thinking.clone(),
        verbosity: options.verbosity.clone(),
        response_format: None,
        responses_api_metadata: None,
        request_override: options.request_override.clone(),
    }
}

pub(super) use agena_runtime::merge_system_prompts;

/// A background notification is an at-least-once wake, but it must not become
/// an at-least-forever wake when the selected provider is unavailable. The
/// attempt count is durable on `agena_background_deliveries`, so this bound
/// survives runtime restarts and multiple dispatcher processes.
pub(crate) const MAX_BACKGROUND_DELIVERY_ATTEMPTS: u32 = 8;
const BACKGROUND_DELIVERY_RETRY_BASE_MS: i64 = 1_000;
const BACKGROUND_DELIVERY_RETRY_MAX_MS: i64 = 60_000;

fn background_delivery_retry_delay_ms(attempts: u32) -> i64 {
    let exponent = attempts.saturating_sub(1).min(6);
    BACKGROUND_DELIVERY_RETRY_BASE_MS
        .saturating_mul(1_i64 << exponent)
        .min(BACKGROUND_DELIVERY_RETRY_MAX_MS)
}

fn background_delivery_error(error: &AppError) -> serde_json::Value {
    serde_json::json!({
        "message": error.to_string(),
        "retryable": error.retryable(),
        "public_message": error.public_message(),
    })
}

fn background_delivery_should_retry(error: &AppError) -> bool {
    if error.retryable() {
        return true;
    }
    // The provider registry deliberately emits an unclassified Provider error
    // when its circuit is open. Treat that specific fail-fast signal as a
    // transient condition, still bounded by the durable delivery attempt cap.
    matches!(
        error,
        AppError::Provider(message) if message.contains("circuit breaker is open")
    )
}

#[derive(Debug, Clone)]
struct SessionUserRunRequest {
    run: SessionExecutionRequest,
    parts: Vec<TypedContent>,
    idempotency_key: Option<String>,
}

impl SessionUserRunRequest {
    fn new(session_id: i64, options: SessionRunOptions, parts: Vec<TypedContent>) -> Self {
        Self {
            run: SessionExecutionRequest::new(session_id, options),
            parts,
            idempotency_key: None,
        }
    }
}

#[derive(Debug, Clone)]
/// Request to run a subtask within a session.
pub struct SessionSubtaskRequest {
    pub parent_session_id: i64,
    pub description: String,
    pub prompt: String,
    pub access: agena_domain::ExecutionAccess,
    /// Optional Skill names or aliases to resolve and attach to the child
    /// session's first user message as immutable Skill references.
    pub skills: Option<Vec<String>>,
    pub task_id: Option<String>,
    pub requested_model_selection: agena_domain::ModelSelectionConfig,
    pub timeout_ms: Option<u64>,
    pub max_tokens: Option<u64>,
    pub max_cost_microusd: Option<u64>,
}

#[derive(Debug, Clone)]
/// Response of a session subtask.
pub struct SessionSubtaskResponse {
    pub session: Session,
    pub task_id: String,
    pub parent_session_id: i64,
    pub status: agena_domain::SubtaskStatus,
    pub resumed: bool,
    pub final_text: Option<String>,
    pub failure: Option<agena_failure::Failure>,
    pub usage: agena_provider::CompletionUsage,
    pub model_provider_id: Option<String>,
    pub model_adapter_id: Option<String>,
    pub model_id: Option<String>,
    pub budget_exceeded: bool,
}

#[derive(Debug, Clone)]
/// A chunk of subtask output.
pub struct SessionSubtaskOutputChunk {
    pub cursor: i64,
    pub role: agena_domain::Role,
    pub text: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
/// Output of a session subtask.
pub struct SessionSubtaskOutput {
    pub session_id: i64,
    pub chunks: Vec<SessionSubtaskOutputChunk>,
    pub next_cursor: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy)]
struct PromptTurnBudget {
    max_prompt_chars: usize,
    max_prompt_tokens: u64,
    model_context_window_tokens: Option<u32>,
}

/// Stable canonical conversation identity shared by every execution that
/// contributes to one assistant reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConversationIdentity {
    turn_id: agena_domain::TurnId,
    reply_id: agena_domain::AssistantReplyId,
}

/// Declares how a new execution attaches to the canonical conversation.
///
/// Callers resolving an interactive request must pass the exact reply that
/// owns that request. This keeps execution registration from guessing based
/// on whichever turn happens to be newest when the reply reaches the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionConversationTarget {
    NewTurn,
    LatestReply,
    ExistingReply(ConversationIdentity),
}

#[derive(Debug, Clone)]
struct ResolvedPendingTool {
    pending: SessionPendingTool,
    operation_id: String,
    call_id: i64,
    launch_run_id: i64,
    launch_tool_part_id: i64,
    invocation: ToolInvocation,
    advertised_tool_identity: Option<String>,
    prepared_shell_command: Option<PreparedShellCommand>,
    lifecycle: TimeRange,
    session_runtime: crate::session::SessionRuntimeState,
}

impl ResolvedPendingTool {
    fn scheduled_job_launch_provenance(
        &self,
        session_id: i64,
    ) -> agena_scheduler::ScheduledJobLaunchProvenance {
        agena_scheduler::ScheduledJobLaunchProvenance {
            session_id,
            run_id: self.launch_run_id,
            tool_part_id: self.launch_tool_part_id,
            call_id: self.call_id,
        }
    }
}

struct PendingHostUserInput {
    session_id: i64,
    response: oneshot::Sender<agena_plugin_host::sdk::host_api::AskUserResponse>,
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
    provider_registry: Arc<ProviderRegistry>,
    context_governor: ContextGovernor,
    processor: SessionProcessor,
    tool_executor: ToolExecutor,
    config: RuntimeSessionManagerConfig,
    tool_execution_semaphore: Arc<Semaphore>,
    shared_permission: Arc<StdRwLock<crate::authorization::PermissionConfig>>,
    shared_session_permissions:
        Arc<StdRwLock<HashMap<i64, crate::authorization::PermissionConfig>>>,
    auto_approval: Arc<StdMutex<HashMap<Option<i64>, agena_permission::DenialBudget>>>,
    rule_snapshots: Arc<StdRwLock<HashMap<Option<i64>, Arc<permission_service::RuleSnapshot>>>>,
    auto_projection: Arc<StdMutex<HashMap<Option<i64>, (usize, String)>>>,
}

/// Per-run collaborators that belong to one execution lifecycle.
///
/// This prevents the session loop API from growing a sequence of unrelated
/// arguments as new lifecycle concerns are introduced.
struct StableRunContext {
    base_run_source: ExecutionSource,
    active_model_turn_id: Option<i64>,
    state: Arc<SessionManagerState>,
    control: Arc<ExecutionControl>,
    steer_rx: mpsc::Receiver<Vec<TypedContent>>,
    usage_budget: Option<SubtaskUsageBudget>,
}

/// A bounded, child-session-relative usage limit. This is deliberately kept
/// inside the session manager rather than the task plugin so every model turn
/// checks it before a new provider request is allowed.
#[derive(Debug, Clone)]
struct SubtaskUsageBudget {
    baseline: agena_provider::CompletionUsage,
    max_tokens: Option<u64>,
    max_cost_microusd: Option<u64>,
}

impl SubtaskUsageBudget {
    fn new(
        baseline: agena_provider::CompletionUsage,
        max_tokens: Option<u64>,
        max_cost_microusd: Option<u64>,
    ) -> Option<Self> {
        (max_tokens.is_some() || max_cost_microusd.is_some()).then_some(Self {
            baseline,
            max_tokens,
            max_cost_microusd,
        })
    }

    fn exceeded_by(&self, aggregate: &agena_provider::CompletionUsage) -> Option<String> {
        let usage = aggregate.saturating_sub(&self.baseline);
        if let Some(max_tokens) = self.max_tokens
            && usage.total_tokens() > max_tokens
        {
            return Some(format!(
                "used {} total tokens, exceeding max_tokens={max_tokens}",
                usage.total_tokens()
            ));
        }
        if let Some(max_cost_microusd) = self.max_cost_microusd {
            let cost_microusd = usage_cost_microusd(usage.effective_cost_usd());
            if cost_microusd > max_cost_microusd {
                return Some(format!(
                    "used {cost_microusd} USD micro-units, exceeding max_cost_microusd={max_cost_microusd}"
                ));
            }
        }
        None
    }

    /// A completed model turn may exactly consume a limit. In that state no
    /// next provider request is permitted (rather than issuing one with a
    /// nonsensical zero output cap), even though the just-finished turn did
    /// not technically exceed its ceiling.
    fn prevents_next_model_turn(
        &self,
        aggregate: &agena_provider::CompletionUsage,
    ) -> Option<String> {
        if let Some(reason) = self.exceeded_by(aggregate) {
            return Some(reason);
        }
        let usage = aggregate.saturating_sub(&self.baseline);
        if let Some(max_tokens) = self.max_tokens
            && usage.total_tokens() >= max_tokens
        {
            return Some(format!(
                "used {} total tokens, reaching max_tokens={max_tokens}",
                usage.total_tokens()
            ));
        }
        if let Some(max_cost_microusd) = self.max_cost_microusd
            && usage_cost_microusd(usage.effective_cost_usd()) >= max_cost_microusd
        {
            return Some(format!(
                "used {} USD micro-units, reaching max_cost_microusd={max_cost_microusd}",
                usage_cost_microusd(usage.effective_cost_usd())
            ));
        }
        None
    }

    fn cap_output_tokens(
        &self,
        aggregate: &agena_provider::CompletionUsage,
        options: &mut SessionRunOptions,
    ) {
        let Some(max_tokens) = self.max_tokens else {
            return;
        };
        let remaining =
            max_tokens.saturating_sub(aggregate.saturating_sub(&self.baseline).total_tokens());
        let remaining = remaining.min(u64::from(u32::MAX)) as u32;
        options.max_output_tokens = Some(
            options
                .max_output_tokens
                .map_or(remaining, |existing| existing.min(remaining)),
        );
    }
}

fn usage_cost_microusd(cost_usd: f64) -> u64 {
    if !cost_usd.is_finite() || cost_usd <= 0.0 {
        return 0;
    }
    (cost_usd * 1_000_000.0).ceil().min(u64::MAX as f64) as u64
}

mod compact;
mod helpers;
mod history;
mod permission_service;
mod replies;
mod runs;
mod session_mutation;
mod session_prompt;
mod sessions;
mod stats;
#[cfg(test)]
mod tests;

use self::helpers::*;
use self::replies::operation_from_part;

impl SessionManagerState {
    fn new(
        provider_registry: Arc<ProviderRegistry>,
        context_governor: ContextGovernor,
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
        config: RuntimeSessionManagerConfig,
    ) -> Self {
        let shared_permission = Arc::new(StdRwLock::new(config.permission.clone()));
        Self::new_with_permission_stores(
            provider_registry,
            context_governor,
            processor,
            tool_executor,
            config,
            shared_permission,
            Arc::new(StdRwLock::new(HashMap::new())),
            Arc::new(StdMutex::new(HashMap::new())),
            Arc::new(StdRwLock::new(HashMap::new())),
            Arc::new(StdMutex::new(HashMap::new())),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_permission_stores(
        provider_registry: Arc<ProviderRegistry>,
        context_governor: ContextGovernor,
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
        config: RuntimeSessionManagerConfig,
        shared_permission: Arc<StdRwLock<crate::authorization::PermissionConfig>>,
        shared_session_permissions: Arc<
            StdRwLock<HashMap<i64, crate::authorization::PermissionConfig>>,
        >,
        auto_approval: Arc<StdMutex<HashMap<Option<i64>, agena_permission::DenialBudget>>>,
        rule_snapshots: Arc<StdRwLock<HashMap<Option<i64>, Arc<permission_service::RuleSnapshot>>>>,
        auto_projection: Arc<StdMutex<HashMap<Option<i64>, (usize, String)>>>,
    ) -> Self {
        let tool_execution_semaphore = Arc::new(Semaphore::new(config.max_concurrent_tools));
        Self {
            provider_registry,
            context_governor,
            processor,
            tool_executor,
            config,
            tool_execution_semaphore,
            shared_permission,
            shared_session_permissions,
            auto_approval,
            rule_snapshots,
            auto_projection,
        }
    }
}

/// Manager of sessions: creation, runs, replies, and lifecycle.
pub struct SessionManager {
    /// The v2 sealed data facade (14.1). All chat data — creation, runs,
    /// parts, state, exports — flows through this adapter; the manager never
    /// touches raw storage or leases (15.6).
    store: Arc<StoreAdapter>,
    /// Kept infrastructure (design 19.1): permission-rule persistence ports.
    permission_rules: Arc<dyn agena_storage::PermissionRuleRepository>,
    permission_rule_writer:
        Arc<dyn agena_storage::PermissionRuleTransactionWriter<sea_orm::DatabaseTransaction>>,
    /// Workspace identity resolution (kept infra, 19.1). Resolved lazily from
    /// the tool executor's workspace root so the manager stays backend-neutral.
    workspace_repository: Arc<dyn agena_storage::WorkspaceRepository>,
    workspace_id: tokio::sync::OnceCell<i64>,
    execution: ArcSwap<SessionManagerState>,
    execution_registry: Arc<ExecutionRegistry>,
    session_mutations: session_mutation::SessionMutationCoordinator,
    /// Sessions whose interrupted-run reconciliation has already run in this
    /// process. `get_session` reconciles a session once (plus its subagent
    /// children) so the per-session event scan is not repeated on every
    /// refresh; after that, per-run cleanup and per-load projection catch-up
    /// keep the session current.
    reconciled_sessions: Arc<Mutex<HashSet<i64>>>,
    host_user_input_waiters: Arc<Mutex<HashMap<String, PendingHostUserInput>>>,
    host_user_input_sequences: Arc<StdMutex<HashMap<HostUserInputSequenceKey, usize>>>,
}

/// Application-initiated session tools execute directly. This boundary is
/// deliberately outside the model-originated tool permission state machine.
///
#[async_trait::async_trait]
impl agena_runtime::SessionToolExecutionService for SessionManager {
    async fn execute_session_tool(
        &self,
        session_id: i64,
        invocation: ToolInvocation,
    ) -> Result<agena_runtime::SessionToolExecutionOutcome, agena_runtime::SessionToolExecutionError>
    {
        let session = self.get_session(session_id).await.map_err(|error| {
            agena_runtime::SessionToolExecutionError::Execution(error.to_string())
        })?;
        let state = self.execution_state();
        let cancellation = self.execution_registry.cancellation_token(session_id).await;
        let execution_context = session.runtime.execution.clone();
        let executor = state
            .tool_executor
            .for_session_context_async(&execution_context)
            .await
            .with_cancellation_token(cancellation);
        let prepared = async {
            let prepared = executor
                .prepare_invocation(&invocation, session_id, -1)
                .await?;
            let (invocation, prepared_shell_command) = executor
                .prepare_shell_invocation(&prepared.invocation, session_id, -1)
                .await?;
            executor
                .execute_invocation_detailed_with_prepared_shell(
                    &invocation,
                    session_id,
                    -1,
                    prepared_shell_command,
                )
                .await
        }
        .await;
        match prepared {
            Ok(execution) => Ok(agena_runtime::SessionToolExecutionOutcome::Completed(
                execution.summary(),
            )),
            Err(ToolError::CapabilityUnavailable(unavailable)) => {
                Ok(agena_runtime::SessionToolExecutionOutcome::CapabilityUnavailable(unavailable))
            }
            Err(ToolError::ToolUnavailable(unavailable)) => Ok(
                agena_runtime::SessionToolExecutionOutcome::ToolUnavailable(unavailable),
            ),
            Err(error) => Err(agena_runtime::SessionToolExecutionError::Execution(
                error.to_string(),
            )),
        }
    }
}

#[async_trait::async_trait]
impl agena_runtime::SessionPluginCommandService for SessionManager {
    async fn invoke_session_plugin_command(
        &self,
        request: agena_runtime::SessionPluginCommandRequest,
    ) -> Result<agena_plugin_host::sdk::PluginCommandOutput, agena_runtime::SessionPluginCommandError>
    {
        let session = self
            .get_session(request.session_id)
            .await
            .map_err(|error| {
                agena_runtime::SessionPluginCommandError::Execution(error.to_string())
            })?;
        let host = self.tool_executor().plugin_manager().clone();
        let plugin_id = request.plugin_id;
        let input = agena_plugin_host::sdk::PluginCommandInvokeInput {
            session_id: Some(session.id),
            call_id: None,
            workspace_root: request.workspace_root,
            command_id: request.command_id,
            slash: request.slash,
            raw: request.raw,
            input: request.input,
        };
        host.invoke_plugin_command_async(plugin_id.as_str(), input)
            .await
            .map_err(|error| agena_runtime::SessionPluginCommandError::Execution(error.to_string()))
    }
}

fn session_execution_command_error(error: AppError) -> agena_runtime::SessionExecutionCommandError {
    let failure = error.failure();
    tracing::error!(
        failure_id = %failure.id,
        diagnostic = %error,
        "session execution command rejected"
    );
    agena_runtime::SessionExecutionCommandError::from_failure(failure)
}

#[async_trait::async_trait]
impl agena_runtime::SessionExecutionCommandService for SessionManager {
    async fn create_session(
        &self,
        request: agena_runtime::SessionCreateRequest,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::create_session(self, request)
            .await
            .map_err(session_execution_command_error)?;
        Ok(agena_runtime::SessionExecutionCommandOutcome::completed(
            session.id,
        ))
    }

    async fn submit_user_run(
        &self,
        request: agena_runtime::SessionUserRunRequest,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let agena_runtime::SessionUserRunRequest {
            run,
            document,
            idempotency_key,
        } = request;
        let restore_document = document.clone();
        let parts = part_contents_from_composer_document(document)
            .map_err(session_execution_command_error)?;
        let user_idempotency_key = idempotency_key.clone();
        // The wire type already normalized the key (trim / non-empty) in
        // `SessionUserRunRequest::with_idempotency_key`; pass it through.
        let request = SessionUserRunRequest {
            run,
            parts,
            idempotency_key,
        };
        let session_id = request.run.session_id;
        let outcome = SessionManager::start_registered_with_restore(
            self,
            session_id,
            ExecutionSource::User,
            ExecutionConversationTarget::NewTurn,
            "user execution",
            Some(restore_document),
            user_idempotency_key,
            move |manager, control, steer_rx| async move {
                manager
                    .submit_user_run_inner(request, control, steer_rx, None)
                    .await
            },
        )
        .await
        .map_err(session_execution_command_error)?;
        Ok(outcome)
    }

    async fn steer_input(
        &self,
        session_id: i64,
        document: agena_domain::ComposerDocument,
    ) -> Result<(), agena_runtime::SessionExecutionCommandError> {
        let parts = part_contents_from_composer_document(document)
            .map_err(session_execution_command_error)?;
        SessionManager::steer_input(self, session_id, parts)
            .await
            .map_err(session_execution_command_error)
    }

    async fn continue_session(
        &self,
        request: agena_runtime::SessionExecutionRequest,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let outcome = SessionManager::start_continue_session(self, request)
            .await
            .map_err(session_execution_command_error)?;
        Ok(outcome)
    }

    async fn compact_session(
        &self,
        request: agena_runtime::SessionExecutionRequest,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let outcome = SessionManager::start_compact_session(self, request)
            .await
            .map_err(session_execution_command_error)?;
        Ok(outcome)
    }

    async fn rewind_session(
        &self,
        request: agena_runtime::SessionRewindRequest,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::rewind_session(self, request)
            .await
            .map_err(session_execution_command_error)?;
        Ok(agena_runtime::SessionExecutionCommandOutcome::completed(
            session.id,
        ))
    }

    async fn fork_session(
        &self,
        request: agena_runtime::SessionForkRequest,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::fork_session(self, request)
            .await
            .map_err(session_execution_command_error)?;
        Ok(agena_runtime::SessionExecutionCommandOutcome::completed(
            session.id,
        ))
    }

    async fn import_session_jsonl(
        &self,
        jsonl: &str,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::import_session_jsonl(self, jsonl)
            .await
            .map_err(session_execution_command_error)?;
        Ok(agena_runtime::SessionExecutionCommandOutcome::completed(
            session.id,
        ))
    }

    async fn reply_permission(
        &self,
        request: agena_runtime::SessionPermissionReplyRequest,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let outcome = SessionManager::start_reply_permission(self, request)
            .await
            .map_err(session_execution_command_error)?;
        Ok(outcome)
    }

    async fn reply_user_input(
        &self,
        request: agena_runtime::SessionExecutionReplyRequest<agena_domain::UserInputReply>,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let outcome = SessionManager::start_reply_user_input(self, request)
            .await
            .map_err(session_execution_command_error)?;
        Ok(outcome)
    }

    async fn mark_interactive_request_presented(
        &self,
        session_id: i64,
        request_id: String,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session =
            SessionManager::mark_interactive_request_presented(self, session_id, request_id)
                .await
                .map_err(session_execution_command_error)?;
        Ok(agena_runtime::SessionExecutionCommandOutcome::completed(
            session.id,
        ))
    }

    async fn update_session_selection(
        &self,
        session_id: i64,
        options: agena_runtime::SessionRunOptions,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::update_session_selection(self, session_id, options)
            .await
            .map_err(session_execution_command_error)?;
        Ok(agena_runtime::SessionExecutionCommandOutcome::completed(
            session.id,
        ))
    }

    async fn set_session_permission(
        &self,
        session_id: i64,
        permission: agena_domain::PermissionConfig,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::set_session_permission(self, session_id, permission)
            .await
            .map_err(session_execution_command_error)?;
        Ok(agena_runtime::SessionExecutionCommandOutcome::completed(
            session.id,
        ))
    }
}

fn part_contents_from_composer_document(
    document: agena_domain::ComposerDocument,
) -> Result<Vec<TypedContent>, AppError> {
    use agena_domain::{ActivityPayload, ComposerNode, ResourceKind, ResourceReference};

    document
        .0
        .into_iter()
        .map(|node| match node {
            ComposerNode::Text { text } => Ok(TypedContent::Text(TextContent {
                text,
                synthetic: false,
                extra: Default::default(),
            })),
            ComposerNode::Activity { activity } => match activity.payload {
                ActivityPayload::Resource(resource) => {
                    let kind = match resource.kind {
                        ResourceKind::Image => AttachmentKind::Image,
                        ResourceKind::Audio => AttachmentKind::Audio,
                        ResourceKind::Video => AttachmentKind::Video,
                        ResourceKind::Pdf => AttachmentKind::Pdf,
                        ResourceKind::File
                        | ResourceKind::Directory
                        | ResourceKind::Url
                        | ResourceKind::Artifact => AttachmentKind::File,
                    };
                    let source = match resource.reference {
                        ResourceReference::Artifact { uri, .. } => {
                            AttachmentSource::FileId { file_id: uri }
                        }
                        ResourceReference::WorkspacePath { path } => {
                            AttachmentSource::LocalPath { path }
                        }
                        ResourceReference::Url { url } => AttachmentSource::Url { url },
                        ResourceReference::ProviderFile { file_id, .. } => {
                            AttachmentSource::FileId { file_id }
                        }
                    };
                    Ok(TypedContent::FileRef(super::store::file_ref_from_attachment(
                        &crate::part::AttachmentPart {
                            attachments: vec![AttachmentItem {
                                kind,
                                mime: resource.media_type.unwrap_or_else(|| {
                                    if resource.kind == ResourceKind::Directory {
                                        "inode/directory".to_owned()
                                    } else {
                                        String::new()
                                    }
                                }),
                                source,
                                filename: Some(resource.name),
                                title: None,
                                size_bytes: resource.size_bytes,
                                sha256: None,
                                width: resource.width,
                                height: resource.height,
                                duration_ms: resource.duration_ms,
                                page_count: resource.page_count,
                            }],
                        },
                    )))
                }
                ActivityPayload::SkillReference(skill) => Ok(TypedContent::SkillRef(
                    super::store::skill_ref_from_reference(&crate::part::SkillReferencePart {
                        skills: vec![crate::part::SkillReference {
                            name: skill.name,
                            description: skill.description,
                            instructions: skill.instructions,
                            content_hash: skill.content_hash,
                            source: skill.source,
                            aliases: skill.aliases,
                        }],
                    }),
                )),
                ActivityPayload::TextArtifact(artifact) => Ok(TypedContent::Text(TextContent {
                    text: artifact.text,
                    synthetic: false,
                    extra: Default::default(),
                })),
                _ => Err(AppError::Config(
                    "turn input accepts only resource, skill_reference, and text_artifact activities"
                        .to_owned(),
                )),
            },
        })
        .collect()
}

impl SessionManager {
    /// Cooperative cancellation window before the lifecycle owner aborts an
    /// unresponsive operation task and performs registry cleanup itself.
    const OPERATION_CANCELLATION_GRACE: Duration = Duration::from_millis(500);

    /// How long a user-submit waits for a just-cancelled run to unregister
    /// before reporting `ExecutionAlreadyActive` (the interrupt-and-send
    /// race: the client submits the next turn as soon as cancellation is
    /// acknowledged, which can land before the cancelled run has finished
    /// unwinding and unregistered).
    const EXECUTION_CANCEL_UNREGISTER_GRACE: Duration = Duration::from_millis(2_000);

    async fn conversation_identity_for_execution(
        &self,
        session_id: i64,
        source: ExecutionSource,
        target: ExecutionConversationTarget,
    ) -> Result<ConversationIdentity, AppError> {
        match target {
            ExecutionConversationTarget::NewTurn => {
                if source != ExecutionSource::User {
                    return Err(AppError::Internal(format!(
                        "{source:?} execution cannot create a canonical user turn"
                    )));
                }
                return Ok(ConversationIdentity {
                    turn_id: agena_domain::TurnId::new(),
                    reply_id: agena_domain::AssistantReplyId::new(),
                });
            }
            ExecutionConversationTarget::ExistingReply(identity) => return Ok(identity),
            ExecutionConversationTarget::LatestReply => {}
        }

        if source == ExecutionSource::User {
            return Err(AppError::Internal(
                "user execution requires a new canonical turn".to_owned(),
            ));
        }

        // The canonical conversation identity is derived from the persisted
        // assistant run markers (design 19.5): the newest assistant marker
        // that carries the UUID pair it registered with.
        let session = self.store.load_session(session_id).await?;
        let identity = session
            .parts()
            .iter()
            .rev()
            .find_map(|marker| {
                if marker.kind != "run" || marker.role != PartRole::Assistant {
                    return None;
                }
                match (
                    marker
                        .content
                        .get("turn_id")
                        .and_then(serde_json::Value::as_str),
                    marker
                        .content
                        .get("reply_id")
                        .and_then(serde_json::Value::as_str),
                ) {
                    (Some(turn_id), Some(reply_id)) => {
                        let turn_id = uuid::Uuid::parse_str(turn_id).ok()?;
                        let reply_id = uuid::Uuid::parse_str(reply_id).ok()?;
                        Some(ConversationIdentity {
                            turn_id: agena_domain::TurnId(turn_id),
                            reply_id: agena_domain::AssistantReplyId(reply_id),
                        })
                    }
                    _ => None,
                }
            })
            .ok_or_else(|| {
                AppError::Config(format!(
                    "{source:?} requires an existing assistant reply in session {session_id}"
                ))
            })?;
        Ok(identity)
    }

    /// Resolve the canonical reply directly from the durable run marker that
    /// owns an interaction.
    ///
    /// A request is appended to an already-owned assistant message, and that
    /// message's marker carries the canonical UUID pair (design 19.5). The
    /// identity is therefore resolved from the loaded session — no raw
    /// event/turn/reply tables exist in v2.
    async fn conversation_identity_for_message(
        &self,
        session_id: i64,
        message_id: i64,
    ) -> Result<ConversationIdentity, AppError> {
        let session = self.store.load_session(session_id).await?;
        let marker = session
            .parts()
            .iter()
            .find(|part| part.part_id == message_id && part.kind == "run")
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "message {message_id} in session {session_id} has no canonical assistant reply identity"
                ))
            })?;
        let turn_id = marker
            .content
            .get("turn_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .map(agena_domain::TurnId)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "message {message_id} in session {session_id} has no canonical turn identity"
                ))
            })?;
        let reply_id = marker
            .content
            .get("reply_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .map(agena_domain::AssistantReplyId)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "message {message_id} in session {session_id} has no canonical reply identity"
                ))
            })?;
        Ok(ConversationIdentity { turn_id, reply_id })
    }

    async fn finish_execution(
        &self,
        _session_id: i64,
        control: &ExecutionControl,
        outcome: ExecutionOutcome,
    ) -> Result<(), AppError> {
        // The run marker's terminal state is written by the run's own persist
        // (complete_run / cancel_run); nothing else needs a durable lifecycle
        // event. The in-memory control lifecycle still transitions so
        // cancellation coordination and registry cleanup stay correct.
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
                Err(error) => {
                    let failure = error.failure();
                    tracing::error!(
                        failure_id = %failure.id,
                        diagnostic = %error,
                        "session execution failed"
                    );
                    ExecutionOutcome::Failed {
                        failure: failure.into(),
                    }
                }
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
        conversation_target: ExecutionConversationTarget,
        task_name: &'static str,
        operation: F,
    ) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(SessionManager, Arc<ExecutionControl>, mpsc::Receiver<Vec<TypedContent>>) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<T, AppError>> + Send + 'static,
    {
        let identity = self
            .conversation_identity_for_execution(session_id, source, conversation_target)
            .await?;
        let (control, steer_rx) = self
            .execution_registry
            .register(session_id, identity.turn_id, identity.reply_id)
            .await
            .map_err(execution_control_to_app_error)?;

        // There is intentionally no `.await` between successful registration
        // and spawning the lifecycle owner. Once a registry slot exists, its
        // owner therefore always exists as well, even if the calling request
        // is cancelled while awaiting the result.
        let manager = self.background_handle();
        let owner = tokio::spawn(async move {
            manager
                .drive_registered(session_id, task_name, control, steer_rx, operation)
                .await
        });
        owner.await.map_err(|error| {
            AppError::Internal(format!("{task_name} lifecycle owner task failed: {error}"))
        })?
    }

    /// Accept an execution and return its stable identity after
    /// `ExecutionStarted` is durable. The lifecycle owner continues in the
    /// background; provider/tool/cancellation outcomes are reported by
    /// terminal events and never retroactively fail the accepted command.
    async fn start_registered<T, F, Fut>(
        &self,
        session_id: i64,
        source: ExecutionSource,
        conversation_target: ExecutionConversationTarget,
        task_name: &'static str,
        operation: F,
    ) -> Result<crate::SessionExecutionCommandOutcome, AppError>
    where
        T: Send + 'static,
        F: FnOnce(SessionManager, Arc<ExecutionControl>, mpsc::Receiver<Vec<TypedContent>>) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<T, AppError>> + Send + 'static,
    {
        self.start_registered_with_restore(
            session_id,
            source,
            conversation_target,
            task_name,
            None,
            None,
            operation,
        )
        .await
    }

    async fn start_registered_with_restore<T, F, Fut>(
        &self,
        session_id: i64,
        source: ExecutionSource,
        conversation_target: ExecutionConversationTarget,
        task_name: &'static str,
        restore_document: Option<agena_domain::ComposerDocument>,
        user_idempotency_key: Option<String>,
        operation: F,
    ) -> Result<crate::SessionExecutionCommandOutcome, AppError>
    where
        T: Send + 'static,
        F: FnOnce(SessionManager, Arc<ExecutionControl>, mpsc::Receiver<Vec<TypedContent>>) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<T, AppError>> + Send + 'static,
    {
        let identity = self
            .conversation_identity_for_execution(session_id, source, conversation_target)
            .await?;
        // Interrupt-and-send race: the client submits the next user turn as
        // soon as cancellation is acknowledged, which can land before the
        // cancelled run has finished unwinding and unregistered. On the
        // user-submit path only, wait briefly for a cancelling run to release
        // the session; a live (non-cancelling) run still fails immediately
        // with `AlreadyActive`.
        if source == ExecutionSource::User {
            self.execution_registry
                .wait_until_cancelled_released(session_id, Self::EXECUTION_CANCEL_UNREGISTER_GRACE)
                .await
                .map_err(execution_control_to_app_error)?;
        }
        let (control, steer_rx) = self
            .execution_registry
            .register_with_restore(
                session_id,
                identity.turn_id,
                identity.reply_id,
                restore_document,
                user_idempotency_key,
            )
            .await
            .map_err(execution_control_to_app_error)?;
        let outcome = crate::SessionExecutionCommandOutcome::accepted(
            session_id,
            control.execution_id(),
            control.turn_id(),
            control.reply_id(),
        );
        let manager = self.background_handle();
        tokio::spawn(async move {
            if let Err(error) = manager
                .drive_registered(session_id, task_name, control, steer_rx, operation)
                .await
            {
                tracing::error!(
                    session_id,
                    task_name,
                    diagnostic = %error,
                    public_message = %error.public_message(),
                    "accepted execution finished with a failure"
                );
            }
        });
        Ok(outcome)
    }

    async fn drive_registered<T, F, Fut>(
        &self,
        session_id: i64,
        task_name: &'static str,
        control: Arc<ExecutionControl>,
        steer_rx: mpsc::Receiver<Vec<TypedContent>>,
        operation: F,
    ) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(SessionManager, Arc<ExecutionControl>, mpsc::Receiver<Vec<TypedContent>>) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<T, AppError>> + Send + 'static,
    {
        agena_runtime::session_started();
        let manager = self.background_handle();
        let task_control = Arc::clone(&control);
        let mut task = tokio::task::spawn(operation(manager, task_control, steer_rx));
        let joined = tokio::select! {
            biased;
            _ = control.cancel.cancelled() => {
                match tokio::time::timeout(Self::OPERATION_CANCELLATION_GRACE, &mut task).await {
                    Ok(joined) => joined,
                    Err(_) => {
                        task.abort();
                        task.await
                    }
                }
            }
            joined = &mut task => joined,
        };
        let result = match joined {
            Ok(result) => result,
            Err(error) if error.is_cancelled() && control.cancel.is_cancelled() => {
                Err(AppError::Cancelled)
            }
            Err(error) => Err(AppError::Internal(format!(
                "{task_name} task failed: {error}"
            ))),
        };
        agena_runtime::session_finished();

        // Terminalize the execution first: the run's own persist wrote the
        // marker's terminal state (complete_run / cancel_run). Reconcile
        // afterwards so any residual in-flight marker that survived cleanup
        // (for example a run whose persist was interrupted by a crash) is
        // aborted by the facade as `process_restart` (17.4 step 2c).
        let outcome = Self::execution_outcome(control.as_ref(), &result);
        let terminal_result = self
            .finish_execution(session_id, control.as_ref(), outcome)
            .await;
        let reconciliation_result = self.store.reconcile(session_id).await;
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
            permission_rules: Arc::clone(&self.permission_rules),
            permission_rule_writer: Arc::clone(&self.permission_rule_writer),
            workspace_repository: Arc::clone(&self.workspace_repository),
            workspace_id: tokio::sync::OnceCell::new(),
            execution: ArcSwap::from(self.execution.load_full()),
            execution_registry: Arc::clone(&self.execution_registry),
            session_mutations: self.session_mutations.clone(),
            reconciled_sessions: Arc::clone(&self.reconciled_sessions),
            host_user_input_waiters: Arc::clone(&self.host_user_input_waiters),
            host_user_input_sequences: Arc::clone(&self.host_user_input_sequences),
        }
    }

    pub fn new(
        db: sea_orm::DatabaseConnection,
        provider_registry: Arc<ProviderRegistry>,
        context_governor: ContextGovernor,
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
        config: RuntimeSessionManagerConfig,
    ) -> Self {
        let db_arc = Arc::new(db.clone());
        let engine = agena_storage_sqlite::SqliteEngine::new(Arc::clone(&db_arc));
        // One facade owns the lease and notification lifecycle for this
        // process; the manager talks to it exclusively through `StoreAdapter`
        // (14.2, 15.6). `owner_id` is the process identity stamped on every
        // facade write.
        let owner_id = uuid::Uuid::new_v4().to_string();
        let facade: Arc<dyn agena_storage::store::SessionStore> =
            Arc::new(agena_storage::store::SessionFacade::<
                agena_storage_sqlite::SqliteEngine,
            >::new(
                engine,
                owner_id.clone(),
                config.cache_policy().max_sessions,
            ));
        let store = Arc::new(StoreAdapter::new(
            facade,
            owner_id,
            Arc::new(|| Utc::now().timestamp_millis()),
        ));
        let permission_rules = Arc::new(agena_storage_sqlite::SeaPermissionRuleRepository::new(
            Arc::clone(&db_arc),
        ));
        let permission_rule_repository: Arc<dyn agena_storage::PermissionRuleRepository> =
            permission_rules.clone();
        let permission_rule_writer: Arc<
            dyn agena_storage::PermissionRuleTransactionWriter<sea_orm::DatabaseTransaction>,
        > = Arc::new(agena_storage_sqlite::SeaPermissionRuleTransactionWriter);
        // Workspace identity stays database-backed infra (design 19.1): the
        // facade's `NewSession.workspace_id` is derived from the tool
        // executor's workspace root, as in v1.
        let workspace_repository: Arc<dyn agena_storage::WorkspaceRepository> = Arc::new(
            agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::clone(&db_arc)),
        );
        let state = SessionManagerState::new(
            provider_registry,
            context_governor,
            processor,
            tool_executor,
            config,
        );
        Self {
            store,
            permission_rules: permission_rule_repository,
            permission_rule_writer,
            workspace_repository,
            workspace_id: tokio::sync::OnceCell::new(),
            execution: ArcSwap::from_pointee(state),
            execution_registry: Arc::new(ExecutionRegistry::new()),
            session_mutations: session_mutation::SessionMutationCoordinator::new(),
            reconciled_sessions: Arc::new(Mutex::new(HashSet::new())),
            host_user_input_waiters: Arc::new(Mutex::new(HashMap::new())),
            host_user_input_sequences: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    /// The workspace id owning this process's sessions, resolved once from the
    /// tool executor's workspace root. v2 keeps workspace identity as
    /// database-backed infra (19.1) because the facade requires it on every
    /// session create.
    pub(crate) async fn current_workspace_id(&self) -> Result<i64, AppError> {
        if let Some(workspace_id) = self.workspace_id.get() {
            return Ok(*workspace_id);
        }
        let workspace_root = self
            .execution_state()
            .tool_executor
            .workspace_root()
            .display()
            .to_string();
        let workspace_id = self
            .workspace_repository
            .ensure_id(&workspace_root)
            .await
            .map_err(|error| AppError::Internal(format!("resolve workspace id: {error}")))?;
        let _ = self.workspace_id.set(workspace_id);
        Ok(workspace_id)
    }

    pub fn tool_executor(&self) -> ToolExecutor {
        self.execution_state().tool_executor.clone()
    }

    /// The sealed v2 data facade (14.1). Live presentation consumers
    /// subscribe to [`SessionChange`](agena_storage::store::SessionChange)
    /// here instead of any event stream (14.3).
    pub fn session_store(&self) -> Arc<dyn agena_storage::store::SessionStore> {
        self.store.facade.clone()
    }

    /// Execute for a sessionless administrative surface. Administrative and
    /// plugin UI invocations are not model tool calls, so they do not enter
    /// the model permission state machine.
    pub async fn execute_unscoped_tool(
        &self,
        invocation: ToolInvocation,
        call_id: i64,
    ) -> Result<agena_runtime::SessionToolExecutionOutcome, AppError> {
        let executor = self.tool_executor();
        let execution = executor
            .execute_invocation_detailed_with_prepared_shell(&invocation, -1, call_id, None)
            .await
            .map_err(tool_error_to_app_error)?;
        Ok(agena_runtime::SessionToolExecutionOutcome::Completed(
            execution.summary(),
        ))
    }

    pub async fn request_host_user_input(
        &self,
        session_id: i64,
        call_id: i64,
        request: crate::part::AskUserToolInput,
    ) -> Result<agena_plugin_host::sdk::host_api::AskUserResponse, AppError> {
        let state = self.execution_state();
        let session = self.store.load_session(session_id).await?;
        let pending_tool = pending_tool_by_call_id(&session, call_id).ok_or_else(|| {
            AppError::Internal(format!(
                "pending tool not found for host user input: session={session_id}, call={call_id}"
            ))
        })?;
        let resolved_pending = resolve_pending_tool(&session, &pending_tool)?;
        let sequence_index = self.next_host_user_input_sequence(session_id, call_id);
        // Dedup re-entry on the pending tool's durable part id, not the
        // provider operation id: the operation id is `""` when the provider
        // streams no tool-call id, so keying on it would make host asks from
        // unrelated operations (e.g. `plan.review` and `interaction.ask`)
        // share one bucket and mismatch each other's questions. The tool part
        // id is unique per operation and recorded on every request, so it
        // matches exactly the requests this same tool call created.
        if let Some(existing) = replies::user_input_request_for_tool_part(
            &session,
            resolved_pending.pending.part.part_id,
            sequence_index,
        ) {
            if existing.request.questions != request.questions {
                return Err(AppError::Internal(format!(
                    "host user input request mismatch for tool part {} at step {}",
                    resolved_pending.pending.part.part_id, sequence_index
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
        // Every interactive request is bounded: a caller that omits
        // `auto_resolution_ms` gets the system default and any explicit value
        // is capped, so a host/plugin `ask_user` can never wedge the session
        // forever when no client replies.
        let auto_resolution_ms = effective_user_input_timeout_ms(request.auto_resolution_ms);
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
                UserInputSource::Host,
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
        // A host callback may execute the same pending operation that was
        // originally created by the model. Reuse that operation's correlation
        // ids so shell output remains attached to the visible Activity.
        let outer_pending_tool = pending_tool_by_call_id(&session, call_id);
        let resolved_outer_pending = outer_pending_tool
            .as_ref()
            .and_then(|pending| resolve_pending_tool(&session, pending).ok());
        let command_event_sink = resolved_outer_pending
            .as_ref()
            .map(|pending| self.command_event_sink_for_pending(session_id, pending));
        let launch_provenance = resolved_outer_pending
            .as_ref()
            .map(|pending| pending.scheduled_job_launch_provenance(session_id));
        let execution_context = session.runtime.execution.clone();
        let scoped_executor = state
            .tool_executor
            .for_session_context_async(&execution_context)
            .await
            .with_cancellation_token(cancellation.clone())
            .with_command_event_sink(command_event_sink);
        let prepared = scoped_executor
            .prepare_invocation(&invocation, session_id, call_id)
            .await
            .map_err(tool_error_to_app_error)?;
        let (invocation, prepared_shell_command) = scoped_executor
            .prepare_shell_invocation(&prepared.invocation, session_id, call_id)
            .await
            .map_err(tool_error_to_app_error)?;
        // Host/application callbacks are not model tool invocations and do
        // not participate in the model permission state machine.
        if let Some(mut stream) = scoped_executor
            .execute_invocation_streaming(&invocation, session_id, call_id)
            .await
            .map_err(tool_error_to_app_error)?
        {
            let stream_id = stream.stream_id.clone();
            // Streaming output is buffered in memory and written once (bounded)
            // at the end; the durable record never grows per-delta.
            let mut streamed_output = String::new();
            loop {
                let chunk = match cancellation.as_ref() {
                    Some(cancellation) => tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => return Err(AppError::Cancelled),
                        chunk = stream.chunks.recv() => chunk,
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
            }
            if !streamed_output.is_empty()
                && let Some(pending_tool) = outer_pending_tool.as_ref()
            {
                let preview =
                    agena_runtime_tools::truncate_tool_output_text(&streamed_output, 16 * 1024);
                self.apply_streaming_terminal_output(
                    session_id,
                    pending_tool,
                    preview.as_str(),
                    state.clone(),
                )
                .await?;
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

        scoped_executor
            .execute_invocation_detailed_with_launch_provenance(
                &invocation,
                session_id,
                call_id,
                prepared_shell_command,
                launch_provenance,
            )
            .await
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
        session_id: i64,
        request_id: String,
    ) -> oneshot::Receiver<agena_plugin_host::sdk::host_api::AskUserResponse> {
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
        mut response_rx: oneshot::Receiver<agena_plugin_host::sdk::host_api::AskUserResponse>,
    ) -> Result<agena_plugin_host::sdk::host_api::AskUserResponse, AppError> {
        let receive = |result: Result<
            agena_plugin_host::sdk::host_api::AskUserResponse,
            oneshot::error::RecvError,
        >| {
            result.map_err(|_| {
                AppError::Internal(format!(
                    "host user input waiter closed before reply: {request_id}"
                ))
            })
        };
        let Some(timeout_ms) = auto_resolution_ms else {
            // No auto-resolution deadline: keep waiting on the local oneshot,
            // but also poll the database so a reply from another process
            // (which cannot reach this process's oneshot) still wakes us.
            let mut poll = tokio::time::interval(Duration::from_millis(250));
            poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    biased;
                    result = &mut response_rx => return receive(result),
                    _ = poll.tick() => {
                        let _state = self.execution_state();
                        let session = self.store
                            .load_session(session_id)
                            .await?;
                        if replies::has_replied_user_input_request(&session, request_id) {
                            // A concurrent process persisted the reply. The
                            // answer content is durable in the event stream;
                            // returning a default response lets the caller
                            // resume without blocking forever.
                            return Ok(agena_plugin_host::sdk::host_api::AskUserResponse {
                                reply: String::new(),
                                cancelled: false,
                                timed_out: false,
                                answers: Default::default(),
                            });
                        }
                    }
                }
            }
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
                let session = self.store.load_session(session_id).await?;
                let options = self.run_options_from_session_async(&session, state).await?;
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

    async fn cancel_host_interactive_waiters(&self, session_id: i64) {
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
                .send(agena_plugin_host::sdk::host_api::AskUserResponse {
                    cancelled: true,
                    ..Default::default()
                });
        }
    }

    pub fn reconfigure(
        &self,
        provider_registry: Arc<ProviderRegistry>,
        context_governor: ContextGovernor,
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
        config: RuntimeSessionManagerConfig,
    ) {
        let previous = self.execution.load_full();
        if let Ok(mut permission) = previous.shared_permission.write() {
            *permission = config.permission.clone();
        }
        self.execution
            .store(Arc::new(SessionManagerState::new_with_permission_stores(
                provider_registry,
                context_governor,
                processor,
                tool_executor,
                config,
                Arc::clone(&previous.shared_permission),
                Arc::clone(&previous.shared_session_permissions),
                Arc::clone(&previous.auto_approval),
                Arc::clone(&previous.rule_snapshots),
                Arc::clone(&previous.auto_projection),
            )));
    }

    /// The v2 facade's [`MemoryLayer`](agena_storage::store::MemoryLayer) is
    /// LRU-bounded by `max_sessions` and self-evicts on insert (15.3), so
    /// there is nothing for the manager to prune. Kept for API compatibility
    /// with the runtime's maintenance loop.
    pub fn prune_cache(&self) {}

    /// Cache statistics are hidden inside the sealed facade (14.1): the
    /// manager only configured `max_sessions`. Report the stable default so
    /// the runtime surface stays unchanged.
    pub fn cache_stats(&self) -> agena_domain::SessionCacheStats {
        agena_domain::SessionCacheStats::default()
    }

    /// Settle one durable background-operation aggregate and enqueue its
    /// unique terminal delivery. AI-launched work appends the notification to
    /// the assistant run that launched it; work with no assistant launch run
    /// keeps an explicit Runtime ingress. The launch tool part remains the
    /// immutable, already-completed launch receipt.
    pub async fn settle_background_operation(
        &self,
        session_id: i64,
        kind: &str,
        id: &str,
        terminal: PartState,
        outcome: Result<String, Failure>,
        notification: SystemNotificationContent,
    ) -> Result<(), AppError> {
        let operation_kind = BackgroundOperationKind::parse(kind).ok_or_else(|| {
            AppError::Internal(format!("unsupported background operation kind {kind}"))
        })?;
        let operation = self
            .background_operation_for_signal(session_id, operation_kind, id)
            .await?;
        let next_phase = match (terminal, notification.status.as_str()) {
            (_, "timed_out") => BackgroundOperationPhase::TimedOut,
            (_, "interrupted") => BackgroundOperationPhase::Interrupted,
            (PartState::Completed, _) => BackgroundOperationPhase::Completed,
            (PartState::Cancelled, _) => BackgroundOperationPhase::Cancelled,
            _ => BackgroundOperationPhase::Failed,
        };
        let (outcome_value, failure_value) = match &outcome {
            Ok(text) => (Some(serde_json::json!({ "text": text })), None),
            Err(failure) => (
                None,
                Some(serde_json::json!({
                    "id": failure.id.to_string(),
                    "message": failure.user.fallback,
                })),
            ),
        };
        let notification_role = if operation.launch_run_id.is_some() {
            PartRole::Assistant
        } else {
            PartRole::Runtime
        };
        let new_part = new_part_from_content(
            "system_notification",
            notification_role,
            &TypedContent::SystemNotification(notification.clone()),
            PartState::Completed,
        )?;
        let settled = self
            .store
            .record_background_event(BackgroundEventRequest {
                operation_id: operation.operation_id,
                event_key: "terminal".to_owned(),
                event_seq: None,
                next_phase: Some(next_phase),
                outcome: outcome_value,
                failure: failure_value,
                notification: new_part,
            })
            .await?;
        self.dispatch_background_delivery(settled.delivery, notification)
            .await
    }

    /// Resolve a runtime callback through the durable external-id index. This
    /// is intentionally public to the runtime bridge so callback routing never
    /// depends on a process-local PartUpdated observer.
    pub async fn background_operation_owner_for_external(
        &self,
        kinds: &[BackgroundOperationKind],
        external_id: &str,
    ) -> Result<Option<(i64, BackgroundOperationKind)>, AppError> {
        for kind in kinds {
            if let Some(operation) = self
                .store
                .background_operation_by_external_id(*kind, external_id)
                .await?
            {
                return Ok(Some((operation.session_id, *kind)));
            }
        }
        Ok(None)
    }

    /// Terminalize a launch adapter failure without allowing a concurrent
    /// callback/event revision bump to strand the aggregate. A terminal
    /// callback always wins; otherwise stale optimistic revisions are retried
    /// from the durable row a small bounded number of times.
    async fn fail_background_launch_if_active(
        &self,
        operation_id: &str,
        next_phase: BackgroundOperationPhase,
        message: String,
    ) -> Result<(), AppError> {
        for attempt in 0..4 {
            let current = self
                .store
                .background_operation(operation_id)
                .await?
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "background operation {operation_id} disappeared during launch"
                    ))
                })?;
            if current.phase.is_terminal() {
                return Ok(());
            }
            let result = self
                .store
                .transition_background_operation(BackgroundOperationTransition {
                    operation_id: operation_id.to_owned(),
                    expected_revision: current.revision,
                    next_phase,
                    external_id: None,
                    outcome: None,
                    failure: Some(serde_json::json!({ "message": message })),
                    owner_id: None,
                    lease_until_ms: None,
                })
                .await;
            match result {
                Ok(_) => return Ok(()),
                Err(error) if attempt == 3 => return Err(error),
                Err(_) => continue,
            }
        }
        unreachable!("bounded launch-failure retry returns from the loop")
    }

    /// Convert the short launch lease into a renewable runtime ownership lease
    /// after the adapter returns its receipt. This lets another process
    /// distinguish live work from a restart orphan without consulting an
    /// in-memory registry. A very fast completion may already be terminal.
    async fn finish_background_launch_handoff(&self, operation_id: &str) -> Result<(), AppError> {
        for attempt in 0..4 {
            let current = self
                .store
                .background_operation(operation_id)
                .await?
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "background operation {operation_id} disappeared during handoff"
                    ))
                })?;
            if current.phase.is_terminal() {
                return Ok(());
            }
            let result = self
                .store
                .transition_background_operation(BackgroundOperationTransition {
                    operation_id: operation_id.to_owned(),
                    expected_revision: current.revision,
                    next_phase: current.phase,
                    external_id: None,
                    outcome: None,
                    failure: None,
                    owner_id: Some(self.store.background_owner_id().to_owned()),
                    lease_until_ms: Some(Utc::now().timestamp_millis() + 120_000),
                })
                .await;
            match result {
                Ok(_) => return Ok(()),
                Err(error) if attempt == 3 => return Err(error),
                Err(_) => continue,
            }
        }
        unreachable!("bounded launch-handoff retry returns from the loop")
    }

    /// Resolve a completion signal through the durable unique index. The
    /// legacy marker adoption arm is only for v8 rows created before the
    /// operation ledger existed; all new launches create the aggregate before
    /// executing their external side effect.
    async fn background_operation_for_signal(
        &self,
        session_id: i64,
        kind: BackgroundOperationKind,
        external_id: &str,
    ) -> Result<agena_storage::store::BackgroundOperation, AppError> {
        if let Some(operation) = self
            .store
            .background_operation_by_external_id(kind, external_id)
            .await?
        {
            if operation.session_id != session_id {
                return Err(AppError::Internal(format!(
                    "background signal {}:{} belongs to session {}, not {}",
                    kind.as_str(),
                    external_id,
                    operation.session_id,
                    session_id
                )));
            }
            return Ok(operation);
        }

        let session = self.store.load_session(session_id).await?;
        let part = session
            .parts()
            .iter()
            .find(|part| {
                part.kind == "tool_call"
                    && operation_from_part(part).is_some_and(|operation| {
                        operation.background_operation().is_some_and(|marker| {
                            marker.kind == kind.as_str() && marker.id == external_id
                        })
                    })
            })
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "background {}:{} has neither an aggregate nor a legacy marker in session {}",
                    kind.as_str(),
                    external_id,
                    session_id
                ))
            })?;
        let run_id = part.run_id.ok_or_else(|| {
            AppError::Internal(format!(
                "legacy background tool {} has no run",
                part.part_id
            ))
        })?;
        let operation_id = background_operation_id(session_id, part.part_id);
        let created = self
            .store
            .create_background_operation(NewBackgroundOperation {
                operation_id: operation_id.clone(),
                session_id,
                launch_run_id: Some(run_id),
                launch_tool_part_id: Some(part.part_id),
                kind,
            })
            .await?;
        let launching = if created.phase == BackgroundOperationPhase::LaunchRequested {
            self.store
                .transition_background_operation(BackgroundOperationTransition {
                    operation_id: operation_id.clone(),
                    expected_revision: created.revision,
                    next_phase: BackgroundOperationPhase::Launching,
                    external_id: None,
                    outcome: None,
                    failure: None,
                    owner_id: Some("legacy-adoption".to_owned()),
                    lease_until_ms: Some(Utc::now().timestamp_millis() + 30_000),
                })
                .await?
        } else {
            created
        };
        if launching.phase == BackgroundOperationPhase::Launching {
            self.store
                .transition_background_operation(BackgroundOperationTransition {
                    operation_id,
                    expected_revision: launching.revision,
                    next_phase: BackgroundOperationPhase::Running,
                    external_id: Some(external_id.to_owned()),
                    outcome: None,
                    failure: None,
                    owner_id: None,
                    lease_until_ms: None,
                })
                .await
        } else {
            Ok(launching)
        }
    }

    /// Record one sequenced monitor event on the same durable aggregate. The
    /// monitor remains Running; only its event cursor and unique delivery
    /// advance.
    pub async fn settle_background_event(
        &self,
        session_id: i64,
        kind: &str,
        id: &str,
        event_seq: u64,
        notification: SystemNotificationContent,
    ) -> Result<(), AppError> {
        let operation_kind = BackgroundOperationKind::parse(kind).ok_or_else(|| {
            AppError::Internal(format!("unsupported background operation kind {kind}"))
        })?;
        let operation = self
            .background_operation_for_signal(session_id, operation_kind, id)
            .await?;
        let notification_role = if operation.launch_run_id.is_some() {
            PartRole::Assistant
        } else {
            PartRole::Runtime
        };
        let new_part = new_part_from_content(
            "system_notification",
            notification_role,
            &TypedContent::SystemNotification(notification.clone()),
            PartState::Completed,
        )?;
        let settled = self
            .store
            .record_background_event(BackgroundEventRequest {
                operation_id: operation.operation_id,
                event_key: format!("event:{event_seq}"),
                event_seq: Some(event_seq),
                next_phase: None,
                outcome: None,
                failure: None,
                notification: new_part,
            })
            .await?;
        self.dispatch_background_delivery(settled.delivery, notification)
            .await
    }

    /// Deliver one scheduled job through the same durable aggregate and outbox
    /// used by every other background operation.
    ///
    /// Assistant-created schedules retain their exact launch run/tool part and
    /// append every fire to that run. Host-created schedules have no assistant
    /// provenance and use an explicit Runtime ingress. Replaying the same
    /// delivery key is idempotent all the way through the operation,
    /// transcript projection, and model-wake delivery.
    pub async fn deliver_scheduled_job(
        &self,
        session_id: i64,
        job_id: String,
        delivery_key: String,
        prompt: String,
        launch_provenance: Option<agena_scheduler::ScheduledJobLaunchProvenance>,
    ) -> Result<bool, AppError> {
        let (launch_run_id, launch_tool_part_id) = match launch_provenance {
            Some(provenance) if provenance.session_id == session_id => {
                (Some(provenance.run_id), Some(provenance.tool_part_id))
            }
            Some(provenance) => {
                return Err(AppError::Internal(format!(
                    "scheduled job {job_id} belongs to session {}, not {session_id}",
                    provenance.session_id
                )));
            }
            None => (None, None),
        };
        let operation_id = format!("scheduled:{session_id}:{delivery_key}");
        let notification = SystemNotificationContent {
            operation_id: delivery_key.clone(),
            operation_kind: "scheduled_delivery".to_string(),
            status: "submitted".to_string(),
            summary: format!("Scheduled job {job_id} fired"),
            body: prompt.clone(),
            ..Default::default()
        };
        let mut operation = self
            .store
            .create_background_operation(NewBackgroundOperation {
                operation_id: operation_id.clone(),
                session_id,
                launch_run_id,
                launch_tool_part_id,
                kind: BackgroundOperationKind::ScheduledDelivery,
            })
            .await?;
        // Multiple scheduler workers may deliver the same key concurrently.
        // Advance the idempotent aggregate with optimistic retry rather than
        // leaking a harmless revision race as a failed scheduled job.
        for attempt in 0..8 {
            let transition = match operation.phase {
                BackgroundOperationPhase::LaunchRequested => Some(BackgroundOperationTransition {
                    operation_id: operation_id.clone(),
                    expected_revision: operation.revision,
                    next_phase: BackgroundOperationPhase::Launching,
                    external_id: None,
                    outcome: None,
                    failure: None,
                    owner_id: Some(format!("scheduled-delivery:{job_id}")),
                    lease_until_ms: Some(Utc::now().timestamp_millis() + 30_000),
                }),
                BackgroundOperationPhase::Launching => Some(BackgroundOperationTransition {
                    operation_id: operation_id.clone(),
                    expected_revision: operation.revision,
                    next_phase: BackgroundOperationPhase::Running,
                    external_id: Some(delivery_key.clone()),
                    outcome: None,
                    failure: None,
                    owner_id: None,
                    lease_until_ms: None,
                }),
                _ => None,
            };
            let Some(transition) = transition else {
                break;
            };
            match self.store.transition_background_operation(transition).await {
                Ok(updated) => operation = updated,
                Err(error) if attempt == 7 => return Err(error),
                Err(_) => {
                    operation = self
                        .store
                        .background_operation(&operation_id)
                        .await?
                        .ok_or_else(|| {
                            AppError::Internal(format!(
                                "scheduled background operation {operation_id} disappeared"
                            ))
                        })?;
                }
            }
        }
        let new_part = new_part_from_content(
            "system_notification",
            if launch_run_id.is_some() {
                PartRole::Assistant
            } else {
                PartRole::Runtime
            },
            &TypedContent::SystemNotification(notification.clone()),
            PartState::Completed,
        )?;
        let settled = self
            .store
            .record_background_event(BackgroundEventRequest {
                operation_id,
                event_key: "terminal".to_owned(),
                event_seq: None,
                next_phase: Some(BackgroundOperationPhase::Completed),
                outcome: Some(serde_json::json!({ "job_id": job_id })),
                failure: None,
                notification: new_part,
            })
            .await?;
        self.dispatch_background_delivery(settled.delivery, notification)
            .await?;
        Ok(settled.created)
    }

    /// Claim and deliver one persisted notification. Retryable failures release
    /// the claim back to Pending with durable exponential backoff; permanent or
    /// exhausted failures enter the Failed terminal state. Success marks it
    /// Consumed only after the wake execution returns.
    ///
    /// A completed provider round whose marker records this notification in
    /// `input_notification_part_ids` is durable proof that an earlier
    /// dispatcher finished the wake but crashed before consuming the outbox
    /// row. Recovery consumes that row without invoking the model again,
    /// closing the classic response-commit / outbox-ack crash window.
    async fn dispatch_background_delivery(
        &self,
        delivery: BackgroundDelivery,
        notification: SystemNotificationContent,
    ) -> Result<(), AppError> {
        const DELIVERY_CLAIM_MS: i64 = 15 * 60 * 1_000;
        let now_ms = Utc::now().timestamp_millis();
        let Some(claimed) = self
            .store
            .claim_background_delivery(&delivery.delivery_id, now_ms + DELIVERY_CLAIM_MS)
            .await?
        else {
            // Another live dispatcher owns it, or it is already consumed.
            return Ok(());
        };
        if claimed.attempts > MAX_BACKGROUND_DELIVERY_ATTEMPTS {
            self.store
                .fail_background_delivery(
                    &claimed.delivery_id,
                    serde_json::json!({
                        "category": "delivery_retry_exhausted",
                        "message": "background notification delivery exceeded its retry budget",
                        "attempts": claimed.attempts,
                    }),
                )
                .await?;
            tracing::warn!(
                target: "agena_background",
                delivery_id = %claimed.delivery_id,
                attempts = claimed.attempts,
                "background notification delivery exhausted its retry budget before wake"
            );
            return Ok(());
        }
        match self
            .notification_has_completed_assistant_response(
                claimed.session_id,
                claimed.notification_part_id,
            )
            .await
        {
            Ok(true) => {
                self.store
                    .consume_background_delivery(&claimed.delivery_id)
                    .await?;
                return Ok(());
            }
            Ok(false) => {}
            Err(error) => {
                self.store
                    .fail_background_delivery(
                        &claimed.delivery_id,
                        serde_json::json!({
                            "category": "invalid_delivery",
                            "message": error.to_string(),
                            "public_message": "The background notification could not be delivered.",
                        }),
                    )
                    .await?;
                tracing::warn!(
                    target: "agena_background",
                    delivery_id = %claimed.delivery_id,
                    error = %error,
                    "background notification delivery was invalid and was terminalized"
                );
                return Ok(());
            }
        }
        let delivered = self
            .wake_after_notification(
                claimed.session_id,
                claimed.notification_part_id,
                notification,
            )
            .await;
        match delivered {
            Ok(()) => {
                self.store
                    .consume_background_delivery(&claimed.delivery_id)
                    .await?;
                Ok(())
            }
            Err(error) => {
                let diagnostic = background_delivery_error(&error);
                let retryable = background_delivery_should_retry(&error)
                    && claimed.attempts < MAX_BACKGROUND_DELIVERY_ATTEMPTS;
                if retryable {
                    let now_ms = Utc::now().timestamp_millis();
                    let next_attempt_at_ms =
                        now_ms.saturating_add(background_delivery_retry_delay_ms(claimed.attempts));
                    if let Err(storage_error) = self
                        .store
                        .retry_background_delivery(
                            &claimed.delivery_id,
                            diagnostic,
                            next_attempt_at_ms,
                        )
                        .await
                    {
                        tracing::error!(
                            target: "agena_background",
                            delivery_id = %claimed.delivery_id,
                            error = %storage_error,
                            "failed to persist background delivery retry state"
                        );
                    }
                    Err(error)
                } else {
                    self.store
                        .fail_background_delivery(&claimed.delivery_id, diagnostic)
                        .await?;
                    tracing::warn!(
                        target: "agena_background",
                        delivery_id = %claimed.delivery_id,
                        attempts = claimed.attempts,
                        retryable = background_delivery_should_retry(&error),
                        "background notification delivery reached a terminal failure"
                    );
                    // The delivery itself has been handled durably. Do not
                    // return the provider error to the operation reconciler,
                    // otherwise a non-retryable provider failure would be
                    // mistaken for a delivery transaction failure and retried
                    // by another outer layer.
                    Ok(())
                }
            }
        }
    }

    /// Whether the append-only transcript proves that `notification_part_id`
    /// has already received a completed model response.
    ///
    /// Each successful provider round durably lists the notification part ids
    /// present in its actual prompt. This exact input receipt avoids the race
    /// where an older provider request emits output after a notification was
    /// appended but never saw that notification. Legacy Runtime ingress rows
    /// retain the monotonic-id fallback used before round input receipts were
    /// introduced. Delivery progress remains derived from the canonical
    /// transcript instead of adding another mutable `notified` flag.
    async fn notification_has_completed_assistant_response(
        &self,
        session_id: i64,
        notification_part_id: Option<i64>,
    ) -> Result<bool, AppError> {
        let Some(notification_part_id) = notification_part_id else {
            return Ok(false);
        };
        let session = self.store.load_session(session_id).await?;
        let notification = session.parts().iter().find(|part| {
            part.part_id == notification_part_id
                && part.kind == "system_notification"
                && part.state == PartState::Completed
        });
        let Some(notification) = notification else {
            return Err(AppError::Internal(format!(
                "background delivery notification part {notification_part_id} is missing or invalid in session {session_id}"
            )));
        };
        let notification_identity_is_valid = match notification.role {
            PartRole::Assistant => notification.run_id.is_some_and(|run_id| {
                session.parts().iter().any(|marker| {
                    marker.part_id == run_id
                        && marker.is_run_marker()
                        && marker.role == PartRole::Assistant
                })
            }),
            PartRole::Runtime => notification.run_id.is_some_and(|run_id| {
                session.parts().iter().any(|marker| {
                    marker.part_id == run_id
                        && marker.is_run_marker()
                        && marker.role == PartRole::Runtime
                })
            }),
            _ => false,
        };
        if !notification_identity_is_valid {
            return Err(AppError::Internal(format!(
                "background delivery notification part {notification_part_id} has an invalid owner in session {session_id}"
            )));
        }
        let exact_round_receipt =
            crate::session::prompt_window::notification_has_completed_provider_round(
                session.parts(),
                notification_part_id,
            );
        if exact_round_receipt {
            return Ok(true);
        }
        // Backward compatibility for v9 Runtime ingress deliveries committed
        // before exact provider-round input receipts existed. New
        // Assistant-owned notifications never take this fallback because a
        // pre-existing in-flight assistant round can have a lower marker id
        // while emitting output after the notification without seeing it.
        Ok(notification.role == PartRole::Runtime
            && session.parts().iter().any(|part| {
                part.part_id > notification_part_id
                    && part.is_run_marker()
                    && part.role == PartRole::Assistant
                    && part.state == PartState::Completed
                    && part
                        .content
                        .get("run_kind")
                        .and_then(serde_json::Value::as_str)
                        == Some("continue")
            }))
    }

    /// Restart-safe dispatcher backstop. The live bus only lowers latency;
    /// this durable scan is what guarantees a notification committed before a
    /// crash is eventually presented after restart.
    pub async fn recover_background_deliveries(&self, limit: usize) -> Result<usize, AppError> {
        let pending = self.store.pending_background_deliveries(limit).await?;
        let mut recovered = 0usize;
        for delivery in pending {
            let notification =
                serde_json::from_value::<SystemNotificationContent>(delivery.payload.clone())
                    .map_err(|error| {
                        AppError::Internal(format!(
                            "decode background delivery {}: {error}",
                            delivery.delivery_id
                        ))
                    })?;
            match self
                .dispatch_background_delivery(delivery.clone(), notification)
                .await
            {
                Ok(()) => recovered += 1,
                Err(error) => tracing::warn!(
                    target: "agena_background",
                    delivery_id = %delivery.delivery_id,
                    %error,
                    "durable background delivery retry failed"
                ),
            }
        }
        Ok(recovered)
    }

    /// Renew ownership only for work this runtime can prove is still live.
    /// The durable lease is a cross-process liveness hint, never the source of
    /// lifecycle truth; failure to renew merely makes the operation eligible
    /// for reconciliation after the bounded lease window.
    pub async fn renew_background_operation_leases(&self, limit: usize) -> Result<usize, AppError> {
        let operations = self.store.active_background_operations(None, limit).await?;
        let process_summaries = self
            .execution_state()
            .tool_executor
            .monitor_registry()
            .map(|registry| registry.list())
            .unwrap_or_default();
        let mut renewed = 0usize;
        for operation in operations {
            if operation.owner_id.as_deref() != Some(self.store.background_owner_id())
                || operation.phase != BackgroundOperationPhase::Running
            {
                continue;
            }
            let live = match operation.kind {
                BackgroundOperationKind::Shell | BackgroundOperationKind::Monitor => {
                    operation.external_id.as_deref().is_some_and(|external_id| {
                        process_summaries.iter().any(|summary| {
                            summary.process_id == external_id
                                && summary.status == ProcessStatus::Running
                        })
                    })
                }
                BackgroundOperationKind::Task => {
                    let Some(task_id) = operation.external_id.as_deref() else {
                        continue;
                    };
                    let Some(child_id) = self
                        .store
                        .find_subagent_by_task_id(operation.session_id, task_id)
                        .await?
                    else {
                        continue;
                    };
                    self.execution_registry.is_active(child_id).await
                }
                BackgroundOperationKind::ScheduledDelivery => false,
            };
            if !live {
                continue;
            }
            match self
                .store
                .transition_background_operation(BackgroundOperationTransition {
                    operation_id: operation.operation_id,
                    expected_revision: operation.revision,
                    next_phase: BackgroundOperationPhase::Running,
                    external_id: None,
                    outcome: None,
                    failure: None,
                    owner_id: Some(self.store.background_owner_id().to_owned()),
                    lease_until_ms: Some(Utc::now().timestamp_millis() + 120_000),
                })
                .await
            {
                Ok(_) => renewed += 1,
                // An event or completion may have advanced the revision after
                // the scan. The next maintenance tick re-evaluates it; never
                // overwrite that newer state.
                Err(error) => tracing::debug!(
                    target: "agena_background",
                    %error,
                    "background lease renewal lost a concurrent transition"
                ),
            }
        }
        Ok(renewed)
    }

    /// Reconcile shell/monitor operations whose owning runtime disappeared or
    /// whose terminal callback was lost. A live lease owned by another
    /// process wins; after expiry the local process registry is checked and an
    /// absent process becomes an explicit Interrupted notification.
    pub async fn reconcile_background_processes(&self, limit: usize) -> Result<usize, AppError> {
        let mut operations = self
            .store
            .active_background_operations(Some(BackgroundOperationKind::Shell), limit)
            .await?;
        if operations.len() < limit {
            operations.extend(
                self.store
                    .active_background_operations(
                        Some(BackgroundOperationKind::Monitor),
                        limit - operations.len(),
                    )
                    .await?,
            );
        }
        let summaries = self
            .execution_state()
            .tool_executor
            .monitor_registry()
            .map(|registry| registry.list())
            .unwrap_or_default();
        let now_ms = Utc::now().timestamp_millis();
        let mut reconciled = 0usize;
        for operation in operations {
            if operation.lease_until_ms.is_some_and(|until| until > now_ms) {
                continue;
            }
            let Some(external_id) = operation.external_id.clone() else {
                self.record_interrupted_background_operation(
                    operation,
                    "launch ended before an external identity was bound".to_owned(),
                )
                .await?;
                reconciled += 1;
                continue;
            };
            let summary = summaries
                .iter()
                .find(|summary| summary.process_id == external_id);
            match summary.map(|summary| summary.status) {
                Some(ProcessStatus::Running) => {
                    // The local registry proves the process is alive; adopt
                    // ownership if its previous lease merely expired.
                    self.finish_background_launch_handoff(&operation.operation_id)
                        .await?;
                }
                Some(status) => {
                    let notification_status = match status {
                        ProcessStatus::Exited => "completed",
                        ProcessStatus::TimedOut => "timed_out",
                        ProcessStatus::Stopped => "cancelled",
                        ProcessStatus::Failed => "failed",
                        ProcessStatus::Running => unreachable!(),
                    };
                    let text = format!(
                        "Background process {external_id} ended with status {:?}",
                        status
                    );
                    let outcome = if status == ProcessStatus::Exited {
                        Ok(text.clone())
                    } else {
                        Err(AppError::Internal(text.clone()).failure())
                    };
                    self.settle_background_operation(
                        operation.session_id,
                        operation.kind.as_str(),
                        &external_id,
                        if status == ProcessStatus::Exited {
                            PartState::Completed
                        } else if status == ProcessStatus::Stopped {
                            PartState::Cancelled
                        } else {
                            PartState::Failed
                        },
                        outcome,
                        SystemNotificationContent {
                            operation_id: external_id.clone(),
                            operation_kind: operation.kind.as_str().to_owned(),
                            status: notification_status.to_owned(),
                            summary: text.clone(),
                            body: text,
                            ..Default::default()
                        },
                    )
                    .await?;
                    reconciled += 1;
                }
                None => {
                    self.record_interrupted_background_operation(
                        operation,
                        format!(
                            "background process {external_id} is absent after its owner lease expired"
                        ),
                    )
                    .await?;
                    reconciled += 1;
                }
            }
        }
        Ok(reconciled)
    }

    async fn record_interrupted_background_operation(
        &self,
        operation: agena_storage::store::BackgroundOperation,
        reason: String,
    ) -> Result<(), AppError> {
        let external_id = operation
            .external_id
            .clone()
            .unwrap_or_else(|| operation.operation_id.clone());
        let notification = SystemNotificationContent {
            operation_id: external_id,
            operation_kind: operation.kind.as_str().to_owned(),
            status: "interrupted".to_owned(),
            summary: reason.clone(),
            body: reason.clone(),
            ..Default::default()
        };
        let notification_role = if operation.launch_run_id.is_some() {
            PartRole::Assistant
        } else {
            PartRole::Runtime
        };
        let notification_part = new_part_from_content(
            "system_notification",
            notification_role,
            &TypedContent::SystemNotification(notification.clone()),
            PartState::Completed,
        )?;
        let settled = self
            .store
            .record_background_event(BackgroundEventRequest {
                operation_id: operation.operation_id,
                event_key: "terminal".to_owned(),
                event_seq: None,
                next_phase: Some(BackgroundOperationPhase::Interrupted),
                outcome: None,
                failure: Some(serde_json::json!({ "message": reason })),
                notification: notification_part,
            })
            .await?;
        self.dispatch_background_delivery(settled.delivery, notification)
            .await
    }

    /// Reconcile terminal child sessions against active task aggregates.
    ///
    /// The facade observer still lowers notification latency, but it is not a
    /// correctness dependency: after a dropped bus event or process restart,
    /// this scan reads both sides from durable storage and records the same
    /// idempotent terminal event/outbox row.
    pub async fn reconcile_background_tasks(&self, limit: usize) -> Result<usize, AppError> {
        let operations = self
            .store
            .active_background_operations(Some(BackgroundOperationKind::Task), limit)
            .await?;
        let mut reconciled = 0usize;
        for operation in operations {
            let Some(task_id) = operation.external_id.as_deref() else {
                continue;
            };
            let Some(child_id) = self
                .store
                .find_subagent_by_task_id(operation.session_id, task_id)
                .await?
            else {
                continue;
            };
            let child = self.store.load_session(child_id).await?;
            if child.runtime.subtask.status == agena_domain::SubtaskStatus::Running {
                let launch_is_live = operation
                    .lease_until_ms
                    .is_some_and(|until| until > Utc::now().timestamp_millis());
                if launch_is_live {
                    // The task tool published Running immediately before
                    // execute_registered installs the child lease. The
                    // operation's short launch lease closes that handoff race.
                    continue;
                }
                self.reconcile_interrupted_session(child_id).await?;
            }
            let child = self.store.load_session(child_id).await?;
            let status = child.runtime.subtask.status;
            if !status.is_terminal() {
                continue;
            }
            let (terminal, notification_status) = match status {
                agena_domain::SubtaskStatus::Completed => (PartState::Completed, "completed"),
                agena_domain::SubtaskStatus::Failed => (PartState::Failed, "failed"),
                agena_domain::SubtaskStatus::Cancelled => (PartState::Cancelled, "cancelled"),
                agena_domain::SubtaskStatus::TimedOut => (PartState::Failed, "timed_out"),
                agena_domain::SubtaskStatus::Interrupted => (PartState::Cancelled, "interrupted"),
                agena_domain::SubtaskStatus::Created | agena_domain::SubtaskStatus::Running => {
                    continue;
                }
            };
            let title = child.title.trim();
            let task_label = if title.is_empty() {
                task_id.to_owned()
            } else {
                format!("\"{title}\" ({task_id})")
            };
            let final_text = child
                .parts()
                .iter()
                .rev()
                .find_map(|part| {
                    part.content
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                        .or_else(|| part.summary.clone())
                })
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| "Task completed.".to_owned());
            let outcome = if status == agena_domain::SubtaskStatus::Completed {
                Ok(final_text)
            } else {
                Err(child.runtime.subtask.failure.clone().unwrap_or_else(|| {
                    if status == agena_domain::SubtaskStatus::Cancelled {
                        AppError::Cancelled.failure()
                    } else {
                        AppError::Internal(format!(
                            "delegated task {task_id} ended with status {}",
                            status.as_ref()
                        ))
                        .failure()
                    }
                }))
            };
            let summary = match &outcome {
                Ok(_) => format!("Task {task_label} finished"),
                Err(failure) => {
                    let reason = failure.user.fallback.trim().trim_end_matches('.');
                    let verb = match status {
                        agena_domain::SubtaskStatus::Cancelled
                        | agena_domain::SubtaskStatus::Interrupted => "cancelled",
                        agena_domain::SubtaskStatus::TimedOut => "timed out",
                        _ => "failed",
                    };
                    if reason.is_empty() {
                        format!("Task {task_label} {verb}")
                    } else {
                        format!("Task {task_label} {verb}: {reason}")
                    }
                }
            };
            let notification = SystemNotificationContent {
                operation_id: task_id.to_owned(),
                operation_kind: "task".to_owned(),
                status: notification_status.to_owned(),
                summary,
                body: match &outcome {
                    Ok(text) => format!("<result>{text}</result>"),
                    Err(failure) => failure.user.fallback.clone(),
                },
                ..Default::default()
            };
            self.settle_background_operation(
                operation.session_id,
                "task",
                task_id,
                terminal,
                outcome,
                notification,
            )
            .await?;
            reconciled += 1;
        }
        Ok(reconciled)
    }

    /// Wake the model over a freshly-settled notification (a background
    /// completion, a monitor event, or a scheduled delivery), shared by the
    /// three settle paths. The notification part was atomically committed to
    /// its assistant launch run or a launch-less Runtime ingress; this runs
    /// outside that transaction.
    ///
    /// The mid-turn arm steers the active execution and then **verifies the
    /// steer landed**: the stable-run loop acknowledges the notification part
    /// the moment its cursor observes it ([`ExecutionRegistry::ack_notification`]),
    /// so the settle awaits that acknowledgment — or, if the execution exits
    /// without observing the part (the launch turn was already in its final
    /// stop path when the steer was sent), falls back to a fresh idle wake
    /// execution. Without the handshake a steer dropped at the end of a turn
    /// was silently lost: the notification part existed, but the model was
    /// never woken and the session went quiet.
    async fn wake_after_notification(
        &self,
        session_id: i64,
        notification_part_id: Option<i64>,
        notification: SystemNotificationContent,
    ) -> Result<(), AppError> {
        if !self.execution_registry.is_active(session_id).await {
            // Idle: start a fresh execution that picks the appended
            // notification up as its first input.
            self.execute_registered(
                session_id,
                ExecutionSource::User,
                ExecutionConversationTarget::NewTurn,
                "notification execution",
                move |manager, control, steer_rx| async move {
                    manager
                        .notification_run_inner(session_id, control, steer_rx)
                        .await
                },
            )
            .await?;
            return Ok(());
        }
        // Mid-turn: queue the notification for the running execution. The
        // steer receiver is drained only after the current provider/tool part
        // reaches the stable-loop boundary; it never interrupts a streaming
        // part. `drain_steer_input` then reloads the already-persisted hook and
        // notification detection requests the next provider round. Register
        // the delivery handshake first so the loop can acknowledge the part.
        let ack_rx = notification_part_id.map(|part_id| {
            self.execution_registry
                .register_notification_ack(session_id, part_id)
        });
        if let Err(error) = self
            .steer_input(
                session_id,
                vec![TypedContent::SystemNotification(notification.clone())],
            )
            .await
        {
            // The receiver was already dropped: the execution's final drain
            // passed and it is mid-unwind (still registered). Wait for it to
            // release, then a fresh execution picks the appended notification
            // up as its first input.
            tracing::debug!(
                target: "agena_background",
                %session_id, operation_kind = %notification.operation_kind,
                operation_id = %notification.operation_id, %error,
                "notification steer missed the active execution; waking idle"
            );
            self.execution_registry
                .wait_until_released(session_id)
                .await;
            self.wake_idle_notification(session_id).await?;
            return Ok(());
        }
        // The steer was queued for a live execution. Verify it lands: the loop
        // acknowledges the appended part, or it exits without observing it
        // (its final drain already passed) and a fresh wake takes over.
        let Some(ack_rx) = ack_rx else {
            return Err(AppError::Internal(format!(
                "background notification {}:{} has no durable notification part id",
                notification.operation_kind, notification.operation_id
            )));
        };
        let acknowledged_before_release = tokio::select! {
            biased;
            _ = ack_rx => true,
            _ = self.execution_registry.wait_until_released(session_id) => {
                tracing::debug!(
                    target: "agena_background",
                    %session_id, operation_kind = %notification.operation_kind,
                    operation_id = %notification.operation_id,
                    "notification steer acknowledged no new turn; waking idle"
                );
                self.wake_idle_notification(session_id).await?;
                false
            }
        };
        if !acknowledged_before_release {
            return Ok(());
        }

        // Cursor observation only proves that the active loop loaded the
        // notification; it does not prove the answering model round committed.
        // Keep the delivery claimed until that execution ends, then verify the
        // durable assistant response. A crash/failure after observation but
        // before response falls back to a fresh wake instead of consuming and
        // silently losing the notification.
        self.execution_registry
            .wait_until_released(session_id)
            .await;
        if !self
            .notification_has_completed_assistant_response(session_id, notification_part_id)
            .await?
        {
            tracing::debug!(
                target: "agena_background",
                %session_id, operation_kind = %notification.operation_kind,
                operation_id = %notification.operation_id,
                "notification was observed but produced no completed assistant response; waking idle"
            );
            self.wake_idle_notification(session_id).await?;
        }
        Ok(())
    }

    /// Start a fresh notification-wake execution: a model turn over the
    /// appended `system_notification` parts. Shared by the settle wake paths
    /// whenever no live execution will deliver the notification.
    async fn wake_idle_notification(&self, session_id: i64) -> Result<(), AppError> {
        self.execute_registered(
            session_id,
            ExecutionSource::User,
            ExecutionConversationTarget::NewTurn,
            "notification execution",
            move |manager, control, steer_rx| async move {
                manager
                    .notification_run_inner(session_id, control, steer_rx)
                    .await
            },
        )
        .await?;
        Ok(())
    }
}
