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
use crate::event::EventKind;
use crate::message::{
    AttachmentItem, InteractiveRequestPart, Message, MessageMetadata, MessagePart, OperationBlock,
    OperationPart, PartContent, RequestPart,
};
use crate::permission::resolve_permission_with_persisted_rules;
use crate::tool::{StreamingToolExecution, ToolError, ToolExecutor, ToolInvocationExecution};
use agena_domain::ToolInvocation;
use agena_domain::ToolOutput;
use agena_domain::UserInputReply;
use agena_domain::{
    DecisionTraceStep, ExecutionFailureKind, ExecutionFinishedEvent, ExecutionOutcome,
    ExecutionSource, ExecutionStartedEvent, FinishReason, PermissionAction, PermissionDecision,
    PermissionMode, PermissionRepliedEvent, PermissionReply, PermissionReplyKind,
    PermissionRiskLevel, PermissionScope, Role, RunAbortReason, TimeRange, UserInputReplyKind,
};
use agena_domain::{ExecutionStatus, MessageSource};
pub(crate) use agena_domain::{ModelRef, ModelSpeedModeRequestOverride};
use agena_provider::ProviderNativeToolsConfig;
use agena_storage::PersistedPermissionRule;
use agena_tool::PreparedShellCommand;
use std::path::PathBuf;

use super::cache::SessionCachePolicy;
use super::history::{
    MessageId as HistoryMessageId, PartId as HistoryPartId, RunAborted, RunCompleted,
    RunId as HistoryRunId, RunStarted, SystemNoticeAppended, ToolCallCompleted,
    ToolCallId as HistoryToolCallId, TranscriptContent, UserMessageAppended,
};
use super::model::{PromptCompactionRuntime, ProviderPromptAnchor, SessionPendingTool};
use super::processor::{SessionRunRequest, SessionRunTermination};
use super::prompt_window::PromptRequestOptions;
use super::store::{ReservedMessageIds, SessionCommit, SessionStore};
use super::{ExecutionControl, ExecutionControlError, ExecutionRegistry};
use crate::session::{Session, SessionProcessor};
use agena_domain::{SessionListRequest, SessionSummary, UsageStats, UsageStatsQuery};

use agena_runtime::RuntimeSessionManagerConfig;

use agena_runtime::{
    SessionAgentRestoreOutcome, SessionAgentSwitchOutcome, SessionCreateRequest,
    SessionExecutionReplyRequest, SessionExecutionRequest, SessionPermissionReplyRequest,
    SessionRunOptions,
};

