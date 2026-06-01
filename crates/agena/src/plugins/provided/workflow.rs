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
    host_capabilities(HostCapability::SpawnSubtask),
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
    Draft,
    AwaitingReview,
    Executing,
    Paused,
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
struct WorkflowPlanCheckpointInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<WorkflowPlanStepStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct WorkflowPlanStepInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    executor: WorkflowPlanExecutor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<WorkflowPlanStepStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wait_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    checkpoints: Vec<WorkflowPlanCheckpointInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlanCreateInput {
    objective: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    document_markdown: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    steps: Vec<WorkflowPlanStepInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_continue: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlanReplaceInput {
    objective: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    document_markdown: Option<String>,
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
struct PlanUpdateRuntimeInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    phase: Option<WorkflowPlanPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_continue: Option<bool>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct PlanCompleteInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "plan",
    description = "Plan command backed by shared plugin storage. Use it to create, replace, review, execute, and clear the active session plan.",
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
    #[tool(exec = "update_runtime")]
    UpdateRuntime {
        #[serde(flatten)]
        args: PlanUpdateRuntimeInput,
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
    #[tool(exec = "complete")]
    Complete {
        #[serde(flatten)]
        args: PlanCompleteInput,
    },
    #[tool(exec = "clear")]
    Clear,
    #[tool(exec = "next")]
    Next,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "worktree",
    description = "Worktree command. Use action `enter` or `exit`; `enter` uses `target = new|existing` to create or attach to a git worktree and `exit` uses enum `exit_action = keep|remove`.",
    tags(ToolTag::Mutating, ToolTag::FilesystemWrite, ToolTag::Worktree),
    host_capabilities(HostCapability::WorktreeRegistry),
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
            if title.is_empty() {
                return Err(PluginError::invalid_params(format!(
                    "plan step {} requires a non-empty title",
                    step_index + 1
                )));
            }
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
                title: title.to_string(),
                description: step.description.trim().to_string(),
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
            WorkflowPlanPhase::AwaitingReview => "awaiting_review",
            WorkflowPlanPhase::Executing => "executing",
            WorkflowPlanPhase::Paused => "paused",
            WorkflowPlanPhase::Blocked => "blocked",
            WorkflowPlanPhase::Completed => "completed",
            WorkflowPlanPhase::Cancelled => "cancelled",
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

    fn workflow_plan_markdown(plan: &WorkflowPlan) -> String {
        let mut sections = vec![
            format!("# {}", plan.title),
            String::new(),
            format!("Objective: {}", plan.objective),
            format!("Phase: {}", Self::plan_phase_label(plan.phase)),
            format!(
                "Auto-continue: {}",
                if plan.auto_continue.enabled {
                    "on"
                } else {
                    "off"
                }
            ),
        ];
        if !plan.document_markdown.trim().is_empty() {
            sections.push(String::new());
            sections.push(plan.document_markdown.trim().to_string());
        }
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
                if !step.description.trim().is_empty() {
                    sections.push(format!("   {}", step.description.trim()));
                }
                for checkpoint in &step.checkpoints {
                    sections.push(format!(
                        "   - {} {}",
                        Self::step_status_marker(checkpoint.status),
                        checkpoint.text
                    ));
                }
                if !step.note.trim().is_empty() {
                    sections.push(format!("   note: {}", step.note.trim()));
                }
            }
        }
        sections.join("\n")
    }

    fn plan_statusline_content(plan: &WorkflowPlan) -> String {
        let total_steps = plan.steps.len();
        let completed_steps = plan
            .steps
            .iter()
            .filter(|step| {
                matches!(
                    step.status,
                    WorkflowPlanStepStatus::Completed | WorkflowPlanStepStatus::Skipped
                )
            })
            .count();
        if total_steps == 0 {
            return format!("plan:{}", Self::plan_phase_label(plan.phase));
        }
        format!(
            "plan:{}/{} {} {}",
            completed_steps,
            total_steps,
            Self::plan_phase_label(plan.phase),
            if plan.auto_continue.enabled {
                "auto:on"
            } else {
                "auto:off"
            }
        )
    }

    fn next_actionable_step(plan: &WorkflowPlan) -> Option<(usize, &WorkflowPlanStep)> {
        plan.steps.iter().enumerate().find(|(_, step)| {
            !matches!(
                step.status,
                WorkflowPlanStepStatus::Completed | WorkflowPlanStepStatus::Skipped
            )
        })
    }

    fn plan_summary_text(plan: &WorkflowPlan) -> String {
        format!(
            "Plan '{}' [{} | auto:{}]",
            plan.title,
            Self::plan_phase_label(plan.phase),
            if plan.auto_continue.enabled {
                "on"
            } else {
                "off"
            }
        )
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
        matches!(
            plan.phase,
            WorkflowPlanPhase::Draft | WorkflowPlanPhase::AwaitingReview
        )
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
        Ok(ToolInvokeOutput::text(format!(
            "Created a draft plan.\n\n{}",
            Self::workflow_plan_markdown(&plan)
        ))
        .with_title("plan")
        .with_payload(payload))
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
        Ok(ToolInvokeOutput::text(format!(
            "Replaced the active plan and returned it to draft.\n\n{}",
            Self::workflow_plan_markdown(&plan)
        ))
        .with_title("plan")
        .with_payload(payload))
    }

