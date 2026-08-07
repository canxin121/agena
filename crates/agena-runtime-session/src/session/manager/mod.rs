use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock},
    time::Duration,
};

use arc_swap::ArcSwap;
use chrono::Utc;
use sea_orm::{ConnectionTrait, Statement};
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};

use crate::AppError;
use crate::event::EventKind;
use crate::message::{
    AttachmentItem, InteractiveRequestPart, Message, MessageMetadata, MessagePart, OperationPart,
    PartContent, RequestPart,
};
use crate::tool::{StreamingToolExecution, ToolError, ToolExecutor, ToolInvocationExecution};
use agena_domain::ToolInvocation;
use agena_domain::ToolOutput;
use agena_domain::UserInputReply;
use agena_domain::{
    DecisionTraceStep, ExecutionFinishedEvent, ExecutionOutcome, ExecutionSource,
    ExecutionStartedEvent, FinishReason, PermissionAction, PermissionMode, PermissionRepliedEvent,
    PermissionReplyKind, PermissionScope, Role, RunAbortReason, TimeRange, UserInputReplyKind,
};
use agena_domain::{ExecutionStatus, MessageSource};
pub(crate) use agena_domain::{ModelRef, ModelSpeedModeRequestOverride};
use agena_storage::PersistedPermissionRule;
use agena_tool::PreparedShellCommand;
use std::path::PathBuf;

use super::cache::SessionCachePolicy;
use super::history::{
    MessageId as HistoryMessageId, RunAborted, RunCompleted, RunId as HistoryRunId, RunStarted,
    ToolCallCompleted, ToolCallId as HistoryToolCallId, TranscriptContent, UserMessageAppended,
};
use super::model::{PromptCompactionRuntime, ProviderPromptAnchor, SessionPendingTool};
use super::processor::{SessionRunRequest, SessionRunTermination};
use super::prompt_window::PromptRequestOptions;
use super::store::{MessageCheckpoint, ReservedMessageIds, SessionCommit, SessionStore};
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
    messages: Vec<Message>,
    tool_api_functions: Vec<crate::tool::ToolApiBinding>,
    prompt_cache_key: Option<String>,
    previous_response_id: Option<String>,
    prompt_window_generation: Option<u64>,
) -> agena_provider::CompletionRequest {
    agena_runtime::build_completion_request(agena_runtime::CompletionRequestInputs {
        model: options.model.model_id.clone(),
        system,
        messages: messages
            .iter()
            .map(crate::provider::project_completion_input)
            .collect(),
        tool_api_functions: tool_api_functions
            .into_iter()
            .map(|binding| binding.definition())
            .collect(),
        temperature: options.temperature,
        max_output_tokens: options.max_output_tokens,
        prompt_cache_key,
        previous_response_id,
        prompt_window_generation,
        thinking: options.thinking.clone(),
        verbosity: options.verbosity.clone(),
        request_override: options.request_override.clone(),
    })
}

pub(super) use agena_runtime::merge_system_prompts;

#[derive(Debug, Clone)]
struct SessionUserMessageRequest {
    run: SessionExecutionRequest,
    parts: Vec<UserInputPart>,
    idempotency_key: Option<String>,
}

impl SessionUserMessageRequest {
    fn new(session_id: i64, options: SessionRunOptions, parts: Vec<PartContent>) -> Self {
        Self {
            run: SessionExecutionRequest::new(session_id, options),
            parts: parts
                .into_iter()
                .map(UserInputPart::text_or_runtime)
                .collect(),
            idempotency_key: None,
        }
    }

    fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.idempotency_key = (!key.trim().is_empty()).then_some(key);
        self
    }
}

#[derive(Debug, Clone)]
struct UserInputPart {
    activity_id: Option<agena_domain::ActivityId>,
    content: PartContent,
}

impl UserInputPart {
    fn text_or_runtime(content: PartContent) -> Self {
        Self {
            activity_id: None,
            content,
        }
    }
}

