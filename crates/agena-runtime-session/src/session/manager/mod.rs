//! Production session orchestration over the one transactional session kernel.
//! Only active work is retained here; immutable transcript history is paged by
//! readers. Provider and tool effects live in supervised tasks, independently
//! of the lifetime of an HTTP/UI waiter.
use super::{Session, SessionProcessor, kernel::KernelPromptHydrator};
use crate::{
    AppError, ContextGovernor, RuntimeSessionManagerConfig, SessionCreateRequest,
    SessionExecutionRequest, SessionRunOptions,
};
use crate::{
    provider::ProviderRegistry,
    tool::{ToolError, ToolExecutor, ToolInvocationExecution},
};
use agena_domain::{ComposerDocument, ComposerNode, ExecutionId, ExecutionSelection, ModelRef};
use agena_storage::{backend::kernel::KernelPorts, kernel::*};
use arc_swap::ArcSwap;
use chrono::Utc;
use std::{
    collections::HashMap,
    result::Result,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

mod agents;
mod auxiliary;
mod commands;
mod deliveries;
mod execution;
mod host;
mod media;
mod queries;
mod results;
mod results_text;
mod session_prompt;
mod titles;

const MAX_ACTIVE_EXECUTIONS: usize = 128;
const MAX_AUXILIARY_TASKS: usize = 4;
const DEFAULT_MAX_MODEL_TURNS: usize = 500;

#[derive(Clone)]
struct ExecutionState {
    providers: Arc<ProviderRegistry>,
    governor: ContextGovernor,
    processor: SessionProcessor,
    executor: ToolExecutor,
    config: RuntimeSessionManagerConfig,
    tools: Arc<Semaphore>,
}
#[derive(Clone)]
struct AgentBudget {
    max_tokens: Option<u64>,
    max_cost_microusd: Option<u64>,
}
struct RunControl {
    session: SessionId,
    execution: ExecutionId,
    model: ModelRef,
    state: Arc<ExecutionState>,
    cancel: CancellationToken,
    phase: StdMutex<agena_domain::ExecutionPhase>,
    done: Notify,
    budget: Option<AgentBudget>,
    failure: StdMutex<Option<agena_failure::Failure>>,
    settled: std::sync::atomic::AtomicBool,
}
type ToolPermitSlot = Arc<tokio::sync::Mutex<Option<OwnedSemaphorePermit>>>;
#[derive(Clone)]
struct LiveTool {
    tool: ToolHandle,
    control: Arc<RunControl>,
    permit: ToolPermitSlot,
    /// One tool owns one interaction slot. Concurrent plugin callbacks must
    /// serialize approval and may reuse a grant only within this live call.
    permissions: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
}
type ActiveToolMap = HashMap<(i64, i64), LiveTool>;
#[derive(Default)]
struct SessionDispatch {
    submission: tokio::sync::Mutex<()>,
    execution: tokio::sync::Mutex<()>,
    tail: StdMutex<std::sync::Weak<RunControl>>,
    title: StdMutex<titles::TitleWork>,
}
impl RunControl {
    fn phase(&self, phase: agena_domain::ExecutionPhase) {
        *self.phase.lock().unwrap_or_else(|p| p.into_inner()) = phase;
    }
    async fn wait_settled(&self) {
        loop {
            let notified = self.done.notified();
            if self.settled.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

#[derive(Clone)]
pub struct SessionManager {
    kernel_runtime: Arc<agena_storage_sqlite::SqliteKernel>,
    state: Arc<ArcSwap<ExecutionState>>,
    prompt_hydrator: KernelPromptHydrator,
    active: Arc<StdMutex<HashMap<ExecutionId, Arc<RunControl>>>>,
    admission: Arc<Semaphore>,
    agent_settlements: Arc<Semaphore>,
    auxiliary_tasks: Arc<Semaphore>,
    /// Locks exist only while queued/running executions retain them.
    sessions: Arc<StdMutex<HashMap<SessionId, std::sync::Weak<SessionDispatch>>>>,
    tools: Arc<StdMutex<ActiveToolMap>>,
    agent_preparation: Arc<tokio::sync::Mutex<()>>,
    delivery_scan: Arc<StdMutex<Option<SessionSummaryCursor>>>,
    host_sequences: Arc<StdMutex<HashMap<(i64, i64), u64>>>,
    workspace: Arc<tokio::sync::OnceCell<i64>>,
    shutdown: CancellationToken,
}

impl SessionManager {
    pub fn new(
        kernel: Arc<agena_storage_sqlite::SqliteKernel>,
        providers: Arc<ProviderRegistry>,
        governor: ContextGovernor,
        processor: SessionProcessor,
        executor: ToolExecutor,
        config: RuntimeSessionManagerConfig,
    ) -> Self {
        let executor = executor.with_kernel_reader(kernel.ports.reader.clone());
        Self {
            kernel_runtime: kernel,
            state: Arc::new(ArcSwap::from_pointee(ExecutionState {
                providers,
                governor,
                processor,
                executor,
                tools: Arc::new(Semaphore::new(config.max_concurrent_tools.max(1))),
                config,
            })),
            prompt_hydrator: KernelPromptHydrator::default(),
            active: Default::default(),
            admission: Arc::new(Semaphore::new(MAX_ACTIVE_EXECUTIONS)),
            agent_settlements: Arc::new(Semaphore::new(MAX_ACTIVE_EXECUTIONS)),
            auxiliary_tasks: Arc::new(Semaphore::new(MAX_AUXILIARY_TASKS)),
            sessions: Default::default(),
            tools: Default::default(),
            host_sequences: Default::default(),
            agent_preparation: Default::default(),
            delivery_scan: Default::default(),
            workspace: Default::default(),
            shutdown: CancellationToken::new(),
        }
    }
    pub fn reconfigure(
        &self,
        providers: Arc<ProviderRegistry>,
        governor: ContextGovernor,
        processor: SessionProcessor,
        executor: ToolExecutor,
        config: RuntimeSessionManagerConfig,
    ) {
        self.state.store(Arc::new(ExecutionState {
            providers,
            governor,
            processor,
            executor: executor.with_kernel_reader(self.kernel().reader.clone()),
            tools: Arc::new(Semaphore::new(config.max_concurrent_tools.max(1))),
            config,
        }));
    }
    pub fn kernel(&self) -> &KernelPorts {
        &self.kernel_runtime.ports
    }
    pub fn session_kernel(&self) -> KernelPorts {
        self.kernel().clone()
    }
    pub fn tool_executor(&self) -> ToolExecutor {
        self.state.load().executor.clone()
    }
    pub(crate) async fn current_workspace_id(&self) -> Result<i64, AppError> {
        self.workspace
            .get_or_try_init(|| async {
                // Public workspace resolution canonicalizes existing paths.
                // Use the same identity here so filesystem aliases (including
                // macOS /var and /private/var) cannot split one workspace.
                let workspace = tokio::fs::canonicalize(self.tool_executor().workspace_root())
                    .await
                    .map_err(|error| AppError::internal_error(&error))?;
                self.kernel_runtime
                    .workspaces
                    .ensure_id(&workspace.to_string_lossy())
                    .await
                    .map_err(|error| AppError::internal_error(&error))
            })
            .await
            .copied()
    }
    fn session_dispatch(&self, id: SessionId) -> Arc<SessionDispatch> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        match sessions.get(&id).and_then(std::sync::Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(SessionDispatch::default());
                sessions.insert(id, Arc::downgrade(&lock));
                lock
            }
        }
    }
    pub async fn maintenance_tick(&self) -> Result<(), AppError> {
        self.kernel_runtime
            .maintenance
            .collect_history_snapshots()
            .await
            .map_err(kernel_error)?;
        self.kernel_runtime
            .maintenance
            .collect_blobs()
            .await
            .map_err(kernel_error)?;
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|_, weak| weak.strong_count() > 0);
        Ok(())
    }
    pub fn broadcast_active_session_end(
        &self,
        reason: agena_plugin_host::SessionEndReason,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        let sessions = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|r| r.session)
            .collect::<std::collections::BTreeSet<_>>();
        let executor = self.tool_executor();
        async move {
            for session in sessions {
                executor
                    .plugin_manager()
                    .broadcast_session_end(agena_plugin_host::SessionEndInput {
                        session_id: session.get(),
                        reason,
                    })
                    .await;
            }
        }
    }
    pub fn broadcast_session_end(
        &self,
        session_id: i64,
        reason: agena_plugin_host::SessionEndReason,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        let executor = self.tool_executor();
        async move {
            executor
                .plugin_manager()
                .broadcast_session_end(agena_plugin_host::SessionEndInput { session_id, reason })
                .await;
        }
    }
    pub async fn shutdown(&self) -> Result<(), AppError> {
        self.shutdown.cancel();
        let active = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for run in &active {
            run.cancel.cancel();
            let queued = self
                .kernel()
                .reader
                .execution(run.session, run.execution)
                .await
                .is_ok_and(|execution| execution.phase == ExecutionPhase::Queued);
            // Unclaimed input is safe to resume after a service restart. Only
            // executions that could have made external effects are fenced.
            if !queued {
                let _ = self
                    .kernel()
                    .sessions
                    .cancel(RequestId::new(), run.session, run.execution)
                    .await;
            }
        }
        for run in active {
            loop {
                let notified = run.done.notified();
                if run.settled.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                if tokio::time::timeout(Duration::from_secs(5), notified)
                    .await
                    .is_err()
                {
                    return Err(AppError::PersistenceBusy);
                }
            }
        }
        // Preparation owns an execution permit before it can commit input;
        // agent settlement has a separate permit after the provider exits.
        let drain = async {
            let _executions = self
                .admission
                .clone()
                .acquire_many_owned(MAX_ACTIVE_EXECUTIONS as u32)
                .await
                .map_err(|_| AppError::Cancelled)?;
            let _agents = self
                .agent_settlements
                .clone()
                .acquire_many_owned(MAX_ACTIVE_EXECUTIONS as u32)
                .await
                .map_err(|_| AppError::Cancelled)?;
            let _auxiliary = self
                .auxiliary_tasks
                .clone()
                .acquire_many_owned(MAX_AUXILIARY_TASKS as u32)
                .await
                .map_err(|_| AppError::Cancelled)?;
            Ok::<(), AppError>(())
        };
        tokio::time::timeout(Duration::from_secs(5), drain)
            .await
            .map_err(|_| AppError::PersistenceBusy)??;
        Ok(())
    }
}