fn completion_request(
    options: &SessionRunOptions,
    system: Option<String>,
    messages: Vec<Message>,
    tool_api_functions: Vec<crate::tool::ToolApiBinding>,
    provider_native_tools: ProviderNativeToolsConfig,
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
        provider_native_tools,
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

type SessionUserMessageRequest = agena_runtime::SessionUserMessageRequest<PartContent>;

#[derive(Debug, Clone)]
pub(crate) struct SessionSubtaskRequest {
    pub parent_session_id: i64,
    pub description: String,
    pub prompt: String,
    pub profile_name: String,
    pub task_id: Option<String>,
    pub requested_selection: agena_domain::AgentSelectionConfig,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSubtaskResponse {
    pub session: Session,
    pub task_id: String,
    pub parent_session_id: i64,
    pub profile_name: String,
    pub status: agena_domain::SubtaskStatus,
    pub resumed: bool,
    pub final_text: Option<String>,
    pub error: Option<String>,
    pub usage: agena_provider::CompletionUsage,
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
    response: oneshot::Sender<agena_plugin_host::sdk::host_api::AskUserResponse>,
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
    config: RuntimeSessionManagerConfig,
    tool_execution_semaphore: Arc<Semaphore>,
}

/// Per-run collaborators that belong to one execution lifecycle.
///
/// This prevents the session loop API from growing a sequence of unrelated
/// arguments as new lifecycle concerns are introduced.
struct StableRunContext {
    allow_goal_continuation: bool,
    base_run_source: ExecutionSource,
    active_turn_id: Option<i64>,
    state: Arc<SessionManagerState>,
    control: Arc<ExecutionControl>,
    steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
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
        config: RuntimeSessionManagerConfig,
    ) -> Self {
        let tool_execution_semaphore = Arc::new(Semaphore::new(config.max_concurrent_tools));
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

pub(crate) struct SessionManager {
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
pub(crate) struct AuthorizedToolInvocation {
    executor: ToolExecutor,
    invocation: ToolInvocation,
    session_id: i64,
}

impl AuthorizedToolInvocation {
    pub(crate) fn execute(self, call_id: i64) -> Result<ToolInvocationExecution, ToolError> {
        self.executor
            .execute_invocation_detailed_bypassing_permissions(
                &self.invocation,
                self.session_id,
                call_id,
            )
    }
}

pub(crate) enum ToolInvocationAuthorization {
    Allowed(Box<AuthorizedToolInvocation>),
    Ask { reason: String },
    Deny { reason: String },
}

#[async_trait::async_trait]
impl agena_runtime::SessionToolExecutionService for SessionManager {
    async fn execute_session_tool(
        &self,
        session_id: i64,
        invocation: ToolInvocation,
    ) -> Result<agena_tool::ToolExecutionSummary, agena_runtime::SessionToolExecutionError> {
        match self
            .authorize_session_tool_invocation(session_id, invocation)
            .await
            .map_err(|error| {
                agena_runtime::SessionToolExecutionError::Execution(error.to_string())
            })? {
            ToolInvocationAuthorization::Allowed(authorized) => authorized
                .execute(-1)
                .map(|execution| execution.summary())
                .map_err(|error| {
                    agena_runtime::SessionToolExecutionError::Execution(error.to_string())
                }),
            ToolInvocationAuthorization::Ask { reason } => Err(
                agena_runtime::SessionToolExecutionError::ApprovalRequired(reason),
            ),
            ToolInvocationAuthorization::Deny { reason } => {
                Err(agena_runtime::SessionToolExecutionError::Denied(reason))
            }
        }
    }

    fn render_session_tool_output(
        &self,
        session_id: i64,
        invocation: ToolInvocation,
    ) -> Result<String, agena_runtime::SessionToolExecutionError> {
        self.tool_executor()
            .execute_invocation_detailed(&invocation, session_id, -1)
            .map(|execution| execution.view.output_text)
            .map_err(|error| agena_runtime::SessionToolExecutionError::Execution(error.to_string()))
    }

    fn execute_snapshot_command(
        &self,
        session_id: i64,
        command: agena_runtime::SessionSnapshotCommand,
    ) -> Result<agena_runtime::SessionSnapshotCommandResult, agena_runtime::SessionToolExecutionError>
    {
        let (tool_name, input) = match command {
            agena_runtime::SessionSnapshotCommand::Enter { name, path } => (
                "enter_snapshot",
                serde_json::to_value(crate::message::EnterSnapshotToolInput { name, path })
                    .map_err(|error| {
                        agena_runtime::SessionToolExecutionError::Execution(error.to_string())
                    })?,
            ),
            agena_runtime::SessionSnapshotCommand::Exit {
                action,
                discard_changes,
            } => (
                "exit_snapshot",
                serde_json::to_value(crate::message::ExitSnapshotToolInput {
                    action,
                    discard_changes,
                })
                .map_err(|error| {
                    agena_runtime::SessionToolExecutionError::Execution(error.to_string())
                })?,
            ),
        };
        let output = self
            .tool_executor()
            .execute_tool_payload_for_host(tool_name, input, Some(session_id), None, None)
            .map_err(|error| {
                agena_runtime::SessionToolExecutionError::Execution(error.to_string())
            })?;
        Ok(agena_runtime::SessionSnapshotCommandResult {
            payload: output.payload,
        })
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
        let executor = self
            .tool_executor()
            .for_session_context(&session.runtime().execution);
        let permission_name = format!(
            "plugin.command.{}.{}",
            request.plugin_id, request.command_id
        );
        let check = agena_tool::ToolPermissionCheck {
            action: crate::permission::tool_action(
                permission_name.as_str(),
                None,
                &[],
                Some(&executor.agent().tool_policy),
            ),
            decision: executor
                .agent()
                .authorize_tool_names(&[permission_name.as_str()], None, &[]),
        };
        match self
            .resolve_tool_permission_check(Some(session.id), &check)
            .await
            .map_err(|error| {
                agena_runtime::SessionPluginCommandError::Execution(error.to_string())
            })?
            .decision
        {
            agena_domain::PermissionDecision::Allow => {}
            agena_domain::PermissionDecision::Ask { reason } => {
                return Err(agena_runtime::SessionPluginCommandError::ApprovalRequired(
                    reason,
                ));
            }
            agena_domain::PermissionDecision::Deny { reason } => {
                return Err(agena_runtime::SessionPluginCommandError::Denied(reason));
            }
        }
        executor
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
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
    }

    async fn submit_user_message(
        &self,
        request: agena_runtime::SessionUserMessageRequest<agena_runtime::SessionUserMessagePart>,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let agena_runtime::SessionUserMessageRequest { run, parts } = request;
        let parts = parts
            .into_iter()
            .map(|part| match part {
                agena_runtime::SessionUserMessagePart::Text(part) => PartContent::Text(part),
                agena_runtime::SessionUserMessagePart::Attachment(part) => {
                    PartContent::Attachment(part)
                }
            })
            .collect();
        let session = SessionManager::submit_user_message(
            self,
            agena_runtime::SessionUserMessageRequest::new(run.session_id, run.options, parts),
        )
        .await
        .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
    }

    async fn steer_input(
        &self,
        session_id: i64,
        parts: Vec<agena_runtime::SessionUserMessagePart>,
    ) -> Result<(), agena_runtime::SessionExecutionCommandError> {
        let parts = parts
            .into_iter()
            .map(|part| match part {
                agena_runtime::SessionUserMessagePart::Text(part) => PartContent::Text(part),
                agena_runtime::SessionUserMessagePart::Attachment(part) => {
                    PartContent::Attachment(part)
                }
            })
            .collect();
        SessionManager::steer_input(self, session_id, parts)
            .await
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))
    }

    async fn continue_session(
        &self,
        request: agena_runtime::SessionExecutionRequest,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::continue_session(self, request)
            .await
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
    }

    async fn compact_session(
        &self,
        request: agena_runtime::SessionExecutionRequest,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::compact_session(self, request)
            .await
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
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
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
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
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
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
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
    }

    async fn reply_permission(
        &self,
        request: agena_runtime::SessionPermissionReplyRequest,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::reply_permission(self, request)
            .await
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
    }

    async fn reply_user_input(
        &self,
        request: agena_runtime::SessionExecutionReplyRequest<agena_domain::UserInputReply>,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::reply_user_input(self, request)
            .await
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
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
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
    }

    async fn set_session_allowed_tools(
        &self,
        session_id: i64,
        allowed_tools: Vec<String>,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::set_session_allowed_tools(self, session_id, allowed_tools)
            .await
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
    }

    async fn set_session_agent(
        &self,
        session_id: i64,
        agent_name: Option<String>,
    ) -> Result<
        agena_runtime::SessionExecutionCommandOutcome,
        agena_runtime::SessionExecutionCommandError,
    > {
        let session = SessionManager::switch_session_agent(self, session_id, agent_name, false)
            .await
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.session_id,
        })
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
            .map_err(|error| agena_runtime::SessionExecutionCommandError::new(error.to_string()))?;
        Ok(agena_runtime::SessionExecutionCommandOutcome {
            session_id: session.id,
        })
    }
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
        let seq = Arc::new(agena_storage::SequenceAllocator::new());
        let publisher = Arc::new(crate::event::publisher::EventPublisher::new(
            seq,
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
            Arc::new(agena_storage_sqlite::SeaMessageProjectionRepository::new(
                Arc::clone(&db_arc),
            )),
            Arc::new(agena_storage_sqlite::SeaMessageProjectionTransactionWriter),
            Arc::new(agena_storage_sqlite::SeaSessionSummaryRepository::new(
                Arc::clone(&db_arc),
            )),
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

    /// Returns the unified event publisher used by Runtime composition and
    /// service adapters to emit `EventKind`.
    pub(crate) fn event_publisher(&self) -> Arc<crate::event::EventPublisher> {
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
    pub(crate) fn event_bus(&self) -> Arc<dyn crate::event::EventBus<crate::event::EventKind>> {
        Arc::clone(&self.bus)
    }

    pub(crate) fn tool_executor(&self) -> ToolExecutor {
        self.execution_state().tool_executor.clone()
    }

    /// Resolve all static, persisted, and plugin-provided permission decisions
    /// for an externally initiated session tool call without creating a user
    /// approval request. Callers may execute only the returned opaque
    /// capability, never a generic bypass.
    pub(crate) async fn authorize_session_tool_invocation(
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
    ) -> Result<agena_plugin_host::sdk::host_api::AskUserResponse, AppError> {
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

    pub(crate) async fn execute_host_invoked_tool(
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
                .send(agena_plugin_host::sdk::host_api::AskUserResponse {
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
        config: RuntimeSessionManagerConfig,
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

    pub fn cache_stats(&self) -> agena_domain::SessionCacheStats {
        self.store.cache_stats()
    }
}

#[cfg(test)]
mod tests {
    use crate::RuntimeSessionManagerConfig;
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
        time::Duration,
    };

    use agena_domain::{
        ExecutionStatus, FinishReason, PermissionAction, PermissionReplyKind, PermissionRiskLevel,
        StructuredObject, TimeRange,
    };
    use sea_orm::Database;
    use tokio::sync::Notify;

    use super::{
        SessionManager, build_message, host_permission_grant_matches_action, merge_system_prompts,
    };
    use crate::session::history::{
        AssistantMessageFinished, RunCompleted, RunStarted, TranscriptContent, UserMessageAppended,
    };
    use crate::{
        agent::Agent,
        agents::SubagentRegistry,
        event::EventKind,
        message::{MessageMetadata, OperationPart, PartContent},
        permission::{PermissionPolicy, ToolPermissionPolicy},
        provider::{ModelRuntime, ProviderRegistry},
        session::{ContextGovernor, Session, SessionProcessor},
        tool::ToolExecutor,
    };
    use agena_domain::PermissionReply;
    use agena_domain::ToolInvocation;
    use agena_domain::{ExecutionId, ExecutionSource, Role, RunId};
    use agena_domain::{Model, ModelId, ModelRef};
    use agena_plugin_host::sdk::ToolStreamSink;
    use agena_plugin_host::{
        ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig,
        StaticPluginRegistration, ToolPresentationConfig,
    };
    use agena_provider::CompletionRequest;
    use agena_provider::CompletionResponse;
    use agena_runtime::{
        SessionCreateRequest, SessionPermissionReplyRequest, SessionRewindRequest,
        SessionRunOptions,
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

    #[agena_plugin_host::sdk::agena_plugin(
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

    static REPLY_PROBE_STARTED: Notify = Notify::const_new();
    static REPLY_PROBE_CONTINUE: Notify = Notify::const_new();

    #[derive(Default)]
    struct ReplyLockProbeTool;

    #[agena_plugin_host::sdk::agena_plugin(
        namespace = "test",
        name = "reply_probe",
        version = "0.1.0",
        summary = "Permission reply-lock regression fixture."
    )]
    impl ReplyLockProbeTool {
        #[tool(name = "run", summary = "Wait until the reply-lock test releases it.")]
        async fn run(&self) -> String {
            REPLY_PROBE_STARTED.notify_one();
            REPLY_PROBE_CONTINUE.notified().await;
            "reply-probe-complete".to_string()
        }
    }

    struct ReplyTestProvider {
        default_model: ModelId,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for ReplyTestProvider {
        fn id(&self) -> &str {
            "reply-test-provider"
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        async fn list_models(&self) -> Result<Vec<Model>, crate::error::AppError> {
            Ok(Vec::new())
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, crate::error::AppError> {
            Err(crate::error::AppError::Provider(
                "reply lock test provider does not complete".to_string(),
            ))
        }
    }

    async fn test_manager() -> SessionManager {
        let workspace_root = std::env::current_dir().expect("resolve test workspace");
        let mut plugins_config = PluginsConfig::default();
        plugins_config.list.insert(
            "test.stream".to_string(),
            ConfiguredPlugin::static_default(),
        );
        plugins_config.list.insert(
            "test.reply_probe".to_string(),
            ConfiguredPlugin::static_default(),
        );
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![
                StaticPluginRegistration::new(
                    "test.stream".parse().expect("valid test plugin key"),
                    StreamingExecutionTool,
                ),
                StaticPluginRegistration::new(
                    "test.reply_probe"
                        .parse()
                        .expect("valid reply probe plugin key"),
                    ReplyLockProbeTool,
                ),
            ],
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
        let mut providers = ProviderRegistry::new();
        providers.register(ReplyTestProvider {
            default_model: ModelId::new("reply-test-model"),
        });
        let processor = SessionProcessor::new(
            Arc::new(providers),
            ContextGovernor::new(agena_domain::ContextPolicy::default()),
            plugins,
            workspace_root,
        );
        let database = Database::connect("sqlite::memory:")
            .await
            .expect("open in-memory database");
        agena_storage_sqlite::initialize_schema(&database)
            .await
            .expect("migrate in-memory database");
        SessionManager::new(
            database,
            processor,
            executor,
            RuntimeSessionManagerConfig::default(),
        )
    }

    async fn append_completed_text_message(
        manager: &SessionManager,
        mut session: Session,
        role: Role,
        text: &str,
        turn_id: Option<i64>,
        parent_message_id: Option<i64>,
    ) -> (Session, i64) {
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("reserve rewind regression message ids");
        let message_id = ids.message_id;
        let message = build_message(
            ids,
            role,
            ExecutionStatus::Completed,
            vec![PartContent::text(text)],
            MessageMetadata {
                turn_id: Some(turn_id.unwrap_or(message_id)),
                parent_message_id,
                ..Default::default()
            },
        );
        session.messages.push(message.clone());
        let session = manager
            .persist_session_changes(
                session,
                vec![message.clone()],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist rewind regression message");
        let execution_id = ExecutionId::new();
        let run_id = RunId::new();
        let message_event = match role {
            Role::User => EventKind::UserMessageAppended(UserMessageAppended {
                execution_id,
                message_id: agena_domain::MessageId(message.id),
                run_id,
                created_at: message.created_at,
                content: TranscriptContent::from_message_lossy(&message),
                parts: message.parts.clone(),
                metadata: message.metadata.clone(),
                provider_state: message.provider_state.clone(),
            }),
            Role::Assistant => EventKind::AssistantMessageFinished(AssistantMessageFinished {
                execution_id,
                message_id: agena_domain::MessageId(message.id),
                run_id,
                created_at: message.created_at,
                content: TranscriptContent::from_message_lossy(&message),
                status: message.state,
                parts: message.parts.clone(),
                usage: message.usage.clone(),
                finish_reason: FinishReason::Stop,
                metadata: message.metadata.clone(),
                provider_state: message.provider_state.clone(),
            }),
            role => panic!("unsupported rewind regression role: {role}"),
        };
        let session = manager
            .store
            .append_history_items(
                session,
                vec![
                    EventKind::RunStarted(RunStarted {
                        execution_id,
                        run_id,
                        source: ExecutionSource::User,
                        model_id: "test-model".into(),
                        provider_id: "test-provider".into(),
                        request_digest: None,
                    }),
                    message_event,
                    EventKind::RunCompleted(RunCompleted {
                        run_id,
                        finish_reason: FinishReason::Stop,
                    }),
                ],
                manager.execution_state().cache_policy(),
            )
            .await
            .expect("append current rewind regression history");
        (session, message_id)
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

    #[tokio::test]
    async fn rewind_copies_history_without_removing_it_from_the_source_session() {
        let manager = test_manager().await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "rewind source".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create rewind source");
        let source_id = session.id;
        let (session, first_user_id) = append_completed_text_message(
            &manager,
            session,
            Role::User,
            "first prompt",
            None,
            None,
        )
        .await;
        let (session, assistant_id) = append_completed_text_message(
            &manager,
            session,
            Role::Assistant,
            "first response",
            Some(first_user_id),
            Some(first_user_id),
        )
        .await;
        let (_session, rewind_target_id) = append_completed_text_message(
            &manager,
            session,
            Role::User,
            "rewrite this prompt",
            None,
            Some(assistant_id),
        )
        .await;

        let source_before = manager
            .store
            .list_projected_messages(source_id, true)
            .await
            .expect("load source projection before rewind");
        assert_eq!(source_before.len(), 3);

        let child = manager
            .rewind_session(SessionRewindRequest {
                session_id: source_id,
                message_id: rewind_target_id,
                expected_version: None,
            })
            .await
            .expect("rewind current-format session");

        let source_after = manager
            .store
            .list_projected_messages(source_id, true)
            .await
            .expect("reload source projection after rewind");
        assert_eq!(source_after, source_before);
        assert_eq!(
            source_after
                .iter()
                .map(|message| message.as_text_lossy())
                .collect::<Vec<_>>(),
            vec!["first prompt", "first response", "rewrite this prompt"]
        );

        assert_eq!(child.parent_id, Some(source_id));
        assert_eq!(
            child.relation_kind,
            agena_domain::SessionRelationKind::Rewind
        );
        assert_eq!(
            child.lifecycle_state,
            agena_domain::SessionLifecycleState::Ready
        );
        assert_eq!(child.source_message_id, Some(rewind_target_id));
        assert!(child.source_cutoff_seq_global.is_some());
        assert_eq!(
            child
                .messages
                .iter()
                .map(|message| message.as_text_lossy())
                .collect::<Vec<_>>(),
            vec!["first prompt", "first response"]
        );
        let source_message_ids = source_before
            .iter()
            .map(|message| message.id)
            .collect::<HashSet<_>>();
        let source_part_ids = source_before
            .iter()
            .flat_map(|message| message.parts.iter().map(|part| part.id))
            .collect::<HashSet<_>>();
        assert!(
            child
                .messages
                .iter()
                .all(|message| !source_message_ids.contains(&message.id))
        );
        assert!(
            child
                .messages
                .iter()
                .flat_map(|message| &message.parts)
                .all(|part| !source_part_ids.contains(&part.id))
        );
        assert_eq!(
            child.messages[1].metadata.turn_id,
            Some(child.messages[0].id)
        );
        assert_eq!(
            child.messages[1].metadata.parent_message_id,
            Some(child.messages[0].id)
        );

        let child_events = manager
            .store
            .list_session_events(child.id)
            .await
            .expect("load copied child events");
        assert!(child_events.iter().any(|event| {
            matches!(event.kind, crate::event::EventKind::UserMessageAppended(_))
        }));
        assert!(child_events.iter().all(|event| match &event.kind {
            crate::event::EventKind::MessagePartCheckpointed(payload) => {
                payload.session_id == child.id
            }
            _ => true,
        }));
    }

    #[tokio::test]
    async fn session_export_uses_one_current_unversioned_format() {
        let manager = test_manager().await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "current export format".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create export source");
        let bundle = manager
            .export_session_jsonl(session.id)
            .await
            .expect("export current session");
        let header = bundle.lines().next().expect("export header");
        let mut header_value = serde_json::from_str::<serde_json::Value>(header)
            .expect("decode current export header");
        assert!(
            header_value.get("schema").is_none(),
            "current-only exports must not carry a schema generation"
        );
        manager
            .import_session_jsonl(bundle.as_str())
            .await
            .expect("import matching current export");

        header_value
            .as_object_mut()
            .expect("object export header")
            .insert("schema".to_owned(), serde_json::json!(1));
        let versioned_bundle = format!(
            "{}\n",
            serde_json::to_string(&header_value).expect("encode versioned header")
        );
        let error = manager
            .import_session_jsonl(versioned_bundle.as_str())
            .await
            .expect_err("versioned export headers are not accepted");
        assert!(error.to_string().contains("unknown field `schema`"));
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
        assert_eq!(execution.summary().output_text, "stream-terminal");

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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn permission_reply_releases_session_lock_before_tool_continuation() {
        let manager = Arc::new(test_manager().await);
        let call_id = 91;
        let request_id = "reply-lock-probe".to_string();
        let options = SessionRunOptions {
            model: ModelRef::new("reply-test-provider", "reply-test-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: None,
            agent_profile: None,
        };
        let mut session = manager
            .create_session(SessionCreateRequest {
                title: "permission reply lock regression".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create test session");
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("reserve reply probe message ids");
        let invocation = ToolInvocation::new("test.reply_probe.run", StructuredObject::default());
        let operation = OperationPart::pending(
            call_id,
            invocation,
            "Tool reply_probe.run",
            TimeRange::default(),
        );
        let metadata = MessageMetadata {
            turn_id: Some(1),
            model_provider_id: options.model.provider_id.to_string(),
            model_id: options.model.model_id.to_string(),
            ..MessageMetadata::default()
        };
        let mut message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::InProgress,
            vec![PartContent::Operation(operation)],
            metadata,
        );
        message.parts[0].operation_id = Some("reply-lock-operation".to_string());
        session.messages.push(message.clone());
        session = manager
            .persist_session_changes(
                session,
                vec![message],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist reply probe operation");
        let pending = session
            .next_pending_tool()
            .expect("pending reply probe tool");
        let action = PermissionAction::Tool {
            tool_name: "test.reply_probe.run".to_string(),
            qualifier: None,
        };
        session = manager
            .apply_permission_request_with_id(
                session,
                &pending,
                request_id.clone(),
                action.clone(),
                Vec::new(),
                vec![action],
                "reply lock regression".to_string(),
                String::new(),
                Some("static_policy".to_string()),
                None,
                None,
                PermissionRiskLevel::Medium,
                Vec::new(),
                manager.execution_state(),
            )
            .await
            .expect("persist reply probe permission request");

        let reply_manager = Arc::clone(&manager);
        let session_id = session.id;
        let mut reply_task = tokio::spawn(async move {
            reply_manager
                .reply_permission(SessionPermissionReplyRequest::new(
                    session_id,
                    options,
                    PermissionReply {
                        request_id,
                        kind: PermissionReplyKind::AllowOnce,
                        reason: None,
                        scope: None,
                    },
                    None,
                ))
                .await
        });

        tokio::select! {
            _ = REPLY_PROBE_STARTED.notified() => {}
            result = &mut reply_task => panic!("permission reply terminated before tool continuation: {result:?}"),
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                panic!("approved tool continuation did not start")
            }
        }
        let lock_is_available = tokio::time::timeout(Duration::from_secs(1), async {
            let lock = manager.reply_session_lock(session_id).await;
            let _guard = lock.lock().await;
        })
        .await
        .is_ok();
        REPLY_PROBE_CONTINUE.notify_one();
        let _ = tokio::time::timeout(Duration::from_secs(5), reply_task)
            .await
            .expect("permission reply continuation did not terminate")
            .expect("permission reply task panicked");

        assert!(
            lock_is_available,
            "permission reply held the session lock while executing the approved tool"
        );
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
