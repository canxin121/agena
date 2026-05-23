//! `PluginHost` — the central handle agena holds. Owns:
//! - the loaded plugins,
//! - the plugin tool registry,
//! - the dedicated tokio runtime that drives plugin transports,
//! - the host-callback router used by stdio/http plugins.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::config::{PluginEntry, PluginsConfig, TimeoutsConfig};
use crate::dispatcher::{self, call_with_timeout};
use crate::error::{HostError, TransportError};
use crate::loader::{StaticRegistration, load_entry, shutdown_transport};
use crate::logs::{PluginLogEntry, PluginLogStore};
use crate::registry::{
    PluginEntry as RegistryPluginEntry, PluginEntryRegistry,
    effective_host_capabilities_for_manifest,
};
use crate::sdk::host_api::{
    self, AskUserRequest, AskUserResponse, EventSubscription, HostAgentGetRequest,
    HostAgentGetResponse, HostAgentListResponse, HostAgentRegisterRequest, HostAgentRemoveRequest,
    HostAgentRemoveResponse, HostAgentRestoreRequest, HostAgentRestoreResponse,
    HostAgentSwitchRequest, HostAgentSwitchResponse, HostCallbackContext, HostClient,
    HostConfigReloadResponse, HostEnterPlanModeRequest, HostEnterWorktreeRequest,
    HostEntryDescriptor, HostEntryListResponse, HostEntryMutationResponse,
    HostEntryRegisterRequest, HostEntryRemoveRequest, HostEntryUpdateRequest,
    HostExitPlanModeRequest, HostExitWorktreeRequest, HostHookEntry, HostHookListResponse,
    HostLspListDiagnosticsRequest, HostLspListDiagnosticsResponse, HostLspListServersResponse,
    HostMcpAddServerRequest, HostMcpListServersResponse, HostMcpRemoveServerRequest,
    HostMcpRemoveServerResponse, HostNetworkPermissionCheckRequest, HostPathPermissionCheckRequest,
    HostPermissionCheckResponse, HostPlanGetRequest, HostPlanGetResponse, HostPlanListResponse,
    HostPluginStatus, HostPluginStatusGetRequest, HostPluginStatusGetResponse,
    HostPluginStatusListResponse, HostSchedulerCreateRequest, HostSchedulerCreateResponse,
    HostSchedulerDeleteRequest, HostSchedulerDeleteResponse, HostSchedulerListResponse,
    HostSecretDeleteRequest, HostSecretGetRequest, HostSecretGetResponse, HostSecretListResponse,
    HostSecretSetRequest, HostStatuslineContributeRequest, HostStatuslineListResponse,
    HostStatuslineRemoveRequest, HostStatuslineRemoveResponse, HostStatuslineSegment,
    HostStorageDeleteRequest, HostStorageGetRequest, HostStorageGetResponse,
    HostStorageListRequest, HostStorageListResponse, HostStorageSetRequest, HostThemeListResponse,
    HostThemePalette, HostThemeRegisterRequest, HostThemeRemoveRequest, HostThemeRemoveResponse,
    HostTodoWriteRequest, HostWorktreeListResponse, LogLevel, MonitorHandle, MonitorReadRequest,
    MonitorReadResponse, MonitorStartRequest, MonitorStopRequest, NoopHostClient,
    SpawnSubtaskRequest, SpawnSubtaskResponse, ToolDescriptor,
};
use crate::sdk::rpc::method;
use crate::sdk::{
    AgentStopInput, AgentStopPatch, AuthInput, AuthOutput, ChatHeadersInput, ChatHeadersPatch,
    ChatMessageInput, ChatMessagePatch, ChatMessagesTransformInput, ChatMessagesTransformPatch,
    ChatParamsInput, ChatParamsPatch, ChatSystemTransformInput, ChatSystemTransformPatch,
    CommandAfterInput, CommandAfterPatch, CommandBeforeInput, CommandBeforeOutcome,
    CommandBeforeResponse, ConfigInput, ConfigPatch, EventEnvelope, EventFilter, HookSubscription,
    HostCapability, NotificationInput, PermissionAdvice, PermissionAskDecision, PermissionAskInput,
    PermissionDecision, PluginError, PluginErrorCode, PluginManifest, PluginStudioCommand,
    PluginStudioControl, PluginStudioView, PluginTuiContentBlock, PluginUiAction, PostRunInput,
    PreRunInput, ProviderListInput, ProviderListPatch, SessionEndInput, SessionStartInput,
    SessionStartPatch, ShellEnvInput, ShellEnvPatch, ToolAfterInput, ToolAfterPatch,
    ToolBeforeInput, ToolBeforePatch, ToolDefinitionInput, ToolDefinitionPatch, ToolFailureInput,
    ToolInvokeInput, ToolInvokeOutput, ToolPermissionNetworksInput, ToolPermissionPathsInput,
    ToolStreamChunk, ToolStreamEnd, UserPromptSubmitInput, UserPromptSubmitPatch,
};
use crate::transport::PluginTransport;
use crate::transport::inproc::InProcessTransport;

pub struct LoadedPlugin {
    pub id: String,
    pub kind: &'static str,
    pub entry: crate::config::PluginEntry,
    pub manifest: PluginManifest,
    pub transport: Arc<dyn PluginTransport>,
    pub trust_level: String,
    pub provenance: Vec<String>,
}

impl LoadedPlugin {
    pub fn transport(&self) -> Arc<dyn PluginTransport> {
        Arc::clone(&self.transport)
    }

    pub fn entry(&self) -> &crate::config::PluginEntry {
        &self.entry
    }

    pub fn authority_summary(&self) -> PluginAuthoritySummary {
        let plugin_capabilities = effective_host_capabilities_for_manifest(
            &self.manifest.entries,
            &self.manifest.plugin_capabilities,
        )
        .into_iter()
        .map(|capability| format!("{capability:?}"))
        .collect::<Vec<_>>();
        let entry_capabilities = self
            .manifest
            .entries
            .iter()
            .filter(|entry| !entry.host_capabilities.is_empty())
            .map(|entry| {
                (
                    entry.name.clone(),
                    entry
                        .host_capabilities
                        .iter()
                        .map(|capability| format!("{capability:?}"))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        PluginAuthoritySummary {
            trust_level: self.trust_level.clone(),
            provenance: self.provenance.clone(),
            plugin_capabilities,
            entry_capabilities,
        }
    }
}

impl std::fmt::Debug for LoadedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedPlugin")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

impl LoadedPlugin {
    pub fn new(
        id: String,
        kind: &'static str,
        entry: crate::config::PluginEntry,
        transport: Arc<dyn PluginTransport>,
        manifest: PluginManifest,
        trust_level: String,
        provenance: Vec<String>,
    ) -> Self {
        Self {
            id,
            kind,
            entry,
            manifest,
            transport,
            trust_level,
            provenance,
        }
    }

    pub fn subscribes(&self, sub: HookSubscription) -> bool {
        self.manifest.hooks.contains(sub)
    }

    pub fn entry_name_for_tool(&self, tool_name: &str) -> Option<String> {
        self.manifest
            .entries
            .iter()
            .find_map(|entry| (entry.name == tool_name).then(|| entry.name.clone()))
    }
}

fn block_on_handle_or_thread<F>(handle: tokio::runtime::Handle, fut: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    if let Ok(current) = tokio::runtime::Handle::try_current() {
        return if current.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
            tokio::task::block_in_place(|| handle.block_on(fut))
        } else {
            std::thread::spawn(move || handle.block_on(fut))
                .join()
                .expect("plugin host runtime thread panicked")
        };
    }

    handle.block_on(fut)
}

fn block_on_new_thread<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("plugin host fallback runtime");
        rt.block_on(fut)
    })
    .join()
    .expect("plugin host fallback runtime thread panicked")
}

fn block_on_scoped_thread<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("plugin host fallback runtime");
                rt.block_on(fut)
            })
            .join()
            .expect("plugin host fallback runtime thread panicked")
    })
}

fn block_on_runtime_scoped_thread<F>(runtime: &tokio::runtime::Runtime, fut: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(move || runtime.block_on(fut))
            .join()
            .expect("plugin host runtime thread panicked")
    })
}

