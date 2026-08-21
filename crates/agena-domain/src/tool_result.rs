use serde::{Deserialize, Serialize};

/// Lifecycle state of a canonical tool call.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultState {
    #[default]
    Pending,
    Running,
    Completed,
    /// No tool side effect was started because an effective permission rule
    /// explicitly prohibited the invocation.
    PolicyDenied,
    /// No tool side effect was started because the user declined a pending
    /// permission request.
    UserDeclined,
    CapabilityUnavailable,
    ToolUnavailable,
    Failed,
    Cancelled,
}

impl ToolResultState {
    pub const fn is_pending(value: &Self) -> bool {
        matches!(value, Self::Pending)
    }
}

#[cfg(test)]
mod tests {
    use super::ToolResultState;

    #[test]
    fn result_state_and_display_have_stable_defaults() {
        assert_eq!(ToolResultState::default(), ToolResultState::Pending);
        assert_eq!(
            serde_json::to_string(&ToolResultState::Completed).unwrap(),
            "\"completed\""
        );
    }
}
