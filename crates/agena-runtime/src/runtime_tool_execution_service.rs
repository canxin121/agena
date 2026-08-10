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