fn block_on_handle_scoped_thread<F>(handle: &tokio::runtime::Handle, fut: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(move || handle.block_on(fut))
            .join()
            .expect("plugin host runtime thread panicked")
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginAuthoritySummary {
    pub trust_level: String,
    pub provenance: Vec<String>,
    pub plugin_capabilities: Vec<String>,
    pub entry_capabilities: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginInspect {
    pub status: crate::status::PluginStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<PluginManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<PluginAuthoritySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<crate::config::PluginEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginUiCatalog {
    pub tui: PluginTuiUiCatalog,
    pub studio: PluginStudioUiCatalog,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginTuiUiCatalog {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statusline_segments: Vec<HostStatuslineSegment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<HostThemePalette>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_blocks: Vec<PluginTuiContentBlockCatalogItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginStudioUiCatalog {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<PluginStudioCommandCatalogItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<PluginStudioControlCatalogItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<PluginStudioViewCatalogItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginTuiContentBlockCatalogItem {
    pub plugin_id: String,
    #[serde(flatten)]
    pub block: PluginTuiContentBlock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStudioCommandCatalogItem {
    pub plugin_id: String,
    #[serde(flatten)]
    pub command: PluginStudioCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStudioControlCatalogItem {
    pub plugin_id: String,
    #[serde(flatten)]
    pub control: PluginStudioControl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStudioViewCatalogItem {
    pub plugin_id: String,
    #[serde(flatten)]
    pub view: PluginStudioView,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginUiToolInvokeResponse {
    pub plugin_id: String,
    pub tool: String,
    pub title: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// Live handle to an in-flight tool stream. Consume `chunks` for incremental
/// output; once the stream closes (sender dropped), inspect `end` for the
/// final aggregated result.
pub struct ToolInvokeStream {
    pub stream_id: String,
    pub chunks: tokio::sync::mpsc::Receiver<ToolStreamChunk>,
    pub end: tokio::sync::oneshot::Receiver<Result<ToolStreamEnd, PluginError>>,
}

#[derive(Debug, Clone)]
pub enum PermissionAskOutcome {
    Decision {
        plugin_id: String,
        decision: PermissionDecision,
        authority: PluginAuthoritySummary,
    },
    Advice {
        plugin_id: String,
        advice: PermissionAdvice,
        authority: PluginAuthoritySummary,
    },
}

/// Result-bearing facade for a tool call. Wraps async dispatch in a runtime
/// `block_on` so callers from sync code (like `ToolExecutor`) can use it.
pub struct PluginHost {
    plugins: Vec<Arc<LoadedPlugin>>,
    plugins_by_id: HashMap<String, Arc<LoadedPlugin>>,
    entries: Arc<RwLock<PluginEntryRegistry>>,
    statuses: Arc<crate::status::StatusRegistry>,
    logs: Arc<PluginLogStore>,
    timeouts: TimeoutsConfig,
    /// Dedicated runtime used to block_on async transport calls when invoked
    /// from sync code.
    runtime: Option<Arc<tokio::runtime::Runtime>>,
    /// Handle to the runtime that built us (preferred for block_on when sync
    /// callers are themselves driven by an outer runtime).
    runtime_handle: Option<tokio::runtime::Handle>,
    /// Underlying host handle; kept alive for callbacks.
    _host_handle: Arc<HostHandle>,
    /// Plugin ids whose transports we transferred to a successor host;
    /// `shutdown()` skips those so we don't kill what the new host is using.
    transferred_to_successor: tokio::sync::Mutex<std::collections::HashSet<String>>,
}

impl PluginHost {
    pub fn new_empty() -> Arc<Self> {
        let entries = Arc::new(RwLock::new(PluginEntryRegistry::new()));
        let statuses = Arc::new(crate::status::StatusRegistry::new());
        let logs = Arc::new(PluginLogStore::default());
        let host_handle = Arc::new(HostHandle::new_with_components(
            Arc::new(NoopHostClient),
            Arc::clone(&entries),
            Arc::new(RwLock::new(HashMap::new())),
            Arc::clone(&statuses),
            Arc::clone(&logs),
        ));
        Arc::new(Self {
            plugins: Vec::new(),
            plugins_by_id: HashMap::new(),
            entries,
            statuses,
            logs,
            timeouts: TimeoutsConfig::default(),
            runtime: None,
            runtime_handle: None,
            _host_handle: host_handle,
            transferred_to_successor: tokio::sync::Mutex::new(Default::default()),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn plugins(&self) -> &[Arc<LoadedPlugin>] {
        &self.plugins
    }

    pub fn plugin_summary(&self) -> (usize, BTreeMap<&'static str, usize>) {
        let mut by_kind: BTreeMap<&'static str, usize> = BTreeMap::new();
        for p in &self.plugins {
            *by_kind.entry(p.kind).or_insert(0) += 1;
        }
        (self.plugins.len(), by_kind)
    }

    pub fn lookup_entry(&self, exposed_name: &str) -> Option<RegistryPluginEntry> {
        self.entries.read().ok()?.lookup(exposed_name).cloned()
    }

    pub fn entry_entries(&self) -> Vec<RegistryPluginEntry> {
        self.entries
            .read()
            .map(|reg| reg.entries_owned())
            .unwrap_or_default()
    }

    pub fn entry_snapshot(&self) -> crate::registry::PluginEntrySnapshot {
        self.entries
            .read()
            .map(|reg| reg.snapshot())
            .unwrap_or_else(|_| crate::registry::PluginEntrySnapshot {
                generation: 0,
                entries: Vec::new(),
            })
    }

    pub fn entry_generation(&self) -> u64 {
        self.entries.read().map(|reg| reg.generation()).unwrap_or(0)
    }

    pub fn status_registry(&self) -> Arc<crate::status::StatusRegistry> {
        Arc::clone(&self.statuses)
    }

    pub fn plugin_status(&self, plugin_id: &str) -> Option<crate::status::PluginStatus> {
        self.statuses.get(plugin_id)
    }

    pub fn plugin_statuses(&self) -> Vec<crate::status::PluginStatus> {
        self.statuses.list()
    }

    pub fn log_store(&self) -> Arc<PluginLogStore> {
        Arc::clone(&self.logs)
    }

    pub fn append_plugin_log(
        &self,
        plugin_id: impl Into<String>,
        level: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
        fields: serde_json::Value,
    ) -> PluginLogEntry {
        self.logs.append(plugin_id, level, source, message, fields)
    }

    pub fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<PluginLogEntry> {
        self.logs.list(plugin_id, after_seq, limit)
    }

    pub fn plugin_inspect(&self, plugin_id: &str) -> Option<PluginInspect> {
        let status = self.plugin_status(plugin_id)?;
        let plugin = self.plugins_by_id.get(plugin_id);
        let manifest = plugin.as_ref().map(|plugin| plugin.manifest.clone());
        let authority = plugin.map(|plugin| plugin.authority_summary());
        let entry = plugin.as_ref().map(|plugin| plugin.entry.clone());
        Some(PluginInspect {
            status,
            manifest,
            authority,
            entry,
        })
    }

    fn block_on<F>(&self, fut: F) -> F::Output
    where
        F: std::future::Future + Send,
        F::Output: Send,
    {
        let current = tokio::runtime::Handle::try_current().ok();
        let current_is_multithread = current.as_ref().is_some_and(|handle| {
            handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread
        });

        if let Some(rt) = &self.runtime {
            if current.is_some() {
                return if current_is_multithread {
                    tokio::task::block_in_place(|| rt.block_on(fut))
                } else {
                    block_on_runtime_scoped_thread(rt, fut)
                };
            }
            return rt.block_on(fut);
        }

        if let Some(handle) = &self.runtime_handle {
            if current.is_some() {
                return if current_is_multithread {
                    tokio::task::block_in_place(|| handle.block_on(fut))
                } else {
                    block_on_handle_scoped_thread(handle, fut)
                };
            }
            return handle.block_on(fut);
        }

        if let Some(handle) = current {
            return if current_is_multithread {
                tokio::task::block_in_place(|| handle.block_on(fut))
            } else {
                block_on_scoped_thread(fut)
            };
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("plugin host fallback runtime");
        rt.block_on(fut)
    }

    fn block_on_static<F>(&self, fut: F) -> F::Output
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        if let Some(rt) = &self.runtime {
            rt.block_on(fut)
        } else if let Some(handle) = &self.runtime_handle {
            block_on_handle_or_thread(handle.clone(), fut)
        } else if let Ok(handle) = tokio::runtime::Handle::try_current() {
            block_on_handle_or_thread(handle, fut)
        } else {
            block_on_new_thread(fut)
        }
    }

    // ------------------- sync wrappers used by ToolExecutor -------------------

    pub fn dispatch_tool_before(
        &self,
        input: ToolBeforeInput,
    ) -> Result<ToolBeforeInput, PluginError> {
        let timeout = self.timeouts.tool_hook_or(Duration::from_secs(30));
        let plugins = self.plugins.clone();
        let res = self.block_on_static(async move {
            dispatcher::chain_patch_with_context::<ToolBeforeInput, ToolBeforePatch, _, _>(
                &plugins,
                method::HOOK_TOOL_BEFORE,
                HookSubscription::TOOL_BEFORE,
                timeout,
                input,
                |inp, patch| {
                    if let Some(v) = patch.input {
                        inp.input = v;
                    }
                    if let Some(t) = patch.title_override {
                        inp.title_override = Some(t);
                    }
                    for (k, v) in patch.metadata {
                        inp.metadata.insert(k, v);
                    }
                },
                |plugin, input| {
                    Some(tool_hook_context(
                        plugin,
                        &input.tool_name,
                        Some(input.session_id),
                        Some(input.call_id),
                        Some(input.workspace_root.clone()),
                    ))
                },
            )
            .await
        });
        res.map_err(transport_to_plugin_error)
    }

    pub fn dispatch_tool_after(
        &self,
        input: ToolAfterInput,
    ) -> Result<ToolAfterInput, PluginError> {
        let timeout = self.timeouts.tool_hook_or(Duration::from_secs(30));
        let plugins = self.plugins.clone();
        let res = self.block_on_static(async move {
            dispatcher::chain_patch_with_context::<ToolAfterInput, ToolAfterPatch, _, _>(
                &plugins,
                method::HOOK_TOOL_AFTER,
                HookSubscription::TOOL_AFTER,
                timeout,
                input,
                |inp, patch| {
                    if let Some(t) = patch.title {
                        inp.title = t;
                    }
                    if let Some(o) = patch.output_text {
                        inp.output_text = o;
                    }
                    if let Some(p) = patch.payload {
                        inp.payload = Some(p);
                    }
                    for (k, v) in patch.metadata {
                        inp.metadata.insert(k, v);
                    }
                },
                |plugin, input| {
                    Some(tool_hook_context(
                        plugin,
                        &input.tool_name,
                        Some(input.session_id),
                        Some(input.call_id),
                        Some(input.workspace_root.clone()),
                    ))
                },
            )
            .await
        });
        res.map_err(transport_to_plugin_error)
    }

    pub fn invoke_tool(
        &self,
        entry: &RegistryPluginEntry,
        input: ToolInvokeInput,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let plugin = self
            .plugins_by_id
            .get(&entry.plugin_name)
            .cloned()
            .ok_or_else(|| {
                PluginError::new(format!("plugin `{}` not loaded", entry.plugin_name))
            })?;
        let timeout = self.timeouts.tool_invoke_or(Duration::from_secs(300));
        let mut input = input;
        // ensure tool name is the plugin-original name (in case caller passed exposed)
        input.tool_name = entry.original_name.clone();
        let session_id = input.session_id;
        let call_id = input.call_id;
        let workspace_root = input.workspace_root.clone();
        let plugin_id = entry.plugin_name.clone();
        let entry_name = entry.original_name.clone();
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let result = self.block_on_static(async move {
            host_api::with_host_callback_context(
                HostCallbackContext {
                    plugin_id: Some(plugin_id),
                    session_id: Some(session_id),
                    call_id: Some(call_id),
                    workspace_root: Some(workspace_root),
                    entry_name: Some(entry_name),
                },
                call_with_timeout(&plugin, method::HOOK_TOOL_INVOKE, params, timeout),
            )
            .await
        });
        let value = result.map_err(transport_to_plugin_error)?;
        serde_json::from_value(value).map_err(|e| PluginError::invalid_params(e.to_string()))
    }

    pub fn dispatch_tool_permission_paths(
        &self,
        entry: &RegistryPluginEntry,
        input: ToolPermissionPathsInput,
    ) -> Result<Vec<crate::sdk::PathRequest>, PluginError> {
        let plugin = self
            .plugins_by_id
            .get(&entry.plugin_name)
            .cloned()
            .ok_or_else(|| {
                PluginError::new(format!("plugin `{}` not loaded", entry.plugin_name))
            })?;
        let timeout = self.timeouts.tool_hook_or(Duration::from_secs(30));
        let mut input = input;
        input.tool_name = entry.original_name.clone();
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let plugin_id = entry.plugin_name.clone();
        let entry_name = entry.original_name.clone();
        let workspace_root = input.workspace_root.clone();
        let result = self.block_on_static(async move {
            host_api::with_host_callback_context(
                HostCallbackContext {
                    plugin_id: Some(plugin_id),
                    workspace_root: Some(workspace_root),
                    entry_name: Some(entry_name),
                    ..Default::default()
                },
                call_with_timeout(&plugin, method::HOOK_TOOL_PERMISSION_PATHS, params, timeout),
            )
            .await
        });
        let value = result.map_err(transport_to_plugin_error)?;
        serde_json::from_value(value).map_err(|e| PluginError::invalid_params(e.to_string()))
    }

    pub fn dispatch_tool_permission_networks(
        &self,
        entry: &RegistryPluginEntry,
        input: ToolPermissionNetworksInput,
    ) -> Result<Vec<crate::sdk::NetworkRequest>, PluginError> {
        let plugin = self
            .plugins_by_id
            .get(&entry.plugin_name)
            .cloned()
            .ok_or_else(|| {
                PluginError::new(format!("plugin `{}` not loaded", entry.plugin_name))
            })?;
        let timeout = self.timeouts.tool_hook_or(Duration::from_secs(30));
        let mut input = input;
        input.tool_name = entry.original_name.clone();
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let plugin_id = entry.plugin_name.clone();
        let entry_name = entry.original_name.clone();
        let workspace_root = input.workspace_root.clone();
        let result = self.block_on_static(async move {
            host_api::with_host_callback_context(
                HostCallbackContext {
                    plugin_id: Some(plugin_id),
                    workspace_root: Some(workspace_root),
                    entry_name: Some(entry_name),
                    ..Default::default()
                },
                call_with_timeout(
                    &plugin,
                    method::HOOK_TOOL_PERMISSION_NETWORKS,
                    params,
                    timeout,
                ),
            )
            .await
        });
        let value = result.map_err(transport_to_plugin_error)?;
        serde_json::from_value(value).map_err(|e| PluginError::invalid_params(e.to_string()))
    }

    /// Streaming variant: returns a receiver of [`ToolStreamChunk`]s plus a
    /// oneshot for the terminal [`ToolStreamEnd`] (or error). Transports with
    /// native stream support should surface it through `PluginTransport`;
    /// others fall back to a single-chunk emulation built from the regular
    /// `tool_invoke` response.
    pub async fn invoke_tool_stream(
        &self,
        entry: &RegistryPluginEntry,
        input: ToolInvokeInput,
    ) -> Result<ToolInvokeStream, PluginError> {
        let plugin = self
            .plugins_by_id
            .get(&entry.plugin_name)
            .cloned()
            .ok_or_else(|| {
                PluginError::new(format!("plugin `{}` not loaded", entry.plugin_name))
            })?;
        let mut input = input;
        input.tool_name = entry.original_name.clone();

        let context = tool_hook_context(
            &plugin,
            &input.tool_name,
            Some(input.session_id),
            Some(input.call_id),
            Some(input.workspace_root.clone()),
        );
        if let Some(stream) = host_api::with_host_callback_context(
            context.clone(),
            plugin.transport.invoke_stream(input.clone()),
        )
        .await
        .map_err(transport_to_plugin_error)?
        {
            return Ok(ToolInvokeStream {
                stream_id: stream.stream_id,
                chunks: stream.chunks,
                end: stream.end,
            });
        }

        let timeout = self.timeouts.tool_invoke_or(Duration::from_secs(300));
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let invoke_result = host_api::with_host_callback_context(
            context,
            call_with_timeout(&plugin, method::HOOK_TOOL_INVOKE, params, timeout),
        )
        .await
        .map_err(transport_to_plugin_error)?;
        let result: ToolInvokeOutput = serde_json::from_value(invoke_result)
            .map_err(|e| PluginError::invalid_params(e.to_string()))?;

        let (tx, rx) = tokio::sync::mpsc::channel::<ToolStreamChunk>(8);
        let (end_tx, end_rx) = tokio::sync::oneshot::channel();
        let stream_id = format!("emu-{}", uuid::Uuid::new_v4().simple());
        let chunk = ToolStreamChunk {
            stream_id: stream_id.clone(),
            text_delta: Some(result.output_text.clone()),
            payload_delta: result.payload.clone(),
            metadata: result.metadata.clone(),
        };
        let _ = tx.send(chunk).await;
        drop(tx);
        let _ = end_tx.send(Ok(ToolStreamEnd {
            stream_id: stream_id.clone(),
            title: result.title,
            output_text: result.output_text,
            payload: result.payload,
            metadata: result.metadata,
            attachments: result.attachments,
        }));
        Ok(ToolInvokeStream {
            stream_id,
            chunks: rx,
            end: end_rx,
        })
    }

    pub fn dispatch_shell_env(&self, input: ShellEnvInput) -> Result<ShellEnvPatch, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let plugins = self.plugins.clone();
        let res: Result<ShellEnvPatch, TransportError> = self.block_on_static(async move {
            let mut acc = ShellEnvPatch::default();
            for plugin in &plugins {
                if !plugin.subscribes(HookSubscription::SHELL_ENV) {
                    continue;
                }
                let params = serde_json::to_value(&input)?;
                let result =
                    call_with_timeout(plugin, method::HOOK_SHELL_ENV, params, timeout).await?;
                if matches!(&result, serde_json::Value::Null) {
                    continue;
                }
                let patch: Option<ShellEnvPatch> = serde_json::from_value(result)?;
                if let Some(p) = patch {
                    for (k, v) in p.set {
                        acc.set.insert(k, v);
                    }
                    for k in p.unset {
                        acc.set.remove(&k);
                        acc.unset.push(k);
                    }
                }
            }
            Ok(acc)
        });
        res.map_err(transport_to_plugin_error)
    }

    // -------------- async-only helpers for chat / permission etc. --------------

    pub async fn dispatch_chat_message(
        &self,
        input: ChatMessageInput,
    ) -> Result<ChatMessageInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        dispatcher::chain_patch::<ChatMessageInput, ChatMessagePatch, _>(
            &self.plugins,
            method::HOOK_CHAT_MESSAGE,
            HookSubscription::CHAT_MESSAGE,
            timeout,
            input,
            |inp, patch| {
                if let Some(m) = patch.message {
                    inp.message = m;
                }
                if patch.drop {
                    inp.message.content = serde_json::Value::Null;
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    pub async fn dispatch_chat_params(
        &self,
        input: ChatParamsInput,
    ) -> Result<ChatParamsInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        dispatcher::chain_patch::<ChatParamsInput, ChatParamsPatch, _>(
            &self.plugins,
            method::HOOK_CHAT_PARAMS,
            HookSubscription::CHAT_PARAMS,
            timeout,
            input,
            |inp, patch| {
                if let Some(p) = patch.params {
                    merge_json(&mut inp.params, p);
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    pub async fn dispatch_chat_headers(
        &self,
        input: ChatHeadersInput,
    ) -> Result<ChatHeadersInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        dispatcher::chain_patch::<ChatHeadersInput, ChatHeadersPatch, _>(
            &self.plugins,
            method::HOOK_CHAT_HEADERS,
            HookSubscription::CHAT_HEADERS,
            timeout,
            input,
            |inp, patch| {
                for (k, v) in patch.set {
                    inp.headers.insert(k, v);
                }
                for k in patch.remove {
                    inp.headers.remove(&k);
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    /// Sync variant for code paths driven from a non-async context (the
    /// provider request building path runs `block_on` from sync helpers).
    pub fn dispatch_chat_headers_blocking(
        &self,
        input: ChatHeadersInput,
    ) -> Result<ChatHeadersInput, PluginError> {
        self.block_on(self.dispatch_chat_headers(input))
    }

    pub async fn dispatch_chat_system_transform(
        &self,
        input: ChatSystemTransformInput,
    ) -> Result<ChatSystemTransformInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        dispatcher::chain_patch::<ChatSystemTransformInput, ChatSystemTransformPatch, _>(
            &self.plugins,
            method::HOOK_CHAT_SYSTEM_TRANSFORM,
            HookSubscription::CHAT_SYSTEM_TRANSFORM,
            timeout,
            input,
            |inp, patch| {
                if let Some(p) = patch.prepend {
                    inp.current_system = format!("{p}\n{}", inp.current_system);
                }
                if let Some(a) = patch.append {
                    inp.current_system = format!("{}\n{a}", inp.current_system);
                }
                if let Some(s) = patch.system {
                    inp.current_system = s;
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    pub async fn dispatch_permission_ask(
        &self,
        input: PermissionAskInput,
    ) -> Result<Option<PermissionAskOutcome>, PluginError> {
        let timeout = self.timeouts.permission_ask_or(Duration::from_secs(60));
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::PERMISSION_ASK) {
                continue;
            }
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_permission_ask_hook(plugin, params, timeout).await?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let decision: Option<PermissionAskDecision> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            match decision {
                Some(PermissionAskDecision::Decide(d)) => {
                    if !plugin_has_capability(plugin, HostCapability::PermissionDecision) {
                        tracing::warn!(
                            target: "agena_plugin_host::permission",
                            plugin = %plugin.id,
                            "permission.ask_permission returned Decide without PermissionDecision capability; treating as advice"
                        );
                        return Ok(Some(PermissionAskOutcome::Advice {
                            plugin_id: plugin.id.clone(),
                            advice: PermissionAdvice {
                                decision: d,
                                reason: format!(
                                    "plugin {} requested a permission decision without PermissionDecision capability",
                                    plugin.id
                                ),
                                risk: crate::sdk::PermissionRiskLevel::Medium,
                                requested_scope: None,
                            },
                            authority: plugin.authority_summary(),
                        }));
                    }
                    return Ok(Some(PermissionAskOutcome::Decision {
                        plugin_id: plugin.id.clone(),
                        decision: d,
                        authority: plugin.authority_summary(),
                    }));
                }
                Some(PermissionAskDecision::Advise(advice)) => {
                    return Ok(Some(PermissionAskOutcome::Advice {
                        plugin_id: plugin.id.clone(),
                        advice,
                        authority: plugin.authority_summary(),
                    }));
                }
                _ => {}
            }
        }
        Ok(None)
    }

    /// Sync wrapper for callers running in a non-async context (e.g.
    /// `PermissionRuntime`).
    pub fn dispatch_permission_ask_blocking(
        &self,
        input: PermissionAskInput,
    ) -> Result<Option<PermissionAskOutcome>, PluginError> {
        self.block_on(self.dispatch_permission_ask(input))
    }

    pub async fn broadcast_notification(&self, input: NotificationInput) {
        let timeout = Duration::from_secs(5);
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::NOTIFICATION) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            tokio::spawn(async move {
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ = tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_NOTIFICATION, params),
                )
                .await;
            });
        }
    }

    pub async fn dispatch_command_before(
        &self,
        input: CommandBeforeInput,
    ) -> Result<CommandBeforeOutcome, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let mut current = input;
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::COMMAND_BEFORE) {
                continue;
            }
            let params = serde_json::to_value(&current)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_COMMAND_BEFORE, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let resp: Option<CommandBeforeResponse> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            match resp {
                Some(CommandBeforeResponse::Abort { reason }) => {
                    return Ok(CommandBeforeOutcome::Abort(reason));
                }
                Some(CommandBeforeResponse::Patch(p)) => {
                    if let Some(c) = p.command {
                        current.command = c;
                    }
                    if let Some(a) = p.args {
                        current.args = a;
                    }
                    if let Some(c) = p.cwd {
                        current.cwd = c;
                    }
                    if let Some(env) = p.env {
                        for (k, v) in env {
                            current.env.insert(k, v);
                        }
                    }
                }
                None => {}
            }
        }
        Ok(CommandBeforeOutcome::Continue(current))
    }

    pub fn dispatch_command_before_blocking(
        &self,
        input: CommandBeforeInput,
    ) -> Result<CommandBeforeOutcome, PluginError> {
        self.block_on(self.dispatch_command_before(input))
    }

    pub async fn dispatch_auth(&self, input: AuthInput) -> Result<Option<AuthOutput>, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::AUTH) {
                continue;
            }
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_AUTH, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let out: Option<AuthOutput> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if out.is_some() {
                return Ok(out);
            }
        }
        Ok(None)
    }

    pub async fn dispatch_provider_list(
        &self,
        input: ProviderListInput,
    ) -> Result<ProviderListPatch, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let mut acc = ProviderListPatch::default();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::PROVIDER_LIST) {
                continue;
            }
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_PROVIDER_LIST, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let patch: Option<ProviderListPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch {
                acc.add.extend(p.add);
                acc.remove.extend(p.remove);
            }
        }
        Ok(acc)
    }

    pub async fn dispatch_config(&self, input: ConfigInput) -> Result<ConfigInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        dispatcher::chain_patch::<ConfigInput, ConfigPatch, _>(
            &self.plugins,
            method::HOOK_CONFIG,
            HookSubscription::CONFIG,
            timeout,
            input,
            |inp, patch| {
                if let Some(m) = patch.merge {
                    merge_json(&mut inp.current, m);
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    // ── run lifecycle ──────────────────────────────────────────────────────

    pub async fn broadcast_pre_run(&self, input: PreRunInput) {
        self.broadcast_lifecycle(method::HOOK_PRE_RUN, HookSubscription::PRE_RUN, input)
            .await;
    }

    pub async fn broadcast_post_run(&self, input: PostRunInput) {
        self.broadcast_lifecycle(method::HOOK_POST_RUN, HookSubscription::POST_RUN, input)
            .await;
    }

    async fn broadcast_lifecycle<T>(
        &self,
        method: &'static str,
        subscription: HookSubscription,
        input: T,
    ) where
        T: serde::Serialize + Clone + Send + 'static,
    {
        let timeout = Duration::from_secs(5);
        for plugin in &self.plugins {
            if !plugin.subscribes(subscription) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            tokio::spawn(async move {
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ =
                    tokio::time::timeout(timeout, plugin.transport.notify(method, params)).await;
            });
        }
    }

    // ── session.start ──────────────────────────────────────────────────────

    pub async fn dispatch_session_start(
        &self,
        input: SessionStartInput,
    ) -> Result<SessionStartPatch, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        let mut acc = SessionStartPatch::default();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::SESSION_START) {
                continue;
            }
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_SESSION_START, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let patch: Option<SessionStartPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch {
                if let Some(ctx) = p.additional_context {
                    let existing = acc.additional_context.get_or_insert_with(String::new);
                    if !existing.is_empty() {
                        existing.push('\n');
                    }
                    existing.push_str(&ctx);
                }
                if p.initial_user_message.is_some() {
                    acc.initial_user_message = p.initial_user_message;
                }
            }
        }
        Ok(acc)
    }

    // ── session.end ────────────────────────────────────────────────────────

    pub async fn broadcast_session_end(&self, input: SessionEndInput) {
        let timeout = Duration::from_secs(5);
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::SESSION_END) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            tokio::spawn(async move {
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ = tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_SESSION_END, params),
                )
                .await;
            });
        }
    }

    // ── user.prompt.submit ─────────────────────────────────────────────────

    pub async fn dispatch_user_prompt_submit(
        &self,
        input: UserPromptSubmitInput,
    ) -> Result<UserPromptSubmitInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        let mut current = input;
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::USER_PROMPT_SUBMIT) {
                continue;
            }
            let params = serde_json::to_value(&current)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_USER_PROMPT_SUBMIT, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let patch: Option<UserPromptSubmitPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch {
                if let Some(r) = p.block_reason {
                    return Err(PluginError::new(format!("prompt blocked: {r}")));
                }
                if let Some(text) = p.prompt {
                    current.prompt = text;
                }
                if let Some(ctx) = p.additional_context {
                    current.prompt.push('\n');
                    current.prompt.push_str(&ctx);
                }
            }
        }
        Ok(current)
    }

    /// Blocking variant for callers in sync context.
    pub fn dispatch_user_prompt_submit_blocking(
        &self,
        input: UserPromptSubmitInput,
    ) -> Result<UserPromptSubmitInput, PluginError> {
        self.block_on(self.dispatch_user_prompt_submit(input))
    }

    // ── tool.execute.failure ───────────────────────────────────────────────

    pub async fn broadcast_tool_failure(&self, input: ToolFailureInput) {
        let timeout = Duration::from_secs(5);
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::TOOL_FAILURE) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            tokio::spawn(async move {
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ = tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_TOOL_FAILURE, params),
                )
                .await;
            });
        }
    }

    // ── tool.definition ────────────────────────────────────────────────────

    pub async fn dispatch_tool_definition(
        &self,
        input: ToolDefinitionInput,
    ) -> Result<ToolDefinitionInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        dispatcher::chain_patch_with_context::<ToolDefinitionInput, ToolDefinitionPatch, _, _>(
            &self.plugins,
            method::HOOK_TOOL_DEFINITION,
            HookSubscription::TOOL_DEFINITION,
            timeout,
            input,
            |inp, patch| {
                if let Some(d) = patch.description {
                    inp.description = d;
                }
                if patch.summary.is_some() {
                    inp.summary = patch.summary;
                }
                if patch.help.is_some() {
                    inp.help = patch.help;
                }
                if patch.description_mode.is_some() {
                    inp.description_mode = patch.description_mode;
                }
                if let Some(s) = patch.input_schema {
                    inp.input_schema = s;
                }
            },
            |plugin, input| {
                Some(tool_hook_context(
                    plugin,
                    &input.tool_name,
                    None,
                    None,
                    None,
                ))
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    pub fn dispatch_tool_definition_blocking(
        &self,
        input: ToolDefinitionInput,
    ) -> Result<ToolDefinitionInput, PluginError> {
        self.block_on(self.dispatch_tool_definition(input))
    }

    // ── agent.stop ─────────────────────────────────────────────────────────

    pub async fn dispatch_agent_stop(
        &self,
        input: AgentStopInput,
    ) -> Result<AgentStopPatch, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(30));
        let mut acc = AgentStopPatch::default();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::AGENT_STOP) {
                continue;
            }
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let context = HostCallbackContext {
                plugin_id: Some(plugin.id.clone()),
                session_id: Some(input.session_id),
                ..Default::default()
            };
            let v = host_api::with_host_callback_context(
                context,
                call_with_timeout(plugin, method::HOOK_AGENT_STOP, params, timeout),
            )
            .await
            .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let patch: Option<AgentStopPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch
                && p.continue_with_message.is_some()
            {
                acc.continue_with_message = p.continue_with_message;
                acc.reason = p.reason;
                // First plugin that wants to block stop wins.
                break;
            }
        }
        Ok(acc)
    }

    // ── command.execute.after ──────────────────────────────────────────────

    pub async fn dispatch_command_after(
        &self,
        input: CommandAfterInput,
    ) -> Result<CommandAfterInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        dispatcher::chain_patch::<CommandAfterInput, CommandAfterPatch, _>(
            &self.plugins,
            method::HOOK_COMMAND_AFTER,
            HookSubscription::COMMAND_AFTER,
            timeout,
            input,
            |inp, patch| {
                if let Some(s) = patch.stdout {
                    inp.stdout = s;
                }
                if let Some(s) = patch.stderr {
                    inp.stderr = s;
                }
                if patch.exit_code.is_some() {
                    inp.exit_code = patch.exit_code;
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    pub fn dispatch_command_after_blocking(
        &self,
        input: CommandAfterInput,
    ) -> Result<CommandAfterInput, PluginError> {
        self.block_on(self.dispatch_command_after(input))
    }

    // ── chat.messages.transform ────────────────────────────────────────────

    pub async fn dispatch_chat_messages_transform(
        &self,
        input: ChatMessagesTransformInput,
    ) -> Result<ChatMessagesTransformInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(10));
        dispatcher::chain_patch::<ChatMessagesTransformInput, ChatMessagesTransformPatch, _>(
            &self.plugins,
            method::HOOK_CHAT_MESSAGES_TRANSFORM,
            HookSubscription::CHAT_MESSAGES_TRANSFORM,
            timeout,
            input,
            |inp, patch| {
                if let Some(msgs) = patch.messages {
                    inp.messages = msgs;
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    /// Push an `EventEnvelope` to every subscribed plugin (best-effort, no
    /// error propagation — events are notifications).
    pub async fn broadcast_event(&self, env: EventEnvelope) {
        let timeout = Duration::from_secs(2);
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::EVENT) {
                continue;
            }
            let env = env.clone();
            let plugin = plugin.clone();
            tokio::spawn(async move {
                let params = match serde_json::to_value(&env) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ = tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_EVENT, params),
                )
                .await;
            });
        }
    }

    /// Async shutdown — sends `meta/shutdown` and closes every transport.
    /// Plugins whose transport has been transferred to a successor host
    /// (via [`PluginHostBuilder::with_previous`] hot-reload) are skipped.
    pub async fn shutdown(&self) {
        let transferred = self.transferred_to_successor.lock().await.clone();
        for plugin in &self.plugins {
            if transferred.contains(&plugin.id) {
                continue;
            }
            let _ = shutdown_transport(Arc::clone(&plugin.transport)).await;
        }
    }

    /// Direct access to the host-side bidirectional router. Used by HTTP
    /// callback routes and bidirectional stdio transports.
    pub fn host_handle(&self) -> Arc<HostHandle> {
        Arc::clone(&self._host_handle)
    }

    pub fn statusline_segments(&self) -> Vec<HostStatuslineSegment> {
        self.ui_catalog().tui.statusline_segments
    }

    pub fn theme_palettes(&self) -> Vec<HostThemePalette> {
        self.ui_catalog().tui.themes
    }

    pub fn tui_content_blocks(&self) -> Vec<PluginTuiContentBlockCatalogItem> {
        self.ui_catalog().tui.content_blocks
    }

    pub fn studio_commands(&self) -> Vec<PluginStudioCommandCatalogItem> {
        self.ui_catalog().studio.commands
    }

    pub fn studio_controls(&self) -> Vec<PluginStudioControlCatalogItem> {
        self.ui_catalog().studio.controls
    }

    pub fn studio_views(&self) -> Vec<PluginStudioViewCatalogItem> {
        self.ui_catalog().studio.views
    }

    pub fn ui_catalog(&self) -> PluginUiCatalog {
        let mut statusline_by_key = BTreeMap::<(String, String), HostStatuslineSegment>::new();
        let mut themes_by_id = BTreeMap::<String, HostThemePalette>::new();
        let mut content_blocks = Vec::new();
        let mut studio_commands = Vec::new();
        let mut studio_controls = Vec::new();
        let mut studio_views = Vec::new();

        for plugin in &self.plugins {
            for segment in &plugin.manifest.ui.tui.statusline_segments {
                let resolved = HostStatuslineSegment {
                    plugin_id: plugin.id.clone(),
                    segment_id: segment.id.clone(),
                    content: segment.content.clone(),
                    priority: segment.priority,
                    color: segment.color.clone(),
                };
                statusline_by_key.insert(
                    (resolved.plugin_id.clone(), resolved.segment_id.clone()),
                    resolved,
                );
            }

            for theme in &plugin.manifest.ui.tui.themes {
                themes_by_id.insert(
                    theme.id.clone(),
                    HostThemePalette {
                        id: theme.id.clone(),
                        plugin_id: plugin.id.clone(),
                        display_name: theme.display_name.clone(),
                        colors: theme.colors.clone(),
                    },
                );
            }

            content_blocks.extend(plugin.manifest.ui.tui.content_blocks.iter().cloned().map(
                |block| PluginTuiContentBlockCatalogItem {
                    plugin_id: plugin.id.clone(),
                    block,
                },
            ));

            studio_commands.extend(plugin.manifest.ui.studio.commands.iter().cloned().map(
                |command| PluginStudioCommandCatalogItem {
                    plugin_id: plugin.id.clone(),
                    command,
                },
            ));

            studio_controls.extend(plugin.manifest.ui.studio.controls.iter().cloned().map(
                |control| PluginStudioControlCatalogItem {
                    plugin_id: plugin.id.clone(),
                    control,
                },
            ));

            studio_views.extend(plugin.manifest.ui.studio.views.iter().cloned().map(|view| {
                PluginStudioViewCatalogItem {
                    plugin_id: plugin.id.clone(),
                    view,
                }
            }));
        }

        for segment in self._host_handle.statusline_list_response().segments {
            statusline_by_key.insert(
                (segment.plugin_id.clone(), segment.segment_id.clone()),
                segment,
            );
        }
        for theme in self._host_handle.theme_list_response().themes {
            themes_by_id.insert(theme.id.clone(), theme);
        }

        let mut statusline_segments = statusline_by_key.into_values().collect::<Vec<_>>();
        statusline_segments.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
                .then_with(|| a.segment_id.cmp(&b.segment_id))
        });
        let themes = themes_by_id.into_values().collect::<Vec<_>>();
        content_blocks.sort_by(|a, b| {
            b.block
                .priority
                .cmp(&a.block.priority)
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
                .then_with(|| a.block.id.cmp(&b.block.id))
        });
        studio_commands.sort_by(|a, b| {
            a.command
                .location
                .cmp(&b.command.location)
                .then_with(|| a.command.category.cmp(&b.command.category))
                .then_with(|| a.command.title.cmp(&b.command.title))
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
        });
        studio_controls.sort_by(|a, b| {
            a.control
                .location
                .cmp(&b.control.location)
                .then_with(|| a.control.title.cmp(&b.control.title))
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
        });
        studio_views.sort_by(|a, b| {
            a.view
                .location
                .cmp(&b.view.location)
                .then_with(|| a.view.title.cmp(&b.view.title))
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
        });

        PluginUiCatalog {
            tui: PluginTuiUiCatalog {
                statusline_segments,
                themes,
                content_blocks,
            },
            studio: PluginStudioUiCatalog {
                commands: studio_commands,
                controls: studio_controls,
                views: studio_views,
            },
        }
    }

    pub fn resolve_studio_action(
        &self,
        plugin_id: &str,
        action_id: &str,
    ) -> Option<PluginUiAction> {
        let plugin = self.plugins_by_id.get(plugin_id)?;
        for command in &plugin.manifest.ui.studio.commands {
            if command.id == action_id {
                return Some(command.action.clone());
            }
        }
        for control in &plugin.manifest.ui.studio.controls {
            if control.id == action_id {
                return Some(control.action.clone());
            }
        }
        for view in &plugin.manifest.ui.studio.views {
            for control in &view.controls {
                if control.id == action_id {
                    return Some(control.action.clone());
                }
            }
        }
        None
    }

    pub fn resolve_entry_for_plugin_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
    ) -> Option<RegistryPluginEntry> {
        self.entry_entries().into_iter().find(|entry| {
            entry.plugin_name == plugin_id
                && (entry.original_name == tool_name || entry.exposed_name == tool_name)
        })
    }
}

