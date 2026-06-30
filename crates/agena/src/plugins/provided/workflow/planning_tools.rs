use super::*;

use agena_macros::{StaticToolSurface, ToolInputShape};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) view: Option<PlanGetView>,
}

#[derive(
    Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape, Default,
)]
#[tool_input(trim("summary", "step_id", "check_id", "note"))]
#[serde(default, deny_unknown_fields)]
#[schemars(
    description = "Update the current plan. Use `phase` / `autorun` for plan-level state changes, `step_id` + `status` to update a step, or `step_id` + `check_id` + `status` to update a check. Canonical phase values are `planning`, `active`, `blocked`, `completed`, and `cancelled`."
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
    tool = "plan",
    description = "Plan command backed by shared plugin storage. Use it to set the current plan, inspect its current state, or update plan, step, and check status.",
    summary = "Set, get, update, or clear the current plan.",
    handler_receiver = WorkflowPlugin,
    help = "Use action `set` to create or replace the current plan and return it to planning. Use action `get` to inspect the current plan with `view = current|summary|full`. Use action `update` to change the plan phase / autorun flag, a step's status, or a check's status. Use action `clear` to remove the current plan. If workflow plan config disables direct approval, `plan.update` automatically requests review before moving a planning or cancelled plan into active, blocked, or completed.",
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
    #[tool(exec = "get", handle = WorkflowPlugin::invoke_plan_get, default_when_empty = true)]
    Get {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: PlanGetInput,
    },
    #[tool(exec = "set", handle = WorkflowPlugin::invoke_plan_set)]
    Set {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: PlanSetInput,
    },
    #[tool(exec = "update", handle = WorkflowPlugin::invoke_plan_update)]
    Update {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: PlanUpdateInput,
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
