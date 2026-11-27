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
mod bot;
mod channel_recovery;
mod commands;
mod deliveries;
mod discovery_cursor;
mod execution;
mod external_agent;
mod host;
mod integration_execution;
mod managed;
mod media;
mod memory;
mod permission_config;
mod profile;
mod queries;
mod results;
mod results_text;
mod session_prompt;
mod titles;
mod tool_api;
mod transcript;

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
/// The provider, plugin processor, executor, and limits of one independent
/// session scope. Keep global and project settings together during an atomic
/// publication so neither scope can accidentally inherit the other's tools.
pub struct SessionScopeReconfiguration {
    pub providers: Arc<ProviderRegistry>,
    pub processor: SessionProcessor,
    pub executor: ToolExecutor,
    pub config: RuntimeSessionManagerConfig,
}

#[derive(Clone)]
struct AgentBudget {
    max_tokens: Option<u64>,
    max_cost_microusd: Option<u64>,
}
struct RunControl {
    session: SessionId,
    execution: ExecutionId,
    generation: AttemptGeneration,
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
    cancel: CancellationToken,
    permit: ToolPermitSlot,
    /// One tool owns one interaction slot. Concurrent plugin callbacks must
    /// serialize approval and may reuse a grant only within this live call.
    permissions: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
}
type ActiveToolMap = HashMap<(i64, i64), LiveTool>;

type AttemptControlKey = (i64, ExecutionId, AttemptGeneration);
type AttemptControlOwners = Arc<StdMutex<HashMap<AttemptControlKey, usize>>>;

/// Every accepted cancellation keeps its exact-attempt callbacks in the drain
/// boundary even when its transport receiver has disappeared.
struct AttemptControlRegistration {
    owners: AttemptControlOwners,
    key: AttemptControlKey,
}

impl AttemptControlRegistration {
    fn new(owners: AttemptControlOwners, key: AttemptControlKey) -> Self {
        *owners
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(key)
            .or_default() += 1;
        Self { owners, key }
    }
}

impl Drop for AttemptControlRegistration {
    fn drop(&mut self) {
        let mut owners = self.owners.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(count) = owners.get_mut(&self.key) {
            if *count == 1 {
                owners.remove(&self.key);
            } else {
                *count -= 1;
            }
        }
    }
}

/// An operator call has a policy principal and a bounded lifetime, but no
/// transcript, session, or user-interaction authority. Plugin callbacks still
/// need the plugin host's exact, live callback token before reaching this map.
struct OperatorInvocationAuthority {
    executor: ToolExecutor,
    workspace: i64,
    cancel: CancellationToken,
}
type OperatorInvocationMap = HashMap<i64, Arc<OperatorInvocationAuthority>>;
#[derive(Default)]
struct SessionDispatch {
    submission: tokio::sync::Mutex<()>,
    execution: tokio::sync::Mutex<()>,
    tail: StdMutex<std::sync::Weak<RunControl>>,
    external_steering: external_agent::steering::Slot,
    external_work: external_agent::cancellation::Slot,
    external_auth: external_agent::auth::StatusSlot,
    title: StdMutex<titles::TitleWork>,
    memory: StdMutex<memory::MemoryWork>,
    memory_changed: Notify,
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
    acp_scope: Arc<tokio::sync::OnceCell<Arc<Self>>>,
    acp_workspace_source: Option<Arc<agena_storage_sqlite::SqliteKernel>>,
    state: Arc<ArcSwap<ExecutionState>>,
    global_state: Arc<ArcSwap<ExecutionState>>,
    configuration_lock: Arc<parking_lot::Mutex<()>>,
    prompt_hydrator: KernelPromptHydrator,
    active: Arc<StdMutex<HashMap<ExecutionId, Arc<RunControl>>>>,
    admission: Arc<Semaphore>,
    agent_settlements: Arc<Semaphore>,
    auxiliary_tasks: Arc<Semaphore>,
    /// Accepted exact-attempt cancellations outlive their transport waiters.
    control_tasks: tokio_util::task::TaskTracker,
    attempt_controls: AttemptControlOwners,
    /// Locks exist only while queued/running executions retain them.
    sessions: Arc<StdMutex<HashMap<SessionId, std::sync::Weak<SessionDispatch>>>>,
    external_idle_peers: external_agent::idle::IdlePeers,
    external_agent_inputs: external_agent::input_requests::Inputs,
    tools: Arc<StdMutex<ActiveToolMap>>,
    operator_calls: Arc<StdMutex<OperatorInvocationMap>>,
    bot_operations: Arc<tokio::sync::Mutex<()>>,
    agent_preparation: Arc<tokio::sync::Mutex<()>>,
    pending_agent_settlements: Arc<StdMutex<HashMap<SessionId, agents::PendingAgentSettlement>>>,
    delivery_scan: Arc<StdMutex<Option<SessionSummaryCursor>>>,
    host_sequences: Arc<StdMutex<HashMap<(i64, i64), u64>>>,
    workspace: Arc<tokio::sync::OnceCell<i64>>,
    channel_verifier: std::sync::OnceLock<Arc<dyn agena_runtime_contracts::ChannelAccessVerifier>>,
    memory_preferences: Arc<std::sync::OnceLock<Arc<dyn agena_assistant::CompanionRepository>>>,
    shutdown: CancellationToken,
}

