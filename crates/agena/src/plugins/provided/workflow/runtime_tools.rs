use super::*;

use agena_macros::{StaticToolSurface, ToolInputShape, ToolSuite};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "switch",
    summary = "Switch the current runtime agent profile.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_agent_switch,
    handle_field = args,
    display = brief,
    capabilities(HostCapability::AgentRegistry),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeSwitchToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: AgentSwitchToolInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "restore",
    summary = "Restore the previous runtime agent profile.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_agent_restore,
    handle_field = args,
    display = brief,
    capabilities(HostCapability::AgentRegistry),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeRestoreToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: AgentRestoreToolInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("title"), non_empty("title"))]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionRenameToolInput {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "get",
    summary = "Inspect the current session metadata.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_get_session,
    display = brief,
    tags(ToolTag::ReadOnly),
    capabilities(HostCapability::SessionRegistry),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeGetToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "rename",
    summary = "Rename the current session.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_rename_session,
    handle_field = args,
    display = brief,
    tags(ToolTag::Mutating),
    capabilities(HostCapability::SessionRegistry),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeRenameToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: SessionRenameToolInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "request_input",
    summary = "Request short structured input from the user.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_ask_user,
    handle_field = args,
    display = brief,
    tags(ToolTag::Interactive),
    capabilities(HostCapability::AskUser),
    concurrency_safe = false
)]
#[tool(
    min_items("questions", 1),
    max_items("questions", 3),
    max_items("questions[].options", 8),
    max_chars("questions[].header", 12),
    non_empty("questions[].id", "questions[].question"),
    non_empty_if_present("questions[].options[].label"),
    required_unless_present("questions[].allow_custom", "questions[].options"),
    distinct_trimmed("questions[].id"),
    distinct_trimmed_within("questions[].options[].label", "questions[]")
)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeRequestInputToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: AskUserToolInput,
}

#[allow(dead_code)]
#[derive(Debug, ToolSuite)]
#[tool_suite(handler_receiver = WorkflowPlugin)]
pub(crate) enum RuntimeToolSuite {
    Switch(RuntimeSwitchToolInput),
    Restore(RuntimeRestoreToolInput),
    Get(RuntimeGetToolInput),
    Rename(RuntimeRenameToolInput),
    RequestInput(RuntimeRequestInputToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SessionToolResponse {
    pub(crate) session: HostSession,
}
