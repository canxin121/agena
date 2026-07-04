use super::*;

use agena_macros::{StaticToolSurface, ToolInputShape};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "tools",
    aliases("tool_catalog", "tool.help"),
    description = "Tool catalog command. Use action `usage` for examples, `list` to enumerate tools, `search` to find tools, or `help` to fetch detailed usage for a tool. This tool does not execute the target tool for you.",
    before_help = "Quick reference for browsing the registered tool catalog.",
    summary = "Show usage examples, list tools, search tools, or fetch detailed tool help.",
    help = "Use action `usage` or pass `{}` to see quick examples. Use action `list` to enumerate the current model-visible tools. Use action `search` with `query` and optional `limit` to discover tools. Use action `help` with `tool` to retrieve the full registered help text and input schema for any model-visible tool.",
    after_help = "To actually run a tool, call that tool directly after reading its help.",
    handler_receiver = WorkflowPlugin,
    trim("query", "tool"),
    ui_display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Discovery),
    host_capabilities(HostCapability::ListTools),
    concurrency_safe = true
)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ToolsToolInput {
    #[tool(
        exec = "usage",
        handle = WorkflowPlugin::invoke_tools_usage,
        default_when_empty = true
    )]
    Usage,
    #[tool(exec = "list", handle = WorkflowPlugin::invoke_tool_list)]
    List {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: ToolListInput,
    },
    #[tool(
        exec = "search",
        handle = WorkflowPlugin::invoke_tool_search,
        non_empty("query"),
        infer_when_present("query"),
        drop_keys("include_schema", "tool", "verbose")
    )]
    Search {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: ToolSearchToolInput,
    },
    #[tool(
        exec = "help",
        handle = WorkflowPlugin::invoke_tool_help,
        non_empty("tool"),
        infer_when_present("tool"),
        drop_keys("query", "limit")
    )]
    Help {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: ToolsHelpInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("tool"), non_empty("tool"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolsHelpInput {
    /// Registered model-visible tool name to inspect.
    pub tool: String,
    /// Include the sanitized JSON input schema in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_schema: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input()]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolListInput {
    /// Maximum number of tools to return. Omit to return every current model-visible tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Include one-line summaries next to each tool name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbose: Option<bool>,
}

#[cfg(test)]
pub(crate) fn resolve_tools_tool_input(
    input: serde_json::Value,
) -> SdkResult<(String, serde_json::Value)> {
    ToolsToolInput::resolve_tool("tools", input)
}
