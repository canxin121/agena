//! Concrete `HostClient` impl backed by the live `AgenaRuntime`. Plugins
//! that run as subprocess (stdio) or remote (HTTP) call back into this via
//! JSON-RPC; the `HostHandle` in `agena-plugin-host` routes those calls
//! through this client.

use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use agena_plugin_sdk::PluginKey;
use async_trait::async_trait;

use crate::part::AskUserToolInput;
use crate::plugins::storage::{
    PluginSecretStore, PluginStorage, PluginStorageError, StorageLocator,
};
use crate::runtime::AgenaRuntime;
use crate::tool::{MonitorError, MonitorReadParams, MonitorStartParams};
use agena_domain::ToolInvocation;
use agena_domain::{
    AccessKind, PathKind, PermissionDecision, StructuredObject, UserInputOption, UserInputQuestion,
};
use agena_plugin_host::sdk::host_api::{
    AgentHandle, AgentMessageReceipt, AgentMessageRequest, AgentSpawnRequest, AgentTargetRequest,
    AgentWaitRequest, AgentWaitResponse, AskUserRequest, AskUserResponse, EventSubscription,
    HostCallbackContext, HostClient, HostConfigReloadRequestResponse,
    HostConfigReloadStatusRequest, HostConfigReloadStatusResponse, HostContextStatusRequest,
    HostContextStatusResponse, HostGetSessionRequest, HostGetSessionResponse,
    HostImageExecuteRequest, HostImageExecuteResponse, HostImageOperation, HostLspDiagnostic,
    HostLspListDiagnosticsRequest, HostLspListDiagnosticsResponse, HostLspListServersResponse,
    HostLspServer, HostMcpAddServerRequest, HostMcpListServersResponse, HostMcpRemoveServerRequest,
    HostMcpRemoveServerResponse, HostMcpServerSpec, HostPluginStatus, HostPluginStatusGetRequest,
    HostPluginStatusGetResponse, HostPluginStatusListResponse, HostRenameSessionRequest,
    HostRenameSessionResponse, HostSchedulerCreateRequest, HostSchedulerCreateResponse,
    HostSchedulerDeleteRequest, HostSchedulerDeleteResponse, HostSchedulerJob,
    HostSchedulerListResponse, HostSecretDeleteRequest, HostSecretGetRequest,
    HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest, HostSession,
    HostSetSessionModelRequest, HostSetSessionModelResponse, HostStorageDeleteRequest,
    HostStorageGetRequest, HostStorageGetResponse, HostStorageListRequest, HostStorageListResponse,
    HostStorageRecord, HostStorageSetRequest, LogLevel, MonitorEvent, MonitorHandle,
    MonitorReadRequest, MonitorReadResponse, MonitorStartRequest, MonitorStopRequest,
    PathPermissionQuery, PermissionQuery, ToolDescriptor, current_host_callback_context,
};
use agena_plugin_host::{
    EventEnvelope, EventFilter as PluginEventFilter, PluginError, ToolInvokeOutput,
};

mod image;
mod mappers;

use image::*;
use mappers::*;

pub fn install_plugin_host_event_publisher(
    host_handle: Arc<agena_plugin_host::host::HostHandle>,
    runtime: &AgenaRuntime,
) {
    let live_signals = Arc::downgrade(&runtime.live_signals);
    let listener = Arc::new(
        move |event: agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent| {
            if let Some(live_signals) = live_signals.upgrade() {
                live_signals.emit(agena_runtime::RuntimeLiveSignal::ToolRegistryChanged(
                    Box::new(event),
                ));
            }
        },
    );
    host_handle.set_tool_registry_event_listener(Some(listener));
}

fn plugin_error(error: impl std::error::Error + 'static) -> PluginError {
    PluginError::internal_error(&error)
}

fn plugin_error_from_app(error: crate::AppError) -> PluginError {
    match error {
        crate::AppError::PolicyDenied(denial) => {
            let mut error = PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                denial.reason.clone(),
            );
            error.diagnostic.data = Some(serde_json::json!({ "denial": denial }));
            error
        }
        crate::AppError::UserDeclined(decline) => {
            let reason = decline
                .reason
                .clone()
                .unwrap_or_else(|| "the user declined the permission request".to_string());
            let mut error = PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::UserDeclined,
                reason,
            );
            error.diagnostic.data = Some(serde_json::json!({ "decline": decline }));
            error
        }
        crate::AppError::CapabilityUnavailable(unavailable) => {
            let mut error = PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::CapabilityUnavailable,
                unavailable.reason.clone(),
            );
            error.diagnostic.data = Some(serde_json::json!({ "unavailable": unavailable }));
            error
        }
        crate::AppError::ToolUnavailable(unavailable) => {
            let mut error = PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::ToolUnavailable,
                unavailable.reason.clone(),
            );
            error.diagnostic.data = Some(serde_json::json!({ "unavailable": unavailable }));
            error
        }
        crate::AppError::Tool(error) => error.into_plugin_error(),
        other => PluginError::internal(other.to_string()),
    }
}

pub(super) struct RuntimeHostClient {
    generation: u64,
    config: serde_json::Value,
    runtime: OnceLock<super::builder::WeakAgenaRuntime>,
}

impl RuntimeHostClient {
    pub(super) fn candidate(generation: u64, config: serde_json::Value) -> Arc<Self> {
        Arc::new(Self {
            generation,
            config,
            runtime: OnceLock::new(),
        })
    }

    pub(super) fn bind_runtime(&self, runtime: &AgenaRuntime) {
        self.runtime.get_or_init(|| runtime.downgrade());
    }

    fn runtime(&self) -> Result<AgenaRuntime, PluginError> {
        let runtime = self
            .runtime
            .get()
            .and_then(|runtime| runtime.upgrade())
            .ok_or_else(|| {
                host_unavailable("runtime services are not ready or have been released")
            })?;
        if runtime.current_snapshot().generation() < self.generation {
            return Err(host_unavailable(
                "runtime services are not ready for this candidate generation",
            ));
        }
        Ok(runtime)
    }

    fn snapshot(&self) -> Result<Arc<crate::runtime::RuntimeSnapshot>, PluginError> {
        Ok(self.runtime()?.current_snapshot())
    }

    fn session_manager(&self) -> Result<Arc<crate::session::SessionManager>, PluginError> {
        self.snapshot()?
            .session_manager()
            .ok_or_else(|| host_unavailable("session manager is not enabled in this runtime"))
    }

    fn optional_session_manager(&self) -> Option<Arc<crate::session::SessionManager>> {
        self.snapshot()
            .ok()
            .and_then(|snapshot| snapshot.session_manager())
    }

    fn tool_executor(&self) -> Result<crate::tool::ToolExecutor, PluginError> {
        Ok(self.session_manager()?.tool_executor())
    }

