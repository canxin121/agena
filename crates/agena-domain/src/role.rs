use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};

/// Semantic author role of a conversation message.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, AsRefStr, Display, EnumString,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
/// Role of a message: user, assistant, system, or tool.
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

#[cfg(test)]
mod tests {
    use super::Role;

    #[test]
    fn role_has_stable_lowercase_json() {
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(
            serde_json::from_str::<Role>("\"tool\"").unwrap(),
            Role::Tool
        );
    }
}
