use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};

/// Business lifecycle state shared by messages, operations, and interactive
/// requests. Persistence-specific integer encoding lives outside domain.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    Default,
    PartialEq,
    Eq,
    Hash,
    AsRefStr,
    Display,
    EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Status of an execution: pending, running, or terminal.
pub enum ExecutionStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    /// The operation was not executed because an effective permission rule
    /// explicitly denied at least one protected action.
    PolicyDenied,
    /// The operation was not executed because the user declined an
    /// interactive permission request.
    UserDeclined,
    /// The current agent/runtime has no capability that could execute the
    /// operation. User approval cannot create this capability.
    CapabilityUnavailable,
    /// The named tool is not registered or loadable in the current runtime.
    ToolUnavailable,
    Failed,
    Cancelled,
}

impl ExecutionStatus {
    /// Whether a lifecycle update is valid. Identity transitions are
    /// intentionally idempotent.
    pub fn can_transition(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        matches!(
            (self, next),
            (
                Self::Pending,
                Self::InProgress
                    | Self::Completed
                    | Self::PolicyDenied
                    | Self::UserDeclined
                    | Self::CapabilityUnavailable
                    | Self::ToolUnavailable
                    | Self::Failed
                    | Self::Cancelled
            ) | (
                Self::InProgress,
                Self::Completed
                    | Self::PolicyDenied
                    | Self::UserDeclined
                    | Self::CapabilityUnavailable
                    | Self::ToolUnavailable
                    | Self::Failed
                    | Self::Cancelled
            )
        )
    }

    /// Whether no further lifecycle transition is expected. Denial outcomes
    /// (policy/user/capability/tool) are terminal for the operation part even
    /// though the coarse persisted state collapses them to a failed/cancelled
    /// run state.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::PolicyDenied
                | Self::UserDeclined
                | Self::CapabilityUnavailable
                | Self::ToolUnavailable
                | Self::Failed
                | Self::Cancelled
        )
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid execution status transition: {from:?} -> {to:?}")]
/// Error for an invalid execution status transition.
pub struct ExecutionStatusTransitionError {
    pub from: ExecutionStatus,
    pub to: ExecutionStatus,
}

#[cfg(test)]
mod tests {
    use super::ExecutionStatus;

    #[test]
    fn status_wire_names_and_transition_rules_are_stable() {
        assert_eq!(
            serde_json::to_string(&ExecutionStatus::InProgress).expect("serialize status"),
            "\"in_progress\""
        );
        assert!(ExecutionStatus::Pending.can_transition(ExecutionStatus::InProgress));
        assert!(ExecutionStatus::Pending.can_transition(ExecutionStatus::Completed));
        assert!(ExecutionStatus::InProgress.can_transition(ExecutionStatus::Completed));
        assert!(!ExecutionStatus::Completed.can_transition(ExecutionStatus::InProgress));
    }
}
