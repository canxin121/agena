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
use agena_plugin_sdk::host_api::{ImportedPluginBackground, ImportedPluginNotices};
use async_trait::async_trait;

use crate::part::AskUserToolInput;
use crate::plugins::storage::{
    PluginSecretStore, PluginStorage, PluginStorageError, StorageLocator,
};
use crate::runtime::AgenaRuntime;
use crate::tool::{MonitorError, MonitorReadParams, MonitorStartParams};
use agena_domain::ToolInvocation;
use agena_domain::mcp_app_tools::McpAppViewScope;
use agena_domain::mcp_elicitation::{McpElicitation, McpElicitationReply};
use agena_domain::{
    AccessKind, PathKind, PermissionDecision, StructuredObject, UserInputOption, UserInputQuestion,
};
use agena_plugin_host::sdk::host_api::{
    AgentHandle, AgentMessageReceipt, AgentMessageRequest, AgentSpawnRequest, AgentTargetRequest,
    AgentWaitRequest, AgentWaitResponse, AskUserRequest, AskUserResponse, EventSubscription,
    HostCallbackContext, HostClient, HostConfigReloadRequestResponse,
    HostConfigReloadStatusRequest, HostConfigReloadStatusResponse, HostContextStatusRequest,
    HostContextStatusResponse, HostExecutionList, HostExecutionSummary, HostGetSessionRequest,
    HostGetSessionResponse, HostImageExecuteRequest, HostImageExecuteResponse, HostImageOperation,
    HostLspDiagnostic, HostLspListServersResponse, HostLspServer, HostMcpServerSpec, HostPartsPage,
    HostPluginStatus, HostPluginStatusGetRequest, HostPluginStatusGetResponse,
    HostPluginStatusListResponse, HostQueuedMessage, HostRenameSessionRequest,
    HostRenameSessionResponse, HostSecretDeleteRequest, HostSecretGetRequest,
    HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest, HostSession,
    HostSessionAgent, HostSessionModel, HostSessionState, HostSessionStreamKind,
    HostSessionStreamRequest, HostStorageDeleteRequest, HostStorageGetRequest,
    HostStorageGetResponse, HostStorageListRequest, HostStorageListResponse, HostStorageRecord,
    HostStorageSetRequest, HostWorkspaceQuery, HostWorkspaceQueryRequest,
    HostWorkspaceQueryResponse, LogLevel, PathPermissionQuery, PermissionQuery, SessionOp,
    SessionOpResponse, ToolDescriptor, current_host_callback_context,
};

use agena_plugin_host::{
    EventEnvelope, EventFilter as PluginEventFilter, PluginError, ToolInvokeOutput,
};
use agena_storage::kernel::SessionSummaryView;

mod deepseek_mcp;
pub(super) use deepseek_mcp::CleanupLedger as DeepSeekMcpCleanupLedger;
mod image;
mod imported_files;
mod imported_stop;
mod imported_tools;
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

fn is_transcript_parent(child: &SessionSummaryView, parent: &SessionSummaryView) -> bool {
    child.relation_kind.is_subagent()
        && child.parent_session_id == Some(parent.id)
        && child.workspace_id == parent.workspace_id
        && child.root_session_id == parent.root_session_id
        && child.owner_bot_id == parent.owner_bot_id
}

fn bot_plugin_error(error: crate::AppError) -> PluginError {
    match &error {
        crate::AppError::Config(_)
        | crate::AppError::PersistenceConflict(_)
        | crate::AppError::BotConflict(_)
        | crate::AppError::BotLocalHistory { .. }
        | crate::AppError::Conflict { .. } => PluginError::invalid_params_error(&error),
        _ => plugin_error_from_app(error),
    }
}

