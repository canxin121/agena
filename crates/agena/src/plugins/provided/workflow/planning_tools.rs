use super::*;

use agena_macros::{StaticToolSurface, ToolInputShape, ToolSuite};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkflowPlanPhase {
    #[default]
    Planning,
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
    description = "Create or overwrite the current active-session plan in planning. If a plan already exists, this replaces it and resets the phase to planning. Use `steps[].title` for steps, `steps[].checks[].text` for checks, and `autorun` to control whether approved active plans should keep running automatically."
)]
pub(crate) struct PlanSetInput {
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
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape, Default,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlanGetView {
    #[default]
    Current,
    Summary,
    Full,
}

#[derive(
    Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape, Default,
)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct PlanGetInput {
    #[serde(default)]
    pub(crate) view: PlanGetView,
}

#[derive(
    Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape, Default,
)]
#[tool_input(trim("summary", "step_id", "check_id", "note"))]
#[serde(default, deny_unknown_fields)]
#[schemars(
    description = "Update the current plan. Use `phase` / `autorun` for plan-level state changes, `step_id` + `status` to update a step, or `step_id` + `check_id` + `status` to update a check. Do not combine plan-level fields (`phase`, `autorun`, `summary`) with step/check fields. To complete a plan with steps, first mark the relevant steps or checks `completed`, then make a separate plan-level update with `phase: completed`. Canonical phase values are `planning`, `active`, `blocked`, `completed`, and `cancelled`."
)]
pub(crate) struct PlanUpdateInput {
    #[schemars(
        description = "Canonical plan phase. Use `planning`, `active`, `blocked`, `completed`, or `cancelled`."
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) step_id: Option<String>,
    #[serde(default, rename = "check_id", skip_serializing_if = "Option::is_none")]
    pub(crate) checkpoint_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<WorkflowPlanStepStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) wait_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "get",
    summary = "Inspect the current plan state.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_plan_get,
    handle_field = args,
    display = brief,
    tags(ToolTag::Planning, ToolTag::ReadOnly),
    capabilities(
        HostCapability::AskUser,
        HostCapability::PluginStorage,
        HostCapability::Statusline
    ),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanGetToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: PlanGetInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "set",
    summary = "Create or replace the current plan.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_plan_set,
    handle_field = args,
    display = brief,
    tags(ToolTag::Planning, ToolTag::Mutating),
    capabilities(
        HostCapability::AskUser,
        HostCapability::PluginStorage,
        HostCapability::Statusline
    ),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanSetToolInputSurface {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: PlanSetInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "update",
    summary = "Update the current plan.",
    help = "Keep plan-level updates separate from step/check updates: do not send `phase` together with `step_id`, `check_id`, `status`, `wait_until_ms`, or `note`. To complete a plan with steps, mark the required steps/checks `completed` first, then call update separately with `phase: completed`.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_plan_update,
    handle_field = args,
    display = brief,
    tags(ToolTag::Planning, ToolTag::Mutating),
    capabilities(
        HostCapability::AskUser,
        HostCapability::PluginStorage,
        HostCapability::Statusline
    ),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanUpdateToolInputSurface {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: PlanUpdateInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "clear",
    summary = "Remove the current plan.",
    handler_receiver = WorkflowPlugin,
    handle = WorkflowPlugin::invoke_plan_clear,
    display = brief,
    tags(ToolTag::Planning, ToolTag::Mutating),
    capabilities(
        HostCapability::AskUser,
        HostCapability::PluginStorage,
        HostCapability::Statusline
    ),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanClearToolInput {}

#[allow(dead_code)]
#[derive(Debug, ToolSuite)]
#[tool_suite(handler_receiver = WorkflowPlugin)]
pub(crate) enum PlanToolSuite {
    Get(PlanGetToolInput),
    Set(PlanSetToolInputSurface),
    Update(PlanUpdateToolInputSurface),
    Clear(PlanClearToolInput),
}
