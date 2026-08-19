//! `PluginHost` — the central handle agena holds. Owns:
//! - the loaded plugins,
//! - the plugin tool registry,
//! - the dedicated tokio runtime that drives plugin transports,
//! - the host-callback router used by stdio/http plugins.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::config::{ConfiguredPlugin, PluginPackage, PluginsConfig, TimeoutsConfig};
use crate::dispatcher::{self, call_with_timeout};
use crate::effect_scope::{PluginEffectScope, PluginEffectScopeInspect};
use crate::error::{HostError, TransportError};
use crate::loader::{
    PreparedPlugin, StaticRegistration, activate_entry, prepare_entry, shutdown_transport,
};
use crate::logs::{PluginLogRecord, PluginLogStore};
use crate::registry::{PluginToolRegistry, RegisteredTool};
use crate::scoped_registry::{PluginScopeKey, ScopedRegistry};
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
    AgentCancelInput, AgentStopInput, AgentStopPatch, AuthInput, AuthOutput, ChatHeadersInput,
    ChatHeadersPatch, ChatMessageInput, ChatMessagePatch, ChatMessagesTransformInput,
    ChatMessagesTransformPatch, ChatParamsInput, ChatParamsPatch, ChatSystemTransformInput,
    ChatSystemTransformPatch, CommandAfterInput, CommandAfterPatch, CommandBeforeInput,
    CommandBeforeOutcome, CommandBeforeResponse, ConfigInput, ConfigPatch, EventEnvelope,
    EventFilter, HookSubscription, NotificationInput, PluginDisplayContribution, PluginError,
    PluginErrorKind, PluginKey, PluginManifest, PluginOperationDefinition,
    PluginOperationInvokeInput, PluginOperationResult, PluginServiceExport, PluginServiceImport,
    PluginServiceInvokeInput, PluginServiceInvokeOutput, PostRunInput, PreRunInput,
    ProviderListInput, ProviderListPatch, SessionEndInput, SessionStartInput, SessionStartPatch,
    ShellEnvInput, ShellEnvPatch, ToolAfterInput, ToolAfterPatch, ToolBeforeInput, ToolBeforePatch,
    ToolDefinitionInput, ToolDefinitionPatch, ToolFailureInput, ToolInvokeInput, ToolInvokeOutput,
    ToolKey, ToolPermissionNetworksInput, ToolPermissionPathsInput, ToolStreamChunk, ToolStreamEnd,
    UserPromptSubmitInput, UserPromptSubmitPatch,
};
use crate::services::{PluginServiceBinding, PluginServiceBindingKey};
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