fn plugin_error_from_app(error: crate::AppError) -> PluginError {
    match error {
        crate::AppError::Cancelled => PluginError::from_kind(
            agena_plugin_host::sdk::PluginErrorKind::Cancelled,
            "the originating execution was cancelled",
        ),
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

fn imported_skill_error(error: crate::AppError) -> PluginError {
    let public = match &error {
        crate::AppError::Config(message) => Some(message.as_str()),
        crate::AppError::PolicyDenied(denial) => Some(denial.reason.as_str()),
        crate::AppError::CapabilityUnavailable(unavailable) => Some(unavailable.reason.as_str()),
        _ => None,
    };
    let public = public.map(|message| agena_failure::diagnostic::scrubbed_preserve(message, 220));
    let mut error = plugin_error_from_app(error);
    if let Some(public) = public.filter(|message| !message.is_empty()) {
        error.failure.user = agena_failure::UserPresentation::validated(
            format!("{}-detail", error.failure.user.key),
            public,
        );
    }
    error
}

fn plugin_error_from_failure(failure: agena_failure::Failure) -> PluginError {
    use agena_failure::FailureCategory;
    use agena_plugin_sdk::PluginErrorKind;
    let kind = match failure.category {
        FailureCategory::InvalidInput | FailureCategory::NotFound | FailureCategory::Conflict => {
            PluginErrorKind::InvalidParams
        }
        FailureCategory::PermissionRequired | FailureCategory::PermissionDenied => {
            PluginErrorKind::PolicyDenied
        }
        FailureCategory::DependencyUnavailable => PluginErrorKind::HostUnavailable,
        FailureCategory::Timeout => PluginErrorKind::Timeout,
        _ => PluginErrorKind::Internal,
    };
    let mut error = PluginError::from_kind(kind, &failure.user.fallback);
    error.failure = Box::new(failure);
    error
}

pub(super) struct RuntimeHostClient {
    generation: u64,
    config: serde_json::Value,
    runtime: OnceLock<super::builder::WeakAgenaRuntime>,
}

enum McpInteractionOwner {
    Tool { session: i64, call: i64 },
    View(McpAppViewScope),
}

struct RuntimeImportedPluginNotices {
    manager: Arc<crate::session::SessionManager>,
    cancellation: tokio_util::sync::CancellationToken,
    session: i64,
    plugin: String,
}

#[async_trait]
impl ImportedPluginNotices for RuntimeImportedPluginNotices {
    async fn deliver(&self, text: String, wake: bool) -> Result<(), PluginError> {
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Ok(()),
            result = self.manager.queue_imported_plugin_notice(self.session, &self.plugin, text, wake) => {
                result.map_err(plugin_error_from_app)
            }
        }
    }
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

    fn executor_feature<T>(
        &self,
        feature: impl FnOnce(&crate::tool::ToolExecutor) -> Option<T>,
        unavailable: &'static str,
    ) -> Result<(crate::tool::ToolExecutor, T), PluginError> {
        let executor = self.tool_executor()?;
        let feature = feature(&executor).ok_or_else(|| host_unavailable(unavailable))?;
        Ok((executor, feature))
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
        let callback = current_host_callback_context();
        self.session_manager()?
            .host_callback_executor(
                callback.as_ref().and_then(|context| context.session_id),
                callback.as_ref().and_then(|context| context.call_id),
            )
            .await
            .map_err(plugin_error_from_app)
    }

    async fn callback_scoped_resource_executor(
        &self,
    ) -> Result<crate::tool::ToolExecutor, PluginError> {
        let callback = current_host_callback_context();
        self.session_manager()?
            .host_resource_callback_executor(
                callback.as_ref().and_then(|context| context.session_id),
                callback.as_ref().and_then(|context| context.call_id),
            )
            .await
            .map_err(plugin_error_from_app)
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

    async fn mcp_interaction_owner(
        &self,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<McpInteractionOwner, PluginError> {
        let denied = || PluginError::from_kind(
            agena_plugin_sdk::PluginErrorKind::PolicyDenied,
            "MCP interactions require a live session tool or App view from the built-in MCP bridge",
        );
        let (session, call) = self.callback_session_and_call()?;
        if self.callback_plugin_key()?.to_string() != agena_runtime_config::MCP_PLUGIN_ID
            || session <= 0
        {
            return Err(denied());
        }
        if call >= 0 {
            return Ok(McpInteractionOwner::Tool { session, call });
        }
        let app = self.callback_context()?.mcp_app.ok_or_else(denied)?;
        if let Some(task) = app.server_task.filter(|task| task.initial().is_some()) {
            let call = task.parent_tool(cancellation).await.ok_or_else(denied)?;
            return Ok(McpInteractionOwner::Tool { session, call });
        }
        app.view.filter(|view| view.binding.session_id == session)
            .map(McpInteractionOwner::View).ok_or_else(denied)
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

    /// Session plus the call id the delegation runs under.
    ///
    /// A tool-bound callback keeps its live call authority. A session-scoped
    /// hook callback has no transcript tool call and passes the reserved
    /// session-scoped sentinel instead.
    fn callback_scope(&self) -> Result<(i64, i64), PluginError> {
        let context = self.callback_context()?;
        let session_id = context
            .session_id
            .ok_or_else(|| host_unavailable("host callback context is missing session_id"))?;
        Ok((
            session_id,
            context
                .call_id
                .unwrap_or(agena_plugin_host::sdk::host_api::SESSION_SCOPED_CALL_ID),
        ))
    }

    /// Agent capability scope.
    ///
    /// Agent coordination is owned by the `agena.agent` plugin, which is the
    /// only holder of this host capability. Any other plugin reaches the same
    /// behaviour by invoking `agena.agent.*` through `invoke_tool`, so agent
    /// work keeps one permission, transcript and budget path.
    ///
    /// A tool-bound callback uses its live call id; a session-scoped callback
    /// passes none, which the session manager already supports by attributing
    /// the child to the session's latest message.
    fn agent_callback_scope(&self) -> Result<(i64, Option<i64>), PluginError> {
        self.require_agent_delegation()?;
        let context = self.callback_context()?;
        let session_id = context
            .session_id
            .ok_or_else(|| host_unavailable("agent coordination requires a session"))?;
        Ok((session_id, context.call_id.filter(|call| *call >= 0)))
    }

    fn require_agent_delegation(&self) -> Result<(), PluginError> {
        let owner = agena_bundled_plugins::tool::agent_plugin_id();
        let caller = self.callback_plugin_key()?;
        if caller.to_string() == owner {
            return Ok(());
        }
        Err(PluginError::from_kind(
            agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
            format!(
                "agent coordination belongs to `{owner}`; invoke its `agena.agent.*` tools \
                 through host/tool.invoke instead of the host agent callbacks"
            ),
        ))
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

    fn imported_agent_plugin(&self) -> Result<Arc<agena_plugin_host::LoadedPlugin>, PluginError> {
        let caller = self.callback_plugin_key()?;
        self.plugin_manager()?
            .plugins()
            .iter()
            .find(|plugin| plugin.key() == caller && plugin.agent_plugin().is_some())
            .cloned()
            .ok_or_else(|| {
                PluginError::from_kind(
                    agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                    "this callback belongs to the in-process agent plugin adapter",
                )
            })
    }

    fn cordis_workflow_caller(
        &self,
    ) -> Result<(i64, i64, Arc<agena_plugin_host::LoadedPlugin>), PluginError> {
        let plugin = self.imported_agent_plugin()?;
        let (session, call) = self.callback_session_and_call()?;
        if session <= 0 || call < 0 || self.callback_context()?.tool_name.is_none()
            || !plugin.agent_plugin().is_some_and(|package| {
                package.format == agena_plugin_host::AgentPluginFormat::DeepSeekHarness
            })
        {
            return Err(host_unavailable("Cordis workflow jobs require a live DeepSeek tool call"));
        }
        Ok((session, call, plugin))
    }

    fn cordis_subagent_caller(
        &self,
    ) -> Result<(i64, Option<i64>, Arc<agena_plugin_host::LoadedPlugin>), PluginError> {
        let plugin = self.imported_agent_plugin()?;
        let context = self.callback_context()?;
        if let Some(session) = context.session_id.filter(|session| *session > 0)
            && plugin.agent_plugin().is_some_and(|package| {
                package.format == agena_plugin_host::AgentPluginFormat::DeepSeekHarness
            })
        {
            if let Some(call) = context.call_id.filter(|call| *call >= 0)
                && context.tool_name.is_some()
            {
                return Ok((session, Some(call), plugin));
            }
            if context.plugin_command.is_some() && context.call_id.is_none() && context.tool_name.is_none() {
                return Ok((session, None, plugin));
            }
        }
        Err(host_unavailable("Cordis delegation requires a live DeepSeek project tool or command"))
    }

    async fn mcp_server_identity(
        &self,
        id: &str,
    ) -> Result<Option<agena_plugin_sdk::host_api::ImportedMcpServerIdentity>, PluginError> {
        let snapshot = self.snapshot()?;
        let Some(manager) = snapshot.mcp_manager() else {
            return Ok(None);
        };
        // A configured server can deliberately use the same spelling. Only a
        // successfully registered contribution has plugin provenance.
        if !manager.is_plugin_server(id).await {
            return Ok(None);
        }
        let plugins = snapshot.plugin_manager();
        Ok(plugins.plugins().iter().find_map(|plugin| {
            let package = plugin.agent_plugin()?;
            let key = plugin.key().to_string();
            let server = package.mcp_servers.iter().find(|server| {
                agena_plugin_host::agent_plugin::agent_plugin_mcp_server_name(&key, &server.name)
                    == id
            })?;
            Some(agena_plugin_sdk::host_api::ImportedMcpServerIdentity {
                package: package.name.clone(),
                server: server.name.clone(),
            })
        }))
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

    async fn register_browser_controller(
        &self,
        controller: Arc<dyn agena_plugin_sdk::browser::BrowserController>,
    ) -> Result<(), PluginError> {
        let runtime = self.runtime()?;
        let plugin_id = agena_bundled_plugins::tool::browser_plugin_id();
        let session_id = current_host_callback_context()
            .filter(|context| context.plugin_id.as_deref() == Some(plugin_id))
            .and_then(|context| context.session_id)
            .ok_or_else(|| {
                PluginError::from_kind(
                    agena_plugin_sdk::PluginErrorKind::PolicyDenied,
                    "Browser controls must be registered by the Browser plugin for its owning session",
                )
            })?;
        runtime
            .activities
            .register_browser_controller(session_id, &controller)
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
        let executor = self.callback_scoped_resource_executor().await?;
        let check = executor
            .network_permission_check_async(&target)
            .await
            .map_err(|error| error.into_plugin_error())?;
        Ok(permission_query(check.decision))
    }

    async fn mcp_session_roots(&self) -> Result<Option<Vec<std::path::PathBuf>>, PluginError> {
        let context = self.callback_context()?;
        if !context.session_id.is_some_and(|session| session > 0) {
            return Ok(None);
        }
        let executor = self.callback_scoped_resource_executor().await?;
        let workspace = context.workspace_root.as_deref().map(std::path::Path::new);
        if workspace.is_some_and(|root| root != executor.workspace_root()) {
            return Err(host_unavailable(
                "the MCP callback's session workspace changed",
            ));
        }
        Ok(Some(
            executor
                .sandbox_permission_snapshot()
                .authorized_working_directories(),
        ))
    }

    async fn check_resolved_network_permission(
        &self,
        original: String,
        resolved: String,
    ) -> Result<PermissionQuery, PluginError> {
        let original = original
            .parse::<agena_domain::NetworkTarget>()
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let resolved = resolved
            .parse::<agena_domain::NetworkTarget>()
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        // Reload all current numeric-IP decisions in the original callback
        // authority; an exact lookup by the hostname cannot find these denies.
        let executor = self.callback_scoped_resource_executor().await?;
        Ok(permission_query(
            executor
                .sandbox_permission_snapshot()
                .authorize_resolved_network_address(&original, &resolved),
        ))
    }

    async fn require_path_permission(
        &self,
        request: PathPermissionQuery,
    ) -> Result<(), PluginError> {
        let (session, call) = self.callback_scope()?;
        self.session_manager()?
            .require_host_path_permission(session, call, request.kind, request.path)
            .await
            .map_err(plugin_error_from_app)
    }

    async fn require_network_permission(&self, target: String) -> Result<(), PluginError> {
        let (session, call) = self.callback_scope()?;
        self.session_manager()?
            .require_host_network_permission(session, call, target)
            .await
            .map_err(plugin_error_from_app)
    }

    async fn require_pinned_http_permission(&self, target: String) -> Result<(), PluginError> {
        let (session, call) = self.callback_scope()?;
        self.session_manager()?
            .require_host_pinned_http_permission(session, call, target)
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

        self.invoke_callback_tool(&resolution, input, false).await
    }

    async fn imported_mod_tool(
        &self,
        request: agena_plugin_sdk::host_api::ImportedModToolRequest,
    ) -> Result<serde_json::Value, PluginError> {
        self.run_imported_mod_tool(request).await
    }

    async fn ask_user(&self, req: AskUserRequest) -> Result<AskUserResponse, PluginError> {
        let (session_id, call_id) = self.callback_session_and_call()?;
        let input = ask_user_tool_input(req)?;
        self.session_manager()?
            .request_host_user_input(session_id, call_id, input)
            .await
            .map_err(plugin_error)
    }

    async fn mcp_elicitation(
        &self,
        server: &str,
        request: McpElicitation,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<McpElicitationReply, PluginError> {
        let owner = self.mcp_interaction_owner(&cancellation).await?;
        let server = match self.mcp_server_identity(server).await? {
            Some(identity) => format!("plugin:{}:{}", identity.package, identity.server),
            None => server.to_owned(),
        };
        let manager = self.session_manager()?;
        match owner {
            McpInteractionOwner::Tool { session, call } => manager.resolve_mcp_elicitation(
                call,
                agena_plugin_sdk::ImportedMcpElicitationInput {
                    session_id: session, server, request, reply: None,
                },
                cancellation,
            ).await,
            McpInteractionOwner::View(view) => manager.resolve_mcp_app_elicitation(
                &view, &server, request, cancellation,
            ).await,
        }.map_err(plugin_error_from_app)
    }

    async fn companion_context(&self) -> Result<serde_json::Value, PluginError> {
        let source = self
            .callback_context()?
            .session_id
            .ok_or_else(|| host_unavailable("assistant context requires a session"))?;
        let service = self
            .runtime()?
            .assistant_service()
            .ok_or_else(|| host_unavailable("assistant service unavailable"))?;
        service
            .companion_context(source)
            .await
            .map_err(|e| PluginError::invalid_params(e.to_string()))
    }

    async fn mcp_sampling(
        &self,
        server: &str,
        request: agena_domain::mcp_sampling::McpSamplingRequest,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<Option<agena_domain::mcp_sampling::McpSamplingReply>, PluginError> {
        let owner = self.mcp_interaction_owner(&cancellation).await?;
        let server = match self.mcp_server_identity(server).await? {
            Some(identity) => format!("plugin:{}:{}", identity.package, identity.server),
            None => server.to_owned(),
        };
        let manager = self.session_manager()?;
        match owner {
            McpInteractionOwner::Tool { session, call } => manager
                .resolve_mcp_sampling(session, call, &server, request, cancellation)
                .await,
            McpInteractionOwner::View(view) => manager.resolve_mcp_app_sampling(
                &view, &server, "Return model output to the MCP server", request, cancellation,
            ).await,
        }.map_err(plugin_error_from_app)
    }

    async fn call_mcp_app_view_tool(
        &self,
        target: uuid::Uuid,
        arguments: serde_json::Map<String, serde_json::Value>,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<agena_domain::mcp_app_tools::McpAppToolResult, PluginError> {
        let (session, call) = self.callback_session_and_call()?;
        if self.callback_plugin_key()?.to_string() != agena_runtime_config::MCP_PLUGIN_ID
            || session <= 0
            || call < 0
        {
            return Err(PluginError::from_kind(
                agena_plugin_sdk::PluginErrorKind::PolicyDenied,
                "MCP App tools require a live session call from the built-in MCP bridge.",
            ));
        }
        self.session_manager()?
            .call_mcp_app_view_tool(session, call, target, arguments, cancellation)
            .await
            .map_err(plugin_error_from_app)
    }
    async fn companion_operation(
        &self,
        request: agena_plugin_sdk::host_api::CompanionRequest,
    ) -> Result<serde_json::Value, PluginError> {
        let (source, call) = self.callback_session_and_call()?;
        let service = self
            .runtime()?
            .assistant_service()
            .ok_or_else(|| host_unavailable("assistant service unavailable"))?;
        service
            .companion_operation(source, call, request)
            .await
            .map_err(|e| PluginError::invalid_params(e.to_string()))
    }

    async fn manage_integrations(
        &self,
        request: agena_plugin_sdk::host_api::IntegrationRequest,
    ) -> Result<serde_json::Value, PluginError> {
        let (source, call) = self.callback_session_and_call()?;
        let service = self
            .runtime()?
            .assistant_service()
            .ok_or_else(|| host_unavailable("integration service unavailable"))?;
        service
            .conversation_integrations(source, call, request)
            .await
            .map_err(|error| PluginError::invalid_params(error.to_string()))
    }

    async fn bot_context(&self) -> Result<serde_json::Value, PluginError> {
        let session = self
            .callback_context()?
            .session_id
            .ok_or_else(|| host_unavailable("Bot context requires a session"))?;
        self.session_manager()?
            .bot_context(session)
            .await
            .map_err(bot_plugin_error)
    }
    async fn bot_operation(
        &self,
        request: agena_plugin_sdk::host_api::BotRequest,
    ) -> Result<serde_json::Value, PluginError> {
        let (source, call) = self.callback_session_and_call()?;
        self.session_manager()?
            .bot_operation(source, call, request)
            .await
            .map_err(bot_plugin_error)
    }
    async fn bot_profile_operation(
        &self,
        request: agena_plugin_sdk::host_api::BotProfileRequest,
    ) -> Result<serde_json::Value, PluginError> {
        let (source, call) = self.callback_session_and_call()?;
        self.session_manager()?
            .bot_profile_operation(source, call, request)
            .await
            .map_err(bot_plugin_error)
    }
    async fn spawn_agent(&self, req: AgentSpawnRequest) -> Result<AgentHandle, PluginError> {
        let (session, call) = self.agent_callback_scope()?;
        self.session_manager()?
            .spawn_agent(session, call, req)
            .await
            .map_err(plugin_error)
    }

    async fn message_agent(
        &self,
        req: AgentMessageRequest,
    ) -> Result<AgentMessageReceipt, PluginError> {
        let (session, call) = self.agent_callback_scope()?;
        self.session_manager()?
            .message_agent(session, call, req)
            .await
            .map_err(plugin_error)
    }

    async fn wait_agent(&self, req: AgentWaitRequest) -> Result<AgentWaitResponse, PluginError> {
        // Waiting releases this tool's concurrency permit, so it needs a live
        // tool call; without one it would block a hook past its timeout.
        let (session, call) = self.callback_session_and_call()?;
        self.require_agent_delegation()?;
        self.session_manager()?
            .wait_agent(session, Some(call), req)
            .await
            .map_err(plugin_error)
    }

    async fn start_cordis_workflow(
        &self,
        start: agena_plugin_sdk::host_api::CordisWorkflowStart,
        run: agena_plugin_sdk::host_api::CordisWorkflowRunner,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<agena_plugin_sdk::host_api::CordisWorkflowStarted, PluginError> {
        let (session, call, plugin) = self.cordis_workflow_caller()?;
        agena_plugin_sdk::permission_wait::waiting(self.session_manager()?.start_cordis_workflow(
            session, call, plugin.key(), plugin.effect_scope().cancellation_token(), start, run, cancellation,
        ))
        .await
        .map_err(bot_plugin_error)
    }

    async fn start_cordis_job(
        &self,
        start: agena_plugin_sdk::host_api::CordisJobStart,
        produce: agena_plugin_sdk::host_api::CordisJobStarter,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<String, PluginError> {
        use agena_plugin_sdk::host_api::CordisJobInvocation;
        let context = self.callback_context()?;
        let (session, invocation, plugin) = if let Some(command) = context.plugin_command
            && let Some(session) = context.session_id.filter(|session| *session > 0)
            && context.call_id.is_none() && context.tool_name.is_none()
        {
            let plugin = self.imported_agent_plugin()?;
            if !plugin.agent_plugin().is_some_and(|package| {
                package.format == agena_plugin_host::AgentPluginFormat::DeepSeekHarness
            }) {
                return Err(host_unavailable("Cordis jobs require a DeepSeek package"));
            }
            (session, CordisJobInvocation::Command(command), plugin)
        } else {
            let (session, call, plugin) = self.cordis_workflow_caller()?;
            (session, CordisJobInvocation::Tool(call), plugin)
        };
        self.session_manager()?
            .start_cordis_job(
                session,
                invocation,
                plugin.key(),
                plugin.effect_scope().cancellation_token(),
                start,
                produce,
                cancellation,
            )
            .await
    }

    async fn open_cordis_job_registry(
        &self,
        lifetime: tokio_util::sync::CancellationToken,
    ) -> Result<Arc<dyn agena_plugin_sdk::host_api::CordisJobRegistry>, PluginError> {
        let plugin = self.imported_agent_plugin()?;
        let session = self.callback_context()?.session_id
            .filter(|session| *session > 0)
            .ok_or_else(|| host_unavailable("Cordis job registry requires a project session"))?;
        if !plugin.agent_plugin().is_some_and(|package| {
            package.format == agena_plugin_host::AgentPluginFormat::DeepSeekHarness
        }) {
            return Err(host_unavailable("Cordis job registry requires a DeepSeek package"));
        }
        self.session_manager()?
            .open_cordis_job_registry(
                session, plugin.key(), plugin.effect_scope().cancellation_token(), lifetime,
            ).await.map_err(bot_plugin_error)
    }

    async fn cordis_workflow_job(
        &self,
        request: agena_plugin_sdk::host_api::CordisWorkflowJobRequest,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<serde_json::Value, PluginError> {
        let (session, call, plugin) = self.cordis_workflow_caller()?;
        self.session_manager()?
            .cordis_workflow_job(session, call, plugin.key(), request, cancellation)
            .await
            .map_err(bot_plugin_error)
    }

    async fn wait_imported_plugin_agent(
        &self,
        child: i64,
        timeout_ms: u64,
    ) -> Result<ToolInvokeOutput, PluginError> {
        self.imported_agent_plugin()?;
        let (session, call) = self.callback_scope()?;
        self.session_manager()?
            .wait_imported_plugin_agent(session, (call >= 0).then_some(call), child, timeout_ms)
            .await
            .map_err(plugin_error_from_app)
    }

    async fn start_cordis_subagent(
        &self,
        start: agena_plugin_sdk::host_api::CordisSubagentStart,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<agena_plugin_sdk::host_api::CordisSubagentStarted, PluginError> {
        let (session, call, plugin) = self.cordis_subagent_caller()?;
        let lifetime = plugin.effect_scope().cancellation_token();
        let manager = self.session_manager()?;
        let launch = agena_plugin_sdk::permission_wait::waiting(
            manager.start_cordis_subagent(session, call, plugin.key(), start, cancellation.clone()),
        );
        tokio::pin!(launch);
        tokio::select! {
            biased;
            _ = lifetime.cancelled() => { cancellation.cancel(); launch.await },
            result = &mut launch => result,
        }.map_err(bot_plugin_error)
    }

    async fn wait_for_cordis_subagent(
        &self,
        wait: agena_plugin_sdk::host_api::CordisSubagentWait,
    ) -> Result<(), PluginError> {
        let (session, call, _) = self.cordis_subagent_caller()?;
        self.session_manager()?.wait_for_cordis_subagent(session, call, wait)
            .await.map_err(plugin_error_from_app)
    }

    async fn cordis_subagent_control(
        &self,
        request: agena_plugin_sdk::host_api::CordisSubagentControl,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<serde_json::Value, PluginError> {
        let (session, call, plugin) = self.cordis_subagent_caller()?;
        agena_plugin_sdk::permission_wait::waiting(self.session_manager()?.cordis_subagent_control(
            session, call, plugin.key(), request, cancel,
        )).await.map_err(bot_plugin_error)
    }

    async fn cordis_llm_complete(
        &self,
        request: agena_plugin_sdk::host_api::CordisLlmRequest,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<agena_plugin_sdk::host_api::CordisLlmResponse, PluginError> {
        let (session, call, plugin) = self.cordis_subagent_caller()?;
        let lifetime = plugin.effect_scope().cancellation_token();
        let manager = self.session_manager()?;
        let completion =
            manager.complete_cordis_llm(session, call, plugin.key(), request, cancel.clone());
        tokio::pin!(completion);
        tokio::select! {
            biased;
            _ = lifetime.cancelled() => { cancel.cancel(); completion.await }
            result = &mut completion => result,
        }
        .map_err(bot_plugin_error)
    }

    async fn evaluate_imported_plugin_prompt(&self, prompt: String) -> Result<String, PluginError> {
        self.imported_agent_plugin()?;
        let (session, _) = self.callback_scope()?;
        const SYSTEM: &str = "Evaluate the hook instruction using the supplied event. Return one JSON object with a boolean ok and an optional string reason. Set ok to false when the hook condition fails.";
        let result = self
            .session_manager()?
            .complete_auxiliary(session, SYSTEM.into(), prompt, 512)
            .await
            .map_err(plugin_error_from_app)?;
        Ok(result.text)
    }

    async fn run_imported_skill_shell(
        &self,
        request: agena_plugin_sdk::ImportedSkillShellRequest,
    ) -> Result<agena_plugin_sdk::ImportedSkillShellResult, PluginError> {
        let plugin = self.imported_agent_plugin()?;
        let context = self.callback_context()?;
        let session = context.session_id.filter(|id| *id > 0).ok_or_else(|| {
            host_unavailable("Claude skill shell expansion requires a project session")
        })?;
        if context.call_id.is_some() || context.tool_name.is_some() {
            return Err(PluginError::from_kind(
                agena_plugin_sdk::PluginErrorKind::PolicyDenied,
                "this callback belongs to a Claude slash command invocation",
            ));
        }
        self.session_manager()?
            .run_imported_skill_shell(session, plugin.key(), request)
            .await
            .map_err(imported_skill_error)
    }

    async fn render_imported_skill(
        &self,
        path: &std::path::Path,
        document: &str,
        arguments: &str,
    ) -> Result<Option<agena_plugin_sdk::ImportedSkillRender>, PluginError> {
        let context = self.callback_context()?;
        if self.callback_plugin_key()?.to_string()
            != agena_bundled_plugins::tool::skills_plugin_id()
            || context.tool_name.as_deref() != Some("get")
        {
            return Err(PluginError::from_kind(
                agena_plugin_sdk::PluginErrorKind::PolicyDenied,
                "this callback belongs to the built-in skills tool",
            ));
        }
        let (Some(session), Some(call)) = (
            context.session_id.filter(|id| *id > 0),
            context.call_id.filter(|id| *id >= 0),
        ) else {
            return Ok(Some(agena_plugin_sdk::ImportedSkillRender::Inspection));
        };
        self.session_manager()?
            .render_imported_skill(session, call, path, document, arguments)
            .await
            .map_err(imported_skill_error)
    }

    async fn find_imported_plugin_agent(
        &self,
        name: String,
    ) -> Result<Option<agena_plugin_host::sdk::host_api::ImportedPluginAgentState>, PluginError>
    {
        self.imported_agent_plugin()?;
        let (session, call) = self.callback_scope()?;
        self.session_manager()?
            .find_imported_plugin_agent(session, (call >= 0).then_some(call), &name)
            .await
            .map_err(plugin_error_from_app)
    }

    async fn evaluate_imported_plugin_agent(&self, prompt: String) -> Result<String, PluginError> {
        self.imported_agent_plugin()?;
        let context = self.callback_context()?;
        let session = context
            .session_id
            .ok_or_else(|| host_unavailable("an imported agent hook requires a session"))?;
        self.session_manager()?
            .verify_imported_hook(session, prompt)
            .await
            .map_err(plugin_error_from_app)
    }

    async fn bind_imported_plugin_background(
        &self,
    ) -> Result<ImportedPluginBackground, PluginError> {
        let plugin = self.imported_agent_plugin()?;
        let (session, _) = self.callback_scope()?;
        if session <= 0 {
            return Err(PluginError::invalid_params(
                "background hooks require a session",
            ));
        }
        let cancellation = plugin.effect_scope().cancellation_token();
        Ok(ImportedPluginBackground {
            cancellation: cancellation.clone(),
            notices: Arc::new(RuntimeImportedPluginNotices {
                manager: self.session_manager()?,
                cancellation,
                session,
                plugin: plugin.key().to_string(),
            }),
        })
    }

    async fn submit_imported_plugin_prompt(
        &self,
        request: agena_plugin_sdk::host_api::ImportedPluginPromptRequest,
    ) -> Result<serde_json::Value, PluginError> {
        let plugin = self.imported_agent_plugin()?;
        let name = plugin
            .agent_plugin()
            .expect("imported package")
            .name
            .clone();
        let session = self
            .callback_context()?
            .session_id
            .ok_or_else(|| host_unavailable("Plugin prompts require a project session"))?;
        let manager = self.session_manager()?;
        Ok(
            match manager
                .submit_imported_plugin_prompt(session, plugin.key(), name, request)
                .await
            {
                Ok(answer) => answer,
                Err(error) => serde_json::json!({"drop":error.failure().user.fallback}),
            },
        )
    }

    async fn configure_imported_plugin_files(
        &self,
        paths: Option<Vec<PathBuf>>,
        callback_generation: Option<u64>,
    ) -> Result<(), PluginError> {
        let plugin = self.imported_agent_plugin()?;
        let (session, _) = self.callback_scope()?;
        if session <= 0 || callback_generation != Some(plugin.effect_scope().generation()) {
            return Err(PluginError::invalid_params(
                "file hooks require an imported plugin session",
            ));
        }
        let manager = self.session_manager()?;
        let host = self.plugin_manager()?;
        if !host.has_subscription(agena_plugin_sdk::HookSubscription::SESSION_FILE_CHANGED) {
            return Ok(());
        }
        manager
            .imported_file_watches()
            .configure(
                session,
                host.imported_file_watch_filenames().next().is_some(),
                paths,
                imported_files::bind(&self.runtime()?, session),
            )
            .await
            .map_err(PluginError::internal)
    }

    async fn imported_hook_profile(&self) -> Result<Option<String>, PluginError> {
        self.imported_agent_plugin()?;
        let (session, _) = self.callback_scope()?;
        self.session_manager()?
            .imported_hook_profile(session)
            .await
            .map(Some)
            .map_err(plugin_error_from_app)
    }

    async fn check_imported_task(
        &self,
        mut input: agena_plugin_sdk::ImportedTaskBeforeInput,
    ) -> Result<(), PluginError> {
        let (session, _) = self.callback_scope()?;
        let plugin = self.callback_plugin_key()?;
        if plugin.to_string() != "agena.task" || session <= 0 || input.session_id != session {
            return Err(PluginError::from_kind(
                agena_plugin_sdk::PluginErrorKind::PolicyDenied,
                "task hooks require the built-in task bridge and its bound session",
            ));
        }
        let plugins = self.plugin_manager()?;
        if !plugins.has_subscription(agena_plugin_sdk::HookSubscription::IMPORTED_TASK_BEFORE) {
            return Ok(());
        }
        let context = self
            .get_session(HostGetSessionRequest::default())
            .await?
            .session;
        input.agent_id = context.is_subagent.then(|| session.to_string());
        plugins.dispatch_imported_task_before(input).await
    }

    async fn imported_mcp_server_identity(
        &self,
        id: &str,
    ) -> Result<Option<agena_plugin_sdk::host_api::ImportedMcpServerIdentity>, PluginError> {
        self.imported_agent_plugin()?;
        self.mcp_server_identity(id).await
    }

    async fn imported_hook_environment(
        &self,
        callback_generation: Option<u64>,
    ) -> Result<Option<agena_plugin_sdk::host_api::ImportedHookEnvironment>, PluginError> {
        let (session, call) = self.callback_scope()?;
        let plugin = self.imported_agent_plugin()?;
        if session <= 0 || callback_generation != Some(plugin.effect_scope().generation()) {
            return Ok(None);
        }
        let manager = self.session_manager()?;
        let executor = manager
            .host_resource_callback_executor(Some(session), (call >= 0).then_some(call))
            .await
            .map_err(plugin_error_from_app)?;
        if executor.is_global() || !executor.sandbox_enabled() {
            return Ok(None);
        }
        let package = plugin.agent_plugin().expect("native imported plugin");
        manager
            .imported_shells()
            .environment_for_hook(session, plugin.key(), package.root.clone())
            .await
            .map_err(|error| {
                PluginError::internal(format!(
                    "could not create hook environment file: {}",
                    error.kind()
                ))
            })
    }

    async fn imported_hook_transcripts(
        &self,
        format: agena_plugin_sdk::host_api::HookTranscriptFormat,
        scope: agena_plugin_sdk::host_api::HookTranscriptScope,
        callback_generation: Option<u64>,
    ) -> Result<agena_plugin_sdk::host_api::ImportedHookTranscripts, PluginError> {
        use agena_plugin_sdk::host_api::{HookTranscriptScope, ImportedHookTranscripts};

        let (id, call) = self.callback_scope()?;
        let plugin = self.imported_agent_plugin()?;
        if id <= 0 || callback_generation != Some(plugin.effect_scope().generation()) {
            return Ok(ImportedHookTranscripts::default());
        }
        let session = agena_storage::kernel::SessionId::try_from(id)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let manager = self.session_manager()?;
        let executor = manager
            .host_resource_callback_executor(Some(id), (call >= 0).then_some(call))
            .await
            .map_err(plugin_error_from_app)?;
        let key = plugin.key();
        if executor.is_global() || !executor.sandbox_enabled() || !executor.plugin_is_enabled(&key)
        {
            return Ok(ImportedHookTranscripts::default());
        }
        let root = &plugin.agent_plugin().expect("native imported plugin").root;
        let export = async |session, workspace: &std::path::Path| {
            manager
                .imported_shells()
                .transcript_for_hook(session, workspace, format, key.clone(), root.clone())
                .await
                .map_err(|error| PluginError::internal_error(&error))
        };
        if scope == HookTranscriptScope::Current {
            return Ok(ImportedHookTranscripts {
                transcript: export(session, executor.workspace_root()).await?,
                agent: None,
            });
        }

        let reader = &manager.kernel().reader;
        let child = reader
            .summary(session)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let Some(parent) = child
            .parent_session_id
            .filter(|_| child.relation_kind.is_subagent())
        else {
            return Ok(ImportedHookTranscripts::default());
        };
        let parent_summary = reader
            .summary(parent)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let mut files = ImportedHookTranscripts::default();
        if is_transcript_parent(&child, &parent_summary) {
            let parent_executor = manager
                .host_resource_callback_executor(Some(parent.get()), None)
                .await
                .map_err(plugin_error_from_app)?;
            if !parent_executor.is_global()
                && parent_executor.sandbox_enabled()
                && parent_executor.plugin_is_enabled(&key)
            {
                files.transcript = export(parent, parent_executor.workspace_root()).await?;
            }
        }
        if scope == HookTranscriptScope::ParentAndAgent {
            files.agent = export(session, executor.workspace_root()).await?;
        }
        Ok(files)
    }

    async fn imported_claude_stop_work(
        &self,
        parent: bool,
        callback_generation: Option<u64>,
    ) -> Result<Option<agena_plugin_sdk::ImportedClaudeStopWork>, PluginError> {
        self.claude_stop_work(parent, callback_generation).await
    }

    async fn visit_ending_session_history(
        &self,
        visit: &mut (dyn FnMut(agena_plugin_sdk::host_api::SessionHistoryEvent) + Send),
    ) -> Result<(), PluginError> {
        let plugin = self.imported_agent_plugin()?;
        let (session, call) = self.callback_scope()?;
        if call != agena_plugin_sdk::host_api::SESSION_SCOPED_CALL_ID
            || !agena_plugin_host::host::is_session_disposal(session)
        {
            return Err(host_unavailable(
                "session history requires a session disposal hook",
            ));
        }
        self.session_manager()?
            .visit_ending_session_history(session, &plugin.key(), visit)
            .await
            .map_err(plugin_error_from_app)
    }

    async fn call_imported_plugin_mcp(
        &self,
        server: String,
        tool: String,
        arguments: serde_json::Value,
    ) -> Result<ToolInvokeOutput, PluginError> {
        self.imported_agent_plugin()?;
        let context = self.callback_context()?;
        let session = context
            .session_id
            .ok_or_else(|| host_unavailable("an imported MCP hook requires a session"))?;
        let input = serde_json::json!({"server": server, "name": tool, "arguments": arguments});
        self.session_manager()?
            .call_imported_plugin_mcp(session, input)
            .await
            .map(tool_execution_to_invoke_output)
            .map_err(plugin_error_from_app)
    }

    async fn deepseek_mcp_manager(
        &self,
        request: agena_plugin_sdk::host_api::DeepSeekMcpManagerRequest,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<serde_json::Value, PluginError> {
        self.manage_deepseek_mcp(request, cancellation).await
    }

    async fn observe_deepseek_mcp_manager(
        &self,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<tokio::sync::watch::Receiver<serde_json::Value>, PluginError> {
        self.subscribe_deepseek_mcp(cancellation).await
    }

    async fn imported_model_skills(
        &self,
    ) -> Result<Vec<agena_plugin_host::AgentPluginSkill>, PluginError> {
        if self.callback_context()?.plugin_id.as_deref() != Some("agena.skills") {
            return Err(host_unavailable("the skill catalog belongs to the built-in skills tool"));
        }
        let executor = self.callback_scoped_resource_executor().await?;
        let skills = executor.agent_plugin_provided_skills().await;
        Ok(skills
            .into_iter()
            .filter(|skill| skill.invocation.is_none_or(|policy| policy.model_invocable))
            .collect())
    }

    async fn prepare_managed_mcp_http(&self, server: String) -> Result<bool, PluginError> {
        self.admit_managed_mcp_http(&server).await
    }

    async fn followup_agent(&self, req: AgentMessageRequest) -> Result<AgentHandle, PluginError> {
        let (session, call) = self.agent_callback_scope()?;
        self.session_manager()?
            .followup_agent(session, call, req)
            .await
            .map_err(plugin_error)
    }

    async fn read_imported_plugin_mcp(
        &self,
        server: String,
        uri: String,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let plugin = self.imported_agent_plugin()?;
        let key = plugin.key().to_string();
        let package = plugin.agent_plugin().expect("native imported plugin");
        if !package.mcp_servers.iter().any(|entry| {
            agena_plugin_host::agent_plugin::agent_plugin_mcp_server_name(&key, &entry.name)
                == server
        }) {
            return Err(PluginError::from_kind(
                agena_plugin_sdk::PluginErrorKind::PolicyDenied,
                "the MCP resource server must belong to this plugin",
            ));
        }
        let session = self
            .callback_context()?
            .session_id
            .ok_or_else(|| host_unavailable("an imported MCP resource hook requires a session"))?;
        let input = serde_json::json!({"server": server, "uri": uri});
        self.session_manager()?
            .read_imported_plugin_mcp(session, input)
            .await
            .map(tool_execution_to_invoke_output)
            .map_err(plugin_error_from_app)
    }

    async fn cancel_agent(
        &self,
        req: AgentTargetRequest,
    ) -> Result<AgentMessageReceipt, PluginError> {
        let (session, call) = self.agent_callback_scope()?;
        self.session_manager()?
            .cancel_agent(session, call, req)
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
            session: session_resource_from_session(&session),
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
            session: session_resource_from_session(&session),
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

    async fn session_op(&self, op: SessionOp) -> Result<SessionOpResponse, PluginError> {
        let runtime = self.runtime()?;
        let backend = runtime
            .application_services()
            .session_api_backend
            .backend()
            .ok_or_else(|| host_unavailable("the shared application API has not been attached"))?;
        match op {
            SessionOp::Query(query) => backend
                .query(query)
                .await
                .map(SessionOpResponse::Query)
                .map_err(plugin_error_from_failure),
            SessionOp::Command(command) => backend
                .command(command)
                .await
                .map(SessionOpResponse::Command)
                .map_err(plugin_error_from_failure),
        }
    }

    async fn session_stream(
        &self,
        req: HostSessionStreamRequest,
    ) -> Result<EventSubscription, PluginError> {
        let target = self.callback_or_requested_session_id(req.session_id, "session_stream")?;
        // Live delivery to plugins reuses the existing event channel; the
        // stream handle itself is created when the plugin subscribes.
        let _ = req.kind;
        let subscription_id = self
            .subscribe_events(agena_plugin_sdk::hooks::EventFilter {
                kinds: vec!["session".into()],
                session_id: Some(target),
                workspace: None,
            })
            .await?;
        Ok(subscription_id)
    }

    async fn workspace_query(
        &self,
        req: HostWorkspaceQueryRequest,
    ) -> Result<HostWorkspaceQueryResponse, PluginError> {
        let services = self.runtime()?.application_services();
        let repositories = services
            .repositories
            .ok_or_else(|| host_unavailable("workspace repository is not configured"))?;
        let data = match req.query {
            HostWorkspaceQuery::List {
                include_session_count,
            } => {
                let _ = include_session_count;
                let rows = repositories
                    .workspace
                    .list(agena_storage::WorkspaceListQuery {
                        limit: 256,
                        ..Default::default()
                    })
                    .await
                    .map_err(|error| PluginError::invalid_params(error.to_string()))?;
                serde_json::Value::Array(
                    rows.into_iter()
                        .map(|row| {
                            serde_json::json!({
                                "id": row.id,
                                "path": row.path,
                                "created_at_ms": row.created_at_ms,
                                "updated_at_ms": row.updated_at_ms,
                            })
                        })
                        .collect(),
                )
            }
            HostWorkspaceQuery::Get { workspace_id } => {
                let row = repositories
                    .workspace
                    .get(workspace_id)
                    .await
                    .map_err(|error| PluginError::invalid_params(error.to_string()))?;
                match row {
                    Some(row) => serde_json::json!({
                        "id": row.id,
                        "path": row.path,
                        "created_at_ms": row.created_at_ms,
                        "updated_at_ms": row.updated_at_ms,
                    }),
                    None => serde_json::Value::Null,
                }
            }
            HostWorkspaceQuery::Files {
                workspace_id,
                path: _,
                limit,
            } => {
                let root = repositories
                    .workspace
                    .path_by_id(workspace_id)
                    .await
                    .map_err(|error| PluginError::invalid_params(error.to_string()))?
                    .ok_or_else(|| PluginError::invalid_params("workspace not found"))?;
                let _ = (limit, root);
                return Err(host_unavailable(
                    "workspace file listing is not yet reachable from a plugin callback",
                ));
            }
            HostWorkspaceQuery::Read {
                workspace_id,
                path: _,
                max_bytes,
            } => {
                let root = repositories
                    .workspace
                    .path_by_id(workspace_id)
                    .await
                    .map_err(|error| PluginError::invalid_params(error.to_string()))?
                    .ok_or_else(|| PluginError::invalid_params("workspace not found"))?;
                let _ = (max_bytes, root);
                return Err(host_unavailable(
                    "workspace file reads are not yet reachable from a plugin callback",
                ));
            }
        };
        Ok(HostWorkspaceQueryResponse { data })
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
        // Input paths follow the callback's effective cwd, but durable image
        // outputs must use the owning session workspace consumed by downloads.
        // Resolve it before making the provider request, not after its effect.
        let workspace_root = manager
            .session_workspace_root(callback_session_id)
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

    async fn storage_compare_exchange(
        &self,
        req: HostStorageSetRequest,
        expected: Option<String>,
    ) -> Result<bool, PluginError> {
        self.use_plugin_storage(req.scope, req.visibility, move |store, locator| {
            store.compare_exchange(
                locator,
                &req.namespace,
                &req.key,
                expected.as_deref(),
                &req.value,
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

    async fn imported_plugin_storage_directory(&self) -> Result<std::path::PathBuf, PluginError> {
        let plugin = self.imported_agent_plugin()?;
        if !plugin.agent_plugin().is_some_and(|package| {
            package.format == agena_plugin_host::AgentPluginFormat::DeepSeekHarness
        }) {
            return Err(host_unavailable(
                "this imported plugin has no Cordis storage adapter",
            ));
        }
        self.plugin_storage()?
            .private_global_directory(&plugin.key())
            .ok_or_else(|| host_unavailable("private plugin storage directory is unavailable"))
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

    async fn prepare_process_sandbox(
        &self,
        mut resources: agena_plugin_sdk::host_api::HostProcessResources,
    ) -> Result<agena_plugin_sdk::host_api::HostPreparedProcessSandbox, PluginError> {
        let (session, call) = self.callback_scope()?;
        let manager = self.session_manager()?;
        if resources.cwd.is_none()
            && manager.imported_shells().has_cwd(session)
            && self.imported_agent_plugin().is_ok()
        {
            let executor = manager
                .host_resource_callback_executor(Some(session), (call >= 0).then_some(call))
                .await
                .map_err(plugin_error_from_app)?;
            resources.cwd = executor
                .imported_shell_cwd()
                .map(|cwd| cwd.to_string_lossy().into_owned());
        }
        let mut prepared = manager
            .prepare_host_process_sandbox(session, call, resources)
            .await
            .map_err(plugin_error_from_app)?;
        if prepared.call_bound {
            let completion =
                agena_plugin_host::current_plugin_call_completion().ok_or_else(|| {
                    host_unavailable("process approval requires a live originating call")
                })?;
            prepared.scope.push_str(":origin:");
            prepared.scope.push_str(completion.id());
        }
        Ok(prepared)
    }

    async fn prepare_worker_sandbox(
        &self,
        resources: agena_plugin_sdk::host_api::HostProcessResources,
    ) -> Result<agena_plugin_sdk::host_api::HostPreparedProcessSandbox, PluginError> {
        let (session, call) = self.callback_scope()?;
        let mut prepared = self
            .session_manager()?
            .prepare_host_worker_sandbox(session, call, resources)
            .await
            .map_err(plugin_error_from_app)?;
        if prepared.call_bound {
            let completion =
                agena_plugin_host::current_plugin_call_completion().ok_or_else(|| {
                    host_unavailable("worker approval requires a live originating call")
                })?;
            prepared.scope.push_str(":origin:");
            prepared.scope.push_str(completion.id());
        }
        Ok(prepared)
    }

    async fn require_mcp_server_permission(
        &self,
        spec: HostMcpServerSpec,
    ) -> Result<(), PluginError> {
        match spec {
            HostMcpServerSpec::Stdio {
                command, args, cwd, ..
            } => {
                let shell_command = join_monitor_command(
                    &std::iter::once(command).chain(args).collect::<Vec<_>>(),
                )?;
                self.require_callback_invocation(
                    "agena.shell.spawn",
                    serde_json::json!({
                        "command": shell_command, "workdir": cwd,
                        "reads": [], "writes": [], "network": [],
                    }),
                )
                .await
            }
            HostMcpServerSpec::Http { url, .. } => {
                let parsed = url::Url::parse(&url).map_err(|_| {
                    PluginError::invalid_params("MCP endpoint must be an absolute HTTP(S) URL")
                })?;
                if !matches!(parsed.scheme(), "http" | "https")
                    || parsed.host_str().is_none()
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                    || parsed.fragment().is_some()
                {
                    return Err(PluginError::invalid_params(
                        "MCP endpoint must be an absolute HTTP(S) URL without user information or a fragment",
                    ));
                }
                self.require_network_permission(url).await
            }
        }
    }
}

#[cfg(test)]
mod agent_capability_tests {
    use super::*;
    use agena_plugin_host::sdk::host_api::{
        HostCallbackContext, run_in_isolated_host_callback_context,
    };

    fn client() -> RuntimeHostClient {
        RuntimeHostClient {
            generation: 1,
            config: serde_json::Value::Null,
            runtime: OnceLock::new(),
        }
    }

    fn context(plugin: &str) -> HostCallbackContext {
        HostCallbackContext {
            plugin_id: Some(plugin.to_string()),
            session_id: Some(7),
            call_id: Some(11),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn transcripts_are_scoped_to_the_recorded_subagent_parent() {
        use agena_domain::SessionRelationKind;
        use agena_storage::kernel::{RequestId, SessionAgentConfiguration, SessionConfiguration};

        let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap();
        let workspace = runtime
            .workspaces
            .ensure_id("/test-transcript")
            .await
            .unwrap();
        let kernel = &runtime.ports;
        let parent = kernel
            .sessions
            .create(RequestId::new(), workspace, "Parent".into())
            .await
            .unwrap()
            .value
            .id;
        let child = kernel
            .sessions
            .create_configured(
                RequestId::new(),
                workspace,
                "Child".into(),
                SessionConfiguration {
                    parent_session_id: Some(parent),
                    relation_kind: SessionRelationKind::Subagent,
                    agent: Some(SessionAgentConfiguration {
                        origin_message_id: None,
                        workflow: None,
                        cordis: None,
                        name: "worker".into(),
                        status: agena_domain::AgentStatus::Running,
                        started_at_ms: Some(1),
                        finished_at_ms: None,
                        failure: None,
                        limits: Default::default(),
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let parent = kernel.reader.summary(parent).await.unwrap();
        let mut child = (*kernel.reader.summary(child).await.unwrap()).clone();
        assert!(is_transcript_parent(&child, &parent));
        assert!(!is_transcript_parent(&child, &child));

        child.workspace_id = Some(workspace + 1);
        assert!(!is_transcript_parent(&child, &parent));
        child.workspace_id = Some(workspace);
        child.owner_bot_id = Some("other owner".into());
        assert!(!is_transcript_parent(&child, &parent));
        child.owner_bot_id = None;
        child.relation_kind = SessionRelationKind::Child;
        assert!(!is_transcript_parent(&child, &parent));
    }

    /// The bundled agent plugin is the only holder of the agent capability.
    #[tokio::test]
    async fn only_the_agent_plugin_may_coordinate_agents() {
        let owner = agena_bundled_plugins::tool::agent_plugin_id();
        let client = client();
        run_in_isolated_host_callback_context(context(owner), async {
            assert!(client.require_agent_delegation().is_ok());
        })
        .await;

        for foreign in ["example.echo", "agena.fs", "agena.web"] {
            run_in_isolated_host_callback_context(context(foreign), async {
                let error = client
                    .require_agent_delegation()
                    .expect_err("a non-owner plugin must not hold the agent capability");
                assert_eq!(
                    error.kind,
                    agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                    "{foreign} was allowed to coordinate agents"
                );
            })
            .await;
        }
    }

    /// The error names the delegation route so a plugin author can fix it.
    #[tokio::test]
    async fn refusal_points_at_tool_invocation() {
        let client = client();
        run_in_isolated_host_callback_context(context("example.echo"), async {
            let error = client.require_agent_delegation().unwrap_err();
            let text = error.diagnostic_message().to_owned();
            assert!(text.contains("agena.agent.*"), "{text}");
            assert!(text.contains("host/tool.invoke"), "{text}");
        })
        .await;
    }

    /// A callback without a plugin identity cannot obtain the capability.
    #[tokio::test]
    async fn unattributed_callers_are_refused() {
        let client = client();
        let mut bare = context("agena.agent");
        bare.plugin_id = None;
        run_in_isolated_host_callback_context(bare, async {
            assert!(client.require_agent_delegation().is_err());
        })
        .await;
    }
}
