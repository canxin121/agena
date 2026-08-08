//! Runtime boundary for externally initiated session tool execution.

use agena_domain::ToolInvocation;
use agena_tool::ToolExecutionSummary;
use async_trait::async_trait;

/// Infrastructure or execution failure for a session-scoped invocation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
/// Error of a session tool execution.
pub enum SessionToolExecutionError {
    #[error("tool invocation failed: {0}")]
    Execution(String),
}

#[derive(Debug)]
/// Outcome of a session tool execution.
pub enum SessionToolExecutionOutcome {
    Completed(ToolExecutionSummary),
    CapabilityUnavailable(Box<agena_domain::CapabilityUnavailableResult>),
    ToolUnavailable(Box<agena_domain::ToolUnavailableResult>),
}

impl SessionToolExecutionOutcome {
    /// Preserve availability outcomes as a normal tool summary for protocol
    /// surfaces whose contract has only one success envelope (for example
    /// MCP).
    pub fn into_summary(self) -> ToolExecutionSummary {
        match self {
            Self::Completed(summary) => summary,
            Self::CapabilityUnavailable(unavailable) => ToolExecutionSummary {
                title: "Capability unavailable".to_string(),
                summary: unavailable.reason.clone(),
                output_text: format!(
                    "The operation was not executed because the required capability is unavailable: {}",
                    unavailable.reason
                ),
                sections: Vec::new(),
                payload: Some(serde_json::json!({
                    "status": "capability_unavailable",
                    "code": "capability_unavailable",
                    "retryable": unavailable.retryable,
                    "unavailable": unavailable,
                })),
                metadata: [(
                    "agena.outcome".to_string(),
                    "capability_unavailable".to_string(),
                )]
                .into_iter()
                .collect(),
                attachments: Vec::new(),
            },
            Self::ToolUnavailable(unavailable) => ToolExecutionSummary {
                title: "Tool unavailable".to_string(),
                summary: unavailable.reason.clone(),
                output_text: format!(
                    "The operation was not executed because the tool is unavailable: {}",
                    unavailable.reason
                ),
                sections: Vec::new(),
                payload: Some(serde_json::json!({
                    "status": "tool_unavailable",
                    "code": "tool_unavailable",
                    "retryable": unavailable.retryable,
                    "unavailable": unavailable,
                })),
                metadata: [("agena.outcome".to_string(), "tool_unavailable".to_string())]
                    .into_iter()
                    .collect(),
                attachments: Vec::new(),
            },
        }
    }
}

/// Executes a session-scoped application tool through a runtime-neutral
/// summary contract. Model permission checks do not apply to this surface.
#[async_trait]
/// Service that executes tools within a session.
pub trait SessionToolExecutionService: Send + Sync {
    async fn execute_session_tool(
        &self,
        session_id: i64,
        invocation: ToolInvocation,
    ) -> Result<SessionToolExecutionOutcome, SessionToolExecutionError>;
}
