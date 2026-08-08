//! `PluginHost` — the central handle agena holds. Owns:
//! - the loaded plugins,
//! - the plugin tool registry,
//! - the dedicated tokio runtime that drives plugin transports,
//! - the host-callback router used by stdio/http plugins.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::config::{ConfiguredPlugin, PluginPackage, PluginsConfig, TimeoutsConfig};
use crate::dispatcher::{self, call_with_timeout};
use crate::error::{HostError, TransportError};
use crate::loader::{StaticRegistration, load_entry, shutdown_transport};
use crate::logs::{PluginLogRecord, PluginLogStore};
use crate::registry::{PluginToolRegistry, RegisteredTool, validate_tool_definition};
use crate::sdk::host_api::{
    self, AskUserRequest, AskUserResponse, CancelSubtaskRequest, EventSubscription,
    HostCallbackContext, HostClient, HostConfigReloadResponse, HostContextStatusRequest,
    HostContextStatusResponse, HostDisplayContributeRequest, HostDisplayRemoveRequest,
    HostDisplayRemoveResponse, HostEnterSnapshotRequest, HostExitSnapshotRequest,
    HostHookDescriptor, HostHookListResponse, HostHookRegistration, HostImageExecuteRequest,
    HostImageExecuteResponse, HostLspListDiagnosticsRequest, HostLspListDiagnosticsResponse,
    HostLspListServersResponse, HostMcpAddServerRequest, HostMcpListServersResponse,
    HostMcpRemoveServerRequest, HostMcpRemoveServerResponse, HostPluginStatus,
    HostPluginStatusGetRequest, HostPluginStatusGetResponse, HostPluginStatusListResponse,
    HostRegisteredToolDescriptor, HostRegisteredToolListResponse, HostSchedulerCreateRequest,
    HostSchedulerCreateResponse, HostSchedulerDeleteRequest, HostSchedulerDeleteResponse,
    HostSchedulerListResponse, HostSecretDeleteRequest, HostSecretGetRequest,
    HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest,
    HostSetSessionModelRequest, HostSnapshotListResponse, HostStorageDeleteRequest,
    HostStorageGetRequest, HostStorageGetResponse, HostStorageListRequest, HostStorageListResponse,
    HostStorageSetRequest, HostThemeListResponse, HostThemePalette, HostThemeRegisterRequest,
    HostThemeRemoveRequest, HostThemeRemoveResponse, HostToolMutationResponse,
    HostToolRegisterRequest, HostToolRemoveRequest, HostToolUpdateRequest, LogLevel,
    MessageSubtaskRequest, MonitorHandle, MonitorReadRequest, MonitorReadResponse,
    MonitorStartRequest, MonitorStopRequest, NoopHostClient, PluginNotifyAction,
    PluginNotifyRequest, ReadSubtaskOutputRequest, ReadSubtaskOutputResponse, RunSubtaskRequest,
    RunSubtaskResponse, SubtaskControlResponse, ToolDescriptor, ToolRegistryChangeKind,
    ToolRegistryChangedEvent,
};
use crate::sdk::rpc::method;
use crate::sdk::{
    AgentStopInput, AgentStopPatch, AuthInput, AuthOutput, ChatHeadersInput, ChatHeadersPatch,
    ChatMessageInput, ChatMessagePatch, ChatMessagesTransformInput, ChatMessagesTransformPatch,
    ChatParamsInput, ChatParamsPatch, ChatSystemTransformInput, ChatSystemTransformPatch,
    CommandAfterInput, CommandAfterPatch, CommandBeforeInput, CommandBeforeOutcome,
    CommandBeforeResponse, ConfigInput, ConfigPatch, EventEnvelope, EventFilter, HookSubscription,
    NotificationInput, PluginCommandDefinition, PluginCommandInvokeInput, PluginCommandOutput,
    PluginDisplayContribution, PluginError, PluginErrorKind, PluginKey, PluginManifest,
    PluginStudioControl, PluginStudioView, PluginUiAction, PostRunInput, PreRunInput,
    ProviderListInput, ProviderListPatch, SessionEndInput, SessionStartInput, SessionStartPatch,
    ShellEnvInput, ShellEnvPatch, ToolAfterInput, ToolAfterPatch, ToolBeforeInput, ToolBeforePatch,
    ToolDefinitionInput, ToolDefinitionPatch, ToolFailureInput, ToolInvokeInput, ToolInvokeOutput,
    ToolKey, ToolPermissionNetworksInput, ToolPermissionPathsInput, ToolStreamChunk, ToolStreamEnd,
    UserPromptSubmitInput, UserPromptSubmitPatch,
};
use crate::transport::PluginTransport;
use crate::transport::inproc::InProcessTransport;

