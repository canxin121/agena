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
    /// inspect. Use names returned by `tools_list` or `tools_search`.
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
    /// Optional single tag filter such as `query` or `network`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Optional tag filters. When present, all normalized tags must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolApiSearchInput {
    /// One search query, or a non-empty array of queries, used to rank matching
    /// tool names and summaries. Batched queries are evaluated independently.
    pub query: ToolApiStringBatch,
    /// Number of matching tools to skip before returning results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    /// Maximum number of search results to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Optional plugin selector: one plugin id or a non-empty array of ids,
    /// with OR semantics. It scopes tools by owner for `tools_search` and
    /// plugin records directly for `plugins_search`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<ToolApiStringBatch>,
    /// Optional single tag filter such as `query` or `network`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Optional tag filters. When present, all normalized tags must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolApiTagsInput {
    /// Number of tags to skip before returning results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    /// Maximum number of tags to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Optional plugin selector: one plugin id or a non-empty array of ids.
    /// Only tags belonging to tools or plugins from any selected plugin are
    /// counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<ToolApiStringBatch>,
}

use super::{Deserialize, JsonSchema, Serialize};
