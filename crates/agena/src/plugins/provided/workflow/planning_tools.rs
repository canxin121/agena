use super::*;

use agena_macros::{StaticToolSurface, ToolInputShape};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkflowPlanPhase {
    #[default]
    Draft,
    #[serde(rename = "active")]
    Active,
    Blocked,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkflowPlanExecutor {
    #[default]
    Ai,
    Human,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkflowPlanStepStatus {
    #[default]
    Pending,
    InProgress,
    Blocked,
    Completed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WorkflowPlanCheckpoint {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) status: WorkflowPlanStepStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WorkflowPlanStep {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) executor: WorkflowPlanExecutor,
    pub(crate) status: WorkflowPlanStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) wait_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) note: String,
    #[serde(default, rename = "checks", skip_serializing_if = "Vec::is_empty")]
    pub(crate) checkpoints: Vec<WorkflowPlanCheckpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WorkflowPlan {
    pub(crate) title: String,
    pub(crate) objective: String,
    pub(crate) phase: WorkflowPlanPhase,
    pub(crate) autorun: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) document_markdown: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) steps: Vec<WorkflowPlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
#[schemars(description = "Plan check input. Each check item should use `text`.")]
pub(crate) struct WorkflowPlanCheckpointInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,
    #[schemars(description = "Check text.")]
    pub(crate) text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<WorkflowPlanStepStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
#[schemars(
    description = "Plan step input. Each step uses `title`; nested checks under `checks` use `text`."
)]
pub(crate) struct WorkflowPlanStepInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,
    #[schemars(description = "Human-readable step title.")]
    pub(crate) title: String,
    #[schemars(
        description = "Optional longer explanation for the step. If omitted, the step title can serve as the short description."
    )]
    #[serde(default)]
    pub(crate) description: String,
    #[schemars(
        description = "Who should execute the step. Use `ai` for agent work and `human` for manual work."
    )]
    #[serde(default)]
    pub(crate) executor: WorkflowPlanExecutor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<WorkflowPlanStepStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) wait_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) note: Option<String>,
    #[schemars(
        description = "Optional checklist checks for this step. Each check item uses `text`, not `title`."
    )]
    #[serde(default, rename = "checks", skip_serializing_if = "Vec::is_empty")]
    pub(crate) checkpoints: Vec<WorkflowPlanCheckpointInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[serde(deny_unknown_fields)]
#[schemars(
    description = "Create or overwrite the current active-session plan in draft. If a plan already exists, this replaces it. Use `steps[].title` for steps, `steps[].checks[].text` for checks, and `autorun` to control whether approved active plans should keep running automatically."
)]
pub(crate) struct PlanCreateInput {
    pub(crate) objective: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) document_markdown: Option<String>,
    #[schemars(
        description = "Ordered plan steps. Each step item uses `title`; nested checks use `text`."
    )]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) steps: Vec<WorkflowPlanStepInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) autorun: Option<bool>,
}

#[derive(
    Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape, Default,
)]
#[tool_input(trim("summary"), at_least_one_of("phase", "autorun"))]
#[serde(default, deny_unknown_fields)]
#[schemars(
    description = "Set the plan phase or autorun flag. Canonical phase values are `draft`, `active`, `blocked`, `completed`, and `cancelled`."
)]
pub(crate) struct PlanSetStatusInput {
    #[schemars(
        description = "Canonical plan phase. Use `draft`, `active`, `blocked`, `completed`, or `cancelled`."
    )]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) phase: Option<WorkflowPlanPhase>,
    #[schemars(description = "Whether an approved active plan should keep running automatically.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) autorun: Option<bool>,
    #[schemars(
        description = "Optional completion summary. This is only applied when `phase` is `completed`."
    )]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("step_id", "note"), non_empty("step_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanUpdateStepInput {
    pub(crate) step_id: String,
    pub(crate) status: WorkflowPlanStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) wait_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("step_id", "check_id"), non_empty("step_id", "check_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanUpdateCheckpointInput {
    pub(crate) step_id: String,
    #[serde(rename = "check_id")]
    pub(crate) checkpoint_id: String,
    pub(crate) status: WorkflowPlanStepStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "todo",
    description = "Todo command. Use action `write` to replace the session todo list.",
    summary = "Replace the session todo list.",
    handler_receiver = WorkflowPlugin,
    display = brief,
    tags(ToolTag::Mutating, ToolTag::Planning),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum TodoToolInput {
    #[tool(exec = "write", handle = WorkflowPlugin::invoke_todo_write)]
    Write {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: TodoWriteToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "plan",
    description = "Plan command backed by shared plugin storage. Use it to create or overwrite the current draft plan, inspect the current step, and manage the active session plan.",
    summary = "Create or overwrite plans, inspect the current step, update steps/checks, and use `set_status` for phase changes.",
    handler_receiver = WorkflowPlugin,
    help = "Use action `create` to write the current draft plan; if a plan already exists, `create` overwrites it and returns it to draft. Use action `current` to inspect the current actionable step and its goal. Use action `set_status` to move the plan between draft, active, blocked, completed, or cancelled. Use action `update_check` to update an individual check inside a step. Autorun on/off distinguishes active plans that should keep running automatically. If workflow plan config disables direct approval, plan.set_status automatically requests review before moving a draft or cancelled plan into active, blocked, or completed.",
    display = brief,
    tags(ToolTag::Planning, ToolTag::Mutating),
    host_capabilities(
        HostCapability::AskUser,
        HostCapability::PluginStorage,
        HostCapability::Statusline
    ),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum PlanToolInput {
    #[tool(exec = "current", handle = WorkflowPlugin::invoke_plan_current)]
    Current,
    #[tool(exec = "create", handle = WorkflowPlugin::invoke_plan_create)]
    Create {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: PlanCreateInput,
    },
    #[tool(
        exec = "set_status",
        handle = WorkflowPlugin::invoke_plan_set_status,
        at_least_one_of("phase", "autorun")
    )]
    SetStatus {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: PlanSetStatusInput,
    },
    #[tool(exec = "update_step", handle = WorkflowPlugin::invoke_plan_update_step)]
    UpdateStep {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: PlanUpdateStepInput,
    },
    #[tool(
        exec = "update_check",
        handle = WorkflowPlugin::invoke_plan_update_checkpoint
    )]
    UpdateCheck {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: PlanUpdateCheckpointInput,
    },
    #[tool(exec = "clear", handle = WorkflowPlugin::invoke_plan_clear)]
    Clear,
}

#[cfg(test)]
pub(crate) fn resolve_plan_tool_input(
    input: serde_json::Value,
) -> SdkResult<(String, serde_json::Value)> {
    PlanToolInput::resolve_tool("plan", input)
}
