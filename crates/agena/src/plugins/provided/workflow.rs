//! `agena.workflow` plugin: orchestration tools (task, tool catalog, todo,
//! session, user input, plan, worktree, and workflow prompts).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::message::{
    AgentRestoreToolInput, AgentSwitchToolInput, AskUserToolInput, EnterWorktreeToolInput,
    ExitWorktreeToolInput, TaskToolInput, TodoItem, TodoPriority, TodoStatus, TodoWriteToolInput,
    ToolSearchToolInput, WorkflowPromptToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{
    AskUserOption as HostAskUserOption, AskUserQuestion as HostAskUserQuestion, AskUserRequest,
    HostAgentRestoreRequest, HostAgentRestoreResponse, HostAgentSwitchRequest,
    HostAgentSwitchResponse, HostClient, HostEnterWorktreeRequest, HostExitWorktreeRequest,
    HostGetSessionRequest, HostRenameSessionRequest, HostSession, HostStatuslineContributeRequest,
    HostStatuslineRemoveRequest, HostStorageDeleteRequest, HostStorageGetRequest, HostStorageScope,
    HostStorageSetRequest, HostStorageVisibility, HostTodoItem, HostTodoPriority, HostTodoStatus,
    HostTodoWriteRequest, SpawnSubtaskRequest, ToolDescriptor,
};
use crate::plugin::sdk::{
    CommandBeforeInput, CommandBeforeResponse, HookSubscription, HostCapability, InitContext,
    InitOutcome, NetworkRequest, PathRequest, Plugin, PluginManifest, PluginToolDecl,
    Result as SdkResult, ToolBeforeInput, ToolBeforePatch, ToolInvokeInput, ToolInvokeOutput,
    ToolTag,
};
use crate::search::tool_catalog::{ToolCatalogDocument, search_tool_catalog};
use crate::tool::{ToolExecutionView, ToolPayloadExecution, ToolPayloadOutput, ask_user};
use agena_macros::StaticToolSurface;
use async_trait::async_trait;
use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const WORKFLOW_PLUGIN_ID: &str = "agena.workflow";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct WorkflowPluginConfig {
    tool_catalog: WorkflowToolCatalogConfig,
    plan: WorkflowPlanConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct WorkflowToolCatalogConfig {
    search: WorkflowToolCatalogSearchConfig,
    help: WorkflowToolCatalogHelpConfig,
}

impl Default for WorkflowToolCatalogConfig {
    fn default() -> Self {
        Self {
            search: WorkflowToolCatalogSearchConfig::default(),
            help: WorkflowToolCatalogHelpConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct WorkflowToolCatalogSearchConfig {
    default_limit: u32,
    max_limit: u32,
}

impl Default for WorkflowToolCatalogSearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 8,
            max_limit: 25,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct WorkflowToolCatalogHelpConfig {
    include_schema_by_default: bool,
}

impl Default for WorkflowToolCatalogHelpConfig {
    fn default() -> Self {
        Self {
            include_schema_by_default: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct WorkflowPlanConfig {
    default_auto_continue: bool,
}

impl Default for WorkflowPlanConfig {
    fn default() -> Self {
        Self {
            default_auto_continue: true,
        }
    }
}

fn workflow_config_schema() -> serde_json::Value {
    let mut schema =
        crate::tool::definition::json_schema_for_with_default(WorkflowPluginConfig::default());
    for (pointer, title, description) in [
        (
            "",
            "Workflow Plugin Config",
            "Defaults for workflow catalog search and tool help rendering.",
        ),
        (
            "/properties/tool_catalog",
            "Tool Catalog",
            "Controls how the workflow plugin previews and searches the registered tool catalog.",
        ),
        (
            "/properties/tool_catalog/properties/search",
            "Search",
            "Default search behavior for agena.workflow/tools with action=search.",
        ),
        (
            "/properties/tool_catalog/properties/search/properties/default_limit",
            "Default Limit",
            "Number of tool search results returned when the caller omits limit.",
        ),
        (
            "/properties/tool_catalog/properties/search/properties/max_limit",
            "Max Limit",
            "Upper bound enforced for tool catalog search results.",
        ),
        (
            "/properties/tool_catalog/properties/help",
            "Help",
            "Defaults for agena.workflow/tools with action=help.",
        ),
        (
            "/properties/tool_catalog/properties/help/properties/include_schema_by_default",
            "Include Schema by Default",
            "When enabled, tool help includes the registered input schema unless the caller opts out.",
        ),
        (
            "/properties/plan",
            "Plan",
            "Defaults for the workflow plugin's shared-storage plan state machine.",
        ),
        (
            "/properties/plan/properties/default_auto_continue",
            "Default Auto Continue",
            "Default auto-continue value applied when plan.create omits the override.",
        ),
    ] {
        crate::tool::definition::set_schema_metadata(
            &mut schema,
            pointer,
            Some(title),
            Some(description),
        );
    }
    schema
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "workflow",
    description = "Workflow scaffold command. Use action `init`, `review`, or `security_review` to generate reusable workflow instructions; this tool does not execute shell or filesystem actions by itself.",
    tags(ToolTag::ReadOnly),
    host_capabilities(HostCapability::AgentRegistry),
    concurrency_safe = true
)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum WorkflowToolInput {
    #[tool(exec = "init")]
    Init {
        #[serde(flatten)]
        args: WorkflowPromptToolInput,
    },
    #[tool(exec = "review")]
    Review {
        #[serde(flatten)]
        args: WorkflowPromptToolInput,
    },
    #[tool(exec = "security_review")]
    SecurityReview {
        #[serde(flatten)]
        args: WorkflowPromptToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "tools",
    description = "Tool catalog command. Use action `search` to find tools or `help` to fetch detailed usage for a tool. This tool does not execute the target tool for you.",
    summary = "Search tools or fetch detailed tool help.",
    help = "Use action `search` with `query` and optional `limit` to discover tools. Use action `help` with `tool` to retrieve the full registered help text and input schema for any model-visible tool. To actually run a tool, call that tool directly after reading its help.",
    tags(ToolTag::ReadOnly, ToolTag::Discovery),
    host_capabilities(HostCapability::ListTools),
    concurrency_safe = true
)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum ToolsToolInput {
    #[tool(exec = "search")]
    Search {
        #[serde(flatten)]
        args: ToolSearchToolInput,
    },
    #[tool(exec = "help")]
    Help {
        #[serde(flatten)]
        args: ToolsHelpInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ToolsHelpInput {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_schema: Option<bool>,
}

fn resolve_tools_tool_input(input: serde_json::Value) -> SdkResult<(String, serde_json::Value)> {
    if input.as_object().is_some_and(serde_json::Map::is_empty) {
        return Ok((
            "usage".to_string(),
            serde_json::Value::Object(serde_json::Map::new()),
        ));
    }

    match ToolsToolInput::resolve_tool("tools", input.clone()) {
        Ok(resolved) => Ok(resolved),
        Err(primary) => match normalize_tools_tool_input(&input) {
            Some(normalized) if normalized != input => {
                ToolsToolInput::resolve_tool("tools", normalized)
            }
            _ => Err(primary),
        },
    }
}

fn normalize_tools_tool_input(input: &serde_json::Value) -> Option<serde_json::Value> {
    let object = input.as_object()?;
    let action = object
        .get("action")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            if object.contains_key("query") {
                Some("search".to_string())
            } else if object.contains_key("tool")
                || object.contains_key("name")
                || object.contains_key("tool_name")
            {
                Some("help".to_string())
            } else {
                None
            }
        })?;

    match action.as_str() {
        "search" => normalize_tools_search_input(object),
        "help" => normalize_tools_help_input(object),
        _ => None,
    }
}

fn normalize_tools_search_input(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "action" | "query" | "limit" | "include_schema" | "tool"
        )
    }) {
        return None;
    }

    let mut normalized = serde_json::Map::new();
    normalized.insert(
        "action".to_string(),
        serde_json::Value::String("search".to_string()),
    );
    if let Some(query) = object.get("query") {
        normalized.insert("query".to_string(), query.clone());
    }
    if let Some(limit) = object.get("limit") {
        normalized.insert("limit".to_string(), limit.clone());
    }

    Some(serde_json::Value::Object(normalized))
}

fn normalize_tools_help_input(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "action" | "tool" | "include_schema" | "query" | "limit" | "name" | "tool_name"
        )
    }) {
        return None;
    }

    let tool = object
        .get("tool")
        .or_else(|| object.get("name"))
        .or_else(|| object.get("tool_name"))?;

    let mut normalized = serde_json::Map::new();
    normalized.insert(
        "action".to_string(),
        serde_json::Value::String("help".to_string()),
    );
    normalized.insert("tool".to_string(), tool.clone());
    if let Some(include_schema) = object.get("include_schema") {
        normalized.insert("include_schema".to_string(), include_schema.clone());
    }

    Some(serde_json::Value::Object(normalized))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "agent",
    description = "Runtime agent profile command. Use action `switch` to change the current session's active agent profile or `restore` to bring back a saved profile. This tool does not spawn delegated subagent work; use `task` for that.",
    host_capabilities(HostCapability::AgentRegistry),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum AgentToolInput {
    #[tool(exec = "switch")]
    Switch {
        #[serde(flatten)]
        args: AgentSwitchToolInput,
    },
    #[tool(exec = "restore")]
    Restore {
        #[serde(flatten)]
        args: AgentRestoreToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "todo",
    description = "Todo command. Use action `write` to replace the session todo list.",
    tags(ToolTag::Mutating, ToolTag::Planning),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum TodoToolInput {
    #[tool(exec = "write")]
    Write {
        #[serde(flatten)]
        args: TodoWriteToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "task",
    description = "Delegated subagent task command. Use action `run` to create or resume a typed child task session for explore, implement, or verify work. This tool launches or resumes a separate task session; it does not switch the current runtime agent profile.",
    tags(ToolTag::Task, ToolTag::Subtask),
    host_capabilities(HostCapability::SpawnSubtask, HostCapability::PluginStorage),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum TaskToolActionInput {
    #[tool(exec = "run")]
    Run {
        #[serde(flatten)]
        args: TaskToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
struct SessionRenameToolInput {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "session",
    description = "Session metadata command. Use action `get` to inspect the current session metadata or `rename` to update the session title. This tool does not read chat history or execute workflow actions.",
    tags(ToolTag::ReadOnly, ToolTag::Mutating),
    host_capabilities(HostCapability::SessionRegistry),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum SessionToolInput {
    #[tool(exec = "get")]
    Get,
    #[tool(exec = "rename")]
    Rename {
        #[serde(flatten)]
        args: SessionRenameToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "user",
    description = "User interaction command. Use action `request_input` to request structured short answers.",
    tags(ToolTag::ReadOnly, ToolTag::Interactive),
    host_capabilities(HostCapability::AskUser),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum UserToolInput {
    #[tool(exec = "request_input")]
    RequestInput(AskUserToolInput),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
enum WorkflowPlanPhase {
    #[default]
    #[serde(alias = "awaiting_review")]
    Draft,
    #[serde(
        rename = "active",
        alias = "executing",
        alias = "paused",
        alias = "execution"
    )]
    Active,
    Blocked,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
enum WorkflowPlanApprovalState {
    #[default]
    NotRequested,
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
enum WorkflowPlanExecutor {
    #[default]
    Ai,
    Human,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
enum WorkflowPlanStepStatus {
    #[default]
    Pending,
    InProgress,
    Blocked,
    Completed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct WorkflowPlanAutoContinue {
    enabled: bool,
    source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct WorkflowPlanCheckpoint {
    id: String,
    text: String,
    status: WorkflowPlanStepStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct WorkflowPlanStep {
    id: String,
    title: String,
    description: String,
    executor: WorkflowPlanExecutor,
    status: WorkflowPlanStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wait_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    note: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    checkpoints: Vec<WorkflowPlanCheckpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct WorkflowPlan {
    schema_version: u32,
    plan_id: String,
    session_id: i64,
    title: String,
    objective: String,
    phase: WorkflowPlanPhase,
    approval_state: WorkflowPlanApprovalState,
    auto_continue: WorkflowPlanAutoContinue,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    document_markdown: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    steps: Vec<WorkflowPlanStep>,
    created_at_ms: i64,
    updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    submitted_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
#[schemars(
    description = "Plan checkpoint input. Each checkpoint item should use `text`; `title` and `description` are accepted only as compatibility aliases."
)]
struct WorkflowPlanCheckpointInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[schemars(
        description = "Checkpoint text. Models should send `text`; `title` and `description` are accepted only for compatibility."
    )]
    #[serde(alias = "title", alias = "description")]
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<WorkflowPlanStepStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
#[schemars(
    description = "Plan step input. Each step uses `title`; nested checkpoints under `checkpoints` use `text`."
)]
struct WorkflowPlanStepInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[schemars(
        description = "Human-readable step title. Models should send `title`; legacy `text` is accepted only for compatibility."
    )]
    #[serde(alias = "text")]
    title: String,
    #[schemars(
        description = "Optional longer explanation for the step. If omitted, the step title can serve as the short description."
    )]
    #[serde(default)]
    description: String,
    #[schemars(
        description = "Who should execute the step. Use `ai` for agent work and `human` for manual work."
    )]
    #[serde(default)]
    executor: WorkflowPlanExecutor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<WorkflowPlanStepStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wait_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[schemars(
        description = "Optional checklist checkpoints for this step. Each checkpoint item uses `text`, not `title`."
    )]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    checkpoints: Vec<WorkflowPlanCheckpointInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    description = "Create a new active-session plan. Use `steps[].title` for steps, `steps[].checkpoints[].text` for checkpoints, and `auto_continue` to control whether approved active plans should continue automatically."
)]
struct PlanCreateInput {
    objective: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    document_markdown: Option<String>,
    #[schemars(
        description = "Ordered plan steps. Each step item uses `title`; nested checkpoints use `text`."
    )]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    steps: Vec<WorkflowPlanStepInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_continue: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    description = "Replace the current active-session plan. Use the same structure as `create`: `steps[].title` for steps and `steps[].checkpoints[].text` for checkpoints."
)]
struct PlanReplaceInput {
    objective: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    document_markdown: Option<String>,
    #[schemars(
        description = "Ordered replacement plan steps. Each step item uses `title`; nested checkpoints use `text`."
    )]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    steps: Vec<WorkflowPlanStepInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_continue: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct PlanSubmitInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
#[schemars(
    description = "Set the plan phase or auto-continue flag. Canonical phase values are `draft`, `active`, `blocked`, `completed`, and `cancelled`."
)]
struct PlanSetStatusInput {
    #[schemars(
        description = "Canonical plan phase. Use `draft`, `active`, `blocked`, `completed`, or `cancelled`."
    )]
    #[serde(default, alias = "status", skip_serializing_if = "Option::is_none")]
    phase: Option<WorkflowPlanPhase>,
    #[schemars(description = "Whether an approved active plan should continue automatically.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_continue: Option<bool>,
    #[schemars(
        description = "Optional completion summary. This is only applied when `phase` is `completed`."
    )]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlanUpdateStepInput {
    step_id: String,
    status: WorkflowPlanStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wait_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlanUpdateCheckpointInput {
    step_id: String,
    checkpoint_id: String,
    status: WorkflowPlanStepStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "plan",
    description = "Plan command backed by shared plugin storage. Use it to create, replace, review, and manage the active session plan.",
    summary = "Create plans, update steps, and prefer action `set_status` for plan phase changes.",
    help = "Use action `set_status` to move the plan between draft, active, blocked, completed, or cancelled. Review-pending plans are represented as draft with approval_state=pending, and auto-continue on/off distinguishes active plans that should continue automatically. Legacy actions and legacy phase names such as `complete`, `cancel`, `restore`, `update_runtime`, `awaiting_review`, `executing`, and `paused` remain accepted for compatibility and are normalized to `set_status`.",
    tags(ToolTag::Planning, ToolTag::Mutating),
    host_capabilities(
        HostCapability::AskUser,
        HostCapability::PluginStorage,
        HostCapability::Statusline
    ),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum PlanToolInput {
    #[tool(exec = "get")]
    Get,
    #[tool(exec = "create")]
    Create {
        #[serde(flatten)]
        args: PlanCreateInput,
    },
    #[tool(exec = "replace")]
    Replace {
        #[serde(flatten)]
        args: PlanReplaceInput,
    },
    #[tool(exec = "submit")]
    Submit {
        #[serde(flatten)]
        args: PlanSubmitInput,
    },
    #[tool(exec = "set_status")]
    SetStatus {
        #[serde(flatten)]
        args: PlanSetStatusInput,
    },
    #[tool(exec = "update_step")]
    UpdateStep {
        #[serde(flatten)]
        args: PlanUpdateStepInput,
    },
    #[tool(exec = "update_checkpoint")]
    UpdateCheckpoint {
        #[serde(flatten)]
        args: PlanUpdateCheckpointInput,
    },
    #[tool(exec = "clear")]
    Clear,
    #[tool(exec = "next")]
    Next,
}

fn resolve_plan_tool_input(input: serde_json::Value) -> SdkResult<(String, serde_json::Value)> {
    match PlanToolInput::resolve_tool("plan", input.clone()) {
        Ok(resolved) => Ok(resolved),
        Err(primary) => match normalize_plan_tool_input(&input) {
            Some(normalized) if normalized != input => {
                PlanToolInput::resolve_tool("plan", normalized)
            }
            _ => Err(primary),
        },
    }
}

fn normalize_plan_tool_input(input: &serde_json::Value) -> Option<serde_json::Value> {
    let object = input.as_object()?;
    let action = object.get("action").and_then(serde_json::Value::as_str)?;

    match action {
        "set_status" => Some(plan_set_status_value(object, None)),
        "update_runtime" => Some(plan_set_status_value(object, None)),
        "complete" => Some(plan_set_status_value(
            object,
            Some(WorkflowPlanPhase::Completed),
        )),
        "cancel" => Some(plan_set_status_value(
            object,
            Some(WorkflowPlanPhase::Cancelled),
        )),
        "restore" => Some(plan_set_status_value(
            object,
            Some(
                object
                    .get("phase")
                    .or_else(|| object.get("status"))
                    .and_then(|value| serde_json::from_value(value.clone()).ok())
                    .unwrap_or(WorkflowPlanPhase::Draft),
            ),
        )),
        _ => None,
    }
}

fn plan_set_status_value(
    object: &serde_json::Map<String, serde_json::Value>,
    forced_status: Option<WorkflowPlanPhase>,
) -> serde_json::Value {
    let mut normalized = serde_json::Map::new();
    normalized.insert(
        "action".to_string(),
        serde_json::Value::String("set_status".to_string()),
    );
    if let Some(status) = forced_status {
        normalized.insert(
            "phase".to_string(),
            serde_json::to_value(status).expect("plan status should serialize"),
        );
    } else if let Some(status) = object.get("phase").or_else(|| object.get("status")) {
        normalized.insert("phase".to_string(), status.clone());
    }
    if let Some(auto_continue) = object.get("auto_continue") {
        normalized.insert("auto_continue".to_string(), auto_continue.clone());
    }
    if let Some(summary) = object.get("summary") {
        normalized.insert("summary".to_string(), summary.clone());
    }
    serde_json::Value::Object(normalized)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "worktree",
    description = "Worktree command. Use action `enter` or `exit`; `enter` uses `target = new|existing` to create or attach to a git worktree and `exit` uses enum `exit_action = keep|remove`.",
    tags(ToolTag::Mutating, ToolTag::FilesystemWrite, ToolTag::Worktree),
    host_capabilities(HostCapability::WorktreeRegistry, HostCapability::PluginStorage),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum WorktreeToolInput {
    #[tool(exec = "enter")]
    Enter {
        #[serde(flatten)]
        args: EnterWorktreeCommandInput,
    },
    #[tool(exec = "exit")]
    Exit {
        #[serde(flatten)]
        args: ExitWorktreeCommandInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "target", rename_all = "snake_case")]
enum EnterWorktreeCommandInput {
    New {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Existing {
        path: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ExitWorktreeAction {
    Keep,
    Remove,
}

impl ExitWorktreeAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Remove => "remove",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
struct ExitWorktreeCommandInput {
    #[serde(rename = "exit_action")]
    exit_action: ExitWorktreeAction,
    #[serde(default)]
    discard_changes: bool,
}

fn parse_worktree_enter_input(input: serde_json::Value) -> SdkResult<EnterWorktreeToolInput> {
    match serde_json::from_value::<EnterWorktreeCommandInput>(input.clone()) {
        Ok(EnterWorktreeCommandInput::New { name }) => {
            Ok(EnterWorktreeToolInput { name, path: None })
        }
        Ok(EnterWorktreeCommandInput::Existing { path }) => Ok(EnterWorktreeToolInput {
            name: None,
            path: Some(path),
        }),
        Err(primary) => {
            let primary = PluginError::invalid_params(primary.to_string());
            serde_json::from_value::<EnterWorktreeToolInput>(input).map_err(|_| primary)
        }
    }
}

fn parse_worktree_exit_input(input: serde_json::Value) -> SdkResult<ExitWorktreeToolInput> {
    match serde_json::from_value::<ExitWorktreeCommandInput>(input.clone()) {
        Ok(parsed) => Ok(ExitWorktreeToolInput {
            action: parsed.exit_action.as_str().to_string(),
            discard_changes: parsed.discard_changes,
        }),
        Err(primary) => {
            let primary = PluginError::invalid_params(primary.to_string());
            serde_json::from_value::<ExitWorktreeToolInput>(input).map_err(|_| primary)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SessionToolResponse {
    session: HostSession,
}

const PLAN_NAMESPACE: &str = "workflow_plan";
const PLAN_KEY_ACTIVE: &str = "active";
const PLAN_RUNTIME_NAMESPACE: &str = "workflow_plan_runtime";
const PLAN_RUNTIME_AUTO_SIGNATURE_KEY: &str = "last_auto_signature";
const PLAN_STATUSLINE_SEGMENT_ID: &str = "plan";
const PLAN_SCHEMA_VERSION: u32 = 1;
const PLAN_REVIEW_DECISION_APPROVE_RUN: &str = "Approve and run";
const PLAN_REVIEW_DECISION_APPROVE_PAUSE: &str = "Approve with auto-continue off";
const PLAN_REVIEW_DECISION_KEEP_PLANNING: &str = "Keep in planning";
const PLAN_REVIEW_DECISION_REJECT: &str = "Reject";
const PLAN_REVIEW_DECISION_CANCELLED: &str = "Cancel plan";

pub(crate) struct WorkflowPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
    config: OnceLock<WorkflowPluginConfig>,
    workspace_root: OnceLock<PathBuf>,
}

impl WorkflowPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
            config: OnceLock::new(),
            workspace_root: OnceLock::new(),
        }
    }

    fn provided_workflow(name: &str) -> SdkResult<agena_skills::Skill> {
        agena_skills::bundled::all()
            .map_err(|err| PluginError::new(err.to_string()))?
            .into_iter()
            .find(|skill| skill.frontmatter.name == name)
            .ok_or_else(|| PluginError::new(format!("missing default workflow '{name}'")))
    }

    fn render_workflow_prompt(body: &str, args: &str) -> String {
        let body = body.trim();
        let args = args.trim();
        if body.contains("$ARGUMENTS") {
            return body.replace("$ARGUMENTS", args);
        }
        if args.is_empty() {
            body.to_string()
        } else {
            format!("{body}\n\nUser arguments:\n{args}")
        }
    }

    fn config(&self) -> SdkResult<&WorkflowPluginConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::new("workflow plugin invoked before init"))
    }

    async fn invoke_provided_workflow(
        &self,
        workflow_name: &str,
        input: &WorkflowPromptToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workflow = Self::provided_workflow(workflow_name)?;
        let prompt = Self::render_workflow_prompt(
            workflow.body.as_str(),
            input.args.as_deref().unwrap_or_default(),
        );
        Ok(ToolInvokeOutput::text(prompt)
            .with_title(format!("{} workflow", workflow.frontmatter.name))
            .with_metadata("workflow", workflow.frontmatter.name))
    }

    async fn invoke_agent_workflow(
        &self,
        workflow_name: &str,
        agent_name: &str,
        input: &WorkflowPromptToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let switch = self
            .host()?
            .agent_switch(HostAgentSwitchRequest {
                agent: Some(agent_name.to_string()),
                session_id: None,
                push_previous: true,
            })
            .await?;
        let mut output = self.invoke_provided_workflow(workflow_name, input).await?;
        output = output
            .with_metadata("workflow_agent", agent_name)
            .with_metadata("agent_stack_depth", switch.stack_depth.to_string());
        if let Some(previous) = switch.previous_agent {
            output = output.with_metadata("previous_agent", previous);
        }
        Ok(output)
    }

    async fn switch_agent_for_tool(
        &self,
        agent: Option<String>,
        push_previous: bool,
    ) -> SdkResult<HostAgentSwitchResponse> {
        self.host()?
            .agent_switch(HostAgentSwitchRequest {
                agent,
                session_id: None,
                push_previous,
            })
            .await
    }

    async fn restore_agent_for_tool(&self) -> SdkResult<HostAgentRestoreResponse> {
        self.host()?
            .agent_restore(HostAgentRestoreRequest { session_id: None })
            .await
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("workflow plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("workflow plugin invoked before init"))
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::new("workflow plugin workspace root not initialized"))
    }

    fn host_ask_user_questions(input: &AskUserToolInput) -> Vec<HostAskUserQuestion> {
        input
            .questions
            .iter()
            .map(|question| HostAskUserQuestion {
                id: question.id.clone(),
                header: question.header.clone(),
                question: question.question.clone(),
                options: question
                    .options
                    .iter()
                    .map(|option| HostAskUserOption {
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect(),
                multiple: question.multiple,
                allow_custom: question.allow_custom,
            })
            .collect()
    }

    fn tool_search_document_from_descriptor(descriptor: ToolDescriptor) -> ToolCatalogDocument {
        let name = descriptor.name;
        let description = descriptor
            .summary
            .or(descriptor.description)
            .unwrap_or_default();
        let tags = descriptor
            .tags
            .into_iter()
            .map(|tag| tag.to_string())
            .collect::<Vec<_>>();
        let plugin_id = descriptor.plugin_id;
        ToolCatalogDocument::new(name, description, tags, plugin_id)
    }

    fn host_todo_item(item: &TodoItem) -> HostTodoItem {
        HostTodoItem {
            content: item.content.clone(),
            status: match item.status {
                TodoStatus::Pending => HostTodoStatus::Pending,
                TodoStatus::InProgress => HostTodoStatus::InProgress,
                TodoStatus::Completed => HostTodoStatus::Completed,
                TodoStatus::Cancelled => HostTodoStatus::Cancelled,
            },
            priority: match item.priority {
                TodoPriority::High => HostTodoPriority::High,
                TodoPriority::Medium => HostTodoPriority::Medium,
                TodoPriority::Low => HostTodoPriority::Low,
            },
        }
    }

    fn pretty_json_text(payload: &serde_json::Value) -> String {
        serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
    }

    fn session_tool_payload(session: HostSession) -> SdkResult<serde_json::Value> {
        serde_json::to_value(SessionToolResponse { session })
            .map_err(|err| PluginError::new(err.to_string()))
    }

    fn session_summary(session: &HostSession) -> String {
        let mut parts = vec![format!("Session #{} title: {}", session.id, session.title)];
        if let Some(parent_id) = session.parent_id {
            parts.push(format!("parent #{parent_id}"));
        }
        if session.root_id != session.id {
            parts.push(format!("root #{}", session.root_id));
        }
        if session.is_subagent {
            parts.push("subagent".to_string());
        }
        parts.join(" | ")
    }

    async fn invoke_get_session(&self) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .host()?
            .get_session(HostGetSessionRequest::default())
            .await?;
        let payload = Self::session_tool_payload(response.session.clone())?;
        Ok(ToolInvokeOutput::text(format!(
            "{}\n\n{}",
            Self::session_summary(&response.session),
            Self::pretty_json_text(&payload)
        ))
        .with_title("session")
        .with_payload(payload))
    }

    async fn invoke_rename_session(
        &self,
        input: &SessionRenameToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let title = input.title.trim();
        if title.is_empty() {
            return Err(PluginError::invalid_params(
                "session rename requires a non-empty title",
            ));
        }
        let response = self
            .host()?
            .rename_session(HostRenameSessionRequest {
                session_id: None,
                title: title.to_string(),
            })
            .await?;
        let payload = Self::session_tool_payload(response.session.clone())?;
        Ok(ToolInvokeOutput::text(format!(
            "Renamed session #{} to {}.\n\n{}",
            response.session.id,
            response.session.title,
            Self::pretty_json_text(&payload)
        ))
        .with_title("session")
        .with_payload(payload))
    }

    async fn load_active_plan(&self) -> SdkResult<Option<WorkflowPlan>> {
        let response = self
            .host()?
            .storage_get(HostStorageGetRequest {
                scope: HostStorageScope::Session,
                visibility: HostStorageVisibility::Shared,
                namespace: PLAN_NAMESPACE.to_string(),
                key: PLAN_KEY_ACTIVE.to_string(),
            })
            .await?;
        let Some(value) = response.value else {
            return Ok(None);
        };
        serde_json::from_str::<WorkflowPlan>(&value)
            .map(Some)
            .map_err(|err| PluginError::new(format!("invalid stored plan payload: {err}")))
    }

    async fn save_active_plan(&self, plan: &WorkflowPlan) -> SdkResult<()> {
        let value =
            serde_json::to_string_pretty(plan).map_err(|err| PluginError::new(err.to_string()))?;
        let host = self.host()?;
        host.storage_set(HostStorageSetRequest {
            scope: HostStorageScope::Session,
            visibility: HostStorageVisibility::Shared,
            namespace: PLAN_NAMESPACE.to_string(),
            key: PLAN_KEY_ACTIVE.to_string(),
            value,
        })
        .await?;
        self.clear_auto_continue_signature().await?;
        self.sync_plan_statusline(Some(plan)).await?;
        Ok(())
    }

    async fn clear_active_plan(&self) -> SdkResult<()> {
        let host = self.host()?;
        host.storage_delete(HostStorageDeleteRequest {
            scope: HostStorageScope::Session,
            visibility: HostStorageVisibility::Shared,
            namespace: PLAN_NAMESPACE.to_string(),
            key: PLAN_KEY_ACTIVE.to_string(),
        })
        .await?;
        self.clear_auto_continue_signature().await?;
        self.sync_plan_statusline(None).await?;
        Ok(())
    }

    async fn load_auto_continue_signature(&self) -> SdkResult<Option<String>> {
        Ok(self
            .host()?
            .storage_get(HostStorageGetRequest {
                scope: HostStorageScope::Session,
                visibility: HostStorageVisibility::Private,
                namespace: PLAN_RUNTIME_NAMESPACE.to_string(),
                key: PLAN_RUNTIME_AUTO_SIGNATURE_KEY.to_string(),
            })
            .await?
            .value)
    }

    async fn save_auto_continue_signature(&self, signature: &str) -> SdkResult<()> {
        self.host()?
            .storage_set(HostStorageSetRequest {
                scope: HostStorageScope::Session,
                visibility: HostStorageVisibility::Private,
                namespace: PLAN_RUNTIME_NAMESPACE.to_string(),
                key: PLAN_RUNTIME_AUTO_SIGNATURE_KEY.to_string(),
                value: signature.to_string(),
            })
            .await
    }

    async fn clear_auto_continue_signature(&self) -> SdkResult<()> {
        self.host()?
            .storage_delete(HostStorageDeleteRequest {
                scope: HostStorageScope::Session,
                visibility: HostStorageVisibility::Private,
                namespace: PLAN_RUNTIME_NAMESPACE.to_string(),
                key: PLAN_RUNTIME_AUTO_SIGNATURE_KEY.to_string(),
            })
            .await
    }

    async fn sync_plan_statusline(&self, plan: Option<&WorkflowPlan>) -> SdkResult<()> {
        let host = self.host()?;
        match plan {
            Some(plan) => {
                host.ui_statusline_contribute(HostStatuslineContributeRequest {
                    segment_id: PLAN_STATUSLINE_SEGMENT_ID.to_string(),
                    content: Self::plan_statusline_content(plan),
                    priority: 120,
                    color: None,
                })
                .await?;
            }
            None => {
                let _ = host
                    .ui_statusline_remove(HostStatuslineRemoveRequest {
                        segment_id: PLAN_STATUSLINE_SEGMENT_ID.to_string(),
                    })
                    .await?;
            }
        }
        Ok(())
    }

    fn plan_payload(plan: &WorkflowPlan) -> SdkResult<serde_json::Value> {
        serde_json::to_value(serde_json::json!({ "plan": plan }))
            .map_err(|err| PluginError::new(err.to_string()))
    }

    fn validate_plan_objective(value: &str) -> SdkResult<String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(PluginError::invalid_params(
                "plan objective must not be empty",
            ));
        }
        Ok(trimmed.to_string())
    }

    fn default_plan_title(objective: &str) -> String {
        let line = objective
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or(objective);
        let title = line.trim();
        if title.chars().count() <= 80 {
            return title.to_string();
        }
        title.chars().take(80).collect()
    }

    fn normalize_plan_steps(inputs: &[WorkflowPlanStepInput]) -> SdkResult<Vec<WorkflowPlanStep>> {
        let mut steps = Vec::with_capacity(inputs.len());
        for (step_index, step) in inputs.iter().enumerate() {
            let title = step.title.trim();
            let description = step.description.trim();
            let resolved_title = if !title.is_empty() {
                title
            } else if !description.is_empty() {
                description
            } else {
                return Err(PluginError::invalid_params(format!(
                    "plan step {} requires a non-empty title",
                    step_index + 1
                )));
            };
            let checkpoints = step
                .checkpoints
                .iter()
                .enumerate()
                .map(|(checkpoint_index, checkpoint)| {
                    let text = checkpoint.text.trim();
                    if text.is_empty() {
                        return Err(PluginError::invalid_params(format!(
                            "plan checkpoint {}.{} requires non-empty text",
                            step_index + 1,
                            checkpoint_index + 1
                        )));
                    }
                    Ok(WorkflowPlanCheckpoint {
                        id: checkpoint
                            .id
                            .clone()
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or_else(|| {
                                format!(
                                    "step_{}_checkpoint_{}",
                                    step_index + 1,
                                    checkpoint_index + 1
                                )
                            }),
                        text: text.to_string(),
                        status: checkpoint.status.unwrap_or_default(),
                    })
                })
                .collect::<SdkResult<Vec<_>>>()?;
            steps.push(WorkflowPlanStep {
                id: step
                    .id
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| format!("step_{}", step_index + 1)),
                title: resolved_title.to_string(),
                description: description.to_string(),
                executor: step.executor,
                status: step.status.unwrap_or_default(),
                wait_until_ms: step.wait_until_ms,
                note: step.note.clone().unwrap_or_default().trim().to_string(),
                checkpoints,
            });
        }
        Ok(steps)
    }

    fn build_plan(
        &self,
        session_id: i64,
        objective: &str,
        title: Option<&str>,
        document_markdown: Option<&str>,
        steps: &[WorkflowPlanStepInput],
        auto_continue: Option<bool>,
        previous: Option<&WorkflowPlan>,
    ) -> SdkResult<WorkflowPlan> {
        let objective = Self::validate_plan_objective(objective)?;
        let now_ms = Utc::now().timestamp_millis();
        let title = title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| Self::default_plan_title(&objective));
        let auto_continue_enabled = match auto_continue {
            Some(value) => value,
            None => previous
                .map(|plan| plan.auto_continue.enabled)
                .unwrap_or(self.config()?.plan.default_auto_continue),
        };
        let auto_continue_source = if auto_continue.is_some() {
            "tool_override".to_string()
        } else if previous.is_some() {
            previous
                .map(|plan| plan.auto_continue.source.clone())
                .unwrap_or_else(|| "config_default".to_string())
        } else {
            "config_default".to_string()
        };
        Ok(WorkflowPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            plan_id: previous
                .map(|plan| plan.plan_id.clone())
                .unwrap_or_else(|| format!("plan_{now_ms}")),
            session_id,
            title,
            objective,
            phase: WorkflowPlanPhase::Draft,
            approval_state: WorkflowPlanApprovalState::NotRequested,
            auto_continue: WorkflowPlanAutoContinue {
                enabled: auto_continue_enabled,
                source: auto_continue_source,
            },
            document_markdown: document_markdown
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_default(),
            steps: Self::normalize_plan_steps(steps)?,
            created_at_ms: previous.map(|plan| plan.created_at_ms).unwrap_or(now_ms),
            updated_at_ms: now_ms,
            submitted_at_ms: None,
            completed_at_ms: None,
        })
    }

    fn plan_phase_label(phase: WorkflowPlanPhase) -> &'static str {
        match phase {
            WorkflowPlanPhase::Draft => "draft",
            WorkflowPlanPhase::Active => "active",
            WorkflowPlanPhase::Blocked => "blocked",
            WorkflowPlanPhase::Completed => "completed",
            WorkflowPlanPhase::Cancelled => "cancelled",
        }
    }

    fn plan_approval_state_label(state: WorkflowPlanApprovalState) -> &'static str {
        match state {
            WorkflowPlanApprovalState::NotRequested => "not_requested",
            WorkflowPlanApprovalState::Pending => "pending",
            WorkflowPlanApprovalState::Approved => "approved",
            WorkflowPlanApprovalState::Rejected => "rejected",
        }
    }

    fn plan_step_status_label(status: WorkflowPlanStepStatus) -> &'static str {
        match status {
            WorkflowPlanStepStatus::Pending => "pending",
            WorkflowPlanStepStatus::InProgress => "in_progress",
            WorkflowPlanStepStatus::Blocked => "blocked",
            WorkflowPlanStepStatus::Completed => "completed",
            WorkflowPlanStepStatus::Skipped => "skipped",
        }
    }

    fn step_status_marker(status: WorkflowPlanStepStatus) -> &'static str {
        match status {
            WorkflowPlanStepStatus::Pending => "[ ]",
            WorkflowPlanStepStatus::InProgress => "[>]",
            WorkflowPlanStepStatus::Blocked => "[!]",
            WorkflowPlanStepStatus::Completed => "[x]",
            WorkflowPlanStepStatus::Skipped => "[-]",
        }
    }

    fn step_status_is_terminal(status: WorkflowPlanStepStatus) -> bool {
        matches!(
            status,
            WorkflowPlanStepStatus::Completed | WorkflowPlanStepStatus::Skipped
        )
    }

    fn normalize_identifier(value: &str) -> String {
        value
            .trim()
            .chars()
            .filter_map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    Some(ch.to_ascii_lowercase())
                } else if ch.is_whitespace() || matches!(ch, '_' | '-') {
                    Some('_')
                } else {
                    None
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string()
    }

    fn parse_1_based_index_hint(value: &str, prefixes: &[&str]) -> Option<usize> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(index) = trimmed.parse::<usize>() {
            return index.checked_sub(1);
        }
        let normalized = Self::normalize_identifier(trimmed);
        for prefix in prefixes {
            for candidate in [
                prefix.to_string(),
                format!("{prefix}_"),
                format!("{prefix}-"),
            ] {
                if let Some(rest) = normalized.strip_prefix(candidate.as_str())
                    && let Ok(index) = rest.parse::<usize>()
                {
                    return index.checked_sub(1);
                }
            }
        }
        None
    }

    fn resolve_plan_step_index(plan: &WorkflowPlan, step_id: &str) -> Option<usize> {
        let normalized_target = Self::normalize_identifier(step_id);
        plan.steps
            .iter()
            .position(|step| step.id == step_id)
            .or_else(|| {
                plan.steps.iter().position(|step| {
                    let title = Self::normalize_identifier(step.title.as_str());
                    let description = Self::normalize_identifier(step.description.as_str());
                    !normalized_target.is_empty()
                        && (title == normalized_target || description == normalized_target)
                })
            })
            .or_else(|| Self::parse_1_based_index_hint(step_id, &["step", "s"]))
            .filter(|index| *index < plan.steps.len())
    }

    fn resolve_checkpoint_index(step: &WorkflowPlanStep, checkpoint_id: &str) -> Option<usize> {
        let normalized_target = Self::normalize_identifier(checkpoint_id);
        step.checkpoints
            .iter()
            .position(|checkpoint| checkpoint.id == checkpoint_id)
            .or_else(|| {
                step.checkpoints.iter().position(|checkpoint| {
                    let text = Self::normalize_identifier(checkpoint.text.as_str());
                    !normalized_target.is_empty() && text == normalized_target
                })
            })
            .or_else(|| Self::parse_1_based_index_hint(checkpoint_id, &["checkpoint", "cp", "c"]))
            .filter(|index| *index < step.checkpoints.len())
    }

    fn plan_step_identifier_hint(step: &WorkflowPlanStep, index: usize) -> String {
        format!("step_id={} (step {})", step.id, index + 1)
    }

    fn checkpoint_identifier_hint(checkpoint: &WorkflowPlanCheckpoint, index: usize) -> String {
        format!("checkpoint_id={} (checkpoint {})", checkpoint.id, index + 1)
    }

    fn plan_progress_counts(plan: &WorkflowPlan) -> (usize, usize, usize, usize) {
        let total_steps = plan.steps.len();
        let completed_steps = plan
            .steps
            .iter()
            .filter(|step| Self::step_status_is_terminal(step.status))
            .count();
        let total_checkpoints = plan
            .steps
            .iter()
            .map(|step| step.checkpoints.len())
            .sum::<usize>();
        let completed_checkpoints = plan
            .steps
            .iter()
            .flat_map(|step| step.checkpoints.iter())
            .filter(|checkpoint| Self::step_status_is_terminal(checkpoint.status))
            .count();
        (
            completed_steps,
            total_steps,
            completed_checkpoints,
            total_checkpoints,
        )
    }

    fn workflow_plan_markdown(plan: &WorkflowPlan) -> String {
        let document_markdown = plan.document_markdown.trim();
        let mut sections = Vec::new();

        if document_markdown.is_empty() {
            sections.push(format!("# {}", plan.title));
            if plan.objective.trim() != plan.title.trim() {
                sections.push(String::new());
                sections.push(plan.objective.trim().to_string());
            }
        } else {
            sections.push(document_markdown.to_string());
        }

        let mut metadata = vec![format!(
            "Auto-continue: {}",
            if plan.auto_continue.enabled {
                "on"
            } else {
                "off"
            }
        )];
        if matches!(
            plan.approval_state,
            WorkflowPlanApprovalState::Pending | WorkflowPlanApprovalState::Rejected
        ) {
            metadata.push(format!(
                "Review: {}",
                Self::plan_approval_state_label(plan.approval_state)
            ));
        }
        sections.push(String::new());
        sections.push(format!("_{}_", metadata.join(" · ")));

        if !plan.steps.is_empty() {
            sections.push(String::new());
            sections.push("## Steps".to_string());
            for (index, step) in plan.steps.iter().enumerate() {
                sections.push(format!(
                    "{}. {} {} ({})",
                    index + 1,
                    Self::step_status_marker(step.status),
                    step.title,
                    match step.executor {
                        WorkflowPlanExecutor::Ai => "ai",
                        WorkflowPlanExecutor::Human => "human",
                    }
                ));
                if !step.description.trim().is_empty()
                    && step.description.trim() != step.title.trim()
                {
                    sections.push(format!("   - Details: {}", step.description.trim()));
                }
                for checkpoint in &step.checkpoints {
                    sections.push(format!(
                        "   - {} {}",
                        Self::step_status_marker(checkpoint.status),
                        checkpoint.text
                    ));
                }
                if !step.note.trim().is_empty() {
                    sections.push(format!("   - Note: {}", step.note.trim()));
                }
            }
        }
        sections.join("\n")
    }

    fn plan_statusline_content(plan: &WorkflowPlan) -> String {
        let (completed_steps, total_steps, _, _) = Self::plan_progress_counts(plan);
        let review_segment = match plan.approval_state {
            WorkflowPlanApprovalState::Pending | WorkflowPlanApprovalState::Rejected => {
                format!(
                    " review:{}",
                    Self::plan_approval_state_label(plan.approval_state)
                )
            }
            _ => String::new(),
        };
        if total_steps == 0 {
            return format!(
                "plan:{}{} auto:{}",
                Self::plan_phase_label(plan.phase),
                review_segment,
                if plan.auto_continue.enabled {
                    "on"
                } else {
                    "off"
                }
            );
        }
        format!(
            "plan:{}{} steps:{}/{} auto:{}",
            Self::plan_phase_label(plan.phase),
            review_segment,
            completed_steps,
            total_steps,
            if plan.auto_continue.enabled {
                "on"
            } else {
                "off"
            }
        )
    }

    fn next_actionable_step(plan: &WorkflowPlan) -> Option<(usize, &WorkflowPlanStep)> {
        if matches!(
            plan.phase,
            WorkflowPlanPhase::Completed | WorkflowPlanPhase::Cancelled
        ) {
            return None;
        }
        plan.steps
            .iter()
            .enumerate()
            .find(|(_, step)| !Self::step_status_is_terminal(step.status))
    }

    fn plan_summary_text(plan: &WorkflowPlan) -> String {
        let (completed_steps, total_steps, completed_checkpoints, total_checkpoints) =
            Self::plan_progress_counts(plan);
        let mut parts = vec![
            format!("Status: {}", Self::plan_phase_label(plan.phase)),
            format!("Steps: {completed_steps}/{total_steps} complete"),
        ];
        if matches!(
            plan.approval_state,
            WorkflowPlanApprovalState::Pending | WorkflowPlanApprovalState::Rejected
        ) {
            parts.push(format!(
                "Review: {}",
                Self::plan_approval_state_label(plan.approval_state)
            ));
        }
        if total_checkpoints > 0 {
            parts.push(format!(
                "Checkpoints: {completed_checkpoints}/{total_checkpoints} complete"
            ));
        }
        parts.push(format!(
            "Auto-continue: {}",
            if plan.auto_continue.enabled {
                "on"
            } else {
                "off"
            }
        ));
        parts.join(" | ")
    }

    fn plan_output_text(prefix: &str, plan: &WorkflowPlan) -> String {
        format!(
            "{prefix}\n{}\n\n{}",
            Self::plan_summary_text(plan),
            Self::workflow_plan_markdown(plan)
        )
    }

    fn cascade_terminal_step_status(step: &mut WorkflowPlanStep, status: WorkflowPlanStepStatus) {
        if !matches!(
            status,
            WorkflowPlanStepStatus::Completed | WorkflowPlanStepStatus::Skipped
        ) {
            return;
        }
        for checkpoint in &mut step.checkpoints {
            if !Self::step_status_is_terminal(checkpoint.status) {
                checkpoint.status = status;
            }
        }
    }

    fn reconcile_step_status_from_checkpoints(step: &mut WorkflowPlanStep) {
        if step.checkpoints.is_empty() {
            return;
        }
        let all_terminal = step
            .checkpoints
            .iter()
            .all(|checkpoint| Self::step_status_is_terminal(checkpoint.status));
        if all_terminal {
            if step
                .checkpoints
                .iter()
                .all(|checkpoint| checkpoint.status == WorkflowPlanStepStatus::Skipped)
            {
                step.status = WorkflowPlanStepStatus::Skipped;
            } else {
                step.status = WorkflowPlanStepStatus::Completed;
            }
            return;
        }
        if matches!(
            step.status,
            WorkflowPlanStepStatus::Completed | WorkflowPlanStepStatus::Skipped
        ) {
            step.status = if step
                .checkpoints
                .iter()
                .any(|checkpoint| checkpoint.status == WorkflowPlanStepStatus::Blocked)
            {
                WorkflowPlanStepStatus::Blocked
            } else if step.checkpoints.iter().any(|checkpoint| {
                matches!(
                    checkpoint.status,
                    WorkflowPlanStepStatus::InProgress | WorkflowPlanStepStatus::Completed
                )
            }) {
                WorkflowPlanStepStatus::InProgress
            } else {
                WorkflowPlanStepStatus::Pending
            };
        }
    }

    fn plan_completion_blocker(plan: &WorkflowPlan) -> Option<String> {
        for (step_index, step) in plan.steps.iter().enumerate() {
            if !Self::step_status_is_terminal(step.status) {
                return Some(format!(
                    "step {} ('{}') is still {}",
                    step_index + 1,
                    step.title,
                    Self::plan_step_status_label(step.status)
                ));
            }
            for (checkpoint_index, checkpoint) in step.checkpoints.iter().enumerate() {
                if !Self::step_status_is_terminal(checkpoint.status) {
                    return Some(format!(
                        "checkpoint {}.{} ('{}') is still {}",
                        step_index + 1,
                        checkpoint_index + 1,
                        checkpoint.text,
                        Self::plan_step_status_label(checkpoint.status)
                    ));
                }
            }
        }
        None
    }

    fn ensure_plan_ready_for_completion(plan: &WorkflowPlan) -> SdkResult<()> {
        if let Some(blocker) = Self::plan_completion_blocker(plan) {
            return Err(PluginError::invalid_params(format!(
                "cannot complete plan: {blocker}"
            )));
        }
        Ok(())
    }

    fn append_completion_summary(plan: &mut WorkflowPlan, summary: Option<&str>) {
        let Some(summary) = summary.map(str::trim).filter(|value| !value.is_empty()) else {
            return;
        };
        let summary_section = format!("## Completion Summary\n\n{summary}");
        if plan.document_markdown.trim().is_empty() {
            plan.document_markdown = summary_section;
            return;
        }
        if plan.document_markdown.contains(summary_section.as_str()) {
            return;
        }
        plan.document_markdown = format!("{}\n\n{summary_section}", plan.document_markdown.trim());
    }

    fn plan_approval_state_for_phase(
        phase: WorkflowPlanPhase,
        current: WorkflowPlanApprovalState,
    ) -> WorkflowPlanApprovalState {
        match phase {
            WorkflowPlanPhase::Draft => WorkflowPlanApprovalState::NotRequested,
            WorkflowPlanPhase::Active
            | WorkflowPlanPhase::Blocked
            | WorkflowPlanPhase::Completed => WorkflowPlanApprovalState::Approved,
            WorkflowPlanPhase::Cancelled => {
                if current == WorkflowPlanApprovalState::Pending {
                    WorkflowPlanApprovalState::NotRequested
                } else {
                    current
                }
            }
        }
    }

    fn mark_plan_completed(plan: &mut WorkflowPlan, summary: Option<&str>) -> SdkResult<()> {
        Self::ensure_plan_ready_for_completion(plan)?;
        plan.phase = WorkflowPlanPhase::Completed;
        plan.approval_state = WorkflowPlanApprovalState::Approved;
        plan.completed_at_ms = Some(Utc::now().timestamp_millis());
        Self::append_completion_summary(plan, summary);
        Ok(())
    }

    fn validate_plan_phase_change(plan: &WorkflowPlan, phase: WorkflowPlanPhase) -> SdkResult<()> {
        match phase {
            WorkflowPlanPhase::Completed => Self::ensure_plan_ready_for_completion(plan),
            WorkflowPlanPhase::Active | WorkflowPlanPhase::Blocked => {
                if Self::plan_completion_blocker(plan).is_none() {
                    return Err(PluginError::invalid_params(format!(
                        "cannot set plan status to {}: all steps and checkpoints are already complete; reopen a step or checkpoint first",
                        Self::plan_phase_label(phase)
                    )));
                }
                Ok(())
            }
            WorkflowPlanPhase::Draft | WorkflowPlanPhase::Cancelled => Ok(()),
        }
    }

    fn set_plan_phase(
        plan: &mut WorkflowPlan,
        phase: WorkflowPlanPhase,
        completion_summary: Option<&str>,
    ) -> SdkResult<()> {
        Self::validate_plan_phase_change(plan, phase)?;
        if phase == WorkflowPlanPhase::Completed {
            return Self::mark_plan_completed(plan, completion_summary);
        }
        plan.phase = phase;
        plan.approval_state = Self::plan_approval_state_for_phase(phase, plan.approval_state);
        plan.completed_at_ms = None;
        Ok(())
    }

    fn plan_auto_signature(
        plan: &WorkflowPlan,
        step_index: usize,
        step: &WorkflowPlanStep,
    ) -> String {
        format!(
            "{}:{}:{}:{}",
            plan.plan_id, plan.updated_at_ms, step_index, step.id
        )
    }

    fn review_decision(response: &crate::plugin::sdk::host_api::AskUserResponse) -> Option<String> {
        response
            .answers
            .get("reply")
            .and_then(|values| values.first())
            .cloned()
            .or_else(|| {
                let reply = response.reply.trim();
                (!reply.is_empty()).then_some(reply.to_string())
            })
    }

    fn plan_lock_active(plan: &WorkflowPlan) -> bool {
        plan.phase == WorkflowPlanPhase::Draft
    }

    fn tool_allowed_during_planning(input: &ToolBeforeInput) -> bool {
        if input.tool_name == "plan"
            || input.tool_name == "user"
            || input.tool_name == "tools"
            || input.tool_name == "workflow"
            || input.tool_name == "session"
            || input.tool_name == "agent"
            || input.tool_name == "todo"
        {
            return true;
        }
        if input.tool_name == "task" {
            return serde_json::from_value::<TaskToolActionInput>(input.input.clone())
                .ok()
                .is_some_and(|task| {
                    matches!(
                        task,
                        TaskToolActionInput::Run {
                            args: TaskToolInput {
                                subagent_type: crate::message::TaskSubagentType::Explore
                                    | crate::message::TaskSubagentType::Verify,
                                ..
                            }
                        }
                    )
                });
        }
        if input.tags.iter().any(|tag| matches!(tag, ToolTag::Shell)) {
            return true;
        }
        if input.tags.iter().any(|tag| {
            matches!(
                tag,
                ToolTag::Mutating | ToolTag::FilesystemWrite | ToolTag::Worktree
            )
        }) {
            return false;
        }
        input.tags.iter().any(|tag| {
            matches!(
                tag,
                ToolTag::ReadOnly | ToolTag::Discovery | ToolTag::Interactive | ToolTag::Planning
            )
        })
    }

    fn is_probably_read_only_shell(command: &str) -> bool {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return true;
        }
        if trimmed.contains('>')
            || trimmed.contains(">>")
            || trimmed.contains("<<")
            || trimmed.contains("rm ")
            || trimmed.contains("mv ")
            || trimmed.contains("cp ")
            || trimmed.contains("chmod ")
            || trimmed.contains("chown ")
            || trimmed.contains("touch ")
            || trimmed.contains(';')
            || trimmed.contains("&&")
            || trimmed.contains("||")
        {
            return false;
        }
        let Some(tokens) = shlex::split(trimmed) else {
            return false;
        };
        let Some(command_name) = tokens.first().map(String::as_str) else {
            return true;
        };
        match command_name {
            "cat" | "sed" | "grep" | "rg" | "ls" | "find" | "pwd" | "head" | "tail" | "wc"
            | "stat" | "tree" | "readlink" | "realpath" | "file" | "echo" => true,
            "git" => matches!(
                tokens.get(1).map(String::as_str),
                Some(
                    "status"
                        | "diff"
                        | "show"
                        | "log"
                        | "branch"
                        | "rev-parse"
                        | "remote"
                        | "ls-files"
                        | "grep"
                )
            ),
            _ => false,
        }
    }

    fn command_text_for_policy(input: &CommandBeforeInput) -> String {
        if input.command == "sh"
            && input.args.len() >= 2
            && input.args.first().is_some_and(|arg| arg == "-c")
        {
            return input.args[1].clone();
        }
        std::iter::once(input.command.as_str())
            .chain(input.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn auto_continue_prompt(
        plan: &WorkflowPlan,
        step_index: usize,
        step: &WorkflowPlanStep,
    ) -> String {
        let mut lines = vec![
            "<plan_context>".to_string(),
            "Continue the active approved plan.".to_string(),
            format!("Plan: {}", plan.title),
            format!("Objective: {}", plan.objective),
            format!("Current step {}: {}", step_index + 1, step.title),
        ];
        if !step.description.trim().is_empty() {
            lines.push(format!("Step details: {}", step.description.trim()));
        }
        let pending_checkpoints = step
            .checkpoints
            .iter()
            .filter(|checkpoint| {
                !matches!(
                    checkpoint.status,
                    WorkflowPlanStepStatus::Completed | WorkflowPlanStepStatus::Skipped
                )
            })
            .map(|checkpoint| format!("- {}", checkpoint.text))
            .collect::<Vec<_>>();
        if !pending_checkpoints.is_empty() {
            lines.push("Pending checkpoints:".to_string());
            lines.extend(pending_checkpoints);
        }
        lines.push(
            "Update the plan state as you make progress. If the next step needs human input, stop and say exactly what is needed.".to_string(),
        );
        lines.push("</plan_context>".to_string());
        lines.join("\n")
    }

    async fn invoke_plan_get(&self) -> SdkResult<ToolInvokeOutput> {
        let Some(plan) = self.load_active_plan().await? else {
            let payload = serde_json::json!({ "plan": serde_json::Value::Null });
            return Ok(ToolInvokeOutput::text("No active plan.")
                .with_title("plan")
                .with_payload(payload));
        };
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::text(format!(
            "{}\n\n{}",
            Self::plan_summary_text(&plan),
            Self::workflow_plan_markdown(&plan)
        ))
        .with_title("plan")
        .with_payload(payload))
    }

    async fn invoke_plan_create(
        &self,
        session_id: i64,
        input: &PlanCreateInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let plan = self.build_plan(
            session_id,
            input.objective.as_str(),
            input.title.as_deref(),
            input.document_markdown.as_deref(),
            input.steps.as_slice(),
            input.auto_continue,
            None,
        )?;
        self.save_active_plan(&plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(
            ToolInvokeOutput::text(Self::plan_output_text("Created a draft plan.", &plan))
                .with_title("plan")
                .with_payload(payload),
        )
    }

    async fn invoke_plan_replace(
        &self,
        session_id: i64,
        input: &PlanReplaceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let previous = self.load_active_plan().await?;
        let plan = self.build_plan(
            session_id,
            input.objective.as_str(),
            input.title.as_deref(),
            input.document_markdown.as_deref(),
            input.steps.as_slice(),
            input.auto_continue,
            previous.as_ref(),
        )?;
        self.save_active_plan(&plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::text(Self::plan_output_text(
            "Replaced the active plan and returned it to draft.",
            &plan,
        ))
        .with_title("plan")
        .with_payload(payload))
    }

    async fn invoke_plan_submit(&self, _input: &PlanSubmitInput) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params("no active plan to submit"));
        };
        let now_ms = Utc::now().timestamp_millis();
        Self::set_plan_phase(&mut plan, WorkflowPlanPhase::Draft, None)?;
        plan.approval_state = WorkflowPlanApprovalState::Pending;
        plan.submitted_at_ms = Some(now_ms);
        plan.updated_at_ms = now_ms;
        self.save_active_plan(&plan).await?;

        let response = self
            .host()?
            .ask_user(AskUserRequest {
                title: "Review Plan".to_string(),
                body_markdown: Self::workflow_plan_markdown(&plan),
                kind: "review".to_string(),
                submit_label: "Submit decision".to_string(),
                cancel_label: "Keep in planning".to_string(),
                questions: vec![HostAskUserQuestion {
                    id: "decision".to_string(),
                    header: "Decision".to_string(),
                    question: "Choose what should happen to this plan next.".to_string(),
                    options: vec![
                        HostAskUserOption {
                            label: PLAN_REVIEW_DECISION_APPROVE_RUN.to_string(),
                            description: "Approve the plan and keep auto-continue enabled."
                                .to_string(),
                        },
                        HostAskUserOption {
                            label: PLAN_REVIEW_DECISION_APPROVE_PAUSE.to_string(),
                            description:
                                "Approve the plan but keep it paused for manual execution."
                                    .to_string(),
                        },
                        HostAskUserOption {
                            label: PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string(),
                            description: "Return to draft so the plan can be edited further."
                                .to_string(),
                        },
                        HostAskUserOption {
                            label: PLAN_REVIEW_DECISION_REJECT.to_string(),
                            description:
                                "Reject the current draft and mark the review as rejected."
                                    .to_string(),
                        },
                        HostAskUserOption {
                            label: PLAN_REVIEW_DECISION_CANCELLED.to_string(),
                            description: "Cancel the plan entirely and stop work on it."
                                .to_string(),
                        },
                    ],
                    multiple: false,
                    allow_custom: false,
                }],
                prompt: String::new(),
                options: Vec::new(),
                allow_free_text: false,
            })
            .await?;

        let decision = if response.cancelled {
            PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string()
        } else {
            Self::review_decision(&response)
                .unwrap_or_else(|| PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string())
        };

        let now_ms = Utc::now().timestamp_millis();
        match decision.as_str() {
            PLAN_REVIEW_DECISION_APPROVE_RUN => {
                Self::set_plan_phase(&mut plan, WorkflowPlanPhase::Active, None)?;
            }
            PLAN_REVIEW_DECISION_APPROVE_PAUSE => {
                Self::set_plan_phase(&mut plan, WorkflowPlanPhase::Active, None)?;
                plan.auto_continue.enabled = false;
                plan.auto_continue.source = "review_override".to_string();
            }
            PLAN_REVIEW_DECISION_REJECT => {
                plan.phase = WorkflowPlanPhase::Draft;
                plan.approval_state = WorkflowPlanApprovalState::Rejected;
            }
            PLAN_REVIEW_DECISION_CANCELLED => {
                Self::set_plan_phase(&mut plan, WorkflowPlanPhase::Cancelled, None)?;
            }
            _ => {
                plan.phase = WorkflowPlanPhase::Draft;
                plan.approval_state = WorkflowPlanApprovalState::NotRequested;
            }
        }
        plan.updated_at_ms = now_ms;
        self.save_active_plan(&plan).await?;

        let payload = serde_json::json!({
            "plan": plan,
            "decision": decision,
        });
        Ok(ToolInvokeOutput::text(Self::plan_output_text(
            format!("Plan review decision: {decision}.").as_str(),
            &plan,
        ))
        .with_title("plan review")
        .with_payload(payload))
    }

    async fn invoke_plan_set_status(
        &self,
        input: &PlanSetStatusInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params("no active plan to update"));
        };
        if input.phase.is_none() && input.auto_continue.is_none() {
            return Err(PluginError::invalid_params(
                "plan set_status requires at least one of phase/status or auto_continue",
            ));
        }
        let completion_summary = match input.phase {
            Some(WorkflowPlanPhase::Completed) => input.summary.as_deref(),
            _ => None,
        };
        if let Some(status) = input.phase {
            Self::set_plan_phase(&mut plan, status, completion_summary)?;
        }
        if let Some(auto_continue) = input.auto_continue {
            plan.auto_continue.enabled = auto_continue;
            plan.auto_continue.source = "tool_override".to_string();
        }
        let message = match input.phase {
            Some(status) => format!(
                "Updated the plan status to {}.",
                Self::plan_phase_label(status)
            ),
            None => "Updated the plan status settings.".to_string(),
        };
        plan.updated_at_ms = Utc::now().timestamp_millis();
        self.save_active_plan(&plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(
            ToolInvokeOutput::text(Self::plan_output_text(message.as_str(), &plan))
                .with_title("plan")
                .with_payload(payload),
        )
    }

    async fn invoke_plan_update_step(
        &self,
        input: &PlanUpdateStepInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params("no active plan to update"));
        };
        let Some(step_index) = Self::resolve_plan_step_index(&plan, input.step_id.as_str()) else {
            return Err(PluginError::invalid_params(format!(
                "unknown plan step '{}'; available steps: {}",
                input.step_id,
                plan.steps
                    .iter()
                    .enumerate()
                    .map(|(index, step)| format!(
                        "'{}' [{}]",
                        step.title,
                        Self::plan_step_identifier_hint(step, index)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        let step = &mut plan.steps[step_index];
        step.status = input.status;
        Self::cascade_terminal_step_status(step, input.status);
        step.wait_until_ms = input.wait_until_ms;
        if let Some(note) = input.note.as_deref() {
            step.note = note.trim().to_string();
        }
        let step_title = step.title.clone();
        plan.updated_at_ms = Utc::now().timestamp_millis();
        self.save_active_plan(&plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::text(Self::plan_output_text(
            format!("Updated step '{step_title}'.").as_str(),
            &plan,
        ))
        .with_title("plan")
        .with_payload(payload))
    }

    async fn invoke_plan_update_checkpoint(
        &self,
        input: &PlanUpdateCheckpointInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params("no active plan to update"));
        };
        let Some(step_index) = Self::resolve_plan_step_index(&plan, input.step_id.as_str()) else {
            return Err(PluginError::invalid_params(format!(
                "unknown plan step '{}'; available steps: {}",
                input.step_id,
                plan.steps
                    .iter()
                    .enumerate()
                    .map(|(index, step)| format!(
                        "'{}' [{}]",
                        step.title,
                        Self::plan_step_identifier_hint(step, index)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        let step = &mut plan.steps[step_index];
        let checkpoint_text = {
            let Some(checkpoint_index) =
                Self::resolve_checkpoint_index(step, input.checkpoint_id.as_str())
            else {
                return Err(PluginError::invalid_params(format!(
                    "unknown checkpoint '{}' for step '{}'; available checkpoints: {}",
                    input.checkpoint_id,
                    input.step_id,
                    step.checkpoints
                        .iter()
                        .enumerate()
                        .map(|(index, checkpoint)| format!(
                            "'{}' [{}]",
                            checkpoint.text,
                            Self::checkpoint_identifier_hint(checkpoint, index)
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            };
            let checkpoint = &mut step.checkpoints[checkpoint_index];
            checkpoint.status = input.status;
            checkpoint.text.clone()
        };
        Self::reconcile_step_status_from_checkpoints(step);
        plan.updated_at_ms = Utc::now().timestamp_millis();
        self.save_active_plan(&plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::text(Self::plan_output_text(
            format!("Updated checkpoint '{checkpoint_text}'.").as_str(),
            &plan,
        ))
        .with_title("plan")
        .with_payload(payload))
    }

    async fn invoke_plan_clear(&self) -> SdkResult<ToolInvokeOutput> {
        let existing = self.load_active_plan().await?;
        self.clear_active_plan().await?;
        let payload = serde_json::json!({
            "cleared": existing.is_some(),
        });
        let text = if existing.is_some() {
            "Cleared the active plan."
        } else {
            "No active plan to clear."
        };
        Ok(ToolInvokeOutput::text(text)
            .with_title("plan")
            .with_payload(payload))
    }

    async fn invoke_plan_next(&self) -> SdkResult<ToolInvokeOutput> {
        let Some(plan) = self.load_active_plan().await? else {
            let payload = serde_json::json!({ "plan": serde_json::Value::Null, "next_step": serde_json::Value::Null });
            return Ok(ToolInvokeOutput::text("No active plan.")
                .with_title("plan")
                .with_payload(payload));
        };
        let payload = match Self::next_actionable_step(&plan) {
            Some((index, step)) => serde_json::json!({
                "plan": plan,
                "next_step": step,
                "next_step_index": index,
            }),
            None => serde_json::json!({
                "plan": plan,
                "next_step": serde_json::Value::Null,
            }),
        };
        let text = match Self::next_actionable_step(&plan) {
            Some((index, step)) => format!(
                "Next step {} is '{}' ({:?}) [{}].",
                index + 1,
                step.title,
                step.executor,
                Self::plan_step_identifier_hint(step, index)
            ),
            None => "The active plan has no remaining actionable steps.".to_string(),
        };
        Ok(ToolInvokeOutput::text(text)
            .with_title("plan next")
            .with_payload(payload))
    }

    async fn invoke_ask_user(&self, input: &AskUserToolInput) -> SdkResult<ToolInvokeOutput> {
        ask_user::validate(input).map_err(|err| PluginError::invalid_params(err.to_string()))?;
        let host = self.host()?;
        let response = host
            .ask_user(AskUserRequest {
                title: input.title.clone(),
                body_markdown: input.body_markdown.clone(),
                kind: input.kind.clone(),
                submit_label: input.submit_label.clone(),
                cancel_label: input.cancel_label.clone(),
                questions: Self::host_ask_user_questions(input),
                prompt: String::new(),
                options: Vec::new(),
                allow_free_text: false,
            })
            .await?;
        if response.cancelled {
            let reason = if response.reply.trim().is_empty() {
                "user declined to answer requested questions".to_string()
            } else {
                response.reply
            };
            return Err(PluginError::new(reason));
        }

        let mut answers = response.answers;
        if answers.is_empty()
            && let Some(question) = input.questions.first()
            && !response.reply.trim().is_empty()
        {
            answers.insert(question.id.clone(), vec![response.reply]);
        }

        let execution = ask_user::execution_from_answers(input, answers);
        Ok(crate::plugins::provided::router::tool_execution_to_invoke_output(execution))
    }

    async fn invoke_task(&self, input: &TaskToolInput) -> SdkResult<ToolInvokeOutput> {
        let host = self.host()?;
        let response = host
            .spawn_subtask(SpawnSubtaskRequest {
                subagent_type: input.subagent_type.to_string(),
                description: input.description.clone(),
                prompt: input.prompt.clone(),
                task_id: input.task_id.clone(),
                command: input.command.clone(),
                model: None,
            })
            .await?;

        let session_id = response.metadata.get("session_id").cloned();
        let model_provider_id = response.metadata.get("model_provider_id").cloned();
        let model_id = response.metadata.get("model_id").cloned();
        let output_text = if response.final_text.trim().is_empty() {
            format!(
                "Created/resumed subtask session {} for profile '{}' in workspace {}.",
                session_id.as_deref().unwrap_or("unknown"),
                input.subagent_type,
                "<unknown>"
            )
        } else {
            response.final_text
        };
        let mut view = ToolExecutionView::simple(
            format!("Task {} ({})", input.description, input.subagent_type),
            output_text,
        );
        view.metadata
            .insert("description".to_string(), input.description.clone());
        view.metadata
            .insert("subagent_type".to_string(), input.subagent_type.to_string());
        view.metadata.insert(
            "profile_guidance".to_string(),
            input.subagent_type.guidance().to_string(),
        );
        if let Some(session_id_value) = session_id.clone() {
            view.metadata
                .insert("session_id".to_string(), session_id_value);
        }
        if let Some(command) = input.command.clone() {
            view.metadata.insert("command".to_string(), command);
        }
        if let Some(model_provider_id) = model_provider_id.clone() {
            view.metadata
                .insert("model_provider_id".to_string(), model_provider_id);
        }
        if let Some(model_id) = model_id.clone() {
            view.metadata.insert("model_id".to_string(), model_id);
        }
        for (key, value) in response.metadata {
            view.metadata.entry(key).or_insert(value);
        }

        let output = ToolPayloadOutput::Task {
            session_id,
            model_provider_id,
            model_id,
        };
        Ok(
            crate::plugins::provided::router::tool_execution_to_invoke_output(
                ToolPayloadExecution::new(output, view),
            ),
        )
    }

    async fn invoke_agent_switch(
        &self,
        input: &AgentSwitchToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .switch_agent_for_tool(input.agent.clone(), input.push_previous)
            .await?;
        let payload =
            serde_json::to_value(&response).map_err(|err| PluginError::new(err.to_string()))?;
        let current = response
            .current_agent
            .as_deref()
            .unwrap_or("default runtime context");
        let previous = response.previous_agent.as_deref().unwrap_or("none");
        Ok(ToolInvokeOutput::text(format!(
            "Switched session {} agent to {current}. Previous agent: {previous}.",
            response.session_id
        ))
        .with_title("agent switch")
        .with_payload(payload))
    }

    async fn invoke_agent_restore(
        &self,
        _input: &AgentRestoreToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let response = self.restore_agent_for_tool().await?;
        let payload =
            serde_json::to_value(&response).map_err(|err| PluginError::new(err.to_string()))?;
        let text = if response.restored {
            let current = response
                .current_agent
                .as_deref()
                .unwrap_or("default runtime context");
            let previous = response.previous_agent.as_deref().unwrap_or("none");
            format!(
                "Restored session {} agent to {current}. Previous agent: {previous}.",
                response.session_id
            )
        } else {
            format!(
                "No agent restore point is available for session {}.",
                response.session_id
            )
        };
        Ok(ToolInvokeOutput::text(text)
            .with_title("agent restore")
            .with_payload(payload))
    }

    async fn invoke_tool_search(&self, input: &ToolSearchToolInput) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.trim();
        if query.is_empty() {
            return Err(PluginError::invalid_params(
                "tools search requires a non-empty query",
            ));
        }
        let config = self.config()?;
        let limit = input
            .limit
            .unwrap_or(config.tool_catalog.search.default_limit)
            .clamp(1, config.tool_catalog.search.max_limit) as usize;
        let host = self.host()?;
        let catalog = host
            .list_tools()
            .await?
            .into_iter()
            .map(Self::tool_search_document_from_descriptor)
            .collect::<Vec<_>>();
        let results = search_tool_catalog(&catalog, query, limit)
            .map_err(|err| PluginError::new(format!("tool search failed: {err}")))?;
        let names = results
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        let mut lines = vec![format!(
            "Found {} tool(s) matching '{}'.",
            names.len(),
            query
        )];
        for tool in &results {
            lines.push(format!(
                "- {} [{}]: {}",
                tool.name,
                tags_summary(tool.tags.as_slice()),
                tool.description
            ));
        }
        if !names.is_empty() {
            lines.push(
                "Call `tools` with action `help` and an exact tool name for detailed usage."
                    .to_string(),
            );
        }
        let payload = serde_json::json!({ "results": names });
        Ok(ToolInvokeOutput::text(lines.join("\n"))
            .with_title("Tool search")
            .with_payload(payload)
            .with_metadata("query", query)
            .with_metadata("matched_tools", results.len().to_string()))
    }

    async fn invoke_tool_help(&self, input: &ToolsHelpInput) -> SdkResult<ToolInvokeOutput> {
        let requested = input.tool.trim();
        if requested.is_empty() {
            return Err(PluginError::invalid_params(
                "tools help requires a non-empty tool name",
            ));
        }
        let config = self.config()?;
        let tools = self.host()?.list_tools().await?;
        let mut exact = None;
        let mut case_insensitive = None;
        for tool in tools {
            if tool.name == requested {
                exact = Some(tool);
                break;
            }
            if case_insensitive.is_none() && tool.name.eq_ignore_ascii_case(requested) {
                case_insensitive = Some(tool);
            }
        }
        let Some(descriptor) = exact.or(case_insensitive) else {
            return Err(PluginError::invalid_params(format!(
                "unknown tool '{requested}'"
            )));
        };

        let mut lines = vec![format!("Tool: {}", descriptor.name)];
        if let Some(plugin_id) = descriptor.plugin_id.as_deref() {
            lines.push(format!("Plugin: {plugin_id}"));
        }
        if !descriptor.tags.is_empty() {
            let tags = descriptor
                .tags
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("Tags: {tags}"));
        }
        if let Some(summary) = descriptor
            .summary
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            lines.push(format!("Summary: {summary}"));
        }
        if let Some(description) = descriptor
            .description
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            lines.push(format!("Description: {description}"));
        }
        if let Some(help) = descriptor.help.as_deref().filter(|value| !value.is_empty()) {
            lines.push("Help:".to_string());
            lines.push(help.to_string());
        }
        if input
            .include_schema
            .unwrap_or(config.tool_catalog.help.include_schema_by_default)
            && let Some(schema) = descriptor.input_schema.as_ref()
        {
            lines.push("Input schema:".to_string());
            lines.push(
                serde_json::to_string_pretty(schema)
                    .map_err(|err| PluginError::new(err.to_string()))?,
            );
        }

        Ok(
            ToolInvokeOutput::text(lines.join("\n"))
                .with_title(format!("{} help", descriptor.name)),
        )
    }

    fn invoke_tool_catalog_usage() -> ToolInvokeOutput {
        ToolInvokeOutput::text(
            [
                "Tool catalog usage:",
                r#"- Search: {"action":"search","query":"web","limit":8}"#,
                r#"- Help: {"action":"help","tool":"agena.web/search"}"#,
                "This command only inspects tool help; call the target tool directly to execute it.",
            ]
            .join("\n"),
        )
        .with_title("Tool catalog usage")
    }
}

pub(crate) fn new_plugin() -> WorkflowPlugin {
    WorkflowPlugin::new()
}

#[async_trait]
impl Plugin for WorkflowPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder(WORKFLOW_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description("Workflow orchestration tools.")
            .hooks(
                HookSubscription::TOOL_INVOKE
                    | HookSubscription::TOOL_BEFORE
                    | HookSubscription::COMMAND_BEFORE
                    | HookSubscription::AGENT_STOP,
            )
            .plugin_capability(HostCapability::AgentRegistry)
            .plugin_capability(HostCapability::PluginStorage)
            .plugin_capability(HostCapability::Statusline)
            .tools(tools())
            .config_schema(workflow_config_schema())
            .build()
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config = if ctx.config.is_null() {
            WorkflowPluginConfig::default()
        } else {
            serde_json::from_value(ctx.config)
                .map_err(|err| PluginError::new(format!("invalid workflow config: {err}")))?
        };
        self.config
            .set(config)
            .map_err(|_| PluginError::new("workflow plugin config already initialized"))?;
        self.workspace_root
            .set(ctx.workspace_root)
            .map_err(|_| PluginError::new("workflow plugin workspace root already initialized"))?;
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("workflow plugin host lock poisoned"))? = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        match input.tool_name.as_str() {
            "task" => {
                let (action, action_input) =
                    TaskToolActionInput::resolve_tool("task", input.input)?;
                match action.as_str() {
                    "run" => {
                        self.invoke_task(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown task action '{other}'"
                    ))),
                }
            }
            "tools" => {
                let (action, action_input) = resolve_tools_tool_input(input.input)?;
                match action.as_str() {
                    "search" => {
                        self.invoke_tool_search(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "help" => {
                        self.invoke_tool_help(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "usage" => Ok(Self::invoke_tool_catalog_usage()),
                    other => Err(PluginError::invalid_params(format!(
                        "unknown tools action '{other}'"
                    ))),
                }
            }
            "agent" => {
                let (action, action_input) = AgentToolInput::resolve_tool("agent", input.input)?;
                match action.as_str() {
                    "switch" => {
                        self.invoke_agent_switch(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "restore" => {
                        self.invoke_agent_restore(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown agent action '{other}'"
                    ))),
                }
            }
            "todo" => {
                let (action, action_input) = TodoToolInput::resolve_tool("todo", input.input)?;
                match action.as_str() {
                    "write" => {
                        let args: TodoWriteToolInput = serde_json::from_value(action_input)
                            .map_err(|err| PluginError::invalid_params(err.to_string()))?;
                        self.host()?
                            .todo_write(HostTodoWriteRequest {
                                items: args.items.iter().map(Self::host_todo_item).collect(),
                            })
                            .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown todo action '{other}'"
                    ))),
                }
            }
            "session" => {
                let (action, action_input) =
                    SessionToolInput::resolve_tool(input.tool_name.as_str(), input.input)?;
                match action.as_str() {
                    "get" => self.invoke_get_session().await,
                    "rename" => {
                        self.invoke_rename_session(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown session action '{other}'"
                    ))),
                }
            }
            "user" => {
                let (action, action_input) = UserToolInput::resolve_tool("user", input.input)?;
                match action.as_str() {
                    "request_input" => {
                        self.invoke_ask_user(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown user action '{other}'"
                    ))),
                }
            }
            "plan" => {
                let (action, action_input) = resolve_plan_tool_input(input.input)?;
                match action.as_str() {
                    "get" => self.invoke_plan_get().await,
                    "create" => {
                        self.invoke_plan_create(
                            input.session_id,
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "replace" => {
                        self.invoke_plan_replace(
                            input.session_id,
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "submit" => {
                        self.invoke_plan_submit(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "set_status" => {
                        self.invoke_plan_set_status(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "update_step" => {
                        self.invoke_plan_update_step(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "update_checkpoint" => {
                        self.invoke_plan_update_checkpoint(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "clear" => self.invoke_plan_clear().await,
                    "next" => self.invoke_plan_next().await,
                    other => Err(PluginError::invalid_params(format!(
                        "unknown plan action '{other}'"
                    ))),
                }
            }
            "worktree" => {
                let (action, action_input) =
                    WorktreeToolInput::resolve_tool("worktree", input.input)?;
                match action.as_str() {
                    "enter" => {
                        let args = parse_worktree_enter_input(action_input)?;
                        self.host()?
                            .enter_worktree(HostEnterWorktreeRequest {
                                name: args.name,
                                path: args.path,
                            })
                            .await
                    }
                    "exit" => {
                        let args = parse_worktree_exit_input(action_input)?;
                        self.host()?
                            .exit_worktree(HostExitWorktreeRequest {
                                action: args.action,
                                discard_changes: args.discard_changes,
                            })
                            .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown worktree action '{other}'"
                    ))),
                }
            }
            "workflow" => {
                let (action, action_input) =
                    WorkflowToolInput::resolve_tool("workflow", input.input)?;
                match action.as_str() {
                    "init" => {
                        self.invoke_provided_workflow(
                            "init",
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "review" => {
                        self.invoke_agent_workflow(
                            "review",
                            "reviewer",
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "security_review" => {
                        self.invoke_agent_workflow(
                            "security_review",
                            "reviewer",
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown workflow action '{other}'"
                    ))),
                }
            }
            other => Err(PluginError::invalid_params(format!(
                "unknown workflow plugin tool '{other}'"
            ))),
        }
    }

    async fn permission_paths(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        match tool_name {
            "worktree" => {
                let (action, action_input) =
                    WorktreeToolInput::resolve_tool("worktree", input.clone())?;
                if action != "enter" {
                    return Ok(Vec::new());
                }
                let input = parse_worktree_enter_input(action_input)?;
                if let Some(path) = input.path.filter(|path| !path.trim().is_empty()) {
                    return Ok(vec![
                        PathRequest::read(path.clone()),
                        PathRequest::write(path),
                    ]);
                }
                let worktrees_dir = crate::project_paths::project_state_dir(self.workspace_root()?)
                    .join("worktrees");
                Ok(vec![PathRequest::write(
                    worktrees_dir.to_string_lossy().to_string(),
                )])
            }
            _ => Ok(Vec::new()),
        }
    }

    async fn tool_execute_before(
        &self,
        input: ToolBeforeInput,
    ) -> SdkResult<Option<ToolBeforePatch>> {
        if Self::tool_allowed_during_planning(&input) {
            return Ok(None);
        }
        let Some(plan) = self.load_active_plan().await? else {
            return Ok(None);
        };
        if !Self::plan_lock_active(&plan) {
            return Ok(None);
        }
        Ok(Some(ToolBeforePatch {
            abort_reason: Some(
                "the active plan is still in planning; submit, approve, or clear the plan before using mutating tools"
                    .to_string(),
            ),
            ..ToolBeforePatch::default()
        }))
    }

    async fn command_execute_before(
        &self,
        input: CommandBeforeInput,
    ) -> SdkResult<Option<CommandBeforeResponse>> {
        let Some(_session_id) = input.session_id else {
            return Ok(None);
        };
        let Some(plan) = self.load_active_plan().await? else {
            return Ok(None);
        };
        let command_text = Self::command_text_for_policy(&input);
        if !Self::plan_lock_active(&plan) || Self::is_probably_read_only_shell(&command_text) {
            return Ok(None);
        }
        Ok(Some(CommandBeforeResponse::Abort {
            reason: "the active plan is still in planning; only read-only shell commands are allowed until the plan is approved or cleared".to_string(),
        }))
    }

    async fn agent_stop(
        &self,
        input: crate::plugin::AgentStopInput,
    ) -> SdkResult<Option<crate::plugin::AgentStopPatch>> {
        if input.stop_hook_active {
            return Ok(None);
        }
        let Some(plan) = self.load_active_plan().await? else {
            let _ = self.sync_plan_statusline(None).await;
            return Ok(None);
        };
        self.sync_plan_statusline(Some(&plan)).await?;
        if plan.phase != WorkflowPlanPhase::Active
            || plan.approval_state != WorkflowPlanApprovalState::Approved
            || !plan.auto_continue.enabled
        {
            return Ok(None);
        }
        let Some((step_index, step)) = Self::next_actionable_step(&plan) else {
            return Ok(None);
        };
        if step.executor != WorkflowPlanExecutor::Ai {
            return Ok(None);
        }
        if step
            .wait_until_ms
            .is_some_and(|wait_until_ms| wait_until_ms > Utc::now().timestamp_millis())
        {
            return Ok(None);
        }
        let signature = Self::plan_auto_signature(&plan, step_index, step);
        if self
            .load_auto_continue_signature()
            .await?
            .is_some_and(|current| current == signature)
        {
            return Ok(None);
        }
        self.save_auto_continue_signature(signature.as_str())
            .await?;
        Ok(Some(crate::plugin::AgentStopPatch {
            continue_with_message: Some(Self::auto_continue_prompt(&plan, step_index, step)),
            reason: Some("workflow plan auto-continue".to_string()),
        }))
    }

    async fn permission_networks(
        &self,
        _tool: &str,
        _input: &serde_json::Value,
    ) -> SdkResult<Vec<NetworkRequest>> {
        Ok(Vec::new())
    }
}

fn tags_summary(tags: &[String]) -> String {
    if tags.is_empty() {
        return "untagged".to_string();
    }
    tags.join(", ")
}

fn tools() -> Vec<PluginToolDecl> {
    vec![
        WorkflowToolInput::tool_decl(),
        ToolsToolInput::tool_decl(),
        TaskToolActionInput::tool_decl(),
        AgentToolInput::tool_decl(),
        TodoToolInput::tool_decl(),
        SessionToolInput::tool_decl(),
        UserToolInput::tool_decl(),
        PlanToolInput::tool_decl(),
        WorktreeToolInput::tool_decl(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct WorkflowTestHostClient {
        ask_user_reply: Mutex<Option<String>>,
        ask_user_request: Mutex<Option<crate::plugin::sdk::host_api::AskUserRequest>>,
        statusline: Mutex<BTreeMap<String, String>>,
        storage: Mutex<BTreeMap<(String, String, String, String), String>>,
    }

    impl WorkflowTestHostClient {
        fn new(ask_user_reply: Option<&str>) -> Self {
            Self {
                ask_user_reply: Mutex::new(ask_user_reply.map(str::to_string)),
                ..Self::default()
            }
        }

        fn storage_key(
            scope: crate::plugin::sdk::host_api::HostStorageScope,
            visibility: crate::plugin::sdk::host_api::HostStorageVisibility,
            namespace: &str,
            key: &str,
        ) -> (String, String, String, String) {
            let scope = match scope {
                crate::plugin::sdk::host_api::HostStorageScope::Session => "session",
                crate::plugin::sdk::host_api::HostStorageScope::Workspace => "workspace",
                crate::plugin::sdk::host_api::HostStorageScope::Global => "global",
            };
            let visibility = match visibility {
                crate::plugin::sdk::host_api::HostStorageVisibility::Private => "private",
                crate::plugin::sdk::host_api::HostStorageVisibility::Shared => "shared",
            };
            (
                scope.to_string(),
                visibility.to_string(),
                namespace.to_string(),
                key.to_string(),
            )
        }

        fn statusline_content(&self, segment_id: &str) -> Option<String> {
            self.statusline
                .lock()
                .expect("statusline lock")
                .get(segment_id)
                .cloned()
        }

        fn last_ask_user_request(&self) -> Option<crate::plugin::sdk::host_api::AskUserRequest> {
            self.ask_user_request
                .lock()
                .expect("ask_user_request lock")
                .clone()
        }
    }

    #[async_trait]
    impl HostClient for WorkflowTestHostClient {
        async fn log(
            &self,
            _level: crate::plugin::sdk::host_api::LogLevel,
            _message: String,
            _fields: serde_json::Value,
        ) {
        }

        async fn publish_event(
            &self,
            _env: crate::plugin::sdk::EventEnvelope,
        ) -> crate::plugin::sdk::Result<()> {
            Ok(())
        }

        async fn subscribe_events(
            &self,
            _filter: crate::plugin::sdk::EventFilter,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::host_api::EventSubscription> {
            Ok(crate::plugin::sdk::host_api::EventSubscription {
                id: "workflow-test-subscription".to_string(),
            })
        }

        async fn ask_permission(
            &self,
            _req: crate::plugin::sdk::PermissionAskInput,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::PermissionDecision> {
            Ok(crate::plugin::sdk::PermissionDecision::Allow)
        }

        async fn read_config(
            &self,
            _path: Option<String>,
        ) -> crate::plugin::sdk::Result<serde_json::Value> {
            Ok(json!({}))
        }

        async fn invoke_tool(
            &self,
            tool: String,
            _input: serde_json::Value,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
            Err(crate::plugin::PluginError::new(format!(
                "test host cannot invoke tool '{tool}'"
            )))
        }

        async fn ask_user(
            &self,
            req: crate::plugin::sdk::host_api::AskUserRequest,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::host_api::AskUserResponse> {
            *self.ask_user_request.lock().expect("ask_user_request lock") = Some(req);
            Ok(crate::plugin::sdk::host_api::AskUserResponse {
                reply: self
                    .ask_user_reply
                    .lock()
                    .expect("ask_user lock")
                    .clone()
                    .unwrap_or_else(|| PLAN_REVIEW_DECISION_APPROVE_RUN.to_string()),
                answers: BTreeMap::new(),
                cancelled: false,
            })
        }

        async fn storage_get(
            &self,
            req: crate::plugin::sdk::host_api::HostStorageGetRequest,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::host_api::HostStorageGetResponse>
        {
            let value = self
                .storage
                .lock()
                .expect("storage lock")
                .get(&Self::storage_key(
                    req.scope,
                    req.visibility,
                    req.namespace.as_str(),
                    req.key.as_str(),
                ))
                .cloned();
            Ok(crate::plugin::sdk::host_api::HostStorageGetResponse { value })
        }

        async fn storage_set(
            &self,
            req: crate::plugin::sdk::host_api::HostStorageSetRequest,
        ) -> crate::plugin::sdk::Result<()> {
            self.storage.lock().expect("storage lock").insert(
                Self::storage_key(
                    req.scope,
                    req.visibility,
                    req.namespace.as_str(),
                    req.key.as_str(),
                ),
                req.value,
            );
            Ok(())
        }

        async fn storage_delete(
            &self,
            req: crate::plugin::sdk::host_api::HostStorageDeleteRequest,
        ) -> crate::plugin::sdk::Result<()> {
            self.storage
                .lock()
                .expect("storage lock")
                .remove(&Self::storage_key(
                    req.scope,
                    req.visibility,
                    req.namespace.as_str(),
                    req.key.as_str(),
                ));
            Ok(())
        }

        async fn ui_statusline_contribute(
            &self,
            req: crate::plugin::sdk::host_api::HostStatuslineContributeRequest,
        ) -> crate::plugin::sdk::Result<()> {
            self.statusline
                .lock()
                .expect("statusline lock")
                .insert(req.segment_id, req.content);
            Ok(())
        }

        async fn ui_statusline_remove(
            &self,
            req: crate::plugin::sdk::host_api::HostStatuslineRemoveRequest,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::host_api::HostStatuslineRemoveResponse>
        {
            let removed = self
                .statusline
                .lock()
                .expect("statusline lock")
                .remove(&req.segment_id)
                .is_some();
            Ok(crate::plugin::sdk::host_api::HostStatuslineRemoveResponse { removed })
        }
    }

    fn init_test_plugin(
        default_auto_continue: bool,
        ask_user_reply: Option<&str>,
    ) -> (
        WorkflowPlugin,
        Arc<WorkflowTestHostClient>,
        tokio::runtime::Runtime,
    ) {
        let plugin = WorkflowPlugin::new();
        let host = Arc::new(WorkflowTestHostClient::new(ask_user_reply));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("workflow test runtime");
        runtime
            .block_on(Plugin::init(
                &plugin,
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: PathBuf::from("."),
                    plugin_id: WORKFLOW_PLUGIN_ID.to_string(),
                    host_callback_url: None,
                    host_callback_token: None,
                    config: json!({
                        "plan": {
                            "default_auto_continue": default_auto_continue
                        }
                    }),
                    protocol_version: 1,
                },
                host.clone(),
            ))
            .expect("workflow plugin should initialize");
        (plugin, host, runtime)
    }

    fn invoke_plan_result(
        runtime: &tokio::runtime::Runtime,
        plugin: &WorkflowPlugin,
        input: serde_json::Value,
    ) -> SdkResult<ToolInvokeOutput> {
        runtime.block_on(Plugin::tool_invoke(
            plugin,
            ToolInvokeInput {
                tool_name: "plan".to_string(),
                session_id: 7,
                call_id: 1,
                workspace_root: ".".to_string(),
                input,
            },
        ))
    }

    fn invoke_plan(
        runtime: &tokio::runtime::Runtime,
        plugin: &WorkflowPlugin,
        input: serde_json::Value,
    ) -> ToolInvokeOutput {
        invoke_plan_result(runtime, plugin, input).expect("plan tool invocation should succeed")
    }

    fn output_plan(output: &ToolInvokeOutput) -> WorkflowPlan {
        serde_json::from_value(
            output
                .payload
                .clone()
                .expect("plan output should include payload")["plan"]
                .clone(),
        )
        .expect("payload should contain a workflow plan")
    }

    #[test]
    fn tools_search_rejects_unknown_fields() {
        let err = ToolsToolInput::resolve_tool(
            "tools",
            serde_json::json!({
                "action": "search",
                "query": "memory",
                "backend": "legacy"
            }),
        )
        .expect_err("tools search should reject unknown fields");
        assert!(err.to_string().contains("unknown field `backend`"));

        let err = resolve_tools_tool_input(json!({
            "action": "search",
            "query": "memory",
            "backend": "legacy"
        }))
        .expect_err("tools resolver should preserve unknown-field rejection");
        assert!(err.to_string().contains("unknown field `backend`"));
    }

    #[test]
    fn tools_search_accepts_query_without_action() {
        let (action, action_input) = resolve_tools_tool_input(json!({
            "query": "web network",
            "limit": 3
        }))
        .expect("query-only tools input should infer search");

        assert_eq!(action, "search");
        let parsed: ToolSearchToolInput =
            serde_json::from_value(action_input).expect("search input");
        assert_eq!(parsed.query, "web network");
        assert_eq!(parsed.limit, Some(3));
    }

    #[test]
    fn tools_search_ignores_help_only_noise_fields() {
        let (action, action_input) = resolve_tools_tool_input(json!({
            "action": "search",
            "include_schema": false,
            "limit": 10,
            "query": "network tools web fetch search crawl",
            "tool": ""
        }))
        .expect("search should ignore known help-only fields");

        assert_eq!(action, "search");
        let parsed: ToolSearchToolInput =
            serde_json::from_value(action_input).expect("search input");
        assert_eq!(parsed.query, "network tools web fetch search crawl");
        assert_eq!(parsed.limit, Some(10));
    }

    #[test]
    fn tools_help_accepts_tool_name_without_action() {
        let (action, action_input) = resolve_tools_tool_input(json!({
            "tool_name": "agena.web/search",
            "include_schema": false
        }))
        .expect("tool_name-only tools input should infer help");

        assert_eq!(action, "help");
        let parsed: ToolsHelpInput = serde_json::from_value(action_input).expect("help input");
        assert_eq!(parsed.tool, "agena.web/search");
        assert_eq!(parsed.include_schema, Some(false));
    }

    #[test]
    fn tools_help_ignores_search_only_noise_fields() {
        let (action, action_input) = resolve_tools_tool_input(json!({
            "action": "help",
            "tool": "agena.web/search",
            "query": "web",
            "limit": 10,
            "include_schema": true
        }))
        .expect("help should ignore known search-only fields");

        assert_eq!(action, "help");
        let parsed: ToolsHelpInput = serde_json::from_value(action_input).expect("help input");
        assert_eq!(parsed.tool, "agena.web/search");
        assert_eq!(parsed.include_schema, Some(true));
    }

    #[test]
    fn tools_empty_input_returns_usage() {
        let (action, action_input) =
            resolve_tools_tool_input(json!({})).expect("empty tools input should return usage");

        assert_eq!(action, "usage");
        assert_eq!(action_input, json!({}));
    }

    #[test]
    fn workflow_plugin_config_accepts_nested_tool_catalog_defaults() {
        let config: WorkflowPluginConfig = serde_json::from_value(json!({
            "tool_catalog": {
                "search": {
                    "default_limit": 6,
                    "max_limit": 30
                },
                "help": {
                    "include_schema_by_default": false
                }
            }
        }))
        .expect("workflow config should parse");

        assert_eq!(config.tool_catalog.search.default_limit, 6);
        assert_eq!(config.tool_catalog.search.max_limit, 30);
        assert!(!config.tool_catalog.help.include_schema_by_default);
    }

    #[test]
    fn workflow_plugin_config_rejects_legacy_tool_search_shape() {
        let err = serde_json::from_value::<WorkflowPluginConfig>(json!({
            "tool_search": {
                "url": "https://example.com/catalog"
            }
        }))
        .expect_err("legacy workflow config should fail");

        assert!(err.to_string().contains("unknown field `tool_search`"));
    }

    #[test]
    fn allowlisted_workflow_tools_skip_plan_storage_probe_before_init() {
        let plugin = WorkflowPlugin::new();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("workflow test runtime");

        let patch = rt
            .block_on(Plugin::tool_execute_before(
                &plugin,
                ToolBeforeInput {
                    tool_name: "session".to_string(),
                    plugin_name: WORKFLOW_PLUGIN_ID.to_string(),
                    session_id: 1,
                    call_id: 1,
                    workspace_root: ".".to_string(),
                    tags: vec![ToolTag::ReadOnly],
                    input: json!({ "action": "get" }),
                    title_override: None,
                    metadata: Default::default(),
                },
            ))
            .expect("allowlisted workflow tool should not need plan storage");

        assert!(patch.is_none());
    }

    #[test]
    fn task_and_worktree_tools_declare_plan_storage_capability() {
        let task_decl = TaskToolActionInput::tool_decl();
        assert!(
            task_decl
                .host_capabilities
                .contains(&HostCapability::PluginStorage)
        );

        let worktree_decl = WorktreeToolInput::tool_decl();
        assert!(
            worktree_decl
                .host_capabilities
                .contains(&HostCapability::PluginStorage)
        );
    }

    #[test]
    fn plan_create_accepts_model_payload_when_step_title_falls_back_to_description() {
        let plugin = WorkflowPlugin::new();
        let (action, action_input) = resolve_plan_tool_input(json!({
            "action": "create",
            "auto_continue": false,
            "checkpoint_id": "cp1",
            "document_markdown": "# Plan: 尝试 plan 功能\n\n目标：演示并验证 plan 工具可用。",
            "note": "初次创建计划",
            "objective": "尝试一下 plan 功能",
            "phase": "draft",
            "status": "pending",
            "step_id": "step1",
            "steps": [
                {
                    "checkpoints": [
                        {
                            "id": "cp1",
                            "status": "pending",
                            "text": "创建一个最小可用计划"
                        }
                    ],
                    "description": "创建计划",
                    "executor": "ai",
                    "id": "step1",
                    "note": null,
                    "status": "pending",
                    "wait_until_ms": null
                }
            ],
            "summary": "最小计划创建演示",
            "wait_until_ms": null
        }))
        .expect("plan payload should parse");
        assert_eq!(action, "create");
        let args: PlanCreateInput =
            serde_json::from_value(action_input).expect("plan create input should deserialize");

        let plan = plugin
            .build_plan(
                2,
                args.objective.as_str(),
                args.title.as_deref(),
                args.document_markdown.as_deref(),
                args.steps.as_slice(),
                args.auto_continue,
                None,
            )
            .expect("plan should build");

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].title, "创建计划");
        assert_eq!(plan.steps[0].description, "创建计划");
    }

    #[test]
    fn plan_create_accepts_step_text_and_checkpoint_title_aliases() {
        let plugin = WorkflowPlugin::new();
        let (action, action_input) = resolve_plan_tool_input(json!({
            "action": "create",
            "objective": "Exercise mixed legacy field names.",
            "auto_continue": false,
            "steps": [
                {
                    "id": "step_1",
                    "text": "创建计划",
                    "executor": "ai",
                    "checkpoints": [
                        {
                            "id": "cp_1",
                            "title": "检查计划内容"
                        }
                    ]
                }
            ]
        }))
        .expect("plan payload should parse");
        assert_eq!(action, "create");
        let args: PlanCreateInput =
            serde_json::from_value(action_input).expect("plan create input should deserialize");

        let plan = plugin
            .build_plan(
                2,
                args.objective.as_str(),
                args.title.as_deref(),
                args.document_markdown.as_deref(),
                args.steps.as_slice(),
                args.auto_continue,
                None,
            )
            .expect("plan should build");

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].title, "创建计划");
        assert_eq!(plan.steps[0].checkpoints.len(), 1);
        assert_eq!(plan.steps[0].checkpoints[0].text, "检查计划内容");
    }

    #[test]
    fn resolve_plan_tool_input_normalizes_legacy_status_actions() {
        let (status_action, status_input) = resolve_plan_tool_input(json!({
            "action": "set_status",
            "status": "active",
            "auto_continue": true
        }))
        .expect("set_status should continue accepting legacy status field");
        assert_eq!(status_action, "set_status");
        let status_args: PlanSetStatusInput =
            serde_json::from_value(status_input).expect("status input should deserialize");
        assert_eq!(status_args.phase, Some(WorkflowPlanPhase::Active));
        assert_eq!(status_args.auto_continue, Some(true));

        let (complete_action, complete_input) = resolve_plan_tool_input(json!({
            "action": "complete",
            "summary": "done"
        }))
        .expect("legacy complete should normalize");
        assert_eq!(complete_action, "set_status");
        let complete_args: PlanSetStatusInput =
            serde_json::from_value(complete_input).expect("complete input should deserialize");
        assert_eq!(complete_args.phase, Some(WorkflowPlanPhase::Completed));
        assert_eq!(complete_args.summary.as_deref(), Some("done"));

        let (restore_action, restore_input) = resolve_plan_tool_input(json!({
            "action": "restore",
            "phase": "paused",
            "auto_continue": true
        }))
        .expect("legacy restore should normalize");
        assert_eq!(restore_action, "set_status");
        let restore_args: PlanSetStatusInput =
            serde_json::from_value(restore_input).expect("restore input should deserialize");
        assert_eq!(restore_args.phase, Some(WorkflowPlanPhase::Active));
        assert_eq!(restore_args.auto_continue, Some(true));

        let (runtime_action, runtime_input) = resolve_plan_tool_input(json!({
            "action": "update_runtime",
            "auto_continue": false
        }))
        .expect("legacy update_runtime should normalize");
        assert_eq!(runtime_action, "set_status");
        let runtime_args: PlanSetStatusInput =
            serde_json::from_value(runtime_input).expect("runtime input should deserialize");
        assert_eq!(runtime_args.phase, None);
        assert_eq!(runtime_args.auto_continue, Some(false));

        let execution_args: PlanSetStatusInput =
            serde_json::from_value(json!({ "phase": "execution" }))
                .expect("execution alias should deserialize");
        assert_eq!(execution_args.phase, Some(WorkflowPlanPhase::Active));
    }

    fn schema_string_literals(
        root: &serde_json::Value,
        value: &serde_json::Value,
    ) -> std::collections::BTreeSet<String> {
        fn collect(
            root: &serde_json::Value,
            value: &serde_json::Value,
            visited_refs: &mut std::collections::BTreeSet<String>,
        ) -> std::collections::BTreeSet<String> {
            match value {
                serde_json::Value::Object(object) => {
                    let mut literals = std::collections::BTreeSet::new();
                    if let Some(values) = object.get("enum").and_then(serde_json::Value::as_array) {
                        literals.extend(
                            values
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .map(ToOwned::to_owned),
                        );
                    }
                    if let Some(value) = object.get("const").and_then(serde_json::Value::as_str) {
                        literals.insert(value.to_owned());
                    }
                    if let Some(reference) = object.get("$ref").and_then(serde_json::Value::as_str)
                    {
                        if let Some(pointer) = reference.strip_prefix('#')
                            && visited_refs.insert(reference.to_owned())
                            && let Some(target) = root.pointer(pointer)
                        {
                            literals.extend(collect(root, target, visited_refs));
                        }
                    }
                    for nested in object.values() {
                        literals.extend(collect(root, nested, visited_refs));
                    }
                    literals
                }
                serde_json::Value::Array(values) => values
                    .iter()
                    .flat_map(|nested| collect(root, nested, visited_refs))
                    .collect(),
                _ => Default::default(),
            }
        }

        collect(root, value, &mut Default::default())
    }

    #[test]
    fn model_safe_plan_schema_keeps_phase_distinct_from_step_status() {
        let safe_schema = crate::tool::model_safe_tool_schema(
            &PlanToolInput::tool_decl().sanitized_input_schema(),
        );
        let properties = safe_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("plan schema should expose properties");
        let phase_literals = properties
            .get("phase")
            .map(|value| schema_string_literals(&safe_schema, value))
            .expect("plan schema should expose phase");
        let status_literals = properties
            .get("status")
            .map(|value| schema_string_literals(&safe_schema, value))
            .expect("plan schema should expose step status");

        assert!(phase_literals.contains("active"));
        assert!(phase_literals.contains("completed"));
        assert!(!phase_literals.contains("awaiting_review"));
        assert!(!phase_literals.contains("executing"));
        assert!(!phase_literals.contains("paused"));
        assert!(!phase_literals.contains("execution"));
        assert!(status_literals.contains("pending"));
        assert!(status_literals.contains("in_progress"));
        assert!(!status_literals.contains("active"));
        assert!(!status_literals.contains("cancelled"));
    }

    #[test]
    fn workflow_plan_markdown_prefers_document_markdown_and_omits_redundant_metadata() {
        let plan = WorkflowPlan {
            title: "Demonstrate creating a plan".to_string(),
            objective: "Demonstrate creating a plan".to_string(),
            document_markdown: "# Plan Demo\n\nUser requested to try the plan tool.".to_string(),
            auto_continue: WorkflowPlanAutoContinue {
                enabled: false,
                source: "tool_override".to_string(),
            },
            ..WorkflowPlan::default()
        };

        let rendered = WorkflowPlugin::workflow_plan_markdown(&plan);

        assert!(rendered.starts_with("# Plan Demo"));
        assert!(rendered.contains("_Auto-continue: off_"));
        assert!(!rendered.contains("# Demonstrate creating a plan"));
        assert!(!rendered.contains("Objective:"));
        assert!(!rendered.contains("Phase:"));
    }

    #[test]
    fn workflow_plan_markdown_avoids_repeating_fallback_step_descriptions() {
        let plan = WorkflowPlan {
            title: "Plan Demo".to_string(),
            objective: "Plan Demo".to_string(),
            auto_continue: WorkflowPlanAutoContinue {
                enabled: true,
                source: "config_default".to_string(),
            },
            steps: vec![WorkflowPlanStep {
                id: "step_1".to_string(),
                title: "Create a simple demo plan".to_string(),
                description: "Create a simple demo plan".to_string(),
                executor: WorkflowPlanExecutor::Ai,
                status: WorkflowPlanStepStatus::Pending,
                wait_until_ms: None,
                note: "Initial demonstration step".to_string(),
                checkpoints: vec![WorkflowPlanCheckpoint {
                    id: "cp_1".to_string(),
                    text: "Create a demo plan".to_string(),
                    status: WorkflowPlanStepStatus::Pending,
                }],
            }],
            ..WorkflowPlan::default()
        };

        let rendered = WorkflowPlugin::workflow_plan_markdown(&plan);

        assert!(rendered.contains("1. [ ] Create a simple demo plan (ai)"));
        assert!(rendered.contains("   - [ ] Create a demo plan"));
        assert!(rendered.contains("   - Note: Initial demonstration step"));
        assert_eq!(rendered.matches("Create a simple demo plan").count(), 1);
        assert!(!rendered.contains("Details: Create a simple demo plan"));
    }

    #[test]
    fn plan_submit_uses_review_question_options_with_descriptions() {
        let (plugin, host, runtime) =
            init_test_plugin(false, Some(PLAN_REVIEW_DECISION_KEEP_PLANNING));

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "create",
                "objective": "Review the plan.",
                "title": "Reviewable Plan",
                "auto_continue": false,
                "steps": [
                    {
                        "id": "step_review",
                        "title": "Review the draft",
                        "executor": "ai"
                    }
                ]
            }),
        );

        invoke_plan(&runtime, &plugin, json!({ "action": "submit" }));
        let request = host
            .last_ask_user_request()
            .expect("submit should issue a host review request");
        assert_eq!(request.kind, "review");
        assert!(request.prompt.is_empty());
        assert!(request.options.is_empty());
        assert_eq!(request.questions.len(), 1);
        assert_eq!(request.questions[0].id, "decision");
        assert!(
            request.questions[0]
                .options
                .iter()
                .any(|option| option.label == PLAN_REVIEW_DECISION_CANCELLED
                    && !option.description.trim().is_empty())
        );
    }

    #[test]
    fn plan_tool_flow_blocks_premature_completion_and_reports_status() {
        let (plugin, host, runtime) = init_test_plugin(false, None);

        let created = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "create",
                "objective": "Test plan creation and retrieval.",
                "title": "Plan Trial",
                "document_markdown": "# Plan Trial\n\n- Test plan creation and retrieval.\n- No code changes required.",
                "auto_continue": false,
                "steps": [
                    {
                        "id": "step_trial",
                        "title": "Create a minimal draft plan.",
                        "executor": "ai",
                        "note": "Initial trial step",
                        "checkpoints": [
                            {
                                "id": "cp_trial",
                                "text": "Create a minimal draft plan."
                            }
                        ]
                    }
                ]
            }),
        );
        assert!(created.output_text.contains(
            "Status: draft | Steps: 0/1 complete | Checkpoints: 0/1 complete | Auto-continue: off"
        ));
        assert_eq!(
            host.statusline_content(PLAN_STATUSLINE_SEGMENT_ID)
                .as_deref(),
            Some("plan:draft steps:0/1 auto:off")
        );

        let completion_err = invoke_plan_result(
            &runtime,
            &plugin,
            json!({
                "action": "complete",
                "summary": "Minimal trial plan"
            }),
        )
        .expect_err("plan completion should fail while work remains");
        assert!(completion_err.to_string().contains(
            "cannot complete plan: step 1 ('Create a minimal draft plan.') is still pending"
        ));

        let runtime_completion_err = invoke_plan_result(
            &runtime,
            &plugin,
            json!({
                "action": "update_runtime",
                "phase": "completed"
            }),
        )
        .expect_err("runtime completion should fail while work remains");
        assert!(runtime_completion_err.to_string().contains(
            "cannot complete plan: step 1 ('Create a minimal draft plan.') is still pending"
        ));

        let updated = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update_step",
                "step_id": "step_trial",
                "status": "completed"
            }),
        );
        let updated_plan = output_plan(&updated);
        assert_eq!(
            updated_plan.steps[0].status,
            WorkflowPlanStepStatus::Completed
        );
        assert_eq!(
            updated_plan.steps[0].checkpoints[0].status,
            WorkflowPlanStepStatus::Completed
        );
        assert!(updated.output_text.contains(
            "Status: draft | Steps: 1/1 complete | Checkpoints: 1/1 complete | Auto-continue: off"
        ));
        assert!(
            updated
                .output_text
                .contains("1. [x] Create a minimal draft plan. (ai)")
        );
        assert!(
            updated
                .output_text
                .contains("- [x] Create a minimal draft plan.")
        );

        let completed = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "complete",
                "summary": "Minimal trial plan"
            }),
        );
        let completed_plan = output_plan(&completed);
        assert_eq!(completed_plan.phase, WorkflowPlanPhase::Completed);
        assert_eq!(
            completed_plan.approval_state,
            WorkflowPlanApprovalState::Approved
        );
        assert!(completed.output_text.contains("Status: completed | Steps: 1/1 complete | Checkpoints: 1/1 complete | Auto-continue: off"));
        assert!(completed.output_text.contains("## Completion Summary"));
        assert_eq!(
            host.statusline_content(PLAN_STATUSLINE_SEGMENT_ID)
                .as_deref(),
            Some("plan:completed steps:1/1 auto:off")
        );

        let fetched = invoke_plan(&runtime, &plugin, json!({ "action": "get" }));
        assert!(fetched.output_text.contains("Status: completed | Steps: 1/1 complete | Checkpoints: 1/1 complete | Auto-continue: off"));

        let next = invoke_plan(&runtime, &plugin, json!({ "action": "next" }));
        assert!(
            next.output_text
                .contains("The active plan has no remaining actionable steps.")
        );
        assert!(next.payload.expect("next output should include payload")["next_step"].is_null());
    }

    #[test]
    fn plan_update_checkpoint_accepts_checkpoint_index_hints() {
        let (plugin, _host, runtime) = init_test_plugin(false, None);

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "create",
                "objective": "Exercise checkpoint matching.",
                "title": "Checkpoint Plan",
                "auto_continue": false,
                "steps": [
                    {
                        "id": "step_review",
                        "title": "Review the plan",
                        "executor": "ai",
                        "checkpoints": [
                            {
                                "id": "checkpoint_review",
                                "text": "Look over the draft"
                            }
                        ]
                    }
                ]
            }),
        );

        let updated = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update_checkpoint",
                "step_id": "step1",
                "checkpoint_id": "cp1",
                "status": "completed"
            }),
        );
        let plan = output_plan(&updated);
        assert_eq!(
            plan.steps[0].checkpoints[0].status,
            WorkflowPlanStepStatus::Completed
        );
        assert_eq!(plan.steps[0].status, WorkflowPlanStepStatus::Completed);
    }

    #[test]
    fn plan_set_status_ignores_summary_until_completed() {
        let (plugin, _host, runtime) = init_test_plugin(false, None);

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "create",
                "objective": "Exercise summary tolerance.",
                "title": "Summary Plan",
                "auto_continue": false,
                "steps": [
                    {
                        "id": "step_summary",
                        "title": "Finish the work",
                        "executor": "ai"
                    }
                ]
            }),
        );

        let blocked = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set_status",
                "phase": "blocked",
                "summary": "This should be ignored."
            }),
        );
        let plan = output_plan(&blocked);
        assert_eq!(plan.phase, WorkflowPlanPhase::Blocked);
        assert!(!plan.document_markdown.contains("Completion Summary"));
    }

    #[test]
    fn plan_cancel_and_restore_actions_switch_phase_flexibly() {
        let (plugin, host, runtime) = init_test_plugin(false, None);

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "create",
                "objective": "Exercise plan cancellation.",
                "title": "Cancelable Plan",
                "auto_continue": false,
                "steps": [
                    {
                        "id": "step_cancel",
                        "title": "Pause the plan lifecycle",
                        "executor": "ai"
                    }
                ]
            }),
        );

        let cancelled = invoke_plan(&runtime, &plugin, json!({ "action": "cancel" }));
        let cancelled_plan = output_plan(&cancelled);
        assert_eq!(cancelled_plan.phase, WorkflowPlanPhase::Cancelled);
        assert_eq!(
            cancelled_plan.approval_state,
            WorkflowPlanApprovalState::NotRequested
        );
        assert!(
            cancelled
                .output_text
                .contains("Status: cancelled | Steps: 0/1 complete | Auto-continue: off")
        );
        assert_eq!(
            host.statusline_content(PLAN_STATUSLINE_SEGMENT_ID)
                .as_deref(),
            Some("plan:cancelled steps:0/1 auto:off")
        );

        let next = invoke_plan(&runtime, &plugin, json!({ "action": "next" }));
        assert!(
            next.output_text
                .contains("The active plan has no remaining actionable steps.")
        );

        let restored = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "restore",
                "phase": "paused",
                "auto_continue": true
            }),
        );
        let restored_plan = output_plan(&restored);
        assert_eq!(restored_plan.phase, WorkflowPlanPhase::Active);
        assert_eq!(
            restored_plan.approval_state,
            WorkflowPlanApprovalState::Approved
        );
        assert!(restored_plan.auto_continue.enabled);
        assert!(
            restored
                .output_text
                .contains("Status: active | Steps: 0/1 complete | Auto-continue: on")
        );
        assert_eq!(
            host.statusline_content(PLAN_STATUSLINE_SEGMENT_ID)
                .as_deref(),
            Some("plan:active steps:0/1 auto:on")
        );

        let reset_to_draft = invoke_plan(&runtime, &plugin, json!({ "action": "restore" }));
        let reset_plan = output_plan(&reset_to_draft);
        assert_eq!(reset_plan.phase, WorkflowPlanPhase::Draft);
        assert_eq!(
            reset_plan.approval_state,
            WorkflowPlanApprovalState::NotRequested
        );

        let completed_cancel = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "cancel"
            }),
        );
        let completed_cancelled_plan = output_plan(&completed_cancel);
        assert_eq!(completed_cancelled_plan.phase, WorkflowPlanPhase::Cancelled);
    }

    #[test]
    fn plan_restore_reopens_completed_plans() {
        let (plugin, _host, runtime) = init_test_plugin(false, None);

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "create",
                "objective": "Exercise reopening completed plans.",
                "title": "Reopenable Plan",
                "auto_continue": false,
                "steps": [
                    {
                        "id": "step_done",
                        "title": "Finish the only step",
                        "executor": "ai"
                    }
                ]
            }),
        );

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update_step",
                "step_id": "step_done",
                "status": "completed"
            }),
        );

        let completed = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "complete",
                "summary": "Everything finished."
            }),
        );
        let completed_plan = output_plan(&completed);
        assert_eq!(completed_plan.phase, WorkflowPlanPhase::Completed);
        assert!(completed_plan.completed_at_ms.is_some());

        let restored = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "restore",
                "phase": "draft"
            }),
        );
        let restored_plan = output_plan(&restored);
        assert_eq!(restored_plan.phase, WorkflowPlanPhase::Draft);
        assert_eq!(
            restored_plan.approval_state,
            WorkflowPlanApprovalState::NotRequested
        );
        assert_eq!(restored_plan.completed_at_ms, None);
        assert!(
            restored
                .output_text
                .contains("Status: draft | Steps: 1/1 complete | Auto-continue: off")
        );

        let blocked_err = invoke_plan_result(
            &runtime,
            &plugin,
            json!({
                "action": "set_status",
                "status": "blocked"
            }),
        )
        .expect_err("active phases should require remaining incomplete work");
        assert!(blocked_err.to_string().contains(
            "cannot set plan status to blocked: all steps and checkpoints are already complete"
        ));

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update_step",
                "step_id": "step_done",
                "status": "pending"
            }),
        );

        let blocked = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set_status",
                "status": "blocked"
            }),
        );
        let blocked_plan = output_plan(&blocked);
        assert_eq!(blocked_plan.phase, WorkflowPlanPhase::Blocked);
        assert_eq!(
            blocked_plan.approval_state,
            WorkflowPlanApprovalState::Approved
        );
    }

    #[test]
    fn approved_active_plan_auto_continue_runs_once() {
        let (plugin, _host, runtime) =
            init_test_plugin(true, Some(PLAN_REVIEW_DECISION_APPROVE_RUN));

        let created = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "create",
                "objective": "Verify auto-continue.",
                "title": "Auto Plan",
                "steps": [
                    {
                        "id": "step_auto",
                        "title": "Execute the next AI step",
                        "executor": "ai"
                    }
                ]
            }),
        );
        assert!(created.output_text.contains("Auto-continue: on"));

        let submitted = invoke_plan(&runtime, &plugin, json!({ "action": "submit" }));
        let submitted_plan = output_plan(&submitted);
        assert_eq!(submitted_plan.phase, WorkflowPlanPhase::Active);
        assert_eq!(
            submitted_plan.approval_state,
            WorkflowPlanApprovalState::Approved
        );
        assert!(submitted_plan.auto_continue.enabled);

        let first_patch = runtime
            .block_on(Plugin::agent_stop(
                &plugin,
                crate::plugin::sdk::AgentStopInput {
                    session_id: 7,
                    stop_hook_active: false,
                    last_assistant_message: Some("done".to_string()),
                },
            ))
            .expect("agent_stop should succeed")
            .expect("auto-continue should continue once");
        assert_eq!(
            first_patch.reason.as_deref(),
            Some("workflow plan auto-continue")
        );
        assert!(
            first_patch
                .continue_with_message
                .expect("auto-continue message")
                .contains("Current step 1: Execute the next AI step")
        );

        let second_patch = runtime
            .block_on(Plugin::agent_stop(
                &plugin,
                crate::plugin::sdk::AgentStopInput {
                    session_id: 7,
                    stop_hook_active: false,
                    last_assistant_message: Some("done".to_string()),
                },
            ))
            .expect("agent_stop should succeed");
        assert!(second_patch.is_none());
    }

    #[test]
    fn approved_plan_with_auto_continue_off_does_not_continue() {
        let (plugin, _host, runtime) =
            init_test_plugin(true, Some(PLAN_REVIEW_DECISION_APPROVE_PAUSE));

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "create",
                "objective": "Verify manual active plan handling.",
                "title": "Auto Plan Off",
                "steps": [
                    {
                        "id": "step_auto_off",
                        "title": "Wait for manual continuation",
                        "executor": "ai"
                    }
                ]
            }),
        );
        let submitted = invoke_plan(&runtime, &plugin, json!({ "action": "submit" }));
        let submitted_plan = output_plan(&submitted);
        assert_eq!(submitted_plan.phase, WorkflowPlanPhase::Active);
        assert!(!submitted_plan.auto_continue.enabled);

        let patch = runtime
            .block_on(Plugin::agent_stop(
                &plugin,
                crate::plugin::sdk::AgentStopInput {
                    session_id: 7,
                    stop_hook_active: false,
                    last_assistant_message: Some("done".to_string()),
                },
            ))
            .expect("agent_stop should succeed");
        assert!(patch.is_none());
    }
}
