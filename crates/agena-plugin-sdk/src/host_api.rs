//! `HostClient` — the API plugins use to call back into the host.
//!
//! At runtime the host injects a concrete impl into `Plugin::init`. For tests
//! and minimal plugins, [`NoopHostClient`] returns errors for everything.

use std::collections::BTreeMap;

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

    async fn invoke_tool(
        &self,
        tool: String,
        input: serde_json::Value,
    ) -> Result<ToolInvokeOutput>;

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

    /// Run a command in the platform sandbox (procwarden). Used by `bash`
    /// and `apply_patch`.
    async fn execute_sandboxed_command(
        &self,
        _req: SandboxCommandRequest,
    ) -> Result<SandboxCommandResponse> {
        Err(unavailable())
    }

    /// List all currently registered tools (used by `tool_search`).
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>> {
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
    async fn monitor_stop(&self, _req: MonitorStopRequest) -> Result<()> {
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

// ---------------- ask_user ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskUserRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default)]
    pub allow_free_text: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskUserResponse {
    pub reply: String,
    #[serde(default)]
    pub cancelled: bool,
}

// ---------------- spawn_subtask ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnSubtaskRequest {
    pub subagent_type: String,
    pub description: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnSubtaskResponse {
    pub final_text: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

// ---------------- sandbox command ----------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    ReadOnly,
    WriteSandboxed,
    WriteUnsandboxed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxCommandRequest {
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    pub mode: SandboxMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Paths the command needs write access to (used to widen the sandbox
    /// policy for write-sandboxed mode).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxCommandResponse {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    #[serde(default)]
    pub timed_out: bool,
}

// ---------------- list_tools ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorHandle {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorReadRequest {
    pub id: String,
    #[serde(default)]
    pub follow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorReadResponse {
    pub stdout: String,
    pub stderr: String,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorStopRequest {
    pub id: String,
    #[serde(default)]
    pub force: bool,
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
