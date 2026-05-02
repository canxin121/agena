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
