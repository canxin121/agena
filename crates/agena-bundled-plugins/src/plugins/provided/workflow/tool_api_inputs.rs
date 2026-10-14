use agena_macros::ToolInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(untagged)]
pub(crate) enum ToolApiStringBatch {
    One(#[schemars(length(min = 1))] String),
    Many(#[schemars(length(min = 1))] Vec<String>),
}

impl ToolApiStringBatch {
    pub(crate) fn as_slice(&self) -> &[String] {
        match self {
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolApiHelpInput {
    /// One exact execution-tool name, or a non-empty array of exact names, to
    /// inspect. Use exact names known from the prompt/context or returned by
    /// `tools_list`; known names need no preliminary lookup.
    /// This help checks current availability and supplies the live contract.
    pub tool: ToolApiStringBatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolApiListInput {
    /// Number of tools to skip before returning results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    /// Maximum number of tools to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Optional plugin selector: one plugin id or a non-empty array of ids,
    /// with OR semantics. It scopes tools by owner for `tools_list` and selects
    /// plugin records directly for `plugins_list`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<ToolApiStringBatch>,
}

use super::{Deserialize, JsonSchema, Serialize};
