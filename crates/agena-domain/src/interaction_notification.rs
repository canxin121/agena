use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Visual severity for a non-interactive transcript notification.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteractionNotificationLevel {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

impl InteractionNotificationLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InteractionNotificationLevel;

    #[test]
    fn notification_level_uses_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&InteractionNotificationLevel::Warning).unwrap(),
            "\"warning\""
        );
        assert_eq!(InteractionNotificationLevel::Error.as_str(), "error");
    }
}