fn tool_hook_context(
    plugin: &LoadedPlugin,
    tool_name: &str,
    session_id: Option<i64>,
    call_id: Option<i64>,
    workspace_root: Option<String>,
) -> HostCallbackContext {
    HostCallbackContext {
        plugin_id: Some(plugin.id.clone()),
        session_id,
        call_id,
        workspace_root,
        entry_name: Some(
            plugin
                .entry_name_for_tool(tool_name)
                .unwrap_or_else(|| tool_name.to_string()),
        ),
    }
}

fn plugin_has_capability(plugin: &LoadedPlugin, capability: HostCapability) -> bool {
    effective_host_capabilities_for_manifest(
        &plugin.manifest.entries,
        &plugin.manifest.plugin_capabilities,
    )
    .contains(&capability)
}

fn transport_to_plugin_error(e: TransportError) -> PluginError {
    match e {
        TransportError::Plugin(pe) => pe,
        other => PluginError::new(other.to_string()),
    }
}

fn is_not_implemented_plugin_error(err: &PluginError) -> bool {
    err.code == PluginErrorCode::NotImplemented
        || err.message.contains("method not found")
        || err.message.contains("not implemented")
}

async fn call_permission_ask_hook(
    plugin: &LoadedPlugin,
    params: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, PluginError> {
    match call_with_timeout(plugin, method::HOOK_PERMISSION_ASK, params.clone(), timeout).await {
        Ok(value) => Ok(value),
        Err(err) => {
            let err = transport_to_plugin_error(err);
            if is_not_implemented_plugin_error(&err) {
                call_with_timeout(plugin, method::HOOK_PERMISSION_ASK_LEGACY, params, timeout)
                    .await
                    .map_err(transport_to_plugin_error)
            } else {
                Err(err)
            }
        }
    }
}

async fn dispatch_permission_ask_transport(
    transport: Arc<dyn PluginTransport>,
    context: HostCallbackContext,
    params: serde_json::Value,
) -> Result<serde_json::Value, PluginError> {
    match host_api::with_host_callback_context(
        context.clone(),
        transport.dispatch(method::HOOK_PERMISSION_ASK, params.clone()),
    )
    .await
    {
        Ok(value) => Ok(value),
        Err(err) => {
            let err = transport_to_plugin_error(err);
            if is_not_implemented_plugin_error(&err) {
                host_api::with_host_callback_context(
                    context,
                    transport.dispatch(method::HOOK_PERMISSION_ASK_LEGACY, params),
                )
                .await
                .map_err(transport_to_plugin_error)
            } else {
                Err(err)
            }
        }
    }
}

fn merge_json(into: &mut serde_json::Value, from: serde_json::Value) {
    match (into, from) {
        (serde_json::Value::Object(map_into), serde_json::Value::Object(map_from)) => {
            for (k, v) in map_from {
                merge_json(map_into.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (slot, value) => {
            *slot = value;
        }
    }
}

// ---------- builder ----------

pub struct PluginHostBuilder {
    static_plugins: HashMap<String, StaticRegistration>,
    config: PluginsConfig,
    workspace_root: PathBuf,
    agena_version: String,
    callback_base_url: Option<String>,
    host_client: Option<Arc<dyn HostClient>>,
    /// Optional previous host: for any entry whose config is byte-identical
    /// to the previous run, the old transport is reused (hot-reload).
    previous: Option<Arc<PluginHost>>,
    previous_entries: HashMap<String, PluginEntry>,
}

impl PluginHostBuilder {
    pub fn new(workspace_root: impl Into<PathBuf>, agena_version: impl Into<String>) -> Self {
        Self {
            static_plugins: HashMap::new(),
            config: PluginsConfig::default(),
            workspace_root: workspace_root.into(),
            agena_version: agena_version.into(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_entries: HashMap::new(),
        }
    }

    pub fn with_config(mut self, config: PluginsConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_callback_base_url(mut self, url: impl Into<String>) -> Self {
        self.callback_base_url = Some(url.into());
        self
    }

    pub fn with_host_client(mut self, client: Arc<dyn HostClient>) -> Self {
        self.host_client = Some(client);
        self
    }

    /// Reuse transports from a previous build for entries whose config is
    /// byte-identical. Used for hot-reload across snapshot rebuilds.
    pub fn with_previous(
        mut self,
        previous: Arc<PluginHost>,
        previous_config: &PluginsConfig,
    ) -> Self {
        self.previous_entries = previous_config
            .list
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.previous = Some(previous);
        self
    }

    /// Register a compiled-in plugin under a stable id. Matches the config
    /// entry `kind = "static"` with the same key. If no such entry exists in
    /// the current config, one is added automatically with default options
    /// and timeouts so the plugin participates in the load loop.
    pub fn register_static<P: crate::sdk::Plugin>(
        mut self,
        id: impl Into<String>,
        plugin: P,
    ) -> Self {
        let id = id.into();
        let inproc = InProcessTransport::new(plugin);
        self.static_plugins.insert(
            id.clone(),
            StaticRegistration {
                builder: Box::new(move || Arc::new(inproc) as Arc<dyn PluginTransport>),
            },
        );
        self.config
            .list
            .entry(id)
            .or_insert_with(|| PluginEntry::Static {
                options: serde_json::Value::Null,
                timeouts: Default::default(),
                disabled: false,
            });
        self
    }

    pub async fn build(self) -> Result<Arc<PluginHost>, HostError> {
        if !self.config.enabled {
            tracing::info!(target: "agena_plugin_host", "plugins disabled in config");
            return Ok(PluginHost::new_empty());
        }

        let host_inner = self.host_client.unwrap_or_else(|| Arc::new(NoopHostClient));
        let entries_shared = Arc::new(RwLock::new(PluginEntryRegistry::new()));
        let plugin_indices: Arc<RwLock<HashMap<String, usize>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let statuses_shared = Arc::new(crate::status::StatusRegistry::new());
        let logs_shared = self
            .previous
            .as_ref()
            .map(|previous| previous.log_store())
            .unwrap_or_else(|| Arc::new(PluginLogStore::default()));
        let mut handle = HostHandle::new_with_components(
            host_inner,
            Arc::clone(&entries_shared),
            Arc::clone(&plugin_indices),
            Arc::clone(&statuses_shared),
            Arc::clone(&logs_shared),
        );
        if let Some(url) = self.callback_base_url.clone() {
            handle = handle.with_callback_base_url(url);
        }
        let quotas = Arc::new(crate::quota::QuotaRegistry::new(
            self.config.default_quota.clone(),
        ));
        for (plugin_id, quota) in &self.config.quotas {
            quotas.set_plugin(plugin_id.clone(), quota.clone());
        }
        handle.install_quota_registry(Arc::clone(&quotas));
        let host_handle = Arc::new(handle);
        #[allow(clippy::type_complexity)]
        let env_lookup: Box<dyn Fn(&str) -> Option<String> + Send + Sync> =
            Box::new(|k: &str| std::env::var(k).ok());

        let mut static_registry = self.static_plugins;
        let mut loaded: Vec<Arc<LoadedPlugin>> = Vec::new();
        let mut by_id: HashMap<String, Arc<LoadedPlugin>> = HashMap::new();

        // Sort entries by id for deterministic load order.
        let mut entries: Vec<(String, PluginEntry)> = self.config.list.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Build a quick lookup of previous LoadedPlugin by id for reuse.
        let previous_loaded: HashMap<String, Arc<LoadedPlugin>> = self
            .previous
            .as_ref()
            .map(|p| {
                p.plugins
                    .iter()
                    .map(|lp| (lp.id.clone(), Arc::clone(lp)))
                    .collect()
            })
            .unwrap_or_default();

        for (idx, (id, entry)) in entries.into_iter().enumerate() {
            if entry.disabled() {
                tracing::info!(
                    target: "agena_plugin_host",
                    plugin = %id,
                    kind = entry.kind_str(),
                    "plugin disabled in config; skipping load"
                );
                continue;
            }
            statuses_shared.set(crate::status::PluginStatus::initial(
                id.clone(),
                entry.kind_str(),
            ));
            if let Ok(mut indices) = plugin_indices.write() {
                indices.insert(id.clone(), idx);
            }
            // Hot-reload: if a previous host had this id with a byte-identical
            // entry, reuse the transport (no respawn).
            if let Some(prev_entry) = self.previous_entries.get(&id)
                && prev_entry == &entry
                && let Some(reused) = previous_loaded.get(&id).cloned()
            {
                tracing::info!(
                    target: "agena_plugin_host",
                    plugin = %id,
                    "reusing existing plugin transport (config unchanged)"
                );
                if let Some(prev_host) = &self.previous {
                    prev_host
                        .transferred_to_successor
                        .lock()
                        .await
                        .insert(id.clone());
                }
                reused
                    .transport
                    .attach_host(host_handle.scoped_host_client(reused.id.clone()))
                    .await
                    .map_err(|e| HostError::Load {
                        plugin: reused.id.clone(),
                        message: e.to_string(),
                    })?;
                if let Ok(mut reg) = entries_shared.write() {
                    reg.extend_from_plugin(&reused.id, &reused.manifest.entries);
                }
                host_handle
                    .set_plugin_capabilities(
                        reused.id.clone(),
                        effective_host_capabilities_for_manifest(
                            &reused.manifest.entries,
                            &reused.manifest.plugin_capabilities,
                        ),
                    )
                    .await;
                host_handle
                    .set_plugin_entry_capabilities(
                        reused.id.clone(),
                        crate::registry::per_entry_host_capabilities(&reused.manifest.entries),
                    )
                    .await;
                if let Some(previous_status) = self
                    .previous
                    .as_ref()
                    .and_then(|previous| previous.plugin_status(&reused.id))
                {
                    statuses_shared.set(previous_status);
                }
                by_id.insert(reused.id.clone(), Arc::clone(&reused));
                host_handle
                    .register_plugin_transport(reused.id.clone(), reused.transport())
                    .await;
                loaded.push(reused);
                continue;
            }
            match load_entry(
                &id,
                &entry,
                &mut static_registry,
                Arc::clone(&host_handle),
                &self.agena_version,
                &self.workspace_root,
                &env_lookup,
                &self.config.trusted_keys,
            )
            .await
            {
                Ok(plugin) => {
                    let plugin = Arc::new(plugin);
                    if let Ok(mut reg) = entries_shared.write() {
                        reg.extend_from_plugin(&plugin.id, &plugin.manifest.entries);
                    }
                    host_handle
                        .set_plugin_capabilities(
                            plugin.id.clone(),
                            effective_host_capabilities_for_manifest(
                                &plugin.manifest.entries,
                                &plugin.manifest.plugin_capabilities,
                            ),
                        )
                        .await;
                    host_handle
                        .set_plugin_entry_capabilities(
                            plugin.id.clone(),
                            crate::registry::per_entry_host_capabilities(&plugin.manifest.entries),
                        )
                        .await;
                    let status_kind = plugin.kind;
                    let initial =
                        crate::status::PluginStatus::initial(plugin.id.clone(), status_kind);
                    statuses_shared.set(initial);
                    by_id.insert(plugin.id.clone(), plugin.clone());
                    host_handle
                        .register_plugin_transport(plugin.id.clone(), plugin.transport())
                        .await;
                    loaded.push(plugin);
                }
                Err(err) => {
                    let message = err.to_string();
                    statuses_shared.record_spawn_failure(&id, message.clone());
                    logs_shared.append(
                        id.clone(),
                        "error",
                        "host",
                        format!("failed to load plugin: {message}"),
                        serde_json::Value::Null,
                    );
                    tracing::warn!(
                        target: "agena_plugin_host",
                        plugin = %id,
                        "failed to load plugin: {err}"
                    );
                }
            }
        }

        Ok(Arc::new(PluginHost {
            plugins: loaded,
            plugins_by_id: by_id,
            entries: entries_shared,
            statuses: statuses_shared,
            logs: logs_shared,
            timeouts: self.config.timeouts,
            runtime: None,
            runtime_handle: tokio::runtime::Handle::try_current().ok(),
            _host_handle: host_handle,
            transferred_to_successor: tokio::sync::Mutex::new(Default::default()),
        }))
    }
}

// ---------- HostHandle: routes plugin -> host requests ----------

/// `HostHandle` is the shared object that knows how to answer plugin
/// callbacks. Stdio plugins receive a closure that calls into it; HTTP
/// plugins receive a callback URL + bearer token; cdylib plugins currently
/// don't get callbacks (would require shared FFI surface).
pub struct HostHandle {
    inner: tokio::sync::RwLock<Arc<dyn HostClient>>,
    /// Plugin-level capability union. Used as a fallback when a host call
    /// cannot be attributed to a specific entry (e.g. hook callbacks) or
    /// when the plugin did not register per-entry capabilities.
    capabilities: tokio::sync::RwLock<HashMap<String, Vec<HostCapability>>>,
    /// Per-entry capability map: `plugin_id -> entry_name -> capabilities`.
    /// `tool_invoke` paths look up capabilities by `entry_name` so a plugin
    /// shipping multiple entries cannot have entry A's privileges leak to
    /// callbacks coming back through entry B.
    entry_capabilities: tokio::sync::RwLock<HashMap<String, HashMap<String, Vec<HostCapability>>>>,
    /// Per-plugin bearer tokens for HTTP callbacks.
    tokens: tokio::sync::Mutex<HashMap<String, String>>,
    callback_base_url: Option<String>,
    entries: Arc<RwLock<PluginEntryRegistry>>,
    plugin_indices: Arc<RwLock<HashMap<String, usize>>>,
    statuses: Arc<crate::status::StatusRegistry>,
    logs: Arc<PluginLogStore>,
    statusline: Arc<RwLock<std::collections::BTreeMap<(String, String), HostStatuslineSegment>>>,
    themes: Arc<RwLock<std::collections::BTreeMap<String, HostThemePalette>>>,
    quotas: Arc<crate::quota::QuotaRegistry>,
    /// Plugin id of the registered permission UI handler, if any. When set,
    /// `HOST_PERMISSION_ASK` delegates the prompt to that plugin via
    /// `plugin/permission.render` instead of going to the regular
    /// `HostClient::ask_permission` implementation.
    permission_handler: tokio::sync::RwLock<Option<String>>,
    /// Plugin transport registry shared by the parent [`PluginHost`]. Lets
    /// the handle dispatch host->plugin calls (e.g. permission handler
    /// rendering) without holding a reference to PluginHost itself.
    plugin_transports: tokio::sync::RwLock<HashMap<String, Arc<dyn PluginTransport>>>,
}

impl HostHandle {
    pub fn new(inner: Arc<dyn HostClient>) -> Self {
        Self::new_with_registry(
            inner,
            Arc::new(RwLock::new(PluginEntryRegistry::new())),
            Arc::new(RwLock::new(HashMap::new())),
        )
    }

    pub fn new_with_registry(
        inner: Arc<dyn HostClient>,
        entries: Arc<RwLock<PluginEntryRegistry>>,
        plugin_indices: Arc<RwLock<HashMap<String, usize>>>,
    ) -> Self {
        Self::new_with_components(
            inner,
            entries,
            plugin_indices,
            Arc::new(crate::status::StatusRegistry::new()),
            Arc::new(PluginLogStore::default()),
        )
    }

    pub fn new_with_components(
        inner: Arc<dyn HostClient>,
        entries: Arc<RwLock<PluginEntryRegistry>>,
        plugin_indices: Arc<RwLock<HashMap<String, usize>>>,
        statuses: Arc<crate::status::StatusRegistry>,
        logs: Arc<PluginLogStore>,
    ) -> Self {
        Self {
            inner: tokio::sync::RwLock::new(inner),
            capabilities: tokio::sync::RwLock::new(HashMap::new()),
            entry_capabilities: tokio::sync::RwLock::new(HashMap::new()),
            tokens: tokio::sync::Mutex::new(HashMap::new()),
            callback_base_url: None,
            entries,
            plugin_indices,
            statuses,
            logs,
            statusline: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
            themes: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
            quotas: Arc::new(crate::quota::QuotaRegistry::default()),
            permission_handler: tokio::sync::RwLock::new(None),
            plugin_transports: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    pub fn quota_registry(&self) -> Arc<crate::quota::QuotaRegistry> {
        Arc::clone(&self.quotas)
    }

    pub fn install_quota_registry(&mut self, registry: Arc<crate::quota::QuotaRegistry>) {
        self.quotas = registry;
    }

    /// Register a plugin transport so the handle can dispatch
    /// host->plugin calls (currently used by the permission UI handler).
    pub async fn register_plugin_transport(
        &self,
        plugin_id: impl Into<String>,
        transport: Arc<dyn PluginTransport>,
    ) {
        self.plugin_transports
            .write()
            .await
            .insert(plugin_id.into(), transport);
    }

    pub async fn ingest_stream_event_for_plugin(
        &self,
        plugin_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<bool, PluginError> {
        let transport = self.plugin_transports.read().await.get(plugin_id).cloned();
        let Some(transport) = transport else {
            return Ok(false);
        };
        transport
            .ingest_stream_event(method, params)
            .await
            .map_err(transport_to_plugin_error)
    }

    /// Read-only view of the current permission handler plugin id.
    pub async fn permission_handler(&self) -> Option<String> {
        self.permission_handler.read().await.clone()
    }

    pub fn status_registry(&self) -> Arc<crate::status::StatusRegistry> {
        Arc::clone(&self.statuses)
    }

    pub fn log_store(&self) -> Arc<PluginLogStore> {
        Arc::clone(&self.logs)
    }

    pub fn append_plugin_log(
        &self,
        plugin_id: impl Into<String>,
        level: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
        fields: serde_json::Value,
    ) -> PluginLogEntry {
        self.logs.append(plugin_id, level, source, message, fields)
    }

    pub fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<PluginLogEntry> {
        self.logs.list(plugin_id, after_seq, limit)
    }

    pub fn with_callback_base_url(mut self, url: String) -> Self {
        self.callback_base_url = Some(url);
        self
    }

    /// Replace the underlying [`HostClient`] live (used after the runtime is
    /// constructed and we can install the real implementation).
    pub async fn install_client(&self, client: Arc<dyn HostClient>) {
        *self.inner.write().await = client;
    }

    pub async fn set_plugin_capabilities(
        &self,
        plugin_id: impl Into<String>,
        capabilities: Vec<HostCapability>,
    ) {
        self.capabilities
            .write()
            .await
            .insert(plugin_id.into(), capabilities);
    }

    /// Register the per-entry capability map for `plugin_id`. Lookups on
    /// `tool_invoke` paths consult this first, falling back to the
    /// plugin-level union set via [`set_plugin_capabilities`].
    pub async fn set_plugin_entry_capabilities(
        &self,
        plugin_id: impl Into<String>,
        by_entry: HashMap<String, Vec<HostCapability>>,
    ) {
        self.entry_capabilities
            .write()
            .await
            .insert(plugin_id.into(), by_entry);
    }

    async fn require_capability(
        &self,
        plugin_id: Option<&str>,
        method: &str,
        capability: HostCapability,
    ) -> Result<(), PluginError> {
        let Some(plugin_id) = plugin_id else {
            return Ok(());
        };
        // Prefer per-entry scope if the active host call originates from
        // tool_invoke (entry_name set in HostCallbackContext). Otherwise
        // fall back to the plugin-level union.
        let entry_name =
            host_api::current_host_callback_context().and_then(|ctx| ctx.entry_name.clone());
        if let Some(entry) = entry_name.as_deref() {
            let entry_caps = self.entry_capabilities.read().await;
            if let Some(by_entry) = entry_caps.get(plugin_id)
                && let Some(caps) = by_entry.get(entry)
            {
                if caps.contains(&capability) {
                    return Ok(());
                }
                // Per-entry map exists for this entry but does not grant
                // the requested capability: deny without consulting the
                // plugin-level union, otherwise per-entry scoping would
                // be meaningless.
                return Err(PluginError {
                    code: PluginErrorCode::HostUnavailable,
                    message: format!(
                        "plugin `{plugin_id}` entry `{entry}` cannot call `{method}`: \
                         missing host capability `{capability:?}`"
                    ),
                    hook: Some(method.to_string()),
                    plugin: Some(plugin_id.to_string()),
                    data: None,
                });
            }
        }
        let capabilities = self.capabilities.read().await;
        if capabilities
            .get(plugin_id)
            .is_some_and(|capabilities| capabilities.contains(&capability))
        {
            return Ok(());
        }
        Err(PluginError {
            code: PluginErrorCode::HostUnavailable,
            message: format!(
                "plugin `{plugin_id}` cannot call `{method}`: missing host capability `{capability:?}`"
            ),
            hook: Some(method.to_string()),
            plugin: Some(plugin_id.to_string()),
            data: None,
        })
    }

    pub fn callback_url(&self, plugin_id: &str) -> Option<String> {
        self.callback_base_url
            .as_ref()
            .map(|base| format!("{}/plugin-rpc/{}", base.trim_end_matches('/'), plugin_id))
    }

    pub async fn callback_token(&self, plugin_id: &str) -> Option<String> {
        self.callback_base_url.as_ref()?;
        let mut tokens = self.tokens.lock().await;
        Some(
            tokens
                .entry(plugin_id.to_string())
                .or_insert_with(|| format!("cb-{}", uuid::Uuid::new_v4().simple()))
                .clone(),
        )
    }

    pub async fn validate_callback_token(&self, plugin_id: &str, token: Option<&str>) -> bool {
        let Some(token) = token else {
            return false;
        };
        let tokens = self.tokens.lock().await;
        tokens
            .get(plugin_id)
            .is_some_and(|expected| expected == token)
    }

    pub fn scoped_host_client(
        self: &Arc<Self>,
        plugin_id: impl Into<String>,
    ) -> Arc<dyn HostClient> {
        Arc::new(ScopedHostClient {
            handle: Arc::clone(self),
            plugin_id: plugin_id.into(),
        })
    }

    pub fn host_handler_for(
        self: &Arc<Self>,
        plugin_id: impl Into<String>,
    ) -> crate::transport::stdio::HostHandler {
        let this = Arc::clone(self);
        let plugin_id = plugin_id.into();
        Arc::new(move |method: String, params: serde_json::Value| {
            let this = Arc::clone(&this);
            let plugin_id = plugin_id.clone();
            Box::pin(async move {
                this.handle_call_for_plugin(plugin_id.as_str(), &method, params)
                    .await
            })
        })
    }

    pub async fn handle_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        self.handle_call_for_plugin("", method, params).await
    }

    pub async fn handle_call_for_plugin(
        &self,
        plugin_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        let inner = self.inner.read().await.clone();
        let plugin_id = (!plugin_id.is_empty()).then(|| plugin_id.to_string());
        let callback_context = callback_context_from_params(&params);
        // Per-plugin quota guard. Skipped for callbacks that aren't tied to
        // any plugin (i.e. handle_call without a plugin_id) since those
        // can't be attributed to a quota bucket.
        let _quota_guard = match plugin_id.as_deref() {
            Some(pid) => Some(self.quotas.acquire(pid).map_err(|err| PluginError {
                code: PluginErrorCode::Generic,
                message: err.to_string(),
                hook: Some(method.to_string()),
                plugin: Some(pid.to_string()),
                data: None,
            })?),
            None => None,
        };
        host_api::with_host_callback_context(
            scoped_context(plugin_id.clone(), callback_context),
            async {
                match method {
                    method::HOST_LOG => {
                        let p: HostLogParams = parse(params)?;
                        host_api::with_host_callback_context(
                            scoped_context(plugin_id, None),
                            inner.log(p.level, p.message, p.fields),
                        )
                        .await;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_EVENT_PUBLISH => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PublishEvent,
                        )
                        .await?;
                        let env: EventEnvelope = parse(params)?;
                        host_api::with_host_callback_context(
                            scoped_context(plugin_id, None),
                            inner.publish_event(env),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_EVENT_SUBSCRIBE => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::SubscribeEvents,
                        )
                        .await?;
                        let p: HostSubscribeParams = parse(params)?;
                        let sub: EventSubscription = host_api::with_host_callback_context(
                            scoped_context(plugin_id, None),
                            inner.subscribe_events(p.filter),
                        )
                        .await?;
                        Ok(serde_json::json!({ "subscription_id": sub.id }))
                    }
                    method::HOST_EVENT_UNSUBSCRIBE => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::SubscribeEvents,
                        )
                        .await?;
                        let p: HostUnsubscribeParams = parse(params)?;
                        host_api::with_host_callback_context(
                            scoped_context(plugin_id, None),
                            inner.unsubscribe_events(p.subscription_id),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_PERMISSION_ASK | method::HOST_PERMISSION_ASK_LEGACY => {
                        let req: PermissionAskInput = parse(params)?;
                        // If a permission handler plugin is registered, route
                        // the permission request through that plugin's
                        // `permission.ask_permission` hook. Otherwise fall
                        // back to the regular HostClient method.
                        let handler_id = self.permission_handler.read().await.clone();
                        let d = if let Some(handler_id) = handler_id {
                            let transport = self
                                .plugin_transports
                                .read()
                                .await
                                .get(&handler_id)
                                .cloned();
                            match transport {
                                Some(transport) => {
                                    let params = serde_json::to_value(&req)
                                        .map_err(|e| PluginError::invalid_params(e.to_string()))?;
                                    let value = dispatch_permission_ask_transport(
                                        transport,
                                        scoped_context(plugin_id.clone(), None),
                                        params,
                                    )
                                    .await?;
                                    // Plugin hook returns Option<PermissionAskDecision>.
                                    // Map it back to PermissionDecision for the
                                    // HOST_PERMISSION_ASK contract: Defer / None
                                    // falls through to the underlying HostClient.
                                    #[derive(serde::Deserialize)]
                                    #[serde(
                                        rename_all = "snake_case",
                                        tag = "kind",
                                        content = "value"
                                    )]
                                    enum AskKind {
                                        Decide(PermissionDecision),
                                        Advise(crate::sdk::PermissionAdvice),
                                        Defer,
                                    }
                                    let parsed: Option<AskKind> = serde_json::from_value(value)
                                        .map_err(|e| PluginError::invalid_params(e.to_string()))?;
                                    match parsed {
                                        Some(AskKind::Decide(decision)) => decision,
                                        Some(AskKind::Advise(advice)) => advice.decision,
                                        _ => {
                                            host_api::with_host_callback_context(
                                                scoped_context(plugin_id, None),
                                                inner.ask_permission(req),
                                            )
                                            .await?
                                        }
                                    }
                                }
                                None => {
                                    // Handler is set but transport not registered
                                    // (e.g. unloaded). Fall back rather than fail
                                    // the permission flow.
                                    host_api::with_host_callback_context(
                                        scoped_context(plugin_id, None),
                                        inner.ask_permission(req),
                                    )
                                    .await?
                                }
                            }
                        } else {
                            host_api::with_host_callback_context(
                                scoped_context(plugin_id, None),
                                inner.ask_permission(req),
                            )
                            .await?
                        };
                        serde_json::to_value(d)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_PERMISSION_CHECK_PATH => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PermissionCheck,
                        )
                        .await?;
                        let p: HostPermissionCheckPathParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.check_path_permission(p.request),
                        )
                        .await?;
                        serde_json::to_value(out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_PERMISSION_CHECK_NETWORK => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PermissionCheck,
                        )
                        .await?;
                        let p: HostPermissionCheckNetworkParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.check_network_permission(p.request),
                        )
                        .await?;
                        serde_json::to_value(out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_UI_PERMISSION_SET_HANDLER => {
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("ui.permission.set_handler requires plugin id")
                        })?;
                        self.require_capability(
                            Some(&plugin_id),
                            method,
                            HostCapability::PermissionUi,
                        )
                        .await?;
                        *self.permission_handler.write().await = Some(plugin_id.clone());
                        Ok(serde_json::json!({ "ok": true, "handler": plugin_id }))
                    }
                    method::HOST_UI_PERMISSION_CLEAR_HANDLER => {
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("ui.permission.clear_handler requires plugin id")
                        })?;
                        self.require_capability(
                            Some(&plugin_id),
                            method,
                            HostCapability::PermissionUi,
                        )
                        .await?;
                        let mut guard = self.permission_handler.write().await;
                        let was = guard.clone();
                        if was.as_deref() == Some(plugin_id.as_str()) {
                            *guard = None;
                        }
                        Ok(serde_json::json!({ "ok": true, "previous": was }))
                    }
                    method::HOST_CONFIG_READ => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::ReadConfig,
                        )
                        .await?;
                        let p: HostConfigReadParams = parse(params)?;
                        host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.read_config(p.path),
                        )
                        .await
                    }
                    method::HOST_CONFIG_RELOAD => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::ReloadConfig,
                        )
                        .await?;
                        let p: HostConfigReloadParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.reload_config(),
                        )
                        .await?;
                        serde_json::to_value(out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_TOOL_INVOKE => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::InvokeTool,
                        )
                        .await?;
                        let p: HostInvokeToolParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.invoke_tool(p.tool, p.input),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_ASK_USER => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::AskUser,
                        )
                        .await?;
                        let p: HostAskUserParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.ask_user(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SUBTASK_SPAWN => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::SpawnSubtask,
                        )
                        .await?;
                        let p: HostSpawnSubtaskParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.spawn_subtask(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_TOOL_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::ListTools,
                        )
                        .await?;
                        let p: HostListToolsParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.list_tools(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_TODO_WRITE => {
                        let p: HostTodoWriteParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.todo_write(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_PLAN_ENTER => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PlanRegistry,
                        )
                        .await?;
                        let p: HostEnterPlanModeParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.enter_plan_mode(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_PLAN_EXIT => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PlanRegistry,
                        )
                        .await?;
                        let p: HostExitPlanModeParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.exit_plan_mode(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_WORKTREE_ENTER => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::WorktreeRegistry,
                        )
                        .await?;
                        let p: HostEnterWorktreeParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.enter_worktree(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_WORKTREE_EXIT => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::WorktreeRegistry,
                        )
                        .await?;
                        let p: HostExitWorktreeParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.exit_worktree(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MONITOR_START => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::MonitorRegistry,
                        )
                        .await?;
                        let p: HostMonitorStartParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.monitor_start(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MONITOR_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::MonitorRegistry,
                        )
                        .await?;
                        let p: HostMonitorListParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.monitor_list(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MONITOR_READ => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::MonitorRegistry,
                        )
                        .await?;
                        let p: HostMonitorReadParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.monitor_read(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MONITOR_STOP => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::MonitorRegistry,
                        )
                        .await?;
                        let p: HostMonitorStopParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.monitor_stop(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_ENTRY_REGISTER => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::EntryRegistry,
                        )
                        .await?;
                        let p: HostEntryRegisterParams = parse(params)?;
                        let plugin_id = plugin_id
                            .ok_or_else(|| host_unavailable("entry.register requires plugin id"))?;
                        let response = self.entry_upsert_for_plugin(&plugin_id, p.request.entry)?;
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_ENTRY_UPDATE => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::EntryRegistry,
                        )
                        .await?;
                        let p: HostEntryUpdateParams = parse(params)?;
                        let plugin_id = plugin_id
                            .ok_or_else(|| host_unavailable("entry.update requires plugin id"))?;
                        let response = self.entry_upsert_for_plugin(&plugin_id, p.request.entry)?;
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_ENTRY_REMOVE => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::EntryRegistry,
                        )
                        .await?;
                        let p: HostEntryRemoveParams = parse(params)?;
                        let plugin_id = plugin_id
                            .ok_or_else(|| host_unavailable("entry.remove requires plugin id"))?;
                        let response = self.entry_remove_for_plugin(
                            &plugin_id,
                            &p.request.name,
                            p.request.exposed,
                        )?;
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_ENTRY_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::EntryRegistry,
                        )
                        .await?;
                        let response = self.entry_list_response()?;
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_STORAGE_GET => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PluginStorage,
                        )
                        .await?;
                        let p: HostStorageGetParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.storage_get(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_STORAGE_SET => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PluginStorage,
                        )
                        .await?;
                        let p: HostStorageSetParams = parse(params)?;
                        host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.storage_set(p.request),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_STORAGE_DELETE => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PluginStorage,
                        )
                        .await?;
                        let p: HostStorageDeleteParams = parse(params)?;
                        host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.storage_delete(p.request),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_STORAGE_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PluginStorage,
                        )
                        .await?;
                        let p: HostStorageListParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.storage_list(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SECRET_GET => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PluginSecrets,
                        )
                        .await?;
                        let p: HostSecretGetParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.secret_get(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SECRET_SET => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PluginSecrets,
                        )
                        .await?;
                        let p: HostSecretSetParams = parse(params)?;
                        host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.secret_set(p.request),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_SECRET_DELETE => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PluginSecrets,
                        )
                        .await?;
                        let p: HostSecretDeleteParams = parse(params)?;
                        host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.secret_delete(p.request),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_SECRET_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PluginSecrets,
                        )
                        .await?;
                        let p: HostSecretListParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.secret_list(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_PLUGIN_STATUS_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PluginStatus,
                        )
                        .await?;
                        let response = self.plugin_status_list_response();
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_PLUGIN_STATUS_GET => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PluginStatus,
                        )
                        .await?;
                        let p: HostPluginStatusGetParams = parse(params)?;
                        let response = self.plugin_status_get_response(&p.request.plugin_id);
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_LSP_LIST_SERVERS => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::LspRegistry,
                        )
                        .await?;
                        let p: HostLspListServersParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.lsp_list_servers(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_LSP_LIST_DIAGNOSTICS => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::LspRegistry,
                        )
                        .await?;
                        let p: HostLspListDiagnosticsParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.lsp_list_diagnostics(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_PLAN_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PlanRegistry,
                        )
                        .await?;
                        let p: HostPlanListParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.plan_list(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_PLAN_GET => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::PlanRegistry,
                        )
                        .await?;
                        let p: HostPlanGetParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.plan_get(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_WORKTREE_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::WorktreeRegistry,
                        )
                        .await?;
                        let p: HostWorktreeListParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.worktree_list(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SCHEDULER_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::Scheduler,
                        )
                        .await?;
                        let p: HostSchedulerListParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.scheduler_list(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SCHEDULER_CREATE => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::Scheduler,
                        )
                        .await?;
                        let p: HostSchedulerCreateParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.scheduler_create(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SCHEDULER_DELETE => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::Scheduler,
                        )
                        .await?;
                        let p: HostSchedulerDeleteParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.scheduler_delete(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_AGENT_REGISTER => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::AgentRegistry,
                        )
                        .await?;
                        let p: HostAgentRegisterParams = parse(params)?;
                        host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.agent_register(p.request),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_AGENT_REMOVE => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::AgentRegistry,
                        )
                        .await?;
                        let p: HostAgentRemoveParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.agent_remove(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_AGENT_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::AgentRegistry,
                        )
                        .await?;
                        let p: HostAgentListParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.agent_list(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_AGENT_GET => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::AgentRegistry,
                        )
                        .await?;
                        let p: HostAgentGetParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.agent_get(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_AGENT_SWITCH => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::AgentRegistry,
                        )
                        .await?;
                        let p: HostAgentSwitchParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.agent_switch(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_AGENT_RESTORE => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::AgentRegistry,
                        )
                        .await?;
                        let p: HostAgentRestoreParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.agent_restore(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_HOOK_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::HookRegistry,
                        )
                        .await?;
                        let response = self.hook_list_response().await;
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MCP_LIST_SERVERS => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::McpRegistry,
                        )
                        .await?;
                        let p: HostMcpListServersParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.mcp_list_servers(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MCP_ADD_SERVER => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::McpRegistry,
                        )
                        .await?;
                        let p: HostMcpAddServerParams = parse(params)?;
                        host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.mcp_add_server(p.request),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_MCP_REMOVE_SERVER => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::McpRegistry,
                        )
                        .await?;
                        let p: HostMcpRemoveServerParams = parse(params)?;
                        let out = host_api::with_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.mcp_remove_server(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_UI_STATUSLINE_CONTRIBUTE => {
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("ui.statusline.contribute requires plugin id")
                        })?;
                        self.require_capability(
                            Some(&plugin_id),
                            method,
                            HostCapability::Statusline,
                        )
                        .await?;
                        let p: HostStatuslineContributeParams = parse(params)?;
                        self.statusline_contribute(&plugin_id, p.request);
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_UI_STATUSLINE_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::Statusline,
                        )
                        .await?;
                        let response = self.statusline_list_response();
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_UI_STATUSLINE_REMOVE => {
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("ui.statusline.remove requires plugin id")
                        })?;
                        self.require_capability(
                            Some(&plugin_id),
                            method,
                            HostCapability::Statusline,
                        )
                        .await?;
                        let p: HostStatuslineRemoveParams = parse(params)?;
                        let removed = self.statusline_remove(&plugin_id, &p.request.segment_id);
                        serde_json::to_value(&HostStatuslineRemoveResponse { removed })
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_UI_THEME_REGISTER => {
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("ui.theme.register requires plugin id")
                        })?;
                        self.require_capability(Some(&plugin_id), method, HostCapability::Theme)
                            .await?;
                        let p: HostThemeRegisterParams = parse(params)?;
                        self.theme_register(&plugin_id, p.request);
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_UI_THEME_LIST => {
                        self.require_capability(
                            plugin_id.as_deref(),
                            method,
                            HostCapability::Theme,
                        )
                        .await?;
                        let response = self.theme_list_response();
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_UI_THEME_REMOVE => {
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("ui.theme.remove requires plugin id")
                        })?;
                        self.require_capability(Some(&plugin_id), method, HostCapability::Theme)
                            .await?;
                        let p: HostThemeRemoveParams = parse(params)?;
                        let removed = self.theme_remove(&plugin_id, &p.request.id);
                        serde_json::to_value(&HostThemeRemoveResponse { removed })
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    other => Err(PluginError::not_implemented(other)),
                }
            },
        )
        .await
    }

    fn entry_upsert_for_plugin(
        &self,
        plugin_id: &str,
        decl: crate::sdk::PluginToolDecl,
    ) -> Result<HostEntryMutationResponse, PluginError> {
        let registered = self
            .plugin_indices
            .read()
            .map_err(|_| host_unavailable("plugin index lock poisoned"))?
            .contains_key(plugin_id);
        if !registered {
            return Err(host_unavailable(format!(
                "plugin `{plugin_id}` is not registered"
            )));
        }
        let mut entries = self
            .entries
            .write()
            .map_err(|_| host_unavailable("tool registry lock poisoned"))?;
        let entry = entries.upsert_from_plugin(plugin_id, decl);
        Ok(HostEntryMutationResponse {
            generation: entries.generation(),
            exposed_name: Some(entry.exposed_name.clone()),
            entry: Some(entry.decl.clone()),
        })
    }

    fn entry_remove_for_plugin(
        &self,
        plugin_id: &str,
        name: &str,
        exposed: bool,
    ) -> Result<HostEntryMutationResponse, PluginError> {
        let mut entries = self
            .entries
            .write()
            .map_err(|_| host_unavailable("tool registry lock poisoned"))?;
        let removed = if exposed {
            entries.remove_exposed_from_plugin(plugin_id, name)
        } else {
            entries.remove_from_plugin(plugin_id, name)
        };
        Ok(HostEntryMutationResponse {
            generation: entries.generation(),
            exposed_name: removed.as_ref().map(|entry| entry.exposed_name.clone()),
            entry: removed.map(|entry| entry.decl),
        })
    }

    fn entry_list_response(&self) -> Result<HostEntryListResponse, PluginError> {
        let snapshot = self
            .entries
            .read()
            .map_err(|_| host_unavailable("tool registry lock poisoned"))?
            .snapshot();
        let entries = snapshot
            .entries
            .into_iter()
            .map(|entry| HostEntryDescriptor {
                plugin_id: entry.plugin_name,
                original_name: entry.original_name,
                exposed_name: entry.exposed_name,
                entry: entry.decl,
            })
            .collect();
        Ok(HostEntryListResponse {
            generation: snapshot.generation,
            entries,
        })
    }

    fn plugin_status_list_response(&self) -> HostPluginStatusListResponse {
        HostPluginStatusListResponse {
            entries: self
                .statuses
                .list()
                .into_iter()
                .map(host_status_from)
                .collect(),
        }
    }

    fn plugin_status_get_response(&self, plugin_id: &str) -> HostPluginStatusGetResponse {
        HostPluginStatusGetResponse {
            status: self.statuses.get(plugin_id).map(host_status_from),
        }
    }

    async fn hook_list_response(&self) -> HostHookListResponse {
        // Walk capability metadata to surface plugins that subscribed to
        // any tool/event hook. We approximate by listing every plugin id we
        // know capabilities for; the actual hook subscription bitmask lives
        // on `LoadedPlugin.manifest.hooks` but is not directly accessible
        // from within HostHandle without holding the PluginHost. Plugins
        // can introspect `entry.list` to map capabilities and entries to
        // each plugin id.
        let capabilities = self.capabilities.read().await;
        let entries = capabilities
            .iter()
            .map(|(plugin_id, caps)| HostHookEntry {
                plugin_id: plugin_id.clone(),
                hooks: caps
                    .iter()
                    .map(|cap| format!("{cap:?}"))
                    .collect::<Vec<_>>(),
            })
            .collect();
        HostHookListResponse { entries }
    }

    fn statusline_contribute(&self, plugin_id: &str, req: HostStatuslineContributeRequest) {
        if let Ok(mut guard) = self.statusline.write() {
            let key = (plugin_id.to_string(), req.segment_id.clone());
            guard.insert(
                key,
                HostStatuslineSegment {
                    plugin_id: plugin_id.to_string(),
                    segment_id: req.segment_id,
                    content: req.content,
                    priority: req.priority,
                    color: req.color,
                },
            );
        }
    }

    fn statusline_remove(&self, plugin_id: &str, segment_id: &str) -> bool {
        if let Ok(mut guard) = self.statusline.write() {
            return guard
                .remove(&(plugin_id.to_string(), segment_id.to_string()))
                .is_some();
        }
        false
    }

    pub fn statusline_list_response(&self) -> HostStatuslineListResponse {
        let mut segments: Vec<HostStatuslineSegment> = self
            .statusline
            .read()
            .map(|guard| guard.values().cloned().collect())
            .unwrap_or_default();
        segments.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
                .then_with(|| a.segment_id.cmp(&b.segment_id))
        });
        HostStatuslineListResponse { segments }
    }

    fn theme_register(&self, plugin_id: &str, req: HostThemeRegisterRequest) {
        if let Ok(mut guard) = self.themes.write() {
            guard.insert(
                req.id.clone(),
                HostThemePalette {
                    id: req.id,
                    plugin_id: plugin_id.to_string(),
                    display_name: req.display_name,
                    colors: req.colors,
                },
            );
        }
    }

    fn theme_remove(&self, plugin_id: &str, id: &str) -> bool {
        if let Ok(mut guard) = self.themes.write()
            && let Some(existing) = guard.get(id)
            && existing.plugin_id == plugin_id
        {
            return guard.remove(id).is_some();
        }
        false
    }

    pub fn theme_list_response(&self) -> HostThemeListResponse {
        let themes: Vec<HostThemePalette> = self
            .themes
            .read()
            .map(|guard| guard.values().cloned().collect())
            .unwrap_or_default();
        HostThemeListResponse { themes }
    }
}

