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

/// Explicit snapshot lifecycle commands exposed to presentation layers. These
/// are not generic host payloads: Runtime owns the narrow input/output shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSnapshotCommand {
    Enter {
        name: Option<String>,
        path: Option<String>,
    },
    Exit {
        action: String,
        discard_changes: bool,
    },
}

#[derive(Debug, Clone, Default)]
pub struct SessionSnapshotCommandResult {
    pub payload: Option<serde_json::Value>,
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

    /// Renders a synchronously requested tool prompt using the session's
    /// concrete executor. This is deliberately distinct from the authorized
    /// command execution path above.
    fn render_session_tool_output(
        &self,
        session_id: i64,
        invocation: ToolInvocation,
    ) -> Result<String, SessionToolExecutionError>;

    fn execute_snapshot_command(
        &self,
        session_id: i64,
        command: SessionSnapshotCommand,
    ) -> Result<SessionSnapshotCommandResult, SessionToolExecutionError>;
}
