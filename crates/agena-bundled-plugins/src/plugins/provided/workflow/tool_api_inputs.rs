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
    /// Exact current-session name of the Agena execution tool to run. Obtain
    /// it from `tools_list` or `tools_search`; never invent or reuse a name
    /// from another agent, product, version, or session. The Tool API function
    /// name remains `tools_call`.
    pub tool: String,
    /// One complete execution-tool argument object. Its keys are intentionally
    /// open because every live tool has a different schema; this openness is
    /// not permission to guess. Derive it from current-session `tools_help` or
    /// reusable embedded validation help, preserve every required key and task
    /// value, and never collapse a populated object to `{}`. If validation
    /// fails, read the embedded help and retry directly without another
    /// `tools_help` call.
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
        "description": "One complete argument object for the selected execution tool. Property names are intentionally open because each live tool has its own schema; this openness is not permission to guess. Derive the object from current-session tools_help or reusable embedded validation help, preserve every required key and task value, and never collapse a populated object to `{}`. The Tool API function name is always `tools_call`; the discovered execution-tool name belongs in `tool`. Never make an empty, default-input, or preliminary probe when fields are required. A validation error includes complete help for a direct corrected tools_call retry."
    })
    .try_into()
    .expect("valid schema")
}
use super::{Deserialize, JsonSchema, Serialize};