    async fn use_session_manager<T, E, F>(
        &self,
        use_manager: impl FnOnce(Arc<crate::session::SessionManager>) -> F,
    ) -> Result<T, PluginError>
    where
        E: std::error::Error + 'static,
        F: Future<Output = Result<T, E>>,
    {
        use_manager(self.session_manager()?)
            .await
            .map_err(plugin_error)
    }

    fn snapshot_feature<T>(
        &self,
        feature: impl FnOnce(&crate::runtime::RuntimeSnapshot) -> Option<T>,
        unavailable: &'static str,
    ) -> Result<T, PluginError> {
        let snapshot = self.snapshot()?;
        feature(snapshot.as_ref()).ok_or_else(|| host_unavailable(unavailable))
    }

    fn executor_feature<T>(
        &self,
        feature: impl FnOnce(&crate::tool::ToolExecutor) -> Option<T>,
        unavailable: &'static str,
    ) -> Result<(crate::tool::ToolExecutor, T), PluginError> {
        let executor = self.tool_executor()?;
        let feature = feature(&executor).ok_or_else(|| host_unavailable(unavailable))?;
        Ok((executor, feature))
    }

    async fn callback_session_context(
        &self,
    ) -> Result<Option<crate::session::model::SessionExecutionContext>, PluginError> {
        let Some(session_id) =
            current_host_callback_context().and_then(|context| context.session_id)
        else {
            return Ok(None);
        };
        if session_id < 0 {
            return Ok(None);
        }
        let Some(manager) = self.optional_session_manager() else {
            return Ok(None);
        };
        let session = manager
            .get_session(session_id)
            .await
            .map_err(plugin_error)?;
        Ok(Some(session.execution.clone()))
    }

    async fn callback_scoped_tool_executor(
        &self,
    ) -> Result<
        (
            crate::tool::ToolExecutor,
            Option<crate::session::model::SessionExecutionContext>,
        ),
        PluginError,
    > {
        let executor = self.tool_executor()?;
        let session_context = self.callback_session_context().await?;
        let executor = match session_context.as_ref() {
            Some(context) => executor.for_session_context_async(context).await,
            None => executor,
        };
        Ok((executor, session_context))
    }

    fn callback_monitor_owner(
        &self,
        executor: &crate::tool::ToolExecutor,
    ) -> Result<agena_runtime_tools::TerminalOwner, PluginError> {
        let session = self
            .callback_context()?
            .session_id
            .ok_or_else(|| host_unavailable("monitor access requires a session"))?;
        Ok(agena_runtime_tools::TerminalOwner {
            workspace: executor.workspace_root().to_owned(),
            session_id: Some(session),
        })
    }

    fn require_monitor_owner(
        &self,
        executor: &crate::tool::ToolExecutor,
        registry: &dyn agena_runtime_tools::MonitorService,
        id: &str,
    ) -> Result<(), PluginError> {
        if registry.is_owned(id, &self.callback_monitor_owner(executor)?) {
            return Ok(());
        }
        Err(PluginError::from_kind(
            agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
            "monitor belongs to another session or is unavailable",
        ))
    }

    async fn require_callback_invocation(
        &self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<(), PluginError> {
        let (session, call) = self.callback_session_and_call()?;
        let input = StructuredObject::try_from(input).map_err(PluginError::invalid_params)?;
        agena_plugin_host::sdk::permission_wait::waiting(
            self.session_manager()?.require_host_invocation_permission(
                session,
                call,
                ToolInvocation::new(name, input),
            ),
        )
        .await
        .map_err(plugin_error_from_app)
    }

    fn callback_context(&self) -> Result<HostCallbackContext, PluginError> {
        current_host_callback_context()
            .ok_or_else(|| host_unavailable("host callback context is not available"))
    }

    fn callback_session_and_call(&self) -> Result<(i64, i64), PluginError> {
        let context = self.callback_context()?;
        let session_id = context
            .session_id
            .ok_or_else(|| host_unavailable("host callback context is missing session_id"))?;
        let call_id = context
            .call_id
            .ok_or_else(|| host_unavailable("host callback context is missing call_id"))?;
        Ok((session_id, call_id))
    }

    fn callback_plugin_key(&self) -> Result<PluginKey, PluginError> {
        let plugin_id = self
            .callback_context()?
            .plugin_id
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| host_unavailable("host callback context is missing plugin_id"))?;
        plugin_id
            .parse()
            .map_err(|err| host_unavailable(format!("invalid host callback plugin_id: {err}")))
    }

    fn storage_locator(
        &self,
        scope: agena_plugin_host::sdk::host_api::HostStorageScope,
        visibility: agena_plugin_host::sdk::host_api::HostStorageVisibility,
    ) -> Result<StorageLocator, PluginError> {
        let plugin_id = self.callback_plugin_key()?;
        let context = self.callback_context()?;
        StorageLocator::new(
            scope,
            visibility,
            plugin_id,
            context.session_id,
            context.workspace_root,
        )
        .map_err(map_storage_error)
    }

    fn plugin_storage(&self) -> Result<Arc<dyn PluginStorage>, PluginError> {
        Ok(self.snapshot()?.plugin_storage())
    }

    fn plugin_secret_store(&self) -> Result<Arc<dyn PluginSecretStore>, PluginError> {
        Ok(self.snapshot()?.plugin_secret_store())
    }

    fn plugin_manager(&self) -> Result<Arc<agena_plugin_host::PluginHost>, PluginError> {
        Ok(self.snapshot()?.plugin_manager())
    }

    async fn use_plugin_storage<T: Send + 'static>(
        &self,
        scope: agena_plugin_host::sdk::host_api::HostStorageScope,
        visibility: agena_plugin_host::sdk::host_api::HostStorageVisibility,
        use_store: impl FnOnce(&dyn PluginStorage, &StorageLocator) -> Result<T, PluginStorageError>
        + Send
        + 'static,
    ) -> Result<T, PluginError> {
        let locator = self.storage_locator(scope, visibility)?;
        let store = self.plugin_storage()?;
        // Resolve callback-scoped authorization on this Tokio task before
        // moving the explicit store identity to a blocking worker.
        crate::blocking::PLUGIN_STORAGE
            .run(move || use_store(store.as_ref(), &locator))
            .await
            .map_err(plugin_error)?
            .map_err(map_storage_error)
    }

    async fn use_plugin_secret_store<T: Send + 'static>(
        &self,
        use_store: impl FnOnce(&dyn PluginSecretStore, &PluginKey) -> Result<T, PluginStorageError>
        + Send
        + 'static,
    ) -> Result<T, PluginError> {
        let plugin_id = self.callback_plugin_key()?;
        let store = self.plugin_secret_store()?;
        crate::blocking::PLUGIN_SECRETS
            .run(move || use_store(store.as_ref(), &plugin_id))
            .await
            .map_err(plugin_error)?
            .map_err(map_storage_error)
    }

    fn callback_or_requested_session_id(
        &self,
        requested: Option<i64>,
        action: &str,
    ) -> Result<i64, PluginError> {
        match requested {
            Some(session_id) => Ok(session_id),
            None => self.callback_context()?.session_id.ok_or_else(|| {
                host_unavailable(format!(
                    "host callback context is missing session_id for {action}"
                ))
            }),
        }
    }
}