mod host_handle;
mod host_scoped_client;
mod plugin_host_build;
mod plugin_host_core;

/// One plugin's observed run of the `agent.stop` hook. Callers (for example
/// the session runtime) use these to surface hook execution as transcript
/// activity, so users can see whether a stop hook such as the workflow plan's
/// autorun continuation actually fired.
#[derive(Debug, Clone)]
pub struct AgentStopHookRun {
    pub plugin_id: String,
    /// The hook identifier that ran, for example `agent.stop`.
    pub hook: String,
    /// The injected continuation message when the plugin blocked stop.
    pub continue_with_message: Option<String>,
    /// Human-readable reason the plugin recorded when blocking stop.
    pub reason: Option<String>,
}

impl AgentStopHookRun {
    /// A run that completed without blocking stop.
    pub fn ran(plugin_id: String, hook: &str) -> Self {
        Self {
            plugin_id,
            hook: hook.to_string(),
            continue_with_message: None,
            reason: None,
        }
    }
}

/// Result of dispatching `agent.stop` across plugins: the aggregate patch plus
/// per-plugin observations so callers can record hook activity.
#[derive(Debug, Clone)]
pub struct AgentStopDispatch {
    pub patch: AgentStopPatch,
    pub runs: Vec<AgentStopHookRun>,
}

/// Outcome of one plugin hook invocation, collected by `PluginHost` and
/// drained by the session runtime to surface hook execution as transcript
/// activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookRunStatus {
    /// Hook ran and its patch was applied (or it otherwise had an effect).
    Applied,
    /// Hook ran and returned nothing (no patch / null). No-op runs are not
    /// recorded as transcript activity — only effective runs and failures
    /// are queued, so this status is rarely seen in practice.
    Skipped,
    /// Hook transport call failed.
    Failed,
    /// Hook transport call timed out.
    TimedOut,
}

/// One observed plugin hook run. Pushed at dispatch time with the session
/// the hook ran for (when the hook input carried one); drained by the
/// session runtime and recorded as `HookPart` activity in the transcript.
#[derive(Debug, Clone)]
pub struct HookRunRecord {
    /// The hook identifier that ran, for example `chat.params`.
    pub hook: String,
    /// The plugin that ran the hook.
    pub plugin_id: String,
    /// Session the hook ran for, when the hook input carried one.
    pub session_id: Option<i64>,
    pub status: HookRunStatus,
    /// Human-readable summary of the run.
    pub summary: String,
    /// Optional extra detail (reason, error, injected continuation).
    pub detail: Option<String>,
    /// Wall-clock timestamp (ms since the Unix epoch) captured when the hook
    /// actually ran, not when the record was drained. The session runtime
    /// uses this to place the activity on the transcript timeline at the real
    /// hook call position.
    pub occurred_at_ms: i64,
}

impl HookRunRecord {
    pub fn new(
        hook: impl Into<String>,
        plugin_id: impl Into<String>,
        session_id: Option<i64>,
        status: HookRunStatus,
        summary: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            hook: hook.into(),
            plugin_id: plugin_id.into(),
            session_id,
            status,
            summary: summary.into(),
            detail,
            occurred_at_ms: unix_timestamp_ms(),
        }
    }

    /// Override the recorded hook-call time (used by tests to simulate
    /// deterministic timeline positions).
    pub fn with_occurred_at(mut self, occurred_at_ms: i64) -> Self {
        self.occurred_at_ms = occurred_at_ms;
        self
    }
}

/// Upper bound on the pending hook-run queue. Every dispatch pushes one
/// record per plugin invocation; a host that never opens a session would
/// otherwise grow the queue without bound. Oldest records are dropped once
/// the bound is exceeded.
const MAX_PENDING_HOOK_RUNS: usize = 4096;

/// Push hook runs into the shared queue, dropping the oldest records once
/// the bounded queue overflows. `pub(crate)` so fire-and-forget broadcast
/// tasks can enqueue without holding `&PluginHost`.
pub(crate) fn push_hook_runs_into(
    queue: &Arc<Mutex<Vec<HookRunRecord>>>,
    runs: Vec<HookRunRecord>,
) {
    if runs.is_empty() {
        return;
    }
    let mut pending = queue.lock().expect("hook run queue mutex poisoned");
    pending.extend(runs);
    if pending.len() > MAX_PENDING_HOOK_RUNS {
        let excess = pending.len() - MAX_PENDING_HOOK_RUNS;
        pending.drain(..excess);
    }
}

