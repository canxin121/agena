use super::*;

use agena_macros::{StaticToolSurface, ToolSuite};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "run",
    summary = "Create or resume a delegated subagent task.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_task,
    handle_field = args,
    display = detailed,
    tags(ToolTag::Task, ToolTag::Subtask),
    capabilities(HostCapability::SpawnSubtask, HostCapability::PluginStorage),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskRunToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: TaskToolInput,
}

#[allow(dead_code)]
#[derive(Debug, ToolSuite)]
#[tool_suite(handler_receiver = WorkflowPlugin)]
pub(crate) enum TaskToolSuite {
    Run(TaskRunToolInput),
}