#[async_trait]
impl HostClient for RuntimeHostClient {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value) {
        match level {
            LogLevel::Trace => {
                tracing::trace!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Debug => {
                tracing::debug!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Info => {
                tracing::info!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Warn => {
                tracing::warn!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Error => {
                tracing::error!(target: "plugin", ?fields, "{message}");
            }
        }
        let plugin_id = current_host_callback_context()
            .and_then(|context| context.plugin_id)
            .unwrap_or_else(|| "<unknown>".into());
        if let Ok(host) = self.plugin_manager() {
            let _ = host.append_plugin_log(
                plugin_id,
                format!("{level:?}").to_lowercase(),
                "plugin",
                message,
                fields,
            );
        }
    }

    async fn publish_event(&self, env: EventEnvelope) -> Result<(), PluginError> {
        let plugin_id = current_host_callback_context()
            .and_then(|context| context.plugin_id)
            .unwrap_or_else(|| "<unknown>".into());
        let plugin_id = plugin_id.parse().unwrap_or_else(|_| {
            agena_plugin_host::PluginKey::new("unknown", "unknown").expect("static plugin key")
        });
        // there is no persisted event log: plugin events are ephemeral live
        // signals for in-process presentation consumers, never persisted.
        self.runtime()?
            .live_signals
            .emit(agena_runtime::RuntimeLiveSignal::Plugin {
                session_id: env.session_id,
                plugin_id,
                kind_label: env.kind,
                payload: env.payload,
            });
        Ok(())
    }

    async fn publish_activity(
        &self,
        activity: agena_domain::BackgroundActivity,
    ) -> Result<(), PluginError> {
        self.runtime()?.activities.registry.upsert(activity);
        Ok(())
    }

    async fn register_activity_source(
        &self,
        kind: agena_domain::BackgroundActivityKind,
        adapter: Arc<dyn agena_plugin_sdk::activity::ActivitySourceAdapter>,
    ) -> Result<(), PluginError> {
        self.runtime()?.activities.register_source(kind, adapter);
        Ok(())
    }

    async fn subscribe_events(
        &self,
        _: PluginEventFilter,
    ) -> Result<EventSubscription, PluginError> {
        // Translate the SDK filter to agena's filter and confirm; the actual
        // event push back to the plugin already happens via the snapshot's
        // `event_bridge`. Returning a deterministic id so plugins can ack.
        let id = format!("sub-{}", uuid::Uuid::new_v4().simple());
        Ok(EventSubscription { id })
    }

    async fn read_config(&self, path: Option<String>) -> Result<serde_json::Value, PluginError> {
        agena_domain::get_json_path(&self.config, path.as_deref())
            .map_err(|e| PluginError::invalid_params_error(&e))
    }

    // ---------------- permission queries ----------------

    /// Answer one path-access question from the effective policy.
    ///
    /// The policy is the session's compiled execution principal when the call
    /// carries a session context, and the process-wide principal otherwise.
    /// Both are the same policies the host enforces, so the answer matches
    /// what a host tool would get — including a `Deny` that a host tool call
    /// would also hit. It never waits for a human: an approval that is not
    /// already granted reads as `Ask`.
    async fn check_path_permission(
        &self,
        request: PathPermissionQuery,
    ) -> Result<PermissionQuery, PluginError> {
        let (executor, context) = self.callback_scoped_tool_executor().await?;
        let access = match request.kind {
            PathKind::Read => AccessKind::Read,
            PathKind::Write => AccessKind::Write,
        };
        let check = executor.path_permission_check(access, &request.path);
        let decision = self
            .session_manager()?
            .query_host_permission(
                context
                    .as_ref()
                    .and_then(|context| context.scope_session_id),
                check,
            )
            .await
            .map_err(plugin_error_from_app)?;
        Ok(permission_query(decision))
    }

    async fn check_network_permission(
        &self,
        target: String,
    ) -> Result<PermissionQuery, PluginError> {
        let (executor, context) = self.callback_scoped_tool_executor().await?;
        let check = executor
            .network_permission_check(&target)
            .map_err(|error| error.into_plugin_error())?;
        let decision = self
            .session_manager()?
            .query_host_permission(
                context
                    .as_ref()
                    .and_then(|context| context.scope_session_id),
                check,
            )
            .await
            .map_err(plugin_error_from_app)?;
        Ok(permission_query(decision))
    }

    async fn require_path_permission(
        &self,
        request: PathPermissionQuery,
    ) -> Result<(), PluginError> {
        let (session, call) = self.callback_session_and_call()?;
        self.session_manager()?
            .require_host_path_permission(session, call, request.kind, request.path)
            .await
            .map_err(plugin_error_from_app)
    }

    async fn require_network_permission(&self, target: String) -> Result<(), PluginError> {
        let (session, call) = self.callback_session_and_call()?;
        self.session_manager()?
            .require_host_network_permission(session, call, target)
            .await
            .map_err(plugin_error_from_app)
    }

    async fn request_config_reload(&self) -> Result<HostConfigReloadRequestResponse, PluginError> {
        let runtime = self.runtime()?;
        let completion = agena_plugin_host::current_plugin_call_completion()
            .ok_or_else(|| host_unavailable("reload requests require an admitted plugin call"))?;
        let start = runtime
            .start_plugin_reload_task(completion)
            .map_err(|error| {
                let mut failure = PluginError::from_kind_with_public_detail(
                    agena_plugin_sdk::PluginErrorKind::HostUnavailable,
                    &error,
                    error.to_string(),
                );
                if matches!(
                    error,
                    crate::RuntimeBackgroundTaskControlError::Capacity { .. }
                ) {
                    failure.failure.recovery = agena_failure::RecoveryDirective::Retry;
                }
                failure
            })?;
        Ok(HostConfigReloadRequestResponse {
            task_id: start.task.id,
            started: start.started,
        })
    }

    async fn config_reload_status(
        &self,
        request: HostConfigReloadStatusRequest,
    ) -> Result<HostConfigReloadStatusResponse, PluginError> {
        use crate::RuntimeBackgroundTaskStatus;
        use agena_plugin_sdk::host_api::HostConfigReloadState;
        let task = self
            .runtime()?
            .background_tasks()
            .into_iter()
            .find(|task| {
                task.id == request.task_id
                    && task.kind == crate::RuntimeBackgroundTaskKind::RuntimeReload
            })
            .ok_or_else(|| {
                PluginError::invalid_params("reload task is unknown or its history has expired")
            })?;
        let state = match task.status {
            RuntimeBackgroundTaskStatus::Running => HostConfigReloadState::Running {},
            RuntimeBackgroundTaskStatus::Succeeded => HostConfigReloadState::Succeeded {},
            RuntimeBackgroundTaskStatus::Cancelled => HostConfigReloadState::Cancelled {},
            RuntimeBackgroundTaskStatus::Failed => HostConfigReloadState::Failed {
                problem: task
                    .failure
                    .as_ref()
                    .ok_or_else(|| {
                        PluginError::internal("failed reload task has no failure diagnostic")
                    })?
                    .into(),
            },
        };
        Ok(HostConfigReloadStatusResponse {
            task_id: task.id,
            state,
        })
    }

    async fn invoke_tool(
        &self,
        tool: String,
        input: serde_json::Value,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let host = self.plugin_manager()?;
        let resolution = if let Some(resolution) = host.lookup_tool_for_name(&tool) {
            resolution
        } else {
            let mut candidates = host
                .registered_tools()
                .into_iter()
                .filter(|candidate| {
                    crate::tool::registered_tool_matches_name(candidate, tool.as_str())
                })
                .collect::<Vec<_>>();
            candidates.sort_by_key(|candidate| candidate.canonical_name());
            match candidates.as_slice() {
                [] => return Err(PluginError::internal(format!("tool `{tool}` not found"))),
                [resolution] => resolution.clone(),
                _ => {
                    let names = candidates
                        .iter()
                        .map(|candidate| format!("`{}`", candidate.canonical_name()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(PluginError::invalid_params(format!(
                        "tool `{tool}` is ambiguous; use one of {names}"
                    )));
                }
            }
        };

        let caller = self.callback_context()?;
        let plugin_id = resolution.plugin_full_name();
        if caller
            .plugin_id
            .as_ref()
            .is_some_and(|current| current == &plugin_id)
        {
            return Err(PluginError::internal(format!(
                "host->plugin invoke would re-enter plugin `{plugin_id}` (cycle detected)"
            )));
        }

        let session_id = caller
            .session_id
            .ok_or_else(|| host_unavailable("host/tool.invoke requires session_id"))?;
        let call_id = caller.call_id.unwrap_or(-1);
        let structured = StructuredObject::try_from(input)
            .map_err(|err| PluginError::invalid_params(format!("invoke_tool input: {err}")))?;
        let invocation = ToolInvocation::new(resolution.canonical_name(), structured);
        let _guard = agena_runtime::try_enter_invocation(session_id, call_id, plugin_id.clone())
            .ok_or_else(|| {
                PluginError::internal(format!(
                    "host->plugin invoke would re-enter plugin `{plugin_id}` (cycle detected)"
                ))
            })?;
        let execution = self
            .session_manager()?
            .execute_host_invoked_tool(session_id, call_id, invocation)
            .await
            .map_err(plugin_error_from_app)?;

        Ok(tool_execution_to_invoke_output(execution))
    }

    async fn ask_user(&self, req: AskUserRequest) -> Result<AskUserResponse, PluginError> {
        let (session_id, call_id) = self.callback_session_and_call()?;
        let input = ask_user_tool_input(req)?;
        self.session_manager()?
            .request_host_user_input(session_id, call_id, input)
            .await
            .map_err(plugin_error)
    }

    async fn spawn_agent(&self, req: AgentSpawnRequest) -> Result<AgentHandle, PluginError> {
        let (session, call) = self.callback_session_and_call()?;
        self.session_manager()?
            .spawn_agent(session, Some(call), req)
            .await
            .map_err(plugin_error)
    }

    async fn message_agent(
        &self,
        req: AgentMessageRequest,
    ) -> Result<AgentMessageReceipt, PluginError> {
        let (session, call) = self.callback_session_and_call()?;
        self.session_manager()?
            .message_agent(session, Some(call), req)
            .await
            .map_err(plugin_error)
    }

    async fn wait_agent(&self, req: AgentWaitRequest) -> Result<AgentWaitResponse, PluginError> {
        let (session, call) = self.callback_session_and_call()?;
        self.session_manager()?
            .wait_agent(session, Some(call), req)
            .await
            .map_err(plugin_error)
    }

    async fn followup_agent(&self, req: AgentMessageRequest) -> Result<AgentHandle, PluginError> {
        let (session, call) = self.callback_session_and_call()?;
        self.session_manager()?
            .followup_agent(session, Some(call), req)
            .await
            .map_err(plugin_error)
    }

    async fn cancel_agent(
        &self,
        req: AgentTargetRequest,
    ) -> Result<AgentMessageReceipt, PluginError> {
        let (session, call) = self.callback_session_and_call()?;
        self.session_manager()?
            .cancel_agent(session, Some(call), req)
            .await
            .map_err(plugin_error)
    }

    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, PluginError> {
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let tools = executor.detailed_execution_tools_async().await;
        let names = crate::tool::execution_tool_names(&tools);
        Ok(tools
            .into_iter()
            .zip(names)
            .map(|(tool, name)| {
                let mut descriptor = render_tool_descriptor(tool.into_registered());
                descriptor.name = name;
                descriptor
            })
            .collect())
    }

    async fn get_context_status(
        &self,
        req: HostContextStatusRequest,
    ) -> Result<HostContextStatusResponse, PluginError> {
        let session_id = req
            .session_id
            .or_else(|| current_host_callback_context().and_then(|context| context.session_id))
            .ok_or_else(|| PluginError::invalid_params("session id is required"))?;
        let manager = self.session_manager()?;
        let session = manager
            .get_session(session_id)
            .await
            .map_err(plugin_error)?;
        let usage = manager
            .session_usage_async(&session)
            .await
            .map_err(plugin_error)?;
        let model_status = manager
            .session_model_status(&session)
            .map_err(plugin_error)?;
        let prompt_window = manager
            .compaction_status(session_id)
            .await
            .map_err(plugin_error)?;
        Ok(HostContextStatusResponse {
            session_id,
            current_tokens: usage.current_tokens,
            measured_prompt_tokens: usage.measured_prompt_tokens,
            projected_tokens: usage.projected_tokens,
            input_limit_tokens: usage.input_limit_tokens,
            limit_tokens: usage.limit_tokens,
            remaining_tokens: usage
                .limit_tokens
                .map(|limit| limit.saturating_sub(usage.current_tokens)),
            reserved_tokens: u64::from(usage.reserved_tokens.unwrap_or_default()),
            model_context_window_tokens: usage.model_context_window_tokens,
            model_max_input_tokens: usage.model_max_input_tokens,
            model_max_output_tokens: usage.model_max_output_tokens,
            model_provider_id: model_status.provider,
            model_adapter_id: model_status.adapter,
            model_id: model_status.model,
            thinking_mode: model_status.thinking_mode,
            speed_mode: model_status.speed_mode,
            verbosity: model_status.verbosity,
            prompt_window_generation: prompt_window.generation,
            compacted: prompt_window.checkpoint_entry_id.is_some(),
            last_compaction_before_tokens: prompt_window.last_before_tokens,
            last_compaction_after_tokens: prompt_window.last_after_tokens,
            auto_compaction_disabled: prompt_window.auto_compaction_disabled,
            consecutive_compaction_failures: u8::try_from(
                prompt_window.consecutive_compaction_failures,
            )
            .unwrap_or(u8::MAX),
        })
    }

    async fn get_session(
        &self,
        req: HostGetSessionRequest,
    ) -> Result<HostGetSessionResponse, PluginError> {
        let session_id = self.callback_or_requested_session_id(req.session_id, "get_session")?;
        let session = self
            .use_session_manager(|manager| async move { manager.get_session(session_id).await })
            .await?;
        Ok(HostGetSessionResponse {
            session: host_session_from_session(&session),
        })
    }

    async fn rename_session(
        &self,
        req: HostRenameSessionRequest,
    ) -> Result<HostRenameSessionResponse, PluginError> {
        let session_id = self.callback_or_requested_session_id(req.session_id, "rename_session")?;
        let title = req.title.trim();
        if title.is_empty() {
            return Err(PluginError::invalid_params(
                "session title must not be empty",
            ));
        }
        let title = title.to_string();
        let session = self
            .use_session_manager(|manager| async move {
                manager.rename_session(session_id, title).await
            })
            .await?;
        Ok(HostRenameSessionResponse {
            session: host_session_from_session(&session),
        })
    }

    async fn set_session_model(
        &self,
        req: HostSetSessionModelRequest,
    ) -> Result<HostSetSessionModelResponse, PluginError> {
        use std::str::FromStr;

        let session_id =
            self.callback_or_requested_session_id(req.session_id, "set_session_model")?;
        let model = agena_domain::ModelRef::from_str(req.model.trim()).map_err(|error| {
            PluginError::invalid_params(format!(
                "skill model must be a fully-qualified provider/model reference: {error}"
            ))
        })?;
        let provider_id = model.provider_id.to_string();
        let adapter_id = model.adapter_id.as_ref().map(ToString::to_string);
        let model_id = model.model_id.to_string();
        let session = self
            .use_session_manager(|manager| async move {
                manager.set_session_model_override(session_id, model).await
            })
            .await?;
        Ok(HostSetSessionModelResponse {
            session: host_session_from_session(&session),
            provider_id,
            adapter_id,
            model_id,
        })
    }

    async fn builtin_provider_models(&self) -> Result<serde_json::Value, PluginError> {
        let context = current_host_callback_context()
            .ok_or_else(|| host_unavailable("model discovery requires an active tool callback"))?;
        if context.plugin_id.as_deref() != Some("agena.provider") {
            return Err(host_unavailable(
                "provider model discovery is restricted to the bundled provider plugin",
            ));
        }
        let (session_id, _) = self.callback_session_and_call()?;
        let session = self
            .session_manager()?
            .get_session(session_id)
            .await
            .map_err(plugin_error)?;
        let snapshot = self.snapshot()?;
        let selected = snapshot
            .provider_registry()
            .resolve_default_model_selection(&session.execution.selection)
            .map_err(plugin_error)?
            .ok_or_else(|| host_unavailable("select a provider before discovering models"))?;
        let provider_id = selected.provider_id.to_string();
        let config = snapshot
            .provider_configs()
            .get::<str>(&provider_id)
            .ok_or_else(|| host_unavailable("the current provider has no configuration"))?;
        let targets =
            agena_runtime_provider_adapters::config_support::provider_model_discovery_targets(
                &provider_id,
                config,
            )
            .map_err(plugin_error)?;
        for target in targets {
            self.require_network_permission(target).await?;
        }
        let discovery = snapshot
            .discover_provider_models(&provider_id)
            .await
            .map_err(plugin_error)?;
        let mut result = serde_json::to_value(discovery).map_err(plugin_error)?;
        result["provider_id"] = serde_json::json!(provider_id);
        result["current_model"] = serde_json::json!(selected.model_id);
        result["current_adapter"] = serde_json::json!(selected.adapter_id);
        result["capability_notice"] = serde_json::json!(
            "All enabled adapters of this provider are queried, subject to configured-only discovery policies. Model presence and catalog metadata do not guarantee operation or account access."
        );
        Ok(result)
    }

    async fn builtin_provider_connection(
        &self,
    ) -> Result<agena_plugin_sdk::host_api::BuiltinProviderConnection, PluginError> {
        let context = current_host_callback_context()
            .ok_or_else(|| host_unavailable("provider tools require an active tool callback"))?;
        let protocol = match context.plugin_id.as_deref() {
            Some("agena.chatgpt") => "chatgpt",
            Some("agena.claude") => "claude",
            Some("agena.gemini") => "gemini",
            _ => {
                return Err(host_unavailable(
                    "provider connection is restricted to bundled provider tools",
                ));
            }
        };
        let (session_id, _) = self.callback_session_and_call()?;
        let session = self
            .session_manager()?
            .get_session(session_id)
            .await
            .map_err(plugin_error)?;
        let snapshot = self.snapshot()?;
        let model = snapshot
            .provider_registry()
            .resolve_default_model_selection(&session.execution.selection)
            .map_err(plugin_error)?
            .ok_or_else(|| host_unavailable("select a provider before using provider tools"))?;
        let config = snapshot
            .provider_configs()
            .get::<str>(model.provider_id.as_ref())
            .ok_or_else(|| {
                host_unavailable("the current provider has no connection configuration")
            })?;
        let mut connection =
            agena_runtime_provider_adapters::config_support::resolve_provider_tool_connection(
                model.provider_id.as_ref(),
                config,
                protocol,
                model.adapter_id.as_ref().map(AsRef::as_ref),
                model.model_id.as_ref(),
                &agena_runtime_config::ProcessEnvironment,
                snapshot.client_identity(),
            )
            .await
            .map_err(plugin_error)?;
        connection
            .headers
            .insert("x-session-id".into(), session_id.to_string());
        connection
            .headers
            .insert("session_id".into(), session_id.to_string());
        Ok(agena_plugin_sdk::host_api::BuiltinProviderConnection {
            realtime_ws_url: connection.realtime_ws_url,
            provider_id: model.provider_id.to_string(),
            base_url: connection.base_url,
            messages_url: connection.messages_url,
            headers: connection.headers,
            model: connection.model,
            timeout_secs: config.network.request_timeout_secs,
            connect_timeout_secs: config.network.connect_timeout_secs,
            receipt_key: connection.receipt_key,
        })
    }

    async fn image_execute(
        &self,
        req: HostImageExecuteRequest,
    ) -> Result<HostImageExecuteResponse, PluginError> {
        let (callback_session_id, call_id) = self.callback_session_and_call()?;
        if req
            .session_id
            .is_some_and(|session_id| session_id != callback_session_id)
        {
            return Err(PluginError::invalid_params(
                "direct image requests cannot target a session other than the active tool callback session",
            ));
        }
        let manager = self.session_manager()?;
        let session = manager
            .get_session(callback_session_id)
            .await
            .map_err(plugin_error)?;
        let snapshot = self.snapshot()?;
        let provider_registry = snapshot.provider_registry();
        let model = provider_registry
            .resolve_default_model_selection(&session.execution.selection)
            .map_err(plugin_error)?
            .ok_or_else(|| {
                host_unavailable(
                    "the active session has no selected model; select a model before requesting image generation",
                )
            })?;
        let capabilities = provider_registry
            .image_capabilities(&model)
            .map_err(plugin_error)?
            .ok_or_else(|| {
                PluginError::internal(format!(
                    "selected provider/model route `{model}` does not enable a direct image generation API"
                ))
            })?;
        let operation = match req.operation {
            HostImageOperation::Generate => agena_provider::ProviderImageOperation::Generate,
            HostImageOperation::Edit => agena_provider::ProviderImageOperation::Edit,
        };
        if !capabilities.supports(operation) {
            return Err(PluginError::internal(format!(
                "selected provider/model route `{model}` does not support the requested image operation"
            )));
        }
        let prompt = req.prompt.trim();
        if prompt.is_empty() {
            return Err(PluginError::invalid_params(
                "image prompt must not be empty",
            ));
        }
        match req.operation {
            HostImageOperation::Generate if !req.inputs.is_empty() => {
                return Err(PluginError::invalid_params(
                    "image generate does not accept edit inputs",
                ));
            }
            HostImageOperation::Edit if req.inputs.is_empty() => {
                return Err(PluginError::invalid_params(
                    "image edit requires at least one input image",
                ));
            }
            _ => {}
        }
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        for input in &req.inputs {
            let path = match input {
                agena_plugin_host::sdk::host_api::HostImageInput::Path { path } => {
                    Some(path.as_str())
                }
                agena_plugin_host::sdk::host_api::HostImageInput::Attachment { attachment } => {
                    match &attachment.source {
                        agena_plugin_host::sdk::attachment::AttachmentSource::LocalPath {
                            path,
                        } => Some(path.as_str()),
                        _ => None,
                    }
                }
            };
            if let Some(path) = path {
                self.require_path_permission(PathPermissionQuery::read(path))
                    .await?;
            }
        }
        let inputs =
            prepare_provider_image_inputs(&executor, req.inputs.as_slice(), &capabilities).await?;
        let provider_request = agena_provider::ProviderImageRequest {
            operation,
            prompt: prompt.to_owned(),
            inputs,
            options: agena_provider::ProviderHostedImageGenerationConfig {
                background: req.background,
                size: req.size,
                quality: req.quality,
                moderation: req.moderation,
                // Provider-specific escape hatches stay route-config-owned;
                // the model-visible tool cannot inject arbitrary request keys.
                provider_options: None,
            },
        };
        let response = provider_registry
            .execute_image(&model, provider_request)
            .await
            .map_err(plugin_error)?;
        let workspace_root = self
            .callback_context()?
            .workspace_root
            .map(std::path::PathBuf::from)
            .ok_or_else(|| {
                host_unavailable("host callback context is missing workspace_root for image output")
            })?;
        let attachments = persist_provider_image_artifacts(
            workspace_root.as_path(),
            callback_session_id,
            call_id,
            response.artifacts,
        )
        .await?;
        Ok(HostImageExecuteResponse {
            operation: req.operation,
            provider_id: model.provider_id.to_string(),
            adapter_id: model.adapter_id.as_ref().map(ToString::to_string),
            model_id: model.model_id.to_string(),
            revised_prompt: response.revised_prompt,
            attachments,
        })
    }

    async fn monitor_start(&self, req: MonitorStartRequest) -> Result<MonitorHandle, PluginError> {
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let registry = executor.monitor_registry().cloned().ok_or_else(|| {
            host_unavailable("background process registry is not enabled in this runtime")
        })?;
        let command = join_monitor_command(&req.command)?;
        self.require_callback_invocation("agena.shell.spawn", serde_json::json!({
            "command": command, "workdir": req.cwd, "reads": req.reads, "writes": req.writes, "network": req.network,
        })).await?;
        let cwd = req
            .cwd
            .as_deref()
            .map(|path| executor.resolve_target_path(path))
            .unwrap_or_else(|| executor.workspace_root().to_path_buf());
        let env = if req.env.is_empty() {
            std::env::vars().collect()
        } else {
            req.env.into_iter().collect()
        };
        let summary = registry
            .start(MonitorStartParams {
                argv: None,
                output: None,
                owner: Some(agena_runtime_tools::TerminalOwner {
                    workspace: executor.workspace_root().to_owned(),
                    session_id: self.callback_context()?.session_id,
                }),
                process_id: None,
                description: req.label.unwrap_or_else(|| command.clone()),
                command,
                ws: None,
                workdir: cwd,
                timeout_ms: req.timeout_ms,
                persistent: req.persistent,
                monitored: true,
                watch: None,
                include_pattern: req.include_pattern,
                success_pattern: None,
                failure_pattern: None,
                quiet_period_ms: None,
                max_buffered_lines: req.max_buffered_lines,
                capture_stderr: req.capture_stderr,
                env,
            })
            .map_err(map_monitor_error)?
            .summary;
        Ok(render_monitor_handle(summary))
    }

    async fn monitor_list(&self) -> Result<Vec<MonitorHandle>, PluginError> {
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let registry = executor.monitor_registry().cloned().ok_or_else(|| {
            host_unavailable("background process registry is not enabled in this runtime")
        })?;
        let owner = self.callback_monitor_owner(&executor)?;
        Ok(registry
            .list()
            .into_iter()
            .filter(|monitor| registry.is_owned(&monitor.process_id, &owner))
            .map(render_monitor_handle)
            .collect())
    }

    async fn monitor_read(
        &self,
        req: MonitorReadRequest,
    ) -> Result<MonitorReadResponse, PluginError> {
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let registry = executor.monitor_registry().cloned().ok_or_else(|| {
            host_unavailable("background process registry is not enabled in this runtime")
        })?;
        self.require_monitor_owner(&executor, registry.as_ref(), &req.id)?;
        let read = crate::blocking::PROCESS_LOG_READS
            .run(move || {
                registry.read(MonitorReadParams {
                    monitor_id: req.id,
                    since_seq: req.since_seq,
                    limit: req.limit,
                    wait_ms: if req.follow && req.wait_ms == 0 {
                        30_000
                    } else {
                        req.wait_ms
                    },
                })
            })
            .await
            .map_err(|error| PluginError::internal(format!("monitor read worker failed: {error}")))?
            .map_err(map_monitor_error)?;
        Ok(render_monitor_read(read))
    }

    async fn monitor_stop(&self, req: MonitorStopRequest) -> Result<MonitorHandle, PluginError> {
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let registry = executor.monitor_registry().cloned().ok_or_else(|| {
            host_unavailable("background process registry is not enabled in this runtime")
        })?;
        self.require_monitor_owner(&executor, registry.as_ref(), &req.id)?;
        self.require_callback_invocation(
            "agena.shell.stop",
            serde_json::json!({"process_id":req.id}),
        )
        .await?;
        let stop = registry.stop(req.id.as_str()).map_err(map_monitor_error)?;
        Ok(render_monitor_handle(stop.summary))
    }

    async fn storage_get(
        &self,
        req: HostStorageGetRequest,
    ) -> Result<HostStorageGetResponse, PluginError> {
        let value = self
            .use_plugin_storage(req.scope, req.visibility, move |store, locator| {
                store.get(locator, req.namespace.as_str(), req.key.as_str())
            })
            .await?;
        Ok(HostStorageGetResponse { value })
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> Result<(), PluginError> {
        self.use_plugin_storage(req.scope, req.visibility, move |store, locator| {
            store.set(
                locator,
                req.namespace.as_str(),
                req.key.as_str(),
                req.value.as_str(),
            )
        })
        .await
    }

    async fn storage_delete(&self, req: HostStorageDeleteRequest) -> Result<(), PluginError> {
        self.use_plugin_storage(req.scope, req.visibility, move |store, locator| {
            store.delete(locator, req.namespace.as_str(), req.key.as_str())
        })
        .await
    }

    async fn storage_list(
        &self,
        req: HostStorageListRequest,
    ) -> Result<HostStorageListResponse, PluginError> {
        let records = self
            .use_plugin_storage(req.scope, req.visibility, move |store, locator| {
                store.list(locator, req.namespace.as_deref(), req.prefix.as_deref())
            })
            .await?
            .into_iter()
            .map(|entry| HostStorageRecord {
                namespace: entry.namespace,
                key: entry.key,
            })
            .collect();
        Ok(HostStorageListResponse { records })
    }

    async fn secret_get(
        &self,
        req: HostSecretGetRequest,
    ) -> Result<HostSecretGetResponse, PluginError> {
        let value = self
            .use_plugin_secret_store(move |store, plugin_id| {
                store.get(plugin_id, req.name.as_str())
            })
            .await?;
        Ok(HostSecretGetResponse { value })
    }

    async fn secret_set(&self, req: HostSecretSetRequest) -> Result<(), PluginError> {
        self.use_plugin_secret_store(move |store, plugin_id| {
            store.set(plugin_id, req.name.as_str(), req.value.as_str())
        })
        .await
    }

    async fn secret_delete(&self, req: HostSecretDeleteRequest) -> Result<(), PluginError> {
        self.use_plugin_secret_store(move |store, plugin_id| {
            store.delete(plugin_id, req.name.as_str())
        })
        .await
    }

    async fn secret_list(&self) -> Result<HostSecretListResponse, PluginError> {
        let names = self
            .use_plugin_secret_store(|store, plugin_id| store.list(plugin_id))
            .await?;
        Ok(HostSecretListResponse { names })
    }

    async fn plugin_status_list(&self) -> Result<HostPluginStatusListResponse, PluginError> {
        let host = self.plugin_manager()?;
        let statuses = host
            .plugin_statuses()
            .into_iter()
            .map(host_status_to_sdk)
            .collect();
        Ok(HostPluginStatusListResponse { statuses })
    }

    async fn plugin_status_get(
        &self,
        req: HostPluginStatusGetRequest,
    ) -> Result<HostPluginStatusGetResponse, PluginError> {
        let host = self.plugin_manager()?;
        Ok(HostPluginStatusGetResponse {
            status: host
                .plugin_status_by_key(&req.plugin_id)
                .map(host_status_to_sdk),
        })
    }

    async fn lsp_list_servers(&self) -> Result<HostLspListServersResponse, PluginError> {
        let (_, registry) = self.executor_feature(
            |executor| executor.lsp_registry().cloned(),
            "lsp registry is not enabled in this runtime",
        )?;
        let specs = registry
            .server_statuses()
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let servers = specs
            .into_iter()
            .map(|status| HostLspServer {
                name: status.spec.name,
                command: status.spec.command,
                args: status.spec.args,
                file_extensions: status.spec.file_extensions,
                root_markers: status.spec.root_markers,
                command_available: status.command_available,
                executable: status.executable.map(|path| path.display().to_string()),
                running_roots: status
                    .running_roots
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
            })
            .collect();
        Ok(HostLspListServersResponse { servers })
    }

    async fn lsp_list_diagnostics(
        &self,
        req: HostLspListDiagnosticsRequest,
    ) -> Result<HostLspListDiagnosticsResponse, PluginError> {
        let (_, registry) = self.executor_feature(
            |executor| executor.lsp_registry().cloned(),
            "lsp registry is not enabled in this runtime",
        )?;
        let pairs = registry.collect_diagnostics().await;
        let mut diagnostics_out = Vec::new();
        for (uri, diagnostics) in pairs {
            if let Some(filter) = req.uri.as_ref()
                && filter != &uri
            {
                continue;
            }
            let path = url::Url::parse(&uri)
                .ok()
                .and_then(|uri| uri.to_file_path().ok())
                .ok_or_else(|| {
                    PluginError::invalid_params("LSP diagnostics must reference a local file URI")
                })?;
            self.require_path_permission(PathPermissionQuery::read(path.to_string_lossy()))
                .await?;
            for diagnostic in diagnostics {
                diagnostics_out.push(HostLspDiagnostic {
                    uri: uri.clone(),
                    severity: lsp_severity_string(diagnostic.severity),
                    message: diagnostic.message,
                    start_line: diagnostic.range.start.line,
                    start_character: diagnostic.range.start.character,
                    end_line: diagnostic.range.end.line,
                    end_character: diagnostic.range.end.character,
                    source: diagnostic.source,
                    code: diagnostic.code.map(|code| match code {
                        agena_lsp::lsp_types::NumberOrString::Number(n) => n.to_string(),
                        agena_lsp::lsp_types::NumberOrString::String(s) => s,
                    }),
                });
            }
        }
        Ok(HostLspListDiagnosticsResponse {
            diagnostics: diagnostics_out,
        })
    }

    async fn scheduler_list(&self) -> Result<HostSchedulerListResponse, PluginError> {
        let (_, scheduler) = self.executor_feature(
            |executor| executor.scheduler().cloned(),
            "scheduler is not enabled in this runtime",
        )?;
        self.require_callback_invocation("agena.cron.list", serde_json::json!({}))
            .await?;
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let workspace = tokio::fs::canonicalize(executor.workspace_root())
            .await
            .map_err(plugin_error)?
            .display()
            .to_string();
        let session = self.callback_context()?.session_id;
        let jobs = scheduler
            .list_owned(&workspace, session)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let entries = jobs.into_iter().map(scheduler_job_to_sdk).collect();
        Ok(HostSchedulerListResponse { jobs: entries })
    }

    async fn scheduler_create(
        &self,
        req: HostSchedulerCreateRequest,
    ) -> Result<HostSchedulerCreateResponse, PluginError> {
        let (_, scheduler) = self.executor_feature(
            |executor| executor.scheduler().cloned(),
            "scheduler is not enabled in this runtime",
        )?;
        let (session, _) = self.callback_session_and_call()?;
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let (owner, prompt, expression) = match &req {
            HostSchedulerCreateRequest::Cron {
                owner_session_id,
                prompt,
                expression,
                ..
            } => (*owner_session_id, prompt.clone(), expression.clone()),
            HostSchedulerCreateRequest::Once {
                owner_session_id,
                prompt,
                ..
            } => (*owner_session_id, prompt.clone(), "0 * * * * *".to_owned()),
        };
        if owner.is_some_and(|owner| owner != session) {
            return Err(PluginError::invalid_params(
                "schedules must belong to the callback session",
            ));
        }
        self.require_callback_invocation(
            "agena.cron.create",
            serde_json::json!({"expression":expression,"prompt":prompt,"timezone":"UTC"}),
        )
        .await?;
        let mut job = match req {
            HostSchedulerCreateRequest::Cron {
                expression,
                prompt,
                max_age_days,
                owner_session_id,
            } => {
                let mut job = agena_scheduler::ScheduledJob::new_cron(
                    expression,
                    prompt,
                    max_age_days.unwrap_or(7),
                )
                .map_err(|error| PluginError::invalid_params_error(&error))?;
                if let Some(session) = owner_session_id {
                    job.set_owner(session);
                }
                job
            }
            HostSchedulerCreateRequest::Once {
                at_ms,
                prompt,
                owner_session_id,
            } => {
                let at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(at_ms)
                    .ok_or_else(|| PluginError::invalid_params("invalid at_ms"))?;
                let mut job = agena_scheduler::ScheduledJob::new_once(at, prompt);
                if let Some(session) = owner_session_id {
                    job.set_owner(session);
                }
                job
            }
        };
        job.set_owner(session);
        job.owner_workspace = Some(
            tokio::fs::canonicalize(executor.workspace_root())
                .await
                .map_err(plugin_error)?
                .display()
                .to_string(),
        );
        let id = job.id;
        scheduler
            .add(job)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        Ok(HostSchedulerCreateResponse { id: id.to_string() })
    }

    async fn scheduler_delete(
        &self,
        req: HostSchedulerDeleteRequest,
    ) -> Result<HostSchedulerDeleteResponse, PluginError> {
        let (_, scheduler) = self.executor_feature(
            |executor| executor.scheduler().cloned(),
            "scheduler is not enabled in this runtime",
        )?;
        let id = uuid::Uuid::parse_str(&req.id)
            .map_err(|err| PluginError::invalid_params(format!("invalid scheduler id: {err}")))?;
        self.require_callback_invocation("agena.cron.delete", serde_json::json!({"id":req.id}))
            .await?;
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let workspace = tokio::fs::canonicalize(executor.workspace_root())
            .await
            .map_err(plugin_error)?
            .display()
            .to_string();
        let removed = scheduler
            .remove_owned(&workspace, self.callback_context()?.session_id, id)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        Ok(HostSchedulerDeleteResponse { removed })
    }

    async fn mcp_list_servers(&self) -> Result<HostMcpListServersResponse, PluginError> {
        let manager = self.snapshot_feature(
            |snapshot| snapshot.mcp_manager(),
            "mcp manager is not enabled in this runtime",
        )?;
        let servers = manager.server_names().await;
        Ok(HostMcpListServersResponse { servers })
    }

    async fn mcp_add_server(&self, req: HostMcpAddServerRequest) -> Result<(), PluginError> {
        let manager = self.snapshot_feature(
            |snapshot| snapshot.mcp_manager(),
            "mcp manager is not enabled in this runtime",
        )?;
        let spec = match req.spec {
            HostMcpServerSpec::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let shell_command = join_monitor_command(
                    &std::iter::once(command.clone())
                        .chain(args.iter().cloned())
                        .collect::<Vec<_>>(),
                )?;
                self.require_callback_invocation("agena.shell.spawn", serde_json::json!({"command":shell_command,"workdir":cwd,"reads":[],"writes":[],"network":[]})).await?;
                agena_mcp_client::ServerSpec::Stdio {
                    command,
                    args,
                    env: env.into_iter().collect(),
                    cwd: cwd.map(std::path::PathBuf::from),
                    tool_policy: Default::default(),
                }
            }
            HostMcpServerSpec::Http {
                url,
                bearer,
                headers,
            } => {
                self.require_network_permission(url.clone()).await?;
                let url = url::Url::parse(&url)
                    .map_err(|e| PluginError::invalid_params(format!("invalid mcp url: {e}")))?;
                let auth = bearer.map(agena_mcp_client::HttpAuth::Bearer);
                agena_mcp_client::ServerSpec::Http {
                    url,
                    headers: headers.into_iter().collect(),
                    auth,
                    tool_policy: Default::default(),
                }
            }
        };
        manager.add_server(&req.name, spec).await.map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "mcp.add_server failed",
                &error,
            ))
        })
    }

    async fn mcp_remove_server(
        &self,
        req: HostMcpRemoveServerRequest,
    ) -> Result<HostMcpRemoveServerResponse, PluginError> {
        let manager = self.snapshot_feature(
            |snapshot| snapshot.mcp_manager(),
            "mcp manager is not enabled in this runtime",
        )?;
        match manager.remove_server(&req.name).await {
            Ok(()) => Ok(HostMcpRemoveServerResponse { removed: true }),
            Err(agena_mcp_client::McpError::ServerNotConnected(_)) => {
                Ok(HostMcpRemoveServerResponse { removed: false })
            }
            Err(error) => Err(PluginError::internal(
                agena_failure::diagnostic::format_error_chain_with_context(
                    "mcp.remove_server failed",
                    &error,
                ),
            )),
        }
    }
}
