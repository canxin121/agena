//! Runtime boundary for externally initiated session tool execution.

use agena_domain::ToolInvocation;
use agena_tool::ToolExecutionSummary;
use async_trait::async_trait;

/// Infrastructure or execution failure for a session-scoped invocation.
/// Authorization outcomes are deliberately returned through
/// [`SessionToolExecutionOutcome`] instead of this error channel.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionToolExecutionError {
    #[error("tool invocation failed: {0}")]
    Execution(String),
}

#[derive(Debug)]
pub enum SessionToolExecutionOutcome {
    Completed(ToolExecutionSummary),
    ApprovalRequired {
        request_id: Option<String>,
        reason: String,
    },
    PolicyDenied(Box<agena_domain::PolicyDeniedResult>),
    CapabilityUnavailable(Box<agena_domain::CapabilityUnavailableResult>),
    ToolUnavailable(Box<agena_domain::ToolUnavailableResult>),
}

impl SessionToolExecutionOutcome {
    /// Preserve every authorization/availability outcome as a normal tool
    /// summary for protocol surfaces whose contract has only one success
    /// envelope (for example MCP).
    pub fn into_summary(self) -> ToolExecutionSummary {
        match self {
            Self::Completed(summary) => summary,
            Self::ApprovalRequired { request_id, reason } => ToolExecutionSummary {
                title: "Approval required".to_string(),
                output_text: format!(
                    "The operation was not executed because it requires user approval: {reason}"
                ),
                payload: Some(serde_json::json!({
                    "status": "approval_required",
                    "code": "approval_required",
                    "request_id": request_id,
                    "reason": reason,
                })),
                metadata: [("agena.outcome".to_string(), "approval_required".to_string())]
                    .into_iter()
                    .collect(),
                attachments: Vec::new(),
            },
            Self::PolicyDenied(denial) => ToolExecutionSummary {
                title: "Blocked by permission policy".to_string(),
                output_text: format!(
                    "The operation was not executed because it is blocked by the effective permission policy: {}",
                    denial.reason
                ),
                payload: Some(serde_json::json!({
                    "status": "policy_denied",
                    "code": "permission_policy_denied",
                    "retryable": false,
                    "denial": denial,
                })),
                metadata: [("agena.outcome".to_string(), "policy_denied".to_string())]
                    .into_iter()
                    .collect(),
                attachments: Vec::new(),
            },
            Self::CapabilityUnavailable(unavailable) => ToolExecutionSummary {
                title: "Capability unavailable".to_string(),
                output_text: format!(
                    "The operation was not executed because the required capability is unavailable: {}",
                    unavailable.reason
                ),
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
                output_text: format!(
                    "The operation was not executed because the tool is unavailable: {}",
                    unavailable.reason
                ),
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

#[derive(Debug)]
pub enum HostActionAuthorization {
    Allowed,
    PolicyDenied(Box<agena_domain::PolicyDeniedResult>),
    UserDeclined(agena_domain::UserDeclinedResult),
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
    ) -> Result<SessionToolExecutionOutcome, SessionToolExecutionError>;
}