    async fn invoke_plan_submit(&self, _input: &PlanSubmitInput) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params("no active plan to submit"));
        };
        let now_ms = Utc::now().timestamp_millis();
        plan.phase = WorkflowPlanPhase::AwaitingReview;
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
                questions: Vec::new(),
                prompt: "Plan decision".to_string(),
                options: vec![
                    PLAN_REVIEW_DECISION_APPROVE_RUN.to_string(),
                    PLAN_REVIEW_DECISION_APPROVE_PAUSE.to_string(),
                    PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string(),
                    PLAN_REVIEW_DECISION_REJECT.to_string(),
                ],
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
                plan.phase = WorkflowPlanPhase::Executing;
                plan.approval_state = WorkflowPlanApprovalState::Approved;
            }
            PLAN_REVIEW_DECISION_APPROVE_PAUSE => {
                plan.phase = WorkflowPlanPhase::Executing;
                plan.approval_state = WorkflowPlanApprovalState::Approved;
                plan.auto_continue.enabled = false;
                plan.auto_continue.source = "review_override".to_string();
            }
            PLAN_REVIEW_DECISION_REJECT => {
                plan.phase = WorkflowPlanPhase::Draft;
                plan.approval_state = WorkflowPlanApprovalState::Rejected;
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
        Ok(ToolInvokeOutput::text(format!(
            "Plan review decision: {}.\n\n{}",
            decision,
            Self::workflow_plan_markdown(&plan)
        ))
        .with_title("plan review")
        .with_payload(payload))
    }

    async fn invoke_plan_update_runtime(
        &self,
        input: &PlanUpdateRuntimeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params("no active plan to update"));
        };
        if let Some(phase) = input.phase {
            plan.phase = phase;
            if matches!(phase, WorkflowPlanPhase::Completed) {
                plan.completed_at_ms = Some(Utc::now().timestamp_millis());
            }
        }
        if let Some(auto_continue) = input.auto_continue {
            plan.auto_continue.enabled = auto_continue;
            plan.auto_continue.source = "tool_override".to_string();
        }
        plan.updated_at_ms = Utc::now().timestamp_millis();
        self.save_active_plan(&plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::text(format!(
            "Updated plan runtime.\n\n{}",
            Self::workflow_plan_markdown(&plan)
        ))
        .with_title("plan")
        .with_payload(payload))
    }

    async fn invoke_plan_update_step(
        &self,
        input: &PlanUpdateStepInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params("no active plan to update"));
        };
        let Some(step) = plan.steps.iter_mut().find(|step| step.id == input.step_id) else {
            return Err(PluginError::invalid_params(format!(
                "unknown plan step '{}'",
                input.step_id
            )));
        };
        step.status = input.status;
        step.wait_until_ms = input.wait_until_ms;
        if let Some(note) = input.note.as_deref() {
            step.note = note.trim().to_string();
        }
        let step_title = step.title.clone();
        plan.updated_at_ms = Utc::now().timestamp_millis();
        self.save_active_plan(&plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::text(format!(
            "Updated step '{}'.\n\n{}",
            step_title,
            Self::workflow_plan_markdown(&plan)
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
        let Some(step) = plan.steps.iter_mut().find(|step| step.id == input.step_id) else {
            return Err(PluginError::invalid_params(format!(
                "unknown plan step '{}'",
                input.step_id
            )));
        };
        let Some(checkpoint) = step
            .checkpoints
            .iter_mut()
            .find(|checkpoint| checkpoint.id == input.checkpoint_id)
        else {
            return Err(PluginError::invalid_params(format!(
                "unknown checkpoint '{}' for step '{}'",
                input.checkpoint_id, input.step_id
            )));
        };
        checkpoint.status = input.status;
        let checkpoint_text = checkpoint.text.clone();
        plan.updated_at_ms = Utc::now().timestamp_millis();
        self.save_active_plan(&plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::text(format!(
            "Updated checkpoint '{}'.\n\n{}",
            checkpoint_text,
            Self::workflow_plan_markdown(&plan)
        ))
        .with_title("plan")
        .with_payload(payload))
    }

    async fn invoke_plan_complete(&self, input: &PlanCompleteInput) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params("no active plan to complete"));
        };
        plan.phase = WorkflowPlanPhase::Completed;
        plan.approval_state = WorkflowPlanApprovalState::Approved;
        plan.completed_at_ms = Some(Utc::now().timestamp_millis());
        if let Some(summary) = input
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if plan.document_markdown.trim().is_empty() {
                plan.document_markdown = format!("## Completion Summary\n\n{summary}");
            } else {
                plan.document_markdown = format!(
                    "{}\n\n## Completion Summary\n\n{summary}",
                    plan.document_markdown.trim()
                );
            }
        }
        plan.updated_at_ms = Utc::now().timestamp_millis();
        self.save_active_plan(&plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(ToolInvokeOutput::text(format!(
            "Marked the plan complete.\n\n{}",
            Self::workflow_plan_markdown(&plan)
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
                "Next step {} is '{}' ({:?}).",
                index + 1,
                step.title,
                step.executor
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
                let (action, action_input) = PlanToolInput::resolve_tool("plan", input.input)?;
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
                    "update_runtime" => {
                        self.invoke_plan_update_runtime(
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
                    "complete" => {
                        self.invoke_plan_complete(
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
        let Some(plan) = self.load_active_plan().await? else {
            return Ok(None);
        };
        if !Self::plan_lock_active(&plan) || Self::tool_allowed_during_planning(&input) {
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
        if plan.phase != WorkflowPlanPhase::Executing
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
    use serde_json::json;

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
}