#[derive(serde::Deserialize)]
struct HostInvokeServiceParams {
    request: PluginServiceInvokeInput,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
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
    /// Optional message the hook sent to keep the run going (for example the
    /// workflow plan autorun's `agent.stop` continuation). Recorded onto the
    /// `hook` part's `message` field so the activity carries it, instead of
    /// injecting it as a separate assistant message.
    pub message: Option<String>,
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
            message: None,
            occurred_at_ms: unix_timestamp_ms(),
        }
    }

    /// Attach the message the hook sent to keep the run going.
    pub fn with_message(mut self, message: Option<String>) -> Self {
        self.message = message;
        self
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
    pub effect_scope: Arc<PluginEffectScope>,
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

    pub fn effect_scope(&self) -> Arc<PluginEffectScope> {
        Arc::clone(&self.effect_scope)
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
        let key = PluginKey::new(manifest.namespace.clone(), manifest.name.clone())
            .expect("loaded plugin manifest key should be valid");
        Self::new_with_scope(
            kind,
            configured_plugin,
            transport,
            PluginEffectScope::new(key),
            manifest,
            trust_level,
            provenance,
        )
    }

    pub fn new_with_scope(
        kind: &'static str,
        configured_plugin: crate::config::ConfiguredPlugin,
        transport: Arc<dyn PluginTransport>,
        effect_scope: Arc<PluginEffectScope>,
        manifest: PluginManifest,
        trust_level: String,
        provenance: Vec<String>,
    ) -> Self {
        debug_assert_eq!(
            effect_scope.plugin_id(),
            &PluginKey::new(manifest.namespace.clone(), manifest.name.clone())
                .expect("loaded plugin manifest key should be valid")
        );
        Self {
            kind,
            configured_plugin,
            manifest,
            transport,
            effect_scope,
            trust_level,
            provenance,
        }
    }

    pub fn rebind_effect_scope(&self, effect_scope: Arc<PluginEffectScope>) -> Self {
        Self::new_with_scope(
            self.kind,
            self.configured_plugin.clone(),
            Arc::clone(&self.transport),
            effect_scope,
            self.manifest.clone(),
            self.trust_level.clone(),
            self.provenance.clone(),
        )
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

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Summary of a plugin authority and capabilities.
pub struct PluginAuthoritySummary {
    pub trust_level: String,
    pub provenance: Vec<String>,
    pub plugin_capabilities: Vec<String>,
    pub tool_capabilities: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation: Option<PluginActivationInspect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub services: Option<PluginServiceInspect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<PluginEffectScopeInspect>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginServiceInspect {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<PluginServiceExport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<PluginServiceImportInspect>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginServiceImportInspect {
    #[serde(flatten)]
    pub declaration: PluginServiceImport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<crate::sdk::PluginServiceMethod>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Safe, structured activation state shown even when a plugin never reached
/// `meta/init`. Raw transport/init diagnostics remain in protected host logs.
pub struct PluginActivationInspect {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<PluginKey>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<PluginKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<PluginActivationDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Non-sensitive reason an activation was blocked.
pub struct PluginActivationDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<PluginKey>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginDependencyKind {
    Explicit,
    RequiredService,
    OptionalService,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginDependencyEdge {
    pub consumer_id: PluginKey,
    pub provider_id: PluginKey,
    pub kind: PluginDependencyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginArchitectureNode {
    pub plugin_id: PluginKey,
    pub enabled: bool,
    pub status: crate::status::PluginStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<PluginActivationDiagnostic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_exports: Vec<PluginServiceExport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_imports: Vec<PluginServiceImport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginArchitectureEffect {
    pub plugin_id: PluginKey,
    #[serde(flatten)]
    pub effect: crate::effect_scope::PluginEffectDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginArchitecturePipeline {
    pub definition: crate::event_pipeline::PluginEventDefinition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_policy: Option<crate::event_pipeline::PluginPipelineFailurePolicy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handlers: Vec<crate::event_pipeline::PluginPipelineHandlerDescriptor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginArchitectureCatalog {
    #[serde(default)]
    pub profiles: crate::profiles::PluginProfileResolutionMeta,
    #[serde(default)]
    pub reload: crate::activation::PluginReloadPlan,
    #[serde(default)]
    pub plugins: Vec<PluginArchitectureNode>,
    #[serde(default)]
    pub dependencies: Vec<PluginDependencyEdge>,
    #[serde(default)]
    pub effects: Vec<PluginArchitectureEffect>,
    #[serde(default)]
    pub pipelines: Vec<PluginArchitecturePipeline>,
    #[serde(default)]
    pub tool_registrations: Vec<crate::scoped_registry::ScopedRegistryEntryDescriptor<ToolKey>>,
    #[serde(default)]
    pub operation_registrations: Vec<crate::scoped_registry::ScopedRegistryEntryDescriptor<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Neutral plugin surface catalog. Executable operations and terminal-only
/// presentation are deliberately separate from one another and from any
/// particular renderer.
pub struct PluginSurfaceCatalog {
    #[serde(default)]
    pub operations: Vec<PluginOperationCatalogItem>,
    #[serde(default)]
    pub terminal: PluginTerminalSurfaceCatalog,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Terminal-only presentation catalog.
pub struct PluginTerminalSurfaceCatalog {
    #[serde(default)]
    pub display: Vec<HostDisplayContribution>,
    #[serde(default)]
    pub themes: Vec<HostThemePalette>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Catalog item of a plugin operation.
pub struct PluginOperationCatalogItem {
    pub plugin_id: PluginKey,
    /// Host-derived fact used by every command palette. This is computed from
    /// the validated SettingsContract and cannot drift between clients.
    pub accepts_empty_input: bool,
    /// Deterministic editor/invocation seed produced by the shared contract.
    pub default_input: serde_json::Value,
    #[serde(flatten)]
    pub operation: PluginOperationDefinition,
}

fn operation_registry_name(plugin_id: &PluginKey, operation_id: &str) -> String {
    format!("{plugin_id}/{operation_id}")
}

fn sort_operation_catalog(operations: &mut [PluginOperationCatalogItem]) {
    operations.sort_by(|a, b| {
        a.operation
            .group
            .cmp(&b.operation.group)
            .then_with(|| a.operation.category.cmp(&b.operation.category))
            .then_with(|| a.operation.id.cmp(&b.operation.id))
            .then_with(|| a.operation.title.cmp(&b.operation.title))
            .then_with(|| a.plugin_id.cmp(&b.plugin_id))
    });
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Status of an explicit user-driven plugin tool invocation.
pub enum PluginToolInvokeStatus {
    Completed,
    CapabilityUnavailable,
    ToolUnavailable,
}

#[derive(Clone)]
struct ToolBeforeDispatch {
    input: ToolBeforeInput,
    cancellation: Option<tokio_util::sync::CancellationToken>,
}

enum ToolBeforeBail {
    Abort(String),
    Error(PluginError),
}

#[derive(Clone)]
struct ToolAfterDispatch {
    input: ToolAfterInput,
    cancellation: Option<tokio_util::sync::CancellationToken>,
}

#[derive(Debug, Clone)]
/// Immutable operation identity plus middleware-visible input/context.
/// Middleware may transform only the JSON input; plugin and operation routing
/// stay host-owned and cannot be redirected by an extension.
pub struct PluginOperationDispatch {
    plugin_id: PluginKey,
    operation_id: String,
    input: PluginOperationInvokeInput,
}

impl PluginOperationDispatch {
    fn new(plugin_id: PluginKey, input: PluginOperationInvokeInput) -> Self {
        Self {
            operation_id: input.operation_id.clone(),
            plugin_id,
            input,
        }
    }

    pub fn plugin_id(&self) -> &PluginKey {
        &self.plugin_id
    }

    pub fn operation_id(&self) -> &str {
        self.operation_id.as_str()
    }

    pub fn input(&self) -> &serde_json::Value {
        &self.input.input
    }

    pub fn set_input(&mut self, value: serde_json::Value) {
        self.input.input = value;
    }

    pub fn session_id(&self) -> Option<i64> {
        self.input.session_id
    }

    pub fn workspace_root(&self) -> Option<&str> {
        self.input.workspace_root.as_deref()
    }

    fn into_input(self) -> PluginOperationInvokeInput {
        self.input
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Presentation-neutral result of an explicit plugin tool invocation.
pub struct PluginToolInvokeResponse {
    pub plugin_id: PluginKey,
    pub tool: String,
    pub status: PluginToolInvokeStatus,
    pub title: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// Host-resolved declarative display contribution: the contributing plugin
/// plus its pure content contribution. The host owns placement and priority
/// ordering; plugins never target a location or color (Phase 6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostDisplayContribution {
    pub plugin_id: PluginKey,
    pub contribution: PluginDisplayContribution,
}

/// Live handle to an in-flight tool stream. Consume `chunks` for incremental
/// output; once the stream closes (sender dropped), inspect `end` for the
/// final aggregated result.
pub struct ToolInvokeStream {
    pub stream_id: String,
    pub chunks: tokio::sync::mpsc::Receiver<ToolStreamChunk>,
    pub end: tokio::sync::oneshot::Receiver<Result<ToolStreamEnd, PluginError>>,
}

/// Result-bearing asynchronous facade for plugin hooks and tool calls.
pub struct PluginHost {
    plugins: Vec<Arc<LoadedPlugin>>,
    plugins_by_id: HashMap<PluginKey, Arc<LoadedPlugin>>,
    tool_registry: Arc<RwLock<PluginToolRegistry>>,
    operation_registry: Arc<ScopedRegistry<String, PluginOperationCatalogItem>>,
    operation_pipeline: Arc<
        crate::event_pipeline::PluginAroundPipeline<
            PluginOperationDispatch,
            PluginOperationResult,
            PluginError,
        >,
    >,
    tool_before_pipeline:
        Arc<crate::event_pipeline::PluginTransformBailPipeline<ToolBeforeDispatch, ToolBeforeBail>>,
    tool_after_pipeline: Arc<crate::event_pipeline::PluginTransformPipeline<ToolAfterDispatch>>,
    statuses: Arc<crate::status::StatusRegistry>,
    logs: Arc<PluginLogStore>,
    configured_plugins: BTreeMap<String, ConfiguredPlugin>,
    activation_blocks: BTreeMap<String, crate::activation::PluginActivationBlock>,
    activation_epochs: BTreeMap<String, u64>,
    reload_plan: crate::activation::PluginReloadPlan,
    profile_resolution: crate::profiles::PluginProfileResolutionMeta,
    prefetched_manifests: BTreeMap<String, PluginManifest>,
    service_bindings: BTreeMap<PluginServiceBindingKey, PluginServiceBinding>,
    timeouts: TimeoutsConfig,
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
        authority_token: None,
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

/// Static registration of a bundled plugin.
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

/// Configuration for building a plugin host.
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
    scoped_tools: Arc<ScopedRegistry<ToolKey, RegisteredTool>>,
    operation_registry: Arc<ScopedRegistry<String, PluginOperationCatalogItem>>,
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
    plugin_transports: Arc<tokio::sync::RwLock<HashMap<PluginKey, Arc<dyn PluginTransport>>>>,
    service_bindings: tokio::sync::RwLock<BTreeMap<PluginServiceBindingKey, PluginServiceBinding>>,
    effect_scopes: RwLock<HashMap<PluginKey, Arc<PluginEffectScope>>>,
    /// Short-lived callback authorities minted only while Host→Plugin work is
    /// in flight. They prevent stdio/http plugins from forging another
    /// session, call, workspace, or tool capability in plugin→host callbacks.
    callback_authorities: Arc<Mutex<HashMap<String, CallbackAuthorityRecord>>>,
}

#[derive(Debug, Clone)]
struct CallbackAuthorityRecord {
    plugin_id: PluginKey,
    generation: u64,
    context: HostCallbackContext,
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
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostConfigReloadParams {
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostInvokeToolParams {
    tool: String,
    #[serde(default)]
    input: serde_json::Value,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostAskUserParams {
    request: AskUserRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostRunSubtaskParams {
    request: RunSubtaskRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostCancelSubtaskParams {
    request: CancelSubtaskRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMessageSubtaskParams {
    request: MessageSubtaskRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostReadSubtaskOutputParams {
    request: ReadSubtaskOutputRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostListToolsParams {
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostContextStatusParams {
    request: HostContextStatusRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSetSessionModelParams {
    request: HostSetSessionModelRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostImageExecuteParams {
    request: HostImageExecuteRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostEnterSnapshotParams {
    #[serde(default)]
    request: HostEnterSnapshotRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostExitSnapshotParams {
    request: HostExitSnapshotRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMonitorStartParams {
    request: MonitorStartRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMonitorListParams {
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMonitorReadParams {
    request: MonitorReadRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMonitorStopParams {
    request: MonitorStopRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
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
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostStorageSetParams {
    request: HostStorageSetRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostStorageDeleteParams {
    request: HostStorageDeleteRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostStorageListParams {
    #[serde(default)]
    request: HostStorageListRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSecretGetParams {
    request: HostSecretGetRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSecretSetParams {
    request: HostSecretSetRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSecretDeleteParams {
    request: HostSecretDeleteRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostSecretListParams {
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostPluginStatusGetParams {
    request: HostPluginStatusGetRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostLspListServersParams {
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostLspListDiagnosticsParams {
    #[serde(default)]
    request: HostLspListDiagnosticsRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostSnapshotListParams {
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostSchedulerListParams {
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSchedulerCreateParams {
    request: HostSchedulerCreateRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostSchedulerDeleteParams {
    request: HostSchedulerDeleteRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize, Default)]
struct HostMcpListServersParams {
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMcpAddServerParams {
    request: HostMcpAddServerRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
}

#[derive(serde::Deserialize)]
struct HostMcpRemoveServerParams {
    request: HostMcpRemoveServerRequest,
    #[serde(rename = "context", default)]
    _context: Option<HostCallbackContext>,
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

fn current_tool_scope() -> Option<PluginScopeKey> {
    host_api::current_host_callback_context()
        .and_then(|context| context.session_id)
        .map(PluginScopeKey::session)
}

fn tool_registry_event_visible_in_scope(
    event: &ToolRegistryChangedEvent,
    scope: Option<&PluginScopeKey>,
) -> bool {
    match event.scope.as_deref() {
        None => true,
        Some(event_scope) => scope.is_some_and(|scope| scope.as_str() == event_scope),
    }
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
    plugin_key: PluginKey,
    effect_scope: Arc<PluginEffectScope>,
}
