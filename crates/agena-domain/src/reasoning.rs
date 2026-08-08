use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};

/// The provider field that carries assistant reasoning state for replay.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, AsRefStr, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Field of the reasoning content an assistant produced.
pub enum AssistantReasoningField {
    ReasoningContent,
    ReasoningDetails,
}

#[cfg(test)]
mod tests {
    use super::AssistantReasoningField;

    #[test]
    fn reasoning_field_has_stable_wire_spelling() {
        assert_eq!(
            serde_json::to_string(&AssistantReasoningField::ReasoningDetails).unwrap(),
            "\"reasoning_details\""
        );
    }
}