#[derive(Debug, Clone)]
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
pub struct SessionSubtaskOutputChunk {
    pub cursor: i64,
    pub role: agena_domain::Role,
    pub text: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
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
    invocation: ToolInvocation,
    advertised_tool_identity: Option<String>,
    prepared_shell_command: Option<PreparedShellCommand>,
    lifecycle: TimeRange,
    session_runtime: crate::session::SessionRuntimeState,
    /// The tool Activity's UUID, resolved once from the session so blocking
    /// shell sinks can route real-time output to the correct Activity.
    activity_id: Option<agena_domain::ActivityId>,
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
    steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
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
mod session_prompt;
mod sessions;
mod stats;
mod tests;

use self::helpers::*;

impl SessionManagerState {
    fn new(
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
        config: RuntimeSessionManagerConfig,
    ) -> Self {
        let shared_permission = Arc::new(StdRwLock::new(config.permission.clone()));
        Self::new_with_permission_stores(
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

    fn new_with_permission_stores(
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
        let executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution)
            .with_cancellation_token(self.execution_registry.cancellation_token(session_id).await);
        let prepared = (|| {
            let prepared = executor.prepare_invocation(&invocation, session_id, -1)?;
            let (invocation, prepared_shell_command) =
                executor.prepare_shell_invocation(&prepared.invocation, session_id, -1)?;
            executor.execute_invocation_detailed_with_prepared_shell(
                &invocation,
                session_id,
                -1,
                prepared_shell_command,
            )
        })();
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
        self.tool_executor()
            .plugin_manager()
            .invoke_plugin_command(
                request.plugin_id.as_str(),
                agena_plugin_host::sdk::PluginCommandInvokeInput {
                    session_id: Some(session.id),
                    call_id: None,
                    workspace_root: request.workspace_root,
                    command_id: request.command_id,
                    slash: request.slash,
                    raw: request.raw,
                    input: request.input,
                },
            )
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

    async fn submit_user_message(
        &self,
        request: agena_runtime::SessionUserMessageRequest,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let agena_runtime::SessionUserMessageRequest {
            run,
            document,
            idempotency_key,
        } = request;
        let parts = part_contents_from_composer_document(document)
            .map_err(session_execution_command_error)?;
        let mut request = SessionUserMessageRequest {
            run,
            parts,
            idempotency_key: None,
        };
        if let Some(key) = idempotency_key {
            request = request.with_idempotency_key(key);
        }
        let outcome = SessionManager::start_user_message_parts(self, request)
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
        SessionManager::steer_input(
            self,
            session_id,
            parts.into_iter().map(|part| part.content).collect(),
        )
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
        let session = SessionManager::mark_interactive_request_presented(self, session_id, request_id)
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
) -> Result<Vec<UserInputPart>, AppError> {
    use agena_domain::{ActivityPayload, ComposerNode, ResourceKind, ResourceReference};

    document
        .0
        .into_iter()
        .map(|node| match node {
            ComposerNode::Text { text } => Ok(UserInputPart {
                activity_id: None,
                content: PartContent::text(text),
            }),
            ComposerNode::Activity { activity } => match activity.payload {
                ActivityPayload::Resource(resource) => {
                    let kind = match resource.kind {
                        ResourceKind::Image => crate::message::AttachmentKind::Image,
                        ResourceKind::Audio => crate::message::AttachmentKind::Audio,
                        ResourceKind::Video => crate::message::AttachmentKind::Video,
                        ResourceKind::Pdf => crate::message::AttachmentKind::Pdf,
                        ResourceKind::File
                        | ResourceKind::Directory
                        | ResourceKind::Url
                        | ResourceKind::Artifact => crate::message::AttachmentKind::File,
                    };
                    let source = match resource.reference {
                        ResourceReference::Artifact { uri, .. } => {
                            crate::message::AttachmentSource::FileId { file_id: uri }
                        }
                        ResourceReference::WorkspacePath { path } => {
                            crate::message::AttachmentSource::LocalPath { path }
                        }
                        ResourceReference::Url { url } => {
                            crate::message::AttachmentSource::Url { url }
                        }
                        ResourceReference::ProviderFile { file_id, .. } => {
                            crate::message::AttachmentSource::FileId { file_id }
                        }
                    };
                    Ok(UserInputPart {
                        activity_id: Some(activity.id),
                        content: PartContent::attachments(vec![AttachmentItem {
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
                        }]),
                    })
                }
                ActivityPayload::SkillReference(skill) => {
                    Ok(UserInputPart {
                        activity_id: Some(activity.id),
                        content: PartContent::Activity(
                            crate::message::RuntimeActivity::SkillReference(
                                crate::message::SkillReferencePart {
                                    skills: vec![crate::message::SkillReference {
                            name: skill.name,
                            description: skill.description,
                            instructions: skill.instructions,
                            content_hash: skill.content_hash,
                            source: skill.source,
                            aliases: skill.aliases,
                                    }],
                                },
                            ),
                        ),
                    })
                }
                ActivityPayload::TextArtifact(artifact) => Ok(UserInputPart {
                    activity_id: Some(activity.id),
                    content: PartContent::text(artifact.text),
                }),
                _ => Err(AppError::Config(
                    "turn input accepts only resource, skill_reference, and text_artifact activities"
                        .to_owned(),
                )),
            },
        })
        .collect()
}

impl SessionManager {
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

        let row = self
            .store
            .db
            .query_one(Statement::from_sql_and_values(
                self.store.db.get_database_backend(),
                "SELECT t.turn_id, r.reply_id \
                 FROM agena_turns t \
                 JOIN agena_assistant_replies r ON r.turn_id = t.turn_id \
                 WHERE t.session_id = ? \
                 ORDER BY t.turn_seq DESC LIMIT 1",
                [session_id.into()],
            ))
            .await?
            .ok_or_else(|| {
                AppError::Config(format!(
                    "{source:?} requires an existing user turn in session {session_id}"
                ))
            })?;
        let turn_id: String = row.try_get("", "turn_id")?;
        let reply_id: String = row.try_get("", "reply_id")?;
        let turn_id = uuid::Uuid::parse_str(turn_id.as_str())
            .map(agena_domain::TurnId)
            .map_err(|error| AppError::Internal(format!("invalid canonical turn id: {error}")))?;
        let reply_id = uuid::Uuid::parse_str(reply_id.as_str())
            .map(agena_domain::AssistantReplyId)
            .map_err(|error| AppError::Internal(format!("invalid assistant reply id: {error}")))?;
        Ok(ConversationIdentity { turn_id, reply_id })
    }

    /// Resolve the canonical reply directly from the durable model message
    /// that owns an interaction.
    ///
    /// Interaction Activities are a downstream presentation projection and
    /// can legitimately trail their model-message checkpoint while sibling
    /// tools are still completing. Reply commands must therefore never use
    /// `agena_content_nodes.owner_id` or the request part projection as their
    /// synchronization boundary. A request is appended to an already-owned
    /// assistant message, so message ownership is sufficient and stable.
    async fn conversation_identity_for_message(
        &self,
        session_id: i64,
        message_id: i64,
    ) -> Result<ConversationIdentity, AppError> {
        let row = self
            .store
            .db
            .query_one(Statement::from_sql_and_values(
                self.store.db.get_database_backend(),
                "SELECT r.turn_id, r.reply_id \
                 FROM agena_model_messages m \
                 JOIN agena_reply_executions e ON e.execution_id = m.execution_id \
                 JOIN agena_assistant_replies r ON r.reply_id = e.reply_id \
                 JOIN agena_turns t ON t.turn_id = r.turn_id \
                 WHERE m.message_id = ? AND m.session_id = ? AND t.session_id = ?",
                [
                    message_id.into(),
                    session_id.into(),
                    session_id.into(),
                ],
            ))
            .await?
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "message {message_id} in session {session_id} has no canonical assistant reply identity"
                ))
            })?;
        let turn_id: String = row.try_get("", "turn_id")?;
        let reply_id: String = row.try_get("", "reply_id")?;
        let turn_id = uuid::Uuid::parse_str(turn_id.as_str())
            .map(agena_domain::TurnId)
            .map_err(|error| AppError::Internal(format!("invalid canonical turn id: {error}")))?;
        let reply_id = uuid::Uuid::parse_str(reply_id.as_str())
            .map(agena_domain::AssistantReplyId)
            .map_err(|error| AppError::Internal(format!("invalid assistant reply id: {error}")))?;
        Ok(ConversationIdentity { turn_id, reply_id })
    }

    async fn begin_execution(
        &self,
        session_id: i64,
        control: &ExecutionControl,
        source: ExecutionSource,
    ) -> Result<(), AppError> {
        let event = EventKind::ExecutionStarted(ExecutionStartedEvent {
            session_id,
            execution_id: control.execution_id(),
            turn_id: control.turn_id(),
            reply_id: control.reply_id(),
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
            reply_id: control.reply_id(),
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
        F: FnOnce(
                SessionManager,
                Arc<ExecutionControl>,
                mpsc::UnboundedReceiver<Vec<PartContent>>,
            ) -> Fut
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
        if let Err(error) = self
            .begin_execution(session_id, control.as_ref(), source)
            .await
        {
            self.execution_registry
                .unregister_if_matches(session_id, &control)
                .await;
            return Err(error);
        }

        self.drive_registered(session_id, task_name, control, steer_rx, operation)
            .await
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
        F: FnOnce(
                SessionManager,
                Arc<ExecutionControl>,
                mpsc::UnboundedReceiver<Vec<PartContent>>,
            ) -> Fut
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
            .register(session_id, identity.turn_id, identity.reply_id)
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
        steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
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
        agena_runtime::session_started();
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
        agena_runtime::session_finished();

        let unmatched_run_reason = result
            .as_ref()
            .err()
            .map(run_abort_reason)
            .unwrap_or(RunAbortReason::Internal);
        // Terminalize the execution first: `finish_execution` publishes the
        // authoritative `ExecutionFinished` event and projects the reply
        // outcome. Reconcile afterwards so the synthesis pass (`RunAborted`
        // for any still-hanging run) observes those already-persisted events
        // and cannot race the execution's own cleanup writes with duplicate
        // per-session sequence numbers after a process restart.
        let outcome = Self::execution_outcome(control.as_ref(), &result);
        let terminal_result = self
            .finish_execution(session_id, control.as_ref(), outcome)
            .await;
        let reconciliation_result = self
            .store
            .reconcile_unmatched_runs(session_id, unmatched_run_reason)
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
            reconciled_sessions: Arc::clone(&self.reconciled_sessions),
            host_user_input_waiters: Arc::clone(&self.host_user_input_waiters),
            host_user_input_sequences: Arc::clone(&self.host_user_input_sequences),
        }
    }

    pub fn new(
        db: sea_orm::DatabaseConnection,
        processor: SessionProcessor,
        tool_executor: ToolExecutor,
        config: RuntimeSessionManagerConfig,
    ) -> Self {
        let db_arc = Arc::new(db.clone());
        // The publisher (not the store) consults `EventKind::is_persistent`
        // to decide which events land in SQLite, so the store stays a single
        // generic type.
        let store_inner: Arc<dyn agena_storage::EventStore<crate::event::EventKind>> = Arc::new(
            agena_storage_sqlite::SeaEventStore::<crate::event::EventKind>::new(Arc::clone(
                &db_arc,
            )),
        );
        let bus: Arc<dyn crate::event::EventBus<crate::event::EventKind>> =
            Arc::new(crate::event::InProcessEventBus::<crate::event::EventKind>::new(4096));
        // One database-backed allocator serves both the publisher (event
        // sequences) and the session store (projected message/part ids), so
        // every monotonic id is allocated atomically in the shared database
        // across all processes.
        let seq: Arc<dyn agena_storage::SequenceAllocator> = Arc::new(
            agena_storage_sqlite::SqliteSequenceAllocator::new(Arc::clone(&db_arc)),
        );
        let publisher = Arc::new(crate::event::publisher::EventPublisher::new(
            Arc::clone(&seq),
            Arc::clone(&store_inner),
            Arc::clone(&bus),
        ));
        let permission_rules = Arc::new(agena_storage_sqlite::SeaPermissionRuleRepository::new(
            Arc::clone(&db_arc),
        ));
        let permission_rule_repository: Arc<dyn agena_storage::PermissionRuleRepository> =
            permission_rules.clone();
        let permission_rule_transaction_writer: Arc<
            dyn agena_storage::PermissionRuleTransactionWriter<sea_orm::DatabaseTransaction>,
        > = Arc::new(agena_storage_sqlite::SeaPermissionRuleTransactionWriter);
        let store = Arc::new(SessionStore::new(
            db,
            tool_executor.workspace_root(),
            Arc::clone(&publisher),
            seq,
            Arc::new(agena_storage_sqlite::SeaWorkspaceRepository::new(
                Arc::clone(&db_arc),
            )),
            permission_rule_repository,
            permission_rule_transaction_writer,
            Arc::new(agena_storage_sqlite::SeaSessionStatsRepository::new(
                Arc::clone(&db_arc),
            )),
            Arc::new(agena_storage_sqlite::SeaUsageRepository::new(Arc::clone(
                &db_arc,
            ))),
            Arc::new(agena_storage_sqlite::SeaSessionSummaryRepository::new(
                Arc::clone(&db_arc),
            )),
            Arc::new(agena_storage_sqlite::SeaProjectionLookupRepository::new(
                Arc::clone(&db_arc),
            )),
            Arc::new(agena_storage_sqlite::SeaModelMessageRepository::new(
                Arc::clone(&db_arc),
            )),
            Arc::new(agena_storage_sqlite::SeaModelMessageTransactionWriter),
            Arc::new(agena_storage_sqlite::SeaSessionSummaryRepository::new(
                Arc::clone(&db_arc),
            )),
        ));
        let state = SessionManagerState::new(processor, tool_executor, config);
        let owner_id = uuid::Uuid::new_v4().to_string();
        Self {
            store,
            publisher,
            bus,
            execution: ArcSwap::from_pointee(state),
            execution_registry: Arc::new(ExecutionRegistry::with_lease(
                Arc::clone(&db_arc),
                owner_id.clone(),
            )),
            reply_session_locks: Arc::new(Mutex::new(HashMap::new())),
            reconciled_sessions: Arc::new(Mutex::new(HashSet::new())),
            host_user_input_waiters: Arc::new(Mutex::new(HashMap::new())),
            host_user_input_sequences: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    /// Returns the unified event publisher used by Runtime composition and
    /// service adapters to emit `EventKind`.
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
            .execute_invocation_detailed(&invocation, -1, call_id)
            .map_err(tool_error_to_app_error)?;
        Ok(agena_runtime::SessionToolExecutionOutcome::Completed(
            execution.summary(),
        ))
    }

    pub async fn request_host_user_input(
        &self,
        session_id: i64,
        call_id: i64,
        request: crate::message::AskUserToolInput,
    ) -> Result<agena_plugin_host::sdk::host_api::AskUserResponse, AppError> {
        let state = self.execution_state();
        let session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        let pending_tool = session.pending_tool_by_call_id(call_id).ok_or_else(|| {
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
        let outer_pending_tool = session.pending_tool_by_call_id(call_id);
        let command_event_sink = outer_pending_tool
            .as_ref()
            .and_then(|pending| resolve_pending_tool(&session, pending).ok())
            .map(|pending| self.command_event_sink_for_pending(session_id, &pending));
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution)
            .with_cancellation_token(cancellation.clone())
            .with_command_event_sink(command_event_sink);
        let prepared = scoped_executor
            .prepare_invocation(&invocation, session.id, call_id)
            .map_err(tool_error_to_app_error)?;
        let (invocation, prepared_shell_command) = scoped_executor
            .prepare_shell_invocation(&prepared.invocation, session.id, call_id)
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
                        let state = self.execution_state();
                        let session = self.store
                            .load_session(session_id, state.cache_policy())
                            .await?;
                        if session.has_replied_user_input_request(request_id) {
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

    pub fn prune_cache(&self) {
        let state = self.execution_state();
        self.store.prune_cache(state.cache_policy());
    }

    pub fn cache_stats(&self) -> agena_domain::SessionCacheStats {
        self.store.cache_stats()
    }
}
