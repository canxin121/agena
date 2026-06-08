use super::*;

use agena_macros::StaticToolSurface;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "task",
    description = "Delegated subagent task command. Use action `run` to create or resume a typed child task session for explore, implement, or verify work. This tool launches or resumes a separate task session; it does not switch the current runtime agent profile.",
    summary = "Create or resume a delegated subagent task.",
    handler_receiver = WorkflowPlugin,
    display = detailed,
    tags(ToolTag::Task, ToolTag::Subtask),
    host_capabilities(HostCapability::SpawnSubtask, HostCapability::PluginStorage),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum TaskToolActionInput {
    #[tool(exec = "run", handle = WorkflowPlugin::invoke_task)]
    Run {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: TaskToolInput,
    },
}
