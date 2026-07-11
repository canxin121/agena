use agena_macros::ToolInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("tool"), non_empty("tool"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolsHelpInput {
    /// Registered gateway-visible tool name to inspect.
    pub tool: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, ToolInput)]
#[input(trim("tool"), non_empty("tool"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolCallInput {
    /// Registered gateway-visible tool name to invoke.
    pub tool: String,
    /// Tool input object passed through verbatim to the target tool.
    #[schemars(schema_with = "tool_call_input_schema")]
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolListInput {
    /// Number of tools to skip before returning results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    /// Maximum number of tools to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Optional single tag filter such as `read_only` or `network`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Optional tag filters. When present, all normalized tags must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("query"), non_empty("query"))]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogSearchInput {
    /// Search text used to rank matching tool names and summaries.
    #[serde(default)]
    pub query: String,
    /// Number of matching tools to skip before returning results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    /// Maximum number of search results to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Optional single tag filter such as `read_only` or `network`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Optional tag filters. When present, all normalized tags must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolTagsInput {
    /// Number of tags to skip before returning results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    /// Maximum number of tags to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

fn tool_call_input_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    serde_json::json!({
        "type": "object",
        "description": "Arguments for the target tool. Every `tools_call` requires a dedicated `tools_help` preflight for the same target; that preflight is consumed by the call. The `tool` value must be a catalog target such as `web.search`, not a gateway function name such as `tools_help` or `tools_call`."
    })
    .try_into()
    .expect("valid schema")
}
use super::{Deserialize, JsonSchema, Serialize};