impl SessionManager {
    /// Instance-owned memory namespace for sessions without a project.
    pub fn global_memory_root(&self) -> std::path::PathBuf {
        agena_storage_layout::StorageLayout::current()
            .root()
            .join("global")
    }
    pub fn new(
        kernel: Arc<agena_storage_sqlite::SqliteKernel>,
        providers: Arc<ProviderRegistry>,
        governor: ContextGovernor,
        processor: SessionProcessor,
        executor: ToolExecutor,
        config: RuntimeSessionManagerConfig,
    ) -> Self {
        let initial = Arc::new(ExecutionState {
            providers,
            governor,
            processor,
            executor,
            tools: Arc::new(Semaphore::new(config.max_concurrent_tools.max(1))),
            config,
        });
        Self {
            kernel_runtime: kernel,
            acp_scope: Default::default(),
            acp_workspace_source: None,
            state: Arc::new(ArcSwap::from(initial.clone())),
            global_state: Arc::new(ArcSwap::from(initial)),
            configuration_lock: Default::default(),
            prompt_hydrator: KernelPromptHydrator::default(),
            active: Default::default(),
            admission: Arc::new(Semaphore::new(MAX_ACTIVE_EXECUTIONS)),
            agent_settlements: Arc::new(Semaphore::new(MAX_ACTIVE_EXECUTIONS)),
            auxiliary_tasks: Arc::new(Semaphore::new(MAX_AUXILIARY_TASKS)),
            control_tasks: tokio_util::task::TaskTracker::new(),
            attempt_controls: Default::default(),
            sessions: Default::default(),
            external_idle_peers: Default::default(),
            external_agent_inputs: Default::default(),
            tools: Default::default(),
            operator_calls: Default::default(),
            host_sequences: Default::default(),
            agent_preparation: Default::default(),
            pending_agent_settlements: Default::default(),
            delivery_scan: Default::default(),
            bot_operations: Default::default(),
            workspace: Default::default(),
            channel_verifier: Default::default(),
            memory_preferences: Default::default(),
            shutdown: CancellationToken::new(),
        }
    }
    pub fn install_channel_verifier(
        &self,
        verifier: Arc<dyn agena_runtime_contracts::ChannelAccessVerifier>,
    ) {
        let _ = self.channel_verifier.set(verifier);
    }
    /// Access-control assignments are instance-owned and reloaded atomically.
    pub fn access_control_config(&self) -> agena_domain::AccessControlConfig {
        self.global_state.load().config.access_control.clone()
    }
    pub async fn verify_channel_session(&self, id: i64) -> Result<(), AppError> {
        let session = session_id(id)?;
        let config = self
            .kernel()
            .reader
            .configuration(session)
            .await
            .map_err(kernel_error)?;
        if let Some(authority) = self
            .channel_authority_for_execution(
                session,
                self.running_execution(session).await?,
                config.channel_authority.as_ref(),
                config.relation_kind == agena_domain::SessionRelationKind::Subagent,
            )
            .await?
        {
            self.channel_verifier
                .get()
                .ok_or_else(|| {
                    AppError::Config("channel authorization service is unavailable".into())
                })?
                .authorize(&authority)
                .await
                .map_err(AppError::Config)?;
        }
        Ok(())
    }