/// A loaded plugin with its manifest and transport.
pub struct LoadedPlugin {
    pub kind: &'static str,
    pub configured_plugin: crate::config::ConfiguredPlugin,
    pub manifest: PluginManifest,
    pub transport: Arc<dyn PluginTransport>,
    pub trust_level: String,
    pub provenance: Vec<String>,
}

impl LoadedPlugin {
    pub fn key(&self) -> PluginKey {
        PluginKey::new(self.manifest.namespace.clone(), self.manifest.name.clone())
            .expect("loaded plugin manifest key should be valid")
    }

    pub fn transport(&self) -> Arc<dyn PluginTransport> {
        Arc::clone(&self.transport)
    }

    pub fn configured_plugin(&self) -> &crate::config::ConfiguredPlugin {
        &self.configured_plugin
    }

    pub fn authority_summary(&self) -> PluginAuthoritySummary {
        PluginAuthoritySummary {
            trust_level: self.trust_level.clone(),
            provenance: self.provenance.clone(),
            plugin_capabilities: Vec::new(),
            tool_capabilities: BTreeMap::new(),
        }
    }
}

impl std::fmt::Debug for LoadedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedPlugin")
            .field("key", &self.key())
            .field("kind", &self.kind)
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

impl LoadedPlugin {
    pub fn new(
        kind: &'static str,
        configured_plugin: crate::config::ConfiguredPlugin,
        transport: Arc<dyn PluginTransport>,
        manifest: PluginManifest,
        trust_level: String,
        provenance: Vec<String>,
    ) -> Self {
        Self {
            kind,
            configured_plugin,
            manifest,
            transport,
            trust_level,
            provenance,
        }
    }

    pub fn subscribes(&self, sub: HookSubscription) -> bool {
        self.manifest.hooks.contains(sub)
    }