fn host_status_from(status: crate::status::PluginStatus) -> HostPluginStatus {
    HostPluginStatus {
        plugin_id: status.plugin_id,
        kind: status.kind.to_string(),
        state: status.state.as_str().to_string(),
        pid: status.pid,
        restart_count: status.restart_count,
        last_exit_code: status.last_exit_code,
        last_restart_at_ms: status.last_restart_at_ms,
        last_error: status.last_error,
    }
}

#[derive(serde::Deserialize)]
struct HostLogParams {
    level: LogLevel,
    message: String,
    #[serde(default)]
    fields: serde_json::Value,
}

#[derive(serde::Deserialize)]
struct HostSubscribeParams {
    filter: EventFilter,
}

#[derive(serde::Deserialize)]
struct HostUnsubscribeParams {
    subscription_id: String,
}

#[derive(serde::Deserialize)]
struct HostPermissionCheckPathParams {
    request: HostPathPermissionCheckRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostPermissionCheckNetworkParams {
    request: HostNetworkPermissionCheckRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostConfigReadParams {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostConfigReloadParams {
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostInvokeToolParams {
    tool: String,
    #[serde(default)]
    input: serde_json::Value,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostAskUserParams {
    request: AskUserRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSpawnSubtaskParams {
    request: SpawnSubtaskRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostListToolsParams {
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostTodoWriteParams {
    request: HostTodoWriteRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostEnterPlanModeParams {
    #[serde(default)]
    request: HostEnterPlanModeRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostExitPlanModeParams {
    #[serde(default)]
    request: HostExitPlanModeRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostEnterWorktreeParams {
    #[serde(default)]
    request: HostEnterWorktreeRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostExitWorktreeParams {
    request: HostExitWorktreeRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMonitorStartParams {
    request: MonitorStartRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMonitorListParams {
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMonitorReadParams {
    request: MonitorReadRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMonitorStopParams {
    request: MonitorStopRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostEntryRegisterParams {
    request: HostEntryRegisterRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostEntryUpdateParams {
    request: HostEntryUpdateRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostEntryRemoveParams {
    request: HostEntryRemoveRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostStorageGetParams {
    request: HostStorageGetRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostStorageSetParams {
    request: HostStorageSetRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostStorageDeleteParams {
    request: HostStorageDeleteRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostStorageListParams {
    #[serde(default)]
    request: HostStorageListRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSecretGetParams {
    request: HostSecretGetRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSecretSetParams {
    request: HostSecretSetRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSecretDeleteParams {
    request: HostSecretDeleteRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostSecretListParams {
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostPluginStatusGetParams {
    request: HostPluginStatusGetRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostLspListServersParams {
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostLspListDiagnosticsParams {
    #[serde(default)]
    request: HostLspListDiagnosticsRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostPlanListParams {
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostPlanGetParams {
    request: HostPlanGetRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostWorktreeListParams {
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostSchedulerListParams {
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSchedulerCreateParams {
    request: HostSchedulerCreateRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSchedulerDeleteParams {
    request: HostSchedulerDeleteRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostAgentRegisterParams {
    request: HostAgentRegisterRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostAgentRemoveParams {
    request: HostAgentRemoveRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostAgentListParams {
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostAgentGetParams {
    request: HostAgentGetRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostAgentSwitchParams {
    #[serde(default)]
    request: HostAgentSwitchRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostAgentRestoreParams {
    #[serde(default)]
    request: HostAgentRestoreRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostMcpListServersParams {
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMcpAddServerParams {
    request: HostMcpAddServerRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMcpRemoveServerParams {
    request: HostMcpRemoveServerRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostStatuslineContributeParams {
    request: HostStatuslineContributeRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostStatuslineRemoveParams {
    request: HostStatuslineRemoveRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostThemeRegisterParams {
    request: HostThemeRegisterRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostThemeRemoveParams {
    request: HostThemeRemoveRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

fn host_unavailable(message: impl Into<String>) -> PluginError {
    PluginError {
        code: PluginErrorCode::HostUnavailable,
        message: message.into(),
        hook: None,
        plugin: None,
        data: None,
    }
}

fn scoped_context(
    plugin_id: Option<String>,
    context: Option<HostCallbackContext>,
) -> HostCallbackContext {
    let mut context = context.unwrap_or_default();
    if let Some(plugin_id) = plugin_id {
        context.plugin_id = Some(plugin_id);
    }
    context
}

fn callback_context_from_params(params: &serde_json::Value) -> Option<HostCallbackContext> {
    params
        .as_object()?
        .get("context")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn parse<T: DeserializeOwned>(v: serde_json::Value) -> Result<T, PluginError> {
    serde_json::from_value(v).map_err(|e| PluginError::invalid_params(e.to_string()))
}

struct ScopedHostClient {
    handle: Arc<HostHandle>,
    plugin_id: String,
}

impl ScopedHostClient {
    fn context(&self) -> HostCallbackContext {
        let mut context = host_api::current_host_callback_context().unwrap_or_default();
        context.plugin_id = Some(self.plugin_id.clone());
        context
    }

    async fn require_capability(
        &self,
        method: &str,
        capability: HostCapability,
    ) -> crate::sdk::Result<()> {
        self.handle
            .require_capability(Some(&self.plugin_id), method, capability)
            .await
    }
}

#[async_trait::async_trait]
impl HostClient for ScopedHostClient {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value) {
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.log(level, message, fields))
            .await;
    }

    async fn publish_event(&self, env: EventEnvelope) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_EVENT_PUBLISH, HostCapability::PublishEvent)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.publish_event(env)).await
    }

    async fn subscribe_events(&self, filter: EventFilter) -> crate::sdk::Result<EventSubscription> {
        self.require_capability(
            method::HOST_EVENT_SUBSCRIBE,
            HostCapability::SubscribeEvents,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.subscribe_events(filter)).await
    }

    async fn unsubscribe_events(&self, subscription_id: String) -> crate::sdk::Result<()> {
        self.require_capability(
            method::HOST_EVENT_UNSUBSCRIBE,
            HostCapability::SubscribeEvents,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(
            self.context(),
            inner.unsubscribe_events(subscription_id),
        )
        .await
    }

    async fn ask_permission(
        &self,
        req: PermissionAskInput,
    ) -> crate::sdk::Result<PermissionDecision> {
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.ask_permission(req)).await
    }

    async fn check_path_permission(
        &self,
        req: HostPathPermissionCheckRequest,
    ) -> crate::sdk::Result<HostPermissionCheckResponse> {
        self.require_capability(
            method::HOST_PERMISSION_CHECK_PATH,
            HostCapability::PermissionCheck,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.check_path_permission(req)).await
    }

    async fn check_network_permission(
        &self,
        req: HostNetworkPermissionCheckRequest,
    ) -> crate::sdk::Result<HostPermissionCheckResponse> {
        self.require_capability(
            method::HOST_PERMISSION_CHECK_NETWORK,
            HostCapability::PermissionCheck,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.check_network_permission(req))
            .await
    }

    async fn read_config(&self, path: Option<String>) -> crate::sdk::Result<serde_json::Value> {
        self.require_capability(method::HOST_CONFIG_READ, HostCapability::ReadConfig)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.read_config(path)).await
    }

    async fn reload_config(&self) -> crate::sdk::Result<HostConfigReloadResponse> {
        self.require_capability(method::HOST_CONFIG_RELOAD, HostCapability::ReloadConfig)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.reload_config()).await
    }

    async fn invoke_tool(
        &self,
        tool: String,
        input: serde_json::Value,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.require_capability(method::HOST_TOOL_INVOKE, HostCapability::InvokeTool)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.invoke_tool(tool, input)).await
    }

    async fn ask_user(&self, req: AskUserRequest) -> crate::sdk::Result<AskUserResponse> {
        self.require_capability(method::HOST_ASK_USER, HostCapability::AskUser)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.ask_user(req)).await
    }

    async fn spawn_subtask(
        &self,
        req: SpawnSubtaskRequest,
    ) -> crate::sdk::Result<SpawnSubtaskResponse> {
        self.require_capability(method::HOST_SUBTASK_SPAWN, HostCapability::SpawnSubtask)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.spawn_subtask(req)).await
    }

    async fn list_tools(&self) -> crate::sdk::Result<Vec<ToolDescriptor>> {
        self.require_capability(method::HOST_TOOL_LIST, HostCapability::ListTools)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.list_tools()).await
    }

    async fn todo_write(&self, req: HostTodoWriteRequest) -> crate::sdk::Result<ToolInvokeOutput> {
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.todo_write(req)).await
    }

    async fn get_session(
        &self,
        req: crate::sdk::host_api::HostGetSessionRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostGetSessionResponse> {
        self.require_capability("host/session.get", HostCapability::SessionRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.get_session(req)).await
    }

    async fn rename_session(
        &self,
        req: crate::sdk::host_api::HostRenameSessionRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostRenameSessionResponse> {
        self.require_capability("host/session.rename", HostCapability::SessionRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.rename_session(req)).await
    }

    async fn get_goal(
        &self,
        req: crate::sdk::host_api::HostGetGoalRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostGetGoalResponse> {
        self.require_capability("host/goal.get", HostCapability::GoalRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.get_goal(req)).await
    }

    async fn create_goal(
        &self,
        req: crate::sdk::host_api::HostCreateGoalRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostCreateGoalResponse> {
        self.require_capability("host/goal.create", HostCapability::GoalRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.create_goal(req)).await
    }

    async fn update_goal(
        &self,
        req: crate::sdk::host_api::HostUpdateGoalRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostUpdateGoalResponse> {
        self.require_capability("host/goal.update", HostCapability::GoalRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.update_goal(req)).await
    }

    async fn clear_goal(
        &self,
        req: crate::sdk::host_api::HostClearGoalRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostClearGoalResponse> {
        self.require_capability("host/goal.clear", HostCapability::GoalRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.clear_goal(req)).await
    }

    async fn enter_plan_mode(
        &self,
        req: HostEnterPlanModeRequest,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.require_capability(method::HOST_PLAN_ENTER, HostCapability::PlanRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.enter_plan_mode(req)).await
    }

    async fn exit_plan_mode(
        &self,
        req: HostExitPlanModeRequest,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.require_capability(method::HOST_PLAN_EXIT, HostCapability::PlanRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.exit_plan_mode(req)).await
    }

    async fn enter_worktree(
        &self,
        req: HostEnterWorktreeRequest,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.require_capability(
            method::HOST_WORKTREE_ENTER,
            HostCapability::WorktreeRegistry,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.enter_worktree(req)).await
    }

    async fn exit_worktree(
        &self,
        req: HostExitWorktreeRequest,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.require_capability(method::HOST_WORKTREE_EXIT, HostCapability::WorktreeRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.exit_worktree(req)).await
    }

    async fn monitor_start(&self, req: MonitorStartRequest) -> crate::sdk::Result<MonitorHandle> {
        self.require_capability(method::HOST_MONITOR_START, HostCapability::MonitorRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.monitor_start(req)).await
    }

    async fn monitor_list(&self) -> crate::sdk::Result<Vec<MonitorHandle>> {
        self.require_capability(method::HOST_MONITOR_LIST, HostCapability::MonitorRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.monitor_list()).await
    }

    async fn monitor_read(
        &self,
        req: MonitorReadRequest,
    ) -> crate::sdk::Result<MonitorReadResponse> {
        self.require_capability(method::HOST_MONITOR_READ, HostCapability::MonitorRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.monitor_read(req)).await
    }

    async fn monitor_stop(&self, req: MonitorStopRequest) -> crate::sdk::Result<MonitorHandle> {
        self.require_capability(method::HOST_MONITOR_STOP, HostCapability::MonitorRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.monitor_stop(req)).await
    }

    async fn entry_register(
        &self,
        req: HostEntryRegisterRequest,
    ) -> crate::sdk::Result<HostEntryMutationResponse> {
        self.require_capability(method::HOST_ENTRY_REGISTER, HostCapability::EntryRegistry)
            .await?;
        self.handle
            .entry_upsert_for_plugin(&self.plugin_id, req.entry)
    }

    async fn entry_update(
        &self,
        req: HostEntryUpdateRequest,
    ) -> crate::sdk::Result<HostEntryMutationResponse> {
        self.require_capability(method::HOST_ENTRY_UPDATE, HostCapability::EntryRegistry)
            .await?;
        self.handle
            .entry_upsert_for_plugin(&self.plugin_id, req.entry)
    }

    async fn entry_remove(
        &self,
        req: HostEntryRemoveRequest,
    ) -> crate::sdk::Result<HostEntryMutationResponse> {
        self.require_capability(method::HOST_ENTRY_REMOVE, HostCapability::EntryRegistry)
            .await?;
        self.handle
            .entry_remove_for_plugin(&self.plugin_id, &req.name, req.exposed)
    }

    async fn entry_list(&self) -> crate::sdk::Result<HostEntryListResponse> {
        self.require_capability(method::HOST_ENTRY_LIST, HostCapability::EntryRegistry)
            .await?;
        self.handle.entry_list_response()
    }

    async fn storage_get(
        &self,
        req: HostStorageGetRequest,
    ) -> crate::sdk::Result<HostStorageGetResponse> {
        self.require_capability(method::HOST_STORAGE_GET, HostCapability::PluginStorage)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.storage_get(req)).await
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_STORAGE_SET, HostCapability::PluginStorage)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.storage_set(req)).await
    }

    async fn storage_delete(&self, req: HostStorageDeleteRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_STORAGE_DELETE, HostCapability::PluginStorage)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.storage_delete(req)).await
    }

    async fn storage_list(
        &self,
        req: HostStorageListRequest,
    ) -> crate::sdk::Result<HostStorageListResponse> {
        self.require_capability(method::HOST_STORAGE_LIST, HostCapability::PluginStorage)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.storage_list(req)).await
    }

    async fn secret_get(
        &self,
        req: HostSecretGetRequest,
    ) -> crate::sdk::Result<HostSecretGetResponse> {
        self.require_capability(method::HOST_SECRET_GET, HostCapability::PluginSecrets)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.secret_get(req)).await
    }

    async fn secret_set(&self, req: HostSecretSetRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_SECRET_SET, HostCapability::PluginSecrets)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.secret_set(req)).await
    }

    async fn secret_delete(&self, req: HostSecretDeleteRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_SECRET_DELETE, HostCapability::PluginSecrets)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.secret_delete(req)).await
    }

    async fn secret_list(&self) -> crate::sdk::Result<HostSecretListResponse> {
        self.require_capability(method::HOST_SECRET_LIST, HostCapability::PluginSecrets)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.secret_list()).await
    }

    async fn plugin_status_list(&self) -> crate::sdk::Result<HostPluginStatusListResponse> {
        self.require_capability(
            method::HOST_PLUGIN_STATUS_LIST,
            HostCapability::PluginStatus,
        )
        .await?;
        Ok(self.handle.plugin_status_list_response())
    }

    async fn plugin_status_get(
        &self,
        req: HostPluginStatusGetRequest,
    ) -> crate::sdk::Result<HostPluginStatusGetResponse> {
        self.require_capability(method::HOST_PLUGIN_STATUS_GET, HostCapability::PluginStatus)
            .await?;
        Ok(self.handle.plugin_status_get_response(&req.plugin_id))
    }

    async fn lsp_list_servers(&self) -> crate::sdk::Result<HostLspListServersResponse> {
        self.require_capability(method::HOST_LSP_LIST_SERVERS, HostCapability::LspRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.lsp_list_servers()).await
    }

    async fn lsp_list_diagnostics(
        &self,
        req: HostLspListDiagnosticsRequest,
    ) -> crate::sdk::Result<HostLspListDiagnosticsResponse> {
        self.require_capability(
            method::HOST_LSP_LIST_DIAGNOSTICS,
            HostCapability::LspRegistry,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.lsp_list_diagnostics(req)).await
    }

    async fn plan_list(&self) -> crate::sdk::Result<HostPlanListResponse> {
        self.require_capability(method::HOST_PLAN_LIST, HostCapability::PlanRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.plan_list()).await
    }

    async fn plan_get(&self, req: HostPlanGetRequest) -> crate::sdk::Result<HostPlanGetResponse> {
        self.require_capability(method::HOST_PLAN_GET, HostCapability::PlanRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.plan_get(req)).await
    }

    async fn worktree_list(&self) -> crate::sdk::Result<HostWorktreeListResponse> {
        self.require_capability(method::HOST_WORKTREE_LIST, HostCapability::WorktreeRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.worktree_list()).await
    }

    async fn scheduler_list(&self) -> crate::sdk::Result<HostSchedulerListResponse> {
        self.require_capability(method::HOST_SCHEDULER_LIST, HostCapability::Scheduler)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.scheduler_list()).await
    }

    async fn scheduler_create(
        &self,
        req: HostSchedulerCreateRequest,
    ) -> crate::sdk::Result<HostSchedulerCreateResponse> {
        self.require_capability(method::HOST_SCHEDULER_CREATE, HostCapability::Scheduler)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.scheduler_create(req)).await
    }

    async fn scheduler_delete(
        &self,
        req: HostSchedulerDeleteRequest,
    ) -> crate::sdk::Result<HostSchedulerDeleteResponse> {
        self.require_capability(method::HOST_SCHEDULER_DELETE, HostCapability::Scheduler)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.scheduler_delete(req)).await
    }

    async fn agent_register(&self, req: HostAgentRegisterRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_AGENT_REGISTER, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.agent_register(req)).await
    }

    async fn agent_remove(
        &self,
        req: HostAgentRemoveRequest,
    ) -> crate::sdk::Result<HostAgentRemoveResponse> {
        self.require_capability(method::HOST_AGENT_REMOVE, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.agent_remove(req)).await
    }

    async fn agent_list(&self) -> crate::sdk::Result<HostAgentListResponse> {
        self.require_capability(method::HOST_AGENT_LIST, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.agent_list()).await
    }

    async fn agent_get(
        &self,
        req: HostAgentGetRequest,
    ) -> crate::sdk::Result<HostAgentGetResponse> {
        self.require_capability(method::HOST_AGENT_GET, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.agent_get(req)).await
    }

    async fn agent_switch(
        &self,
        req: HostAgentSwitchRequest,
    ) -> crate::sdk::Result<HostAgentSwitchResponse> {
        self.require_capability(method::HOST_AGENT_SWITCH, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.agent_switch(req)).await
    }

    async fn agent_restore(
        &self,
        req: HostAgentRestoreRequest,
    ) -> crate::sdk::Result<HostAgentRestoreResponse> {
        self.require_capability(method::HOST_AGENT_RESTORE, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.agent_restore(req)).await
    }

    async fn hook_list(&self) -> crate::sdk::Result<HostHookListResponse> {
        self.require_capability(method::HOST_HOOK_LIST, HostCapability::HookRegistry)
            .await?;
        Ok(self.handle.hook_list_response().await)
    }

    async fn mcp_list_servers(&self) -> crate::sdk::Result<HostMcpListServersResponse> {
        self.require_capability(method::HOST_MCP_LIST_SERVERS, HostCapability::McpRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.mcp_list_servers()).await
    }

    async fn mcp_add_server(&self, req: HostMcpAddServerRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_MCP_ADD_SERVER, HostCapability::McpRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.mcp_add_server(req)).await
    }

    async fn mcp_remove_server(
        &self,
        req: HostMcpRemoveServerRequest,
    ) -> crate::sdk::Result<HostMcpRemoveServerResponse> {
        self.require_capability(method::HOST_MCP_REMOVE_SERVER, HostCapability::McpRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::with_host_callback_context(self.context(), inner.mcp_remove_server(req)).await
    }

    async fn ui_statusline_contribute(
        &self,
        req: HostStatuslineContributeRequest,
    ) -> crate::sdk::Result<()> {
        self.require_capability(
            method::HOST_UI_STATUSLINE_CONTRIBUTE,
            HostCapability::Statusline,
        )
        .await?;
        self.handle.statusline_contribute(&self.plugin_id, req);
        Ok(())
    }

    async fn ui_statusline_list(&self) -> crate::sdk::Result<HostStatuslineListResponse> {
        self.require_capability(method::HOST_UI_STATUSLINE_LIST, HostCapability::Statusline)
            .await?;
        Ok(self.handle.statusline_list_response())
    }

    async fn ui_statusline_remove(
        &self,
        req: HostStatuslineRemoveRequest,
    ) -> crate::sdk::Result<HostStatuslineRemoveResponse> {
        self.require_capability(
            method::HOST_UI_STATUSLINE_REMOVE,
            HostCapability::Statusline,
        )
        .await?;
        let removed = self
            .handle
            .statusline_remove(&self.plugin_id, &req.segment_id);
        Ok(HostStatuslineRemoveResponse { removed })
    }

    async fn ui_theme_register(&self, req: HostThemeRegisterRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_UI_THEME_REGISTER, HostCapability::Theme)
            .await?;
        self.handle.theme_register(&self.plugin_id, req);
        Ok(())
    }

    async fn ui_theme_list(&self) -> crate::sdk::Result<HostThemeListResponse> {
        self.require_capability(method::HOST_UI_THEME_LIST, HostCapability::Theme)
            .await?;
        Ok(self.handle.theme_list_response())
    }

    async fn ui_theme_remove(
        &self,
        req: HostThemeRemoveRequest,
    ) -> crate::sdk::Result<HostThemeRemoveResponse> {
        self.require_capability(method::HOST_UI_THEME_REMOVE, HostCapability::Theme)
            .await?;
        let removed = self.handle.theme_remove(&self.plugin_id, &req.id);
        Ok(HostThemeRemoveResponse { removed })
    }
}
