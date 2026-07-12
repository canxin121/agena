//! `HostClient` — the API plugins use to call back into the host.
//!
//! At runtime the host injects a concrete impl into `Plugin::init`. For tests
//! and minimal plugins, [`NoopHostClient`] returns errors for everything.

use std::collections::BTreeMap;
use std::future::Future;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{PluginError, Result};
use crate::hooks::{
    EventEnvelope, EventFilter, PermissionAskInput, PermissionDecision, ToolInvokeOutput,
};
pub use crate::host_api_agents::*;
use crate::identity::{PluginKey, ToolKey};
use crate::manifest::{PathKind, PluginTuiColor, PluginTuiThemeColors, ToolDefinition};

#[async_trait]
pub trait HostClient: Send + Sync + 'static {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value);

    async fn publish_event(&self, env: EventEnvelope) -> Result<()>;

    async fn subscribe_events(&self, filter: EventFilter) -> Result<EventSubscription>;

    async fn unsubscribe_events(&self, subscription_id: String) -> Result<()> {
        let _ = subscription_id;
        Err(unavailable())
    }

    async fn ask_permission(&self, req: PermissionAskInput) -> Result<PermissionDecision>;

    /// Evaluate the current path permission policy for a plugin-internal path
    /// access. The host returns `Prompt` when the same access would require an
    /// interactive permission request in the normal tool flow; plugins should
    /// only proceed on `Allow`.
    async fn check_path_permission(
        &self,
        _req: HostPathPermissionCheckRequest,
    ) -> Result<HostPermissionCheckResponse> {
        Err(unavailable())
    }

    /// Evaluate the current network permission policy for a plugin-internal
    /// outbound connection target. Plugins should only proceed on `Allow`.
    async fn check_network_permission(
        &self,
        _req: HostNetworkPermissionCheckRequest,
    ) -> Result<HostPermissionCheckResponse> {
        Err(unavailable())
    }

    async fn ensure_path_permission(&self, req: HostPathPermissionCheckRequest) -> Result<()> {
        self.check_path_permission(req).await?.ensure_allowed()
    }

    async fn ensure_network_permission(
        &self,
        req: HostNetworkPermissionCheckRequest,
    ) -> Result<()> {
        self.check_network_permission(req).await?.ensure_allowed()
    }

    async fn read_config(&self, path: Option<String>) -> Result<serde_json::Value>;

    /// Reload the runtime after persisted configuration has changed.
    async fn reload_config(&self) -> Result<HostConfigReloadResponse> {
        Err(unavailable())
    }

    async fn invoke_tool(&self, tool: String, input: serde_json::Value)
    -> Result<ToolInvokeOutput>;

    // ---------------- Host workflow capabilities ----------------
    //
    // These are optional host APIs that any plugin can request via
    // `HostCapability`. Hosts that don't expose a capability should return
    // `HostUnavailable`.

    /// Prompt the user for input via the active session UI (used by the
    /// `ask_user` tool).
    async fn ask_user(&self, _req: AskUserRequest) -> Result<AskUserResponse> {
        Err(unavailable())
    }

    /// Spawn a child agent / subtask. Used by the `task` tool.
    async fn spawn_subtask(&self, _req: SpawnSubtaskRequest) -> Result<SpawnSubtaskResponse> {
        Err(unavailable())
    }

    /// List all currently registered tools (used by `tool_search`).
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>> {
        Err(unavailable())
    }

    /// Read the current session metadata.
    async fn get_session(&self, _req: HostGetSessionRequest) -> Result<HostGetSessionResponse> {
        Err(unavailable())
    }

    /// Rename the current session.
    async fn rename_session(
        &self,
        _req: HostRenameSessionRequest,
    ) -> Result<HostRenameSessionResponse> {
        Err(unavailable())
    }

    /// Enter a managed snapshot for the current session.
    async fn enter_snapshot(&self, _req: HostEnterSnapshotRequest) -> Result<ToolInvokeOutput> {
        Err(unavailable())
    }

    /// Exit the current session's active snapshot.
    async fn exit_snapshot(&self, _req: HostExitSnapshotRequest) -> Result<ToolInvokeOutput> {
        Err(unavailable())
    }

    /// Long-lived background process registry — start.
    async fn monitor_start(&self, _req: MonitorStartRequest) -> Result<MonitorHandle> {
        Err(unavailable())
    }

    /// Long-lived background process registry — list active monitors.
    async fn monitor_list(&self) -> Result<Vec<MonitorHandle>> {
        Err(unavailable())
    }

    /// Long-lived background process registry — read pending output.
    async fn monitor_read(&self, _req: MonitorReadRequest) -> Result<MonitorReadResponse> {
        Err(unavailable())
    }

    /// Long-lived background process registry — stop.
    async fn monitor_stop(&self, _req: MonitorStopRequest) -> Result<MonitorHandle> {
        Err(unavailable())
    }

    /// Dynamic tool registry — register a new tool owned by this plugin.
    async fn register_tool(
        &self,
        _req: HostToolRegisterRequest,
    ) -> Result<HostToolMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic tool registry — replace the definition of an existing tool owned by
    /// this plugin (matched by `tool.name`).
    async fn update_tool(&self, _req: HostToolUpdateRequest) -> Result<HostToolMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic tool registry — remove a tool owned by this plugin.
    async fn remove_tool(&self, _req: HostToolRemoveRequest) -> Result<HostToolMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic tool registry — list all registered tools known to the plugin host.
    async fn list_registered_tools(&self) -> Result<HostRegisteredToolListResponse> {
        Err(unavailable())
    }

    /// Plugin-namespaced KV storage — read.
    async fn storage_get(&self, _req: HostStorageGetRequest) -> Result<HostStorageGetResponse> {
        Err(unavailable())
    }

    /// Plugin-namespaced KV storage — write.
    async fn storage_set(&self, _req: HostStorageSetRequest) -> Result<()> {
        Err(unavailable())
    }

    /// Plugin-namespaced KV storage — delete.
    async fn storage_delete(&self, _req: HostStorageDeleteRequest) -> Result<()> {
        Err(unavailable())
    }

    /// Plugin-namespaced KV storage — enumerate keys.
    async fn storage_list(&self, _req: HostStorageListRequest) -> Result<HostStorageListResponse> {
        Err(unavailable())
    }

    /// Plugin-scoped secret storage — read a secret value.
    async fn secret_get(&self, _req: HostSecretGetRequest) -> Result<HostSecretGetResponse> {
        Err(unavailable())
    }

    /// Plugin-scoped secret storage — write a secret value.
    async fn secret_set(&self, _req: HostSecretSetRequest) -> Result<()> {
        Err(unavailable())
    }

    /// Plugin-scoped secret storage — delete a secret value.
    async fn secret_delete(&self, _req: HostSecretDeleteRequest) -> Result<()> {
        Err(unavailable())
    }

    /// Plugin-scoped secret storage — list secret names (never values).
    async fn secret_list(&self) -> Result<HostSecretListResponse> {
        Err(unavailable())
    }

    /// Daemon lifecycle — list every plugin status known to the host.
    async fn plugin_status_list(&self) -> Result<HostPluginStatusListResponse> {
        Err(unavailable())
    }

    /// Daemon lifecycle — fetch a single plugin status by id.
    async fn plugin_status_get(
        &self,
        _req: HostPluginStatusGetRequest,
    ) -> Result<HostPluginStatusGetResponse> {
        Err(unavailable())
    }

    /// LSP read-only observability — list configured servers.
    async fn lsp_list_servers(&self) -> Result<HostLspListServersResponse> {
        Err(unavailable())
    }

    /// LSP read-only observability — list cached diagnostics.
    async fn lsp_list_diagnostics(
        &self,
        _req: HostLspListDiagnosticsRequest,
    ) -> Result<HostLspListDiagnosticsResponse> {
        Err(unavailable())
    }

    /// Snapshot registry — list active snapshots.
    async fn snapshot_list(&self) -> Result<HostSnapshotListResponse> {
        Err(unavailable())
    }

    /// Scheduler — list every queued/recurring job.
    async fn scheduler_list(&self) -> Result<HostSchedulerListResponse> {
        Err(unavailable())
    }

    /// Scheduler — register a new job (cron or one-shot).
    async fn scheduler_create(
        &self,
        _req: HostSchedulerCreateRequest,
    ) -> Result<HostSchedulerCreateResponse> {
        Err(unavailable())
    }

    /// Scheduler — delete a job by id.
    async fn scheduler_delete(
        &self,
        _req: HostSchedulerDeleteRequest,
    ) -> Result<HostSchedulerDeleteResponse> {
        Err(unavailable())
    }

    /// Subagent profile registry — register or update a runtime profile.
    async fn agent_register(&self, _req: HostAgentRegisterRequest) -> Result<()> {
        Err(unavailable())
    }

    /// Subagent profile registry — remove a runtime profile by name.
    async fn agent_remove(&self, _req: HostAgentRemoveRequest) -> Result<HostAgentRemoveResponse> {
        Err(unavailable())
    }

    /// Subagent profile registry — list every profile currently registered.
    async fn agent_list(&self) -> Result<HostAgentListResponse> {
        Err(unavailable())
    }

    /// Subagent profile registry — read one profile by name or alias.
    async fn agent_get(&self, _req: HostAgentGetRequest) -> Result<HostAgentGetResponse> {
        Err(unavailable())
    }

    /// Switch the active session to another runtime agent profile.
    async fn agent_switch(&self, _req: HostAgentSwitchRequest) -> Result<HostAgentSwitchResponse> {
        Err(unavailable())
    }

    /// Restore the active session to the previous agent pushed by
    /// [`HostClient::agent_switch`].
    async fn agent_restore(
        &self,
        _req: HostAgentRestoreRequest,
    ) -> Result<HostAgentRestoreResponse> {
        Err(unavailable())
    }

    /// Hook registry — list every hook currently subscribed across plugins.
    async fn hook_list(&self) -> Result<HostHookListResponse> {
        Err(unavailable())
    }

    /// MCP registry — list known MCP servers.
    async fn mcp_list_servers(&self) -> Result<HostMcpListServersResponse> {
        Err(unavailable())
    }

    /// MCP registry — register or replace a server.
    async fn mcp_add_server(&self, _req: HostMcpAddServerRequest) -> Result<()> {
        Err(unavailable())
    }

    /// MCP registry — remove a server by name.
    async fn mcp_remove_server(
        &self,
        _req: HostMcpRemoveServerRequest,
    ) -> Result<HostMcpRemoveServerResponse> {
        Err(unavailable())
    }

    /// UI statusline — contribute or update a segment.
    async fn ui_statusline_contribute(&self, _req: HostStatuslineContributeRequest) -> Result<()> {
        Err(unavailable())
    }

    /// UI statusline — list every contributed segment in priority order.
    async fn ui_statusline_list(&self) -> Result<HostStatuslineListResponse> {
        Err(unavailable())
    }

    /// UI statusline — remove a segment by id.
    async fn ui_statusline_remove(
        &self,
        _req: HostStatuslineRemoveRequest,
    ) -> Result<HostStatuslineRemoveResponse> {
        Err(unavailable())
    }

    /// UI theme — register or update a palette.
    async fn ui_theme_register(&self, _req: HostThemeRegisterRequest) -> Result<()> {
        Err(unavailable())
    }

    /// UI theme — list every registered palette.
    async fn ui_theme_list(&self) -> Result<HostThemeListResponse> {
        Err(unavailable())
    }

    /// UI theme — remove a palette by id.
    async fn ui_theme_remove(
        &self,
        _req: HostThemeRemoveRequest,
    ) -> Result<HostThemeRemoveResponse> {
        Err(unavailable())
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct EventSubscription {
    pub id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostCallbackContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    /// When the active host call originated from a `tool_invoke`, this holds
    /// the plugin-original tool name. Used for per-tool capability scoping
    /// so that capabilities declared by tool A do not implicitly authorize
    /// host calls coming back through tool B.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

tokio::task_local! {
    static HOST_CALLBACK_CONTEXT: HostCallbackContext;
}

pub async fn run_in_host_callback_context<F>(patch: HostCallbackContext, fut: F) -> F::Output
where
    F: Future,
{
    let mut current = current_host_callback_context().unwrap_or_default();
    if let Some(plugin_id) = patch.plugin_id {
        current.plugin_id = Some(plugin_id);
    }
    if let Some(session_id) = patch.session_id {
        current.session_id = Some(session_id);
    }
    if let Some(call_id) = patch.call_id {
        current.call_id = Some(call_id);
    }
    if let Some(workspace_root) = patch.workspace_root {
        current.workspace_root = Some(workspace_root);
    }
    if let Some(tool_name) = patch.tool_name {
        current.tool_name = Some(tool_name);
    }
    HOST_CALLBACK_CONTEXT.scope(current, fut).await
}

pub fn current_host_callback_context() -> Option<HostCallbackContext> {
    HOST_CALLBACK_CONTEXT.try_with(Clone::clone).ok()
}

// ---------------- permission checks ----------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostPathPermissionCheckRequest {
    pub path: String,
    pub kind: PathKind,
}

impl HostPathPermissionCheckRequest {
    pub fn read(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: PathKind::Read,
        }
    }

    pub fn write(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: PathKind::Write,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostNetworkPermissionCheckRequest {
    pub target: String,
}

impl HostNetworkPermissionCheckRequest {
    pub fn connect(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostPermissionCheckResponse {
    pub decision: PermissionDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub explanation: String,
}

impl HostPermissionCheckResponse {
    pub fn allowed() -> Self {
        Self {
            decision: PermissionDecision::Allow,
            reason: None,
            explanation: String::new(),
        }
    }

    pub fn is_allowed(&self) -> bool {
        self.decision == PermissionDecision::Allow
    }

    pub fn ensure_allowed(&self) -> Result<()> {
        if self.is_allowed() {
            return Ok(());
        }
        let reason = self
            .reason
            .as_deref()
            .filter(|reason| !reason.trim().is_empty())
            .or_else(|| (!self.explanation.trim().is_empty()).then_some(self.explanation.as_str()))
            .unwrap_or("permission check did not allow the requested access");
        Err(PluginError::new(reason.to_string()))
    }
}

// ---------------- ask_user ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskUserOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preview_markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskUserQuestion {
    pub id: String,
    #[serde(default)]
    pub header: String,
    pub question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<AskUserOption>,
    #[serde(default)]
    pub multiple: bool,
    #[serde(default)]
    pub allow_custom: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AskUserRequest {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_markdown: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub submit_label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cancel_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_resolution_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<AskUserQuestion>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default)]
    pub allow_free_text: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AskUserResponse {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reply: String,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub answers: BTreeMap<String, Vec<String>>,
}

// ---------------- spawn_subtask ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnSubtaskRequest {
    pub subagent_type: String,
    pub description: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnSubtaskResponse {
    pub final_text: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

// ---------------- list_tools ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostSession {
    pub id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<i64>,
    pub root_id: i64,
    pub workspace_id: i64,
    pub title: String,
    #[serde(default)]
    pub is_subagent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostGetSessionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostGetSessionResponse {
    pub session: HostSession,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostRenameSessionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostRenameSessionResponse {
    pub session: HostSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostEnterSnapshotRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostExitSnapshotRequest {
    pub action: String,
    #[serde(default)]
    pub discard_changes: bool,
}

// ---------------- monitor ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorStartRequest {
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub persistent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_buffered_lines: Option<u32>,
    #[serde(default = "default_true")]
    pub capture_stderr: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorHandle {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    pub persistent: bool,
    #[serde(default)]
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<i64>,
    #[serde(default)]
    pub buffered_lines: u32,
    #[serde(default)]
    pub last_seq: u64,
    #[serde(default)]
    pub dropped_lines: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorEvent {
    pub seq: u64,
    pub stream: String,
    pub ts_ms: i64,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorReadRequest {
    pub id: String,
    #[serde(default)]
    pub follow: bool,
    #[serde(default)]
    pub since_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default)]
    pub wait_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorReadResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<MonitorEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub monitors: Vec<MonitorHandle>,
    pub stdout: String,
    pub stderr: String,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    pub last_seq: u64,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub dropped_lines: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorStopRequest {
    pub id: String,
    #[serde(default)]
    pub force: bool,
}

// ---------------- tool registry ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostToolRegisterRequest {
    pub tool: ToolDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostToolUpdateRequest {
    pub tool: ToolDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostToolRemoveRequest {
    pub name: String,
    #[serde(default)]
    pub by_model_name: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostToolMutationResponse {
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<ToolRegistryChangedEvent>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolRegistryChangeKind {
    Registered,
    Updated,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolRegistryChangedEvent {
    pub kind: ToolRegistryChangeKind,
    pub generation: u64,
    pub timestamp_ms: i64,
    pub plugin: PluginKey,
    pub tool_key: ToolKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostRegisteredToolDescriptor {
    pub plugin: PluginKey,
    pub tool_key: ToolKey,
    pub tool: ToolDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostRegisteredToolListResponse {
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<HostRegisteredToolDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event: Option<ToolRegistryChangedEvent>,
}

// ---------------- plugin storage ----------------

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostStorageScope {
    Session,
    Workspace,
    #[default]
    Global,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostStorageVisibility {
    #[default]
    Private,
    Shared,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStorageGetRequest {
    #[serde(default)]
    pub scope: HostStorageScope,
    #[serde(default)]
    pub visibility: HostStorageVisibility,
    pub namespace: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStorageGetResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStorageSetRequest {
    #[serde(default)]
    pub scope: HostStorageScope,
    #[serde(default)]
    pub visibility: HostStorageVisibility,
    pub namespace: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStorageDeleteRequest {
    #[serde(default)]
    pub scope: HostStorageScope,
    #[serde(default)]
    pub visibility: HostStorageVisibility,
    pub namespace: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostStorageListRequest {
    #[serde(default)]
    pub scope: HostStorageScope,
    #[serde(default)]
    pub visibility: HostStorageVisibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostStorageListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<HostStorageRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStorageRecord {
    pub namespace: String,
    pub key: String,
}

// ---------------- plugin secrets ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSecretGetRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSecretGetResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSecretSetRequest {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSecretDeleteRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostSecretListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
}

// ---------------- plugin status ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPluginStatus {
    pub plugin_id: PluginKey,
    pub kind: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default)]
    pub restart_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_restart_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostPluginStatusListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<HostPluginStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPluginStatusGetRequest {
    pub plugin_id: PluginKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostPluginStatusGetResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<HostPluginStatus>,
}

// ---------------- lsp ----------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostLspListServersResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<HostLspServer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostLspServer {
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_extensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostLspListDiagnosticsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostLspListDiagnosticsResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<HostLspDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostLspDiagnostic {
    pub uri: String,
    pub severity: String,
    pub message: String,
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

// ---------------- snapshot / scheduler ----------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostSnapshotListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snapshots: Vec<HostSnapshotSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSnapshotSummary {
    pub session_id: i64,
    pub path: String,
    pub branch: String,
    #[serde(default)]
    pub created_here: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostSchedulerListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jobs: Vec<HostSchedulerJob>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSchedulerJob {
    pub id: String,
    pub kind: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_fire_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostSchedulerCreateRequest {
    Cron {
        expression: String,
        prompt: String,
        #[serde(default)]
        max_age_days: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner_session_id: Option<i64>,
    },
    Once {
        at_ms: i64,
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner_session_id: Option<i64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSchedulerCreateResponse {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSchedulerDeleteRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostSchedulerDeleteResponse {
    pub removed: bool,
}

// ---------------- hooks ----------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostHookListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HostHookRegistration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostHookRegistration {
    pub plugin_id: PluginKey,
    pub trust_level: String,
    pub trust_status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<String>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HostHookDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostHookDescriptor {
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    pub trust_level: String,
    pub trust_status: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_hash: Option<String>,
}

// ---------------- mcp ----------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostMcpListServersResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostMcpServerSpec {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    Http {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer: Option<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostMcpAddServerRequest {
    pub name: String,
    pub spec: HostMcpServerSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostMcpRemoveServerRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostMcpRemoveServerResponse {
    pub removed: bool,
}

// ---------------- UI: statusline / theme ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStatuslineSegment {
    pub plugin_id: PluginKey,
    pub segment_id: String,
    pub content: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<PluginTuiColor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStatuslineContributeRequest {
    pub segment_id: String,
    pub content: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<PluginTuiColor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStatuslineRemoveRequest {
    pub segment_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostStatuslineRemoveResponse {
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostStatuslineListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<HostStatuslineSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostConfigReloadResponse {
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostThemePalette {
    pub id: String,
    pub plugin_id: PluginKey,
    pub display_name: String,
    pub colors: PluginTuiThemeColors,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostThemeRegisterRequest {
    pub id: String,
    pub display_name: String,
    pub colors: PluginTuiThemeColors,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostThemeRemoveRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostThemeRemoveResponse {
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostThemeListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<HostThemePalette>,
}

fn default_true() -> bool {
    true
}

/// Default impl returning `HostUnavailable` for every callback. Use in tests.
pub struct NoopHostClient;

#[async_trait]
impl HostClient for NoopHostClient {
    async fn log(&self, _l: LogLevel, _m: String, _f: serde_json::Value) {}

    async fn publish_event(&self, _: EventEnvelope) -> Result<()> {
        Err(unavailable())
    }

    async fn subscribe_events(&self, _: EventFilter) -> Result<EventSubscription> {
        Err(unavailable())
    }

    async fn ask_permission(&self, _: PermissionAskInput) -> Result<PermissionDecision> {
        Err(unavailable())
    }

    async fn check_path_permission(
        &self,
        _: HostPathPermissionCheckRequest,
    ) -> Result<HostPermissionCheckResponse> {
        Err(unavailable())
    }

    async fn check_network_permission(
        &self,
        _: HostNetworkPermissionCheckRequest,
    ) -> Result<HostPermissionCheckResponse> {
        Err(unavailable())
    }

    async fn read_config(&self, _: Option<String>) -> Result<serde_json::Value> {
        Err(unavailable())
    }

    async fn invoke_tool(&self, _: String, _: serde_json::Value) -> Result<ToolInvokeOutput> {
        Err(unavailable())
    }
}

fn unavailable() -> PluginError {
    PluginError {
        code: crate::error::PluginErrorCode::HostUnavailable,
        message: "host is unavailable for this plugin".into(),
        hook: None,
        plugin: None,
        data: None,
    }
}