    pub fn manifest_tool_name(&self, tool_name: &str) -> Option<String> {
        self.manifest
            .tools
            .iter()
            .find_map(|tool| (tool.name == tool_name).then(|| tool.name.clone()))
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
/// Summary of a plugin authority and capabilities.
pub struct PluginAuthoritySummary {
    pub trust_level: String,
    pub provenance: Vec<String>,
    pub plugin_capabilities: Vec<String>,
    pub tool_capabilities: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
/// Inspection view of a plugin.
pub struct PluginInspect {
    pub status: crate::status::PluginStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<PluginManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<PluginAuthoritySummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HostHookRegistration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configured_plugin: Option<crate::config::ConfiguredPlugin>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Plugin UI catalog.
pub struct PluginUiCatalog {
    pub tui: PluginTuiUiCatalog,
    pub studio: PluginStudioUiCatalog,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// TUI plugin UI catalog.
pub struct PluginTuiUiCatalog {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub display: Vec<HostDisplayContribution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<HostThemePalette>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Studio plugin UI catalog.
pub struct PluginStudioUiCatalog {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<PluginCommandCatalogItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<PluginStudioControlCatalogItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<PluginStudioViewCatalogItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Catalog item of a plugin command.
pub struct PluginCommandCatalogItem {
    pub plugin_id: PluginKey,
    #[serde(flatten)]
    pub command: PluginCommandDefinition,
}

/// Host-resolved declarative display contribution: the contributing plugin
/// plus its pure content contribution. The host owns placement and priority
/// ordering; plugins never target a location or color (Phase 6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostDisplayContribution {
    pub plugin_id: PluginKey,
    pub contribution: PluginDisplayContribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Catalog item of a plugin studio control.
pub struct PluginStudioControlCatalogItem {
    pub plugin_id: PluginKey,
    #[serde(flatten)]
    pub control: PluginStudioControl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Catalog item of a plugin studio view.
pub struct PluginStudioViewCatalogItem {
    pub plugin_id: PluginKey,
    #[serde(flatten)]
    pub view: PluginStudioView,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginUiToolInvokeStatus {
    Completed,
    CapabilityUnavailable,
    ToolUnavailable,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginUiToolInvokeResponse {
    pub plugin_id: PluginKey,
    pub tool: String,
    pub status: PluginUiToolInvokeStatus,
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

/// Result-bearing facade for a tool call. Wraps async dispatch in a runtime
/// `block_on` so callers from sync code (like `ToolExecutor`) can use it.
pub struct PluginHost {
    plugins: Vec<Arc<LoadedPlugin>>,
    plugins_by_id: HashMap<PluginKey, Arc<LoadedPlugin>>,
    tool_registry: Arc<RwLock<PluginToolRegistry>>,
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
    transferred_to_successor: tokio::sync::Mutex<std::collections::HashSet<PluginKey>>,
    /// Observed hook runs awaiting recording by the session runtime. Every
    /// dispatch pushes one record per plugin invocation; session operations
    /// drain the queue and attribute runs by `session_id`.
    hook_runs: Arc<Mutex<Vec<HookRunRecord>>>,
}

fn tool_hook_context(
    plugin: &LoadedPlugin,
    tool_name: &str,
    session_id: Option<i64>,
    call_id: Option<i64>,
    workspace_root: Option<String>,
) -> HostCallbackContext {
    HostCallbackContext {
        plugin_id: Some(plugin.key().to_string()),
        session_id,
        call_id,
        workspace_root,
        tool_name: Some(
            plugin
                .manifest_tool_name(tool_name)
                .unwrap_or_else(|| tool_name.to_string()),
        ),
    }
}

fn hook_registration_for_plugin(plugin: &LoadedPlugin) -> HostHookRegistration {
    let (source, source_path) = plugin_source_summary(&plugin.configured_plugin);
    let trust_status = trust_status_for_level(plugin.trust_level.as_str()).to_string();
    let manifest_hash = manifest_hash(&plugin.manifest);
    let hooks = plugin
        .manifest
        .hooks
        .names()
        .into_iter()
        .map(|name| HostHookDescriptor {
            name: name.to_string(),
            enabled: true,
            trust_level: plugin.trust_level.clone(),
            trust_status: trust_status.clone(),
            source: source.clone(),
            source_path: source_path.clone(),
            current_hash: manifest_hash.clone(),
        })
        .collect();

    HostHookRegistration {
        plugin_id: plugin.key(),
        trust_level: plugin.trust_level.clone(),
        trust_status,
        provenance: plugin.provenance.clone(),
        source,
        source_path,
        manifest_hash,
        hooks,
    }
}

fn plugin_source_summary(configured_plugin: &ConfiguredPlugin) -> (String, Option<String>) {
    match &configured_plugin.package {
        PluginPackage::Static {} => ("static".to_string(), None),
        PluginPackage::Cdylib { path, .. } => {
            ("cdylib".to_string(), Some(path.display().to_string()))
        }
        PluginPackage::Stdio { command, .. } => ("stdio".to_string(), Some(command.clone())),
        PluginPackage::Http { url, .. } => ("http".to_string(), Some(url.to_string())),
        PluginPackage::Wasm { path, .. } => ("wasm".to_string(), Some(path.display().to_string())),
    }
}

fn trust_status_for_level(level: &str) -> &'static str {
    match level {
        "static" | "verified" => "trusted",
        "checksummed" => "verified",
        "remote" => "remote",
        _ => "untrusted",
    }
}

fn manifest_hash(manifest: &PluginManifest) -> Option<String> {
    let bytes = serde_json::to_vec(manifest).ok()?;
    Some(blake3::hash(&bytes).to_hex().to_string())
}

fn unix_timestamp_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn transport_to_plugin_error(e: TransportError) -> PluginError {
    match e {
        TransportError::Plugin(pe) => pe,
        other => PluginError::internal(other.to_string()),
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

// ---------- host construction ----------

pub struct StaticPluginRegistration {
    pub key: PluginKey,
    registration: StaticRegistration,
}

impl StaticPluginRegistration {
    pub fn new<P: crate::sdk::Plugin>(key: PluginKey, plugin: P) -> Self {
        let inproc = InProcessTransport::new(plugin);
        Self {
            key,
            registration: StaticRegistration {
                builder: Box::new(move || Arc::new(inproc) as Arc<dyn PluginTransport>),
            },
        }
    }
}

/// A plugin-emitted notification intent through the unified `host.notify`
/// entry (Phase 6). The host keeps a bounded recent queue for frontends;
/// surface/priority/dedupe decisions stay on the host side.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HostNotification {
    pub plugin_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// One of `info` | `success` | `warning` | `error`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<PluginNotifyAction>,
}

pub struct PluginHostBuildConfig {
    pub static_plugins: Vec<StaticPluginRegistration>,
    pub config: PluginsConfig,
    pub workspace_root: PathBuf,
    pub agena_version: String,
    pub callback_base_url: Option<String>,
    pub host_client: Option<Arc<dyn HostClient>>,
    /// Optional previous host: for any configured plugin whose config is byte-identical
    /// to the previous run, the old transport is reused (hot-reload).
    pub previous: Option<Arc<PluginHost>>,
    pub previous_plugins: HashMap<String, ConfiguredPlugin>,
}

impl PluginHostBuildConfig {
    pub fn previous_plugins(previous_config: &PluginsConfig) -> HashMap<String, ConfiguredPlugin> {
        previous_config
            .list
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

// ---------- HostHandle: routes plugin -> host requests ----------

/// `HostHandle` is the shared object that knows how to answer plugin
/// callbacks. Stdio plugins receive a closure that calls into it; HTTP
/// plugins receive a callback URL + bearer token; cdylib plugins currently
/// don't get callbacks (would require shared FFI surface).
pub struct HostHandle {
    inner: tokio::sync::RwLock<Arc<dyn HostClient>>,
    /// Per-plugin bearer tokens for HTTP callbacks.
    tokens: tokio::sync::Mutex<HashMap<PluginKey, String>>,
    callback_base_url: Option<String>,
    tool_registry: Arc<RwLock<PluginToolRegistry>>,
    plugin_indices: Arc<RwLock<HashMap<PluginKey, usize>>>,
    plugin_names: Arc<RwLock<HashMap<PluginKey, String>>>,
    hook_catalog: Arc<RwLock<BTreeMap<PluginKey, HostHookRegistration>>>,
    tool_registry_events: Arc<RwLock<VecDeque<ToolRegistryChangedEvent>>>,
    tool_registry_event_listener: Arc<RwLock<Option<ToolRegistryEventListener>>>,
    statuses: Arc<crate::status::StatusRegistry>,
    logs: Arc<PluginLogStore>,
    display: Arc<RwLock<std::collections::BTreeMap<(PluginKey, String), HostDisplayContribution>>>,
    host_notifications: Arc<RwLock<std::collections::VecDeque<HostNotification>>>,
    themes: Arc<RwLock<BTreeMap<(PluginKey, String), HostThemePalette>>>,
    quotas: Arc<crate::quota::QuotaRegistry>,
    /// Plugin transport registry shared by the parent [`PluginHost`]. Lets
    /// the handle dispatch host->plugin calls (e.g. permission handler
    /// rendering) without holding a reference to PluginHost itself.
    plugin_transports: tokio::sync::RwLock<HashMap<PluginKey, Arc<dyn PluginTransport>>>,
}

type ToolRegistryEventListener = Arc<dyn Fn(ToolRegistryChangedEvent) + Send + Sync>;

fn host_status_from(status: crate::status::PluginStatus) -> HostPluginStatus {
    HostPluginStatus {
        plugin_id: status.plugin_id,
        kind: status.kind.to_string(),
        state: status.state.to_string(),
        pid: status.pid,
        restart_count: status.restart_count,
        last_exit_code: status.last_exit_code,
        last_restart_at_ms: status.last_restart_at_ms,
        last_failure: status.last_failure,
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
struct HostRunSubtaskParams {
    request: RunSubtaskRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostCancelSubtaskParams {
    request: CancelSubtaskRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMessageSubtaskParams {
    request: MessageSubtaskRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostReadSubtaskOutputParams {
    request: ReadSubtaskOutputRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostListToolsParams {
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostContextStatusParams {
    request: HostContextStatusRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSetSessionModelParams {
    request: HostSetSessionModelRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostImageExecuteParams {
    request: HostImageExecuteRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostEnterSnapshotParams {
    #[serde(default)]
    request: HostEnterSnapshotRequest,
    #[serde(default)]
    context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostExitSnapshotParams {
    request: HostExitSnapshotRequest,
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
struct HostToolRegisterParams {
    request: HostToolRegisterRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostToolUpdateParams {
    request: HostToolUpdateRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostToolRemoveParams {
    request: HostToolRemoveRequest,
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
struct HostSnapshotListParams {
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
struct HostDisplayContributeParams {
    request: HostDisplayContributeRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostDisplayRemoveParams {
    request: HostDisplayRemoveRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostNotifyParams {
    request: PluginNotifyRequest,
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
    PluginError::from_kind(PluginErrorKind::HostUnavailable, message.into())
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
