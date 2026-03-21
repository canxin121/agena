use serde::{Deserialize, Serialize};

/// Structured custom payload for dynamic tools.
///
/// NOTE: This is only used by `ToolInput::Custom` / `ToolMetadata::Custom`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct StructuredObject {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<StructuredField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredField {
    pub name: String,
    pub value: StructuredValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StructuredValue {
    Null,
    Boolean {
        value: bool,
    },
    Integer {
        value: i64,
    },
    /// Decimal/scientific notation text to avoid precision loss.
    Number {
        value: String,
    },
    Text {
        value: String,
    },
    Array {
        items: Vec<StructuredValue>,
    },
    Object {
        fields: Vec<StructuredField>,
    },
}
