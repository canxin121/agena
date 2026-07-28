//! Runtime boundary for externally initiated session tool execution.

use agena_domain::ToolInvocation;
use agena_tool::ToolExecutionSummary;
use async_trait::async_trait;

/// Stable outcome failures for a session-scoped tool invocation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionToolExecutionError {
    #[error("tool invocation requires approval: {0}")]
    ApprovalRequired(String),
    #[error("tool invocation denied: {0}")]
    Denied(String),
    #[error("tool invocation failed: {0}")]
    Execution(String),
}

/// Executes an already session-authorized tool through a runtime-neutral
/// summary contract. Concrete permission checks and executor construction stay
/// inside the adapter implementation.
#[async_trait]
pub trait SessionToolExecutionService: Send + Sync {
    async fn execute_session_tool(
        &self,
        session_id: i64,
        invocation: ToolInvocation,
    ) -> Result<ToolExecutionSummary, SessionToolExecutionError>;

    /// Executes a tool after an interactive host surface has collected an
    /// explicit user confirmation for the concrete tool and input. Persisted
    /// deny rules still win; only `Ask` decisions are satisfied by this
    /// one-shot approval.
    async fn execute_session_tool_with_user_approval(
        &self,
        session_id: i64,
        invocation: ToolInvocation,
    ) -> Result<ToolExecutionSummary, SessionToolExecutionError>;

    /// Renders a synchronously requested tool prompt using the session's
    /// concrete executor. This is deliberately distinct from the authorized
    /// command execution path above.
    fn render_session_tool_output(
        &self,
        session_id: i64,
        invocation: ToolInvocation,
    ) -> Result<String, SessionToolExecutionError>;
}
