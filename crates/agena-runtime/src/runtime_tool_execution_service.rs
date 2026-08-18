//! Runtime boundary for tools invoked outside a session lifecycle.
//!
//! MCP server calls have no Agena session ID, so they cannot use the
//! session-authorized tool port. Concrete executor selection (the active
//! session executor or a bootstrap fallback) remains inside the runtime
//! composition adapter.

use agena_domain::ToolInvocation;
use async_trait::async_trait;

#[derive(Debug, Clone)]
/// Descriptor of a runtime tool.
pub struct RuntimeToolDescriptor {
    pub name: String,
    pub summary: Option<String>,
    pub before_help: Option<String>,
    pub after_help: Option<String>,
    pub input_schema: serde_json::Value,
    pub output_schema: Option<serde_json::Value>,
    /// Whether this tool is marked as interactive by either its permission
    /// contract or its discovery metadata. Non-interactive hosts, such as MCP
    /// clients, must not expose it.
    pub interactive: bool,
    /// True only when the authority-bearing permission contract qualifies the
    /// tool for Agena's read-only fast path (no shell, writes, mutation, or
    /// network access).
    pub read_only: bool,
    /// The tool may mutate state, write paths, or execute arbitrary shell
    /// commands. This is exposed to MCP clients as a safety hint and is also
    /// used by the MCP server's default-deny exposure policy.
    pub destructive: bool,
    /// The tool can reach network targets outside the local workspace.
    pub open_world: bool,
    /// Long-running autonomous task tools are not suitable for the stateless
    /// ChatGPT connector surface.
    pub task: bool,
    /// Stable full plugin id (for example, `agena.fs`).
    pub plugin_id: Option<String>,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("runtime tool execution failed: {message}")]
/// Error executing a runtime tool.
pub struct RuntimeToolExecutionError {
    message: String,
}

impl RuntimeToolExecutionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
/// Service executing tools in the runtime.
pub trait RuntimeToolExecutionService: Send + Sync {
    async fn available_runtime_tools(&self) -> Vec<RuntimeToolDescriptor>;

    /// Provider-facing Tool API function declarations available to model
    /// executions. These are provider-contract values, not concrete registry
    /// bindings, so diagnostics and integration fixtures can validate the
    /// advertised model surface without traversing a concrete tool executor.
    async fn available_tool_api_definitions(&self) -> Vec<agena_provider::ToolApiDefinition>;

    async fn execute_runtime_tool(
        &self,
        invocation: &ToolInvocation,
        call_id: i64,
    ) -> Result<crate::SessionToolExecutionOutcome, RuntimeToolExecutionError>;
}
