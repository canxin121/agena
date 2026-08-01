use serde::{Deserialize, Serialize};

/// Lifecycle state of a tool result envelope.
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

/// Compact presentation metadata attached to a tool result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolResultDisplay {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// Explicit, named result sections. These are the canonical expanded
    /// presentation: unlike a rendered Markdown block they remain structured
    /// across persistence, API and UI boundaries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<ToolPresentationSection>,
}

impl ToolResultDisplay {
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.summary.is_empty() && self.sections.is_empty()
    }
}

/// One explicitly named user-facing section of a tool result.
///
/// The domain owns this value because the same tool result must retain its
/// presentation semantics in the executor, persisted transcript and every
/// client. The body is ordinary text/Markdown; rendering is deliberately left
/// to the consuming surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolPresentationSection {
    pub title: String,
    pub text: String,
}

impl ToolResultState {
    pub const fn is_pending(value: &Self) -> bool {
        matches!(value, Self::Pending)
    }
}

#[cfg(test)]
mod tests {
    use super::{ToolResultDisplay, ToolResultState};

    #[test]
    fn result_state_and_display_have_stable_defaults() {
        assert_eq!(ToolResultState::default(), ToolResultState::Pending);
        assert!(ToolResultDisplay::default().is_empty());
        assert_eq!(
            serde_json::to_string(&ToolResultState::Completed).unwrap(),
            "\"completed\""
        );
    }
}
