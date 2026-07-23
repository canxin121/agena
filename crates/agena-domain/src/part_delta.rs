use serde::{Deserialize, Serialize};

/// Identifies the stable field targeted by a streamed message-part delta.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum PartDeltaField {
    Text,
    ReasoningSummary,
    ReasoningRawContent,
    CommandStdout,
    CommandStderr,
    ToolOutputText,
    Custom { name: String },
}

#[cfg(test)]
mod tests {
    use super::PartDeltaField;

    #[test]
    fn part_delta_fields_have_stable_tagged_wire_shapes() {
        assert_eq!(
            serde_json::to_string(&PartDeltaField::ReasoningSummary).unwrap(),
            r#"{"field":"reasoning_summary"}"#
        );
        assert_eq!(
            serde_json::to_string(&PartDeltaField::Custom { name: "x".into() }).unwrap(),
            r#"{"field":"custom","name":"x"}"#
        );
    }
}
