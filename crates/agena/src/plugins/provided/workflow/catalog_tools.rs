use super::*;

use agena_macros::{StaticToolSurface, ToolInputShape, ToolSuite};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "list",
    summary = "Enumerate current tools.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_tool_list,
    handle_field = args,
    ui_display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Discovery),
    capabilities(HostCapability::ListTools, HostCapability::ToolRegistry),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogListToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: ToolListInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "search",
    summary = "Search the current tool catalog.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_tool_search,
    handle_field = args,
    trim("query"),
    non_empty("query"),
    ui_display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Discovery),
    capabilities(HostCapability::ListTools, HostCapability::ToolRegistry),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogSearchToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: CatalogSearchInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "help",
    summary = "Fetch detailed tool help.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_tool_help,
    handle_field = args,
    trim("tool"),
    non_empty("tool"),
    ui_display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Discovery),
    capabilities(HostCapability::ListTools),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogHelpToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: ToolsHelpInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "tags",
    summary = "List tool tags with pagination.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_tool_tags,
    handle_field = args,
    ui_display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Discovery),
    capabilities(HostCapability::ListTools, HostCapability::ToolRegistry),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogTagsToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: ToolTagsInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "call",
    summary = "Invoke a tool after reading its help.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_tool_call,
    handle_field = args,
    trim("tool"),
    non_empty("tool"),
    ui_display = detailed,
    tags(ToolTag::Discovery),
    capabilities(HostCapability::InvokeTool),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogCallToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: ToolCallInput,
}

#[allow(dead_code)]
#[derive(Debug, ToolSuite)]
#[tool_suite(handler_receiver = WorkflowPlugin)]
pub(crate) enum CatalogToolSuite {
    List(CatalogListToolInput),
    Search(CatalogSearchToolInput),
    Help(CatalogHelpToolInput),
    Tags(CatalogTagsToolInput),
    Call(CatalogCallToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("tool"), non_empty("tool"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolsHelpInput {
    /// Registered model-visible tool name to inspect.
    pub tool: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, ToolInputShape)]
#[tool_input(trim("tool"), non_empty("tool"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolCallInput {
    /// Registered model-visible tool name to invoke.
    pub tool: String,
    /// Tool input object passed through verbatim to the target tool.
    #[schemars(schema_with = "tool_call_input_schema")]
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input()]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("query"), non_empty("query"))]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input()]
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
        "description": "Arguments for the target tool. Read agena.tools/help for the exact schema before calling."
    })
    .try_into()
    .expect("valid schema")
}
