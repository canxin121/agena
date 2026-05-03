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
use crate::manifest::PluginEntryDecl;

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

    async fn read_config(&self, path: Option<String>) -> Result<serde_json::Value>;

    async fn invoke_tool(&self, tool: String, input: serde_json::Value)
    -> Result<ToolInvokeOutput>;

    // ---------------- Built-in-style host capabilities ----------------
    //
    // These are used by the in-process built-in plugins (bash, ask_user, task,
    // monitor, ...). External plugins generally don't need to implement them;
    // the default `NoopHostClient` and host implementations that don't expose
    // these capabilities should return `HostUnavailable`.

    /// Prompt the user for input via the active session UI (used by the
    /// `ask_user` built-in tool).
    async fn ask_user(&self, _req: AskUserRequest) -> Result<AskUserResponse> {
        Err(unavailable())
    }

    /// Spawn a child agent / subtask. Used by the `task` built-in tool.
    async fn spawn_subtask(&self, _req: SpawnSubtaskRequest) -> Result<SpawnSubtaskResponse> {
        Err(unavailable())
    }

    /// List all currently registered tools (used by `tool_search`).
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>> {
        Err(unavailable())
    }

    /// Execute a host-owned built-in adapter. This is reserved for the in-process
    /// built-ins plugin and should not be exposed to arbitrary plugins.
    async fn execute_builtin_tool(&self, _req: BuiltinToolRequest) -> Result<ToolInvokeOutput> {
        Err(unavailable())
    }

    /// Read a skill body and metadata by name.
    async fn skill_get(&self, _req: HostSkillGetRequest) -> Result<HostSkillGetResponse> {
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

    /// Dynamic entry registry — register a new entry owned by this plugin.
    async fn entry_register(
        &self,
        _req: HostEntryRegisterRequest,
    ) -> Result<HostEntryMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic entry registry — replace the decl of an existing entry owned by
    /// this plugin (matched by `entry.name`).
    async fn entry_update(
        &self,
        _req: HostEntryUpdateRequest,
    ) -> Result<HostEntryMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic entry registry — remove an entry owned by this plugin.
    async fn entry_remove(
        &self,
        _req: HostEntryRemoveRequest,
    ) -> Result<HostEntryMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic entry registry — list all entries known to the plugin host.
    async fn entry_list(&self) -> Result<HostEntryListResponse> {
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

    /// Plan registry — list active plan sessions.
    async fn plan_list(&self) -> Result<HostPlanListResponse> {
        Err(unavailable())
    }

    /// Plan registry — read a plan by session id.
    async fn plan_get(&self, _req: HostPlanGetRequest) -> Result<HostPlanGetResponse> {
        Err(unavailable())
    }

    /// Worktree registry — list active worktrees.
    async fn worktree_list(&self) -> Result<HostWorktreeListResponse> {
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

    /// Slash command registry — register or update a runtime command.
    async fn command_register(&self, _req: HostCommandRegisterRequest) -> Result<()> {
        Err(unavailable())
    }

    /// Slash command registry — remove a runtime command by name.
    async fn command_remove(
        &self,
        _req: HostCommandRemoveRequest,
    ) -> Result<HostCommandRemoveResponse> {
        Err(unavailable())
    }

    /// Slash command registry — list every command currently registered.
    async fn command_list(&self) -> Result<HostCommandListResponse> {
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
}

tokio::task_local! {
    static HOST_CALLBACK_CONTEXT: HostCallbackContext;
}

pub async fn with_host_callback_context<F>(patch: HostCallbackContext, fut: F) -> F::Output
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
    HOST_CALLBACK_CONTEXT.scope(current, fut).await
}

pub fn current_host_callback_context() -> Option<HostCallbackContext> {
    HOST_CALLBACK_CONTEXT.try_with(Clone::clone).ok()
}

// ---------------- ask_user ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskUserOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
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
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_terms: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
    #[serde(default)]
    pub deferred: bool,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinToolRequest {
    pub tool_name: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSkillGetRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSkillGetResponse {
    pub name: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
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

// ---------------- entry registry ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEntryRegisterRequest {
    pub entry: PluginEntryDecl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEntryUpdateRequest {
    pub entry: PluginEntryDecl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEntryRemoveRequest {
    pub name: String,
    #[serde(default)]
    pub exposed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEntryMutationResponse {
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposed_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<PluginEntryDecl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEntryDescriptor {
    pub plugin_id: String,
    pub original_name: String,
    pub exposed_name: String,
    pub entry: PluginEntryDecl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEntryListResponse {
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<HostEntryDescriptor>,
}

// ---------------- plugin storage ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStorageGetRequest {
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
    pub namespace: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStorageDeleteRequest {
    pub namespace: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostStorageListRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostStorageListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<HostStorageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStorageEntry {
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
    pub plugin_id: String,
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
    pub entries: Vec<HostPluginStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPluginStatusGetRequest {
    pub plugin_id: String,
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
    pub entries: Vec<HostLspDiagnostic>,
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

// ---------------- plan / worktree / scheduler ----------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostPlanListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<HostPlanEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPlanEntry {
    pub session_id: i64,
    pub slug: String,
    pub file_path: String,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPlanGetRequest {
    pub session_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostPlanGetResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<HostPlanEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostWorktreeListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<HostWorktreeEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostWorktreeEntry {
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

// ---------------- commands / agents ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCommandDescriptor {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub body: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCommandRegisterRequest {
    pub command: HostCommandDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCommandRemoveRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostCommandRemoveResponse {
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostCommandListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<HostCommandDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAgentDescriptor {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAgentRegisterRequest {
    pub agent: HostAgentDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAgentRemoveRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostAgentRemoveResponse {
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostAgentListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<HostAgentDescriptor>,
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
