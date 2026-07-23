use serde::{Deserialize, Serialize};

/// Lifecycle state of a tool result envelope.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultState {
    #[default]
    Pending,
    Running,
    Completed,
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
}

impl ToolResultDisplay {
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.summary.is_empty()
    }
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