fn session_id(value: i64) -> Result<SessionId, AppError> {
    SessionId::try_from(value).map_err(|e| AppError::Config(e.to_string()))
}
fn entry_id(value: i64) -> Result<EntryId, AppError> {
    EntryId::try_from(value).map_err(|e| AppError::Config(e.to_string()))
}
fn kernel_error(error: KernelError) -> AppError {
    match error {
        KernelError::Conflict(message) => AppError::Config(message),
        KernelError::NotFound(message) => AppError::Internal(format!("{message} not found")),
        other => AppError::Internal(other.to_string()),
    }
}
fn archive_failure(error: KernelError) -> agena_failure::Failure {
    use agena_failure::{
        Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility,
        RecoveryDirective, RetryDirective, UserPresentation,
    };
    let (code, category, retry, recovery, message) = match &error {
        KernelError::Invalid(_) => (
            "session.archive_invalid",
            FailureCategory::InvalidInput,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            "The session archive request is invalid.",
        ),
        KernelError::NotFound(_) | KernelError::Ownership => (
            "session.archive_not_found",
            FailureCategory::NotFound,
            RetryDirective::AfterRefresh,
            RecoveryDirective::Refresh,
            "The session archive is not available in this scope.",
        ),
        KernelError::Conflict(_) | KernelError::StaleAttempt => (
            "session.archive_conflict",
            FailureCategory::Conflict,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            "This archive request conflicts with an earlier request or session state.",
        ),
        KernelError::Busy | KernelError::Closed => (
            "session.archive_unavailable",
            FailureCategory::DependencyUnavailable,
            RetryDirective::Backoff,
            RecoveryDirective::Retry,
            "Session archives are temporarily unavailable. Retry with the same request identity.",
        ),
        _ => {
            return crate::service_failure::unexpected_service_failure(
                "session.archive_failed",
                "The session archive could not be processed.",
                error,
            );
        }
    };
    let failure = Failure::new(
        FailureCode::new(code),
        category,
        if matches!(error, KernelError::Busy | KernelError::Closed) {
            FailureResponsibility::System
        } else {
            FailureResponsibility::Caller
        },
        retry,
        recovery,
        FailureImpact::RequestRejected,
        UserPresentation::validated_with_context(code, message),
    );
    tracing::warn!(failure_id = %failure.id, diagnostic = %error, "session archive request rejected");
    failure
}
fn tool_error(error: ToolError) -> AppError {
    match error {
        ToolError::Cancelled => AppError::Cancelled,
        ToolError::PolicyDenied(value) => AppError::PolicyDenied(value),
        ToolError::UserDeclined(value) => AppError::UserDeclined(value),
        ToolError::CapabilityUnavailable(value) => AppError::CapabilityUnavailable(value),
        ToolError::ToolUnavailable(value) => AppError::ToolUnavailable(value),
        other => AppError::Tool(Box::new(other)),
    }
}
fn request_key(namespace: &str, key: &str) -> RequestId {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(namespace.as_bytes());
    hash.update([0]);
    hash.update(key.as_bytes());
    hex::encode(hash.finalize())[..32]
        .parse()
        .expect("128 bit request id")
}
fn transient(error: &KernelError) -> bool {
    matches!(
        error,
        KernelError::Storage(_) | KernelError::CommitUnknown { .. } | KernelError::Busy
    )
}
/// The closure replays one storage command with its retained request ID. It
/// must never contain a provider request, plugin hook, or external tool effect.
async fn persist<T, F, Fut>(mut command: F) -> agena_storage::kernel::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = agena_storage::kernel::Result<T>>,
{
    let mut wait = Duration::from_millis(25);
    loop {
        match command().await {
            Ok(value) => return Ok(value),
            Err(error) if transient(&error) => {
                tracing::warn!(%error,"kernel persistence pending; preserving the observed effect");
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(1));
            }
            Err(error) => return Err(error),
        }
    }
}