    /// Install instance-only execution defaults after a fully prepared runtime reload.
    pub fn reconfigure_global(
        &self,
        providers: Arc<ProviderRegistry>,
        processor: SessionProcessor,
        executor: ToolExecutor,
        config: RuntimeSessionManagerConfig,
    ) {
        let project = self.state.load_full();
        self.global_state.store(Arc::new(ExecutionState {
            providers,
            governor: project.governor.clone(),
            processor,
            executor,
            tools: Arc::new(Semaphore::new(config.max_concurrent_tools.max(1))),
            config,
        }));
    }

    async fn state_for_session(&self, session: SessionId) -> Result<Arc<ExecutionState>, AppError> {
        let metadata = read_with_busy_retry(session, "metadata", || {
            self.kernel().reader.metadata(session)
        })
        .await?;
        let _guard = self.configuration_lock.lock();
        Ok(if metadata.workspace_id.is_none() {
            self.global_state.load_full()
        } else {
            self.state.load_full()
        })
    }

    /// Publish both execution scopes as one configuration change for session routing.
    pub fn reconfigure_scopes(
        &self,
        governor: ContextGovernor,
        project: SessionScopeReconfiguration,
        global: SessionScopeReconfiguration,
    ) {
        let _guard = self.configuration_lock.lock();
        self.reconfigure(
            project.providers,
            governor,
            project.processor,
            project.executor,
            project.config,
        );
        self.reconfigure_global(
            global.providers,
            global.processor,
            global.executor,
            global.config,
        );
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
            executor,
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

    pub async fn enable_acp_scope(&self) -> Result<(), AppError> {
        self.acp_scope
            .get_or_try_init(|| async {
                let kernel = agena_storage_sqlite::open_acp_kernel()
                    .await
                    .map_err(kernel_error)?;
                let state = self.state.load_full();
                let mut scope = Self::new(
                    Arc::new(kernel),
                    state.providers.clone(),
                    state.governor.clone(),
                    state.processor.clone(),
                    state.executor.clone(),
                    state.config.clone(),
                );
                scope.state = self.state.clone();
                scope.global_state = self.global_state.clone();
                scope.configuration_lock = self.configuration_lock.clone();
                scope.acp_workspace_source = Some(self.kernel_runtime.clone());
                scope.shutdown = self.shutdown.child_token();
                Ok::<_, AppError>(Arc::new(scope))
            })
            .await?;
        Ok(())
    }

    pub fn acp_scope(&self) -> Option<Arc<Self>> {
        self.acp_scope.get().cloned()
    }

    pub fn acp_permission_rules(&self) -> Arc<dyn agena_storage::PermissionRuleRepository> {
        self.kernel_runtime.permissions.clone()
    }

    pub(super) fn is_acp_scope(&self) -> bool {
        self.acp_workspace_source.is_some()
    }

    async fn permission_rule_rows(
        &self,
        key: Option<&str>,
        session: Option<i64>,
        workspace: Option<i64>,
    ) -> Result<
        Vec<agena_storage::PersistedPermissionRule>,
        agena_storage::PermissionRuleRepositoryError,
    > {
        let mut rules = match (&self.acp_workspace_source, key) {
            (Some(source), Some(key)) => source.permissions.resolve(key, None, workspace).await?,
            (Some(source), None) => source.permissions.resolve_snapshot(None, workspace).await?,
            (None, _) => Vec::new(),
        };
        let local = &self.kernel_runtime.permissions;
        let local_rules = match key {
            Some(key) => local.resolve(key, session, workspace).await?,
            None => local.resolve_snapshot(session, workspace).await?,
        };
        rules.extend(local_rules);
        Ok(rules)
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
    ) -> impl std::future::Future<Output = ()> + Send + 'static + use<> {
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
    ) -> impl std::future::Future<Output = ()> + Send + 'static + use<> {
        let executor = self.tool_executor();
        async move {
            executor
                .plugin_manager()
                .broadcast_session_end(agena_plugin_host::SessionEndInput { session_id, reason })
                .await;
        }
    }
    async fn stop_session_lsp(&self, session: i64) {
        let project = self.state.load().executor.lsp_registry().cloned();
        let global = self.global_state.load().executor.lsp_registry().cloned();
        for registry in [project, global].into_iter().flatten() {
            registry.stop_session(session).await;
        }
    }

    /// Close admission and cancel owned work without dropping its supervisors.
    /// The asynchronous drain remains responsible for fencing/persisting it.
    pub fn request_shutdown(&self) {
        self.shutdown.cancel();
        self.control_tasks.close();
        if let Some(acp) = self.acp_scope.get() {
            acp.request_shutdown();
        }
    }

    pub async fn shutdown(&self) -> Result<(), AppError> {
        self.request_shutdown();
        if let Some(acp) = self.acp_scope.get() {
            let (main, external) = tokio::join!(self.drain_shutdown(), acp.drain_shutdown());
            main?;
            external?;
            return acp.session_kernel().shutdown().await.map_err(kernel_error);
        }
        self.drain_shutdown().await
    }

    async fn drain_shutdown(&self) -> Result<(), AppError> {
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
            self.control_tasks.wait().await;
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
        let project = self.state.load().executor.lsp_registry().cloned();
        let global = self.global_state.load().executor.lsp_registry().cloned();
        for registry in [project, global].into_iter().flatten() {
            registry.shutdown_all().await;
        }
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
        KernelError::Busy => AppError::PersistenceBusy,
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
    // Permission validators produce bounded field-oriented diagnostics that
    // callers need to correct the imported policy. Keep unrelated archive and
    // storage diagnostics behind their existing safe summaries.
    let message = match &error {
        KernelError::Invalid(detail)
            if [
                "session.permission:",
                "session.permission_ceiling:",
                "session.permission_ceiling_layers[",
                "effective session.permission:",
                "permission.network.",
                "permission.network:",
            ]
            .iter()
            .any(|prefix| detail.starts_with(prefix)) =>
        {
            detail.as_str()
        }
        _ => message,
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

/// Give required reads time to acquire capacity during a burst of concurrent
/// observers. A read view can also be too large to admit, so bound the wait.
async fn read_with_busy_retry<T, F, Fut>(
    session: SessionId,
    operation: &'static str,
    mut read: F,
) -> Result<T, AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = agena_storage::kernel::Result<T>>,
{
    use tokio::time::{Instant, sleep_until};

    let mut delay = Duration::from_millis(10);
    let mut deadline = None;
    loop {
        match read().await {
            Err(KernelError::Busy) => {
                let now = Instant::now();
                let until = *deadline.get_or_insert_with(|| {
                    tracing::debug!(
                        session_id = session.get(),
                        operation,
                        "waiting for kernel read capacity"
                    );
                    now + Duration::from_secs(2)
                });
                if now >= until {
                    tracing::warn!(
                        session_id = session.get(),
                        operation,
                        "kernel read capacity remained exhausted"
                    );
                    return Err(AppError::PersistenceBusy);
                }
                sleep_until((now + delay).min(until)).await;
                delay = (delay * 2).min(Duration::from_millis(100));
            }
            result => return result.map_err(kernel_error),
        }
    }
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
