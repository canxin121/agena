use agena_macros::ToolInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(non_empty("tool"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolsHelpInput {
    /// Exact dotted catalog target to inspect, such as `fs.read`. This value is
    /// payload data and must never be used as a provider function name.
    pub tool: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, ToolInput)]
#[input(non_empty("tool"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolCallInput {
    /// Exact dotted catalog target to execute, such as `fs.read`. The provider
    /// function name remains `tools_call`; never call this target directly.
    pub tool: String,
    /// Complete target-specific input object passed through verbatim. If it
    /// does not match the live schema, the rejected result embeds complete
    /// target help so the next call can retry directly without `tools_help`.
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
        "description": "Complete arguments for the dotted catalog target. The provider function name is always `tools_call`, never the target name. `tools_help` is optional reusable schema discovery, not a consumable authorization. Copy every task-supplied value into one complete call; never make an empty, default-input, or preliminary probe call. A schema mismatch returns complete embedded target help for a direct `tools_call` retry, so do not make a separate help call after that error."
    })
    .try_into()
    .expect("valid schema")
}
use super::{Deserialize, JsonSchema, Serialize};
