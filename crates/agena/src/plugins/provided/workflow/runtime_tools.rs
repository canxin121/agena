use super::*;

use agena_macros::{StaticToolSurface, ToolInputShape};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "agent",
    description = "Runtime agent profile command. Use action `switch` to change the current session's active agent profile or `restore` to bring back a saved profile. This tool does not spawn delegated subagent work; use `task` for that.",
    summary = "Switch or restore the current runtime agent profile.",
    handler_receiver = WorkflowPlugin,
    display = brief,
    host_capabilities(HostCapability::AgentRegistry),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum AgentToolInput {
    #[tool(exec = "switch", handle = WorkflowPlugin::invoke_agent_switch)]
    Switch {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: AgentSwitchToolInput,
    },
    #[tool(exec = "restore", handle = WorkflowPlugin::invoke_agent_restore)]
    Restore {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: AgentRestoreToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("title"), non_empty("title"))]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionRenameToolInput {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "session",
    description = "Session metadata command. Use action `get` to inspect the current session metadata or `rename` to update the session title. This tool does not read chat history or execute workflow actions.",
    summary = "Inspect or rename the current session.",
    handler_receiver = WorkflowPlugin,
    display = brief,
    tags(ToolTag::ReadOnly, ToolTag::Mutating),
    host_capabilities(HostCapability::SessionRegistry),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum SessionToolInput {
    #[tool(exec = "get", handle = WorkflowPlugin::invoke_get_session)]
    Get,
    #[tool(exec = "rename", handle = WorkflowPlugin::invoke_rename_session)]
    Rename {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: SessionRenameToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "user",
    description = "User interaction command. Use action `request_input` to request structured short answers.",
    summary = "Request short structured input from the user.",
    handler_receiver = WorkflowPlugin,
    display = brief,
    tags(ToolTag::ReadOnly, ToolTag::Interactive),
    host_capabilities(HostCapability::AskUser),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum UserToolInput {
    #[tool(
        exec = "request_input",
        handle = WorkflowPlugin::invoke_ask_user,
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
    RequestInput {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: AskUserToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SessionToolResponse {
    pub(crate) session: HostSession,
}
