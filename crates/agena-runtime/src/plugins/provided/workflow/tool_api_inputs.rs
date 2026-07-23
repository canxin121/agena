use agena_macros::ToolInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(non_empty("tool"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolApiHelpInput {
    /// Exact name of the Agena execution tool to inspect, such as `fs.read`.
    /// Use a name returned by `tools_list` or `tools_search`.
    pub tool: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, ToolInput)]
#[input(non_empty("tool"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolApiCallInput {
    /// Exact name of the Agena execution tool to run, such as `fs.read`. The
    /// Tool API function name remains `tools_call`.
    pub tool: String,
    /// Complete execution-tool arguments passed through verbatim. Its keys are
    /// intentionally open: preserve every task/help key and value, and do not
    /// collapse a populated object to `{}`. If it does not match the tool's
    /// schema, the rejected result embeds complete help so the next
    /// call can retry directly without `tools_help`.
    #[schemars(schema_with = "tool_api_call_input_schema")]
    pub input: serde_json::Value,
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
pub(crate) struct ToolApiSearchInput {
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
pub(crate) struct ToolApiTagsInput {
    /// Number of tags to skip before returning results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    /// Maximum number of tags to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

fn tool_api_call_input_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    serde_json::json!({
        "type": "object",
        "additionalProperties": true,
        "description": "Complete arguments for the selected execution tool. Property names are intentionally open because each tool has its own schema. Preserve every key and value supplied by the task or returned by tools_help; do not collapse a populated object to `{}`. The Tool API function name is always `tools_call`; the execution-tool name belongs in `tool`. Never make an empty, default-input, or preliminary probe when the tool requires fields. A validation error includes the tool's complete help for a direct tools_call retry."
    })
    .try_into()
    .expect("valid schema")
}
use super::{Deserialize, JsonSchema, Serialize};
