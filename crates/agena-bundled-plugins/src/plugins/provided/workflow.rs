//! Shared implementation for the agent/tools/interaction/planning/repo/session/tasks plugins.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::message::{AskUserToolInput, TaskAccess, TaskToolInput};
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{
    AskUserOption as HostAskUserOption, AskUserQuestion as HostAskUserQuestion, AskUserRequest,
    HostClient, HostDisplayContributeRequest, HostDisplayRemoveRequest, HostEnterSnapshotRequest,
    HostExitSnapshotRequest, HostGetSessionRequest, HostRegisteredToolDescriptor,
    HostRenameSessionRequest, HostSession, HostStorageDeleteRequest, HostStorageGetRequest,
    HostStorageScope, HostStorageSetRequest, HostStorageVisibility, RunSubtaskAccess,
    RunSubtaskModelSelection, RunSubtaskRequest, RunSubtaskStatus, ToolDescriptor,
};
use agena_plugin_host::sdk::{
    CommandBeforeInput, CommandBeforeResponse, ContributionKind, PathRequest, PluginDisplayContent,
    PluginDisplayContribution, Result as SdkResult, ToolBeforeInput, ToolBeforePatch,
    ToolInvokeOutput, ToolTag,
};
use agena_runtime_tools::tool::{
    ToolExecutionView, ToolPayloadExecution, ToolPayloadOutput, ask_user,
};
use agena_tool::tool_search::{ToolSearchDocument, search_tools};
use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod workflow_plan;
mod workflow_runtime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WorkflowPluginConfig {
    pub(crate) tool_discovery: ToolDiscoveryConfig,
    pub(crate) plan: WorkflowPlanConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ToolDiscoveryConfig {
    pub(crate) list: ToolListConfig,
    pub(crate) search: ToolSearchConfig,
    pub(crate) tags: ToolTagsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ToolListConfig {
    pub(crate) default_limit: u32,
    pub(crate) max_limit: u32,
    pub(crate) max_summary_chars: u32,
}

impl Default for ToolListConfig {
    fn default() -> Self {
        Self {
            default_limit: 20,
            max_limit: 50,
            max_summary_chars: 160,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ToolSearchConfig {
    pub(crate) default_limit: u32,
    pub(crate) max_limit: u32,
    pub(crate) max_query_length: u32,
    pub(crate) max_summary_chars: u32,
}

impl Default for ToolSearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 5,
            max_limit: 20,
            max_query_length: 512,
            max_summary_chars: 160,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ToolTagsConfig {
    pub(crate) default_limit: u32,
    pub(crate) max_limit: u32,
}

impl Default for ToolTagsConfig {
    fn default() -> Self {
        Self {
            default_limit: 20,
            max_limit: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WorkflowPlanConfig {
    pub(crate) default_autorun: bool,
    pub(crate) allow_direct_approval: bool,
    pub(crate) require_approval_on_create: bool,
}

impl Default for WorkflowPlanConfig {
    fn default() -> Self {
        Self {
            default_autorun: true,
            allow_direct_approval: true,
            require_approval_on_create: true,
        }
    }
}

pub(crate) fn tool_discovery_config_schema() -> serde_json::Value {
    let mut schema = agena_runtime_tools::tool::definition::json_schema_for_default(
        ToolDiscoveryConfig::default(),
    );
    for (pointer, title, description) in [
        (
            "",
            "Tool Discovery Settings",
            "Defaults for listing and searching available Agena execution tools.",
        ),
        (
            "/properties/list",
            "List",
            "Pagination and summary limits for execution-tool listing.",
        ),
        (
            "/properties/list/properties/default_limit",
            "Default Limit",
            "Number of tools returned when tools_list omits limit.",
        ),
        (
            "/properties/list/properties/max_limit",
            "Max Limit",
            "Upper bound enforced for tools_list results.",
        ),
        (
            "/properties/list/properties/max_summary_chars",
            "Max Summary Characters",
            "Maximum single-line summary length for each listed tool.",
        ),
        (
            "/properties/search",
            "Search",
            "Pagination, query, and summary limits for execution-tool search.",
        ),
        (
            "/properties/search/properties/default_limit",
            "Default Limit",
            "Number of tool search results returned when the caller omits limit.",
        ),
        (
            "/properties/search/properties/max_limit",
            "Max Limit",
            "Upper bound enforced for execution-tool search results.",
        ),
        (
            "/properties/search/properties/max_query_length",
            "Max Query Length",
            "Upper bound enforced for the tool search query length.",
        ),
        (
            "/properties/search/properties/max_summary_chars",
            "Max Summary Characters",
            "Maximum single-line summary length for each matching tool.",
        ),
        (
            "/properties/tags",
            "Tags",
            "Pagination limits for listing execution-tool tags.",
        ),
        (
            "/properties/tags/properties/default_limit",
            "Default Limit",
            "Number of tags returned when tools_tags omits limit.",
        ),
        (
            "/properties/tags/properties/max_limit",
            "Max Limit",
            "Upper bound enforced for tools_tags results.",
        ),
    ] {
        agena_runtime_tools::tool::definition::set_schema_metadata(
            &mut schema,
            pointer,
            Some(title),
            Some(description),
        );
    }
    schema
}

pub(crate) fn planning_plugin_config_schema() -> serde_json::Value {
    let mut schema = agena_runtime_tools::tool::definition::json_schema_for_default(
        WorkflowPlanConfig::default(),
    );
    for (pointer, title, description) in [
        (
            "",
            "Planning Plugin Config",
            "Defaults for the planning plugin's shared-storage plan state machine.",
        ),
        (
            "/properties/default_autorun",
            "Default Autorun",
            "Default autorun value applied when plan.set omits the override.",
        ),
        (
            "/properties/allow_direct_approval",
            "Allow Direct Approval",
            "When enabled, plan.update may move a planning or cancelled plan directly into active, blocked, or completed. Disable this to make plan.update automatically request review before those transitions.",
        ),
        (
            "/properties/require_approval_on_create",
            "Require Approval on Create",
            "When enabled, plan.set asks the user to approve the new plan before it can become active. Disable to save new plans silently in planning.",
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

mod planning_tools;
mod repo_tools;
mod runtime_tools;
mod tool_api_inputs;

pub(crate) use planning_tools::{
    PlanGetInput, PlanGetView, PlanSetInput, PlanUpdateInput, WorkflowPlan, WorkflowPlanCheckpoint,
    WorkflowPlanExecutor, WorkflowPlanPhase, WorkflowPlanStep, WorkflowPlanStepInput,
    WorkflowPlanStepStatus,
};
pub(crate) use repo_tools::{
    EnterSnapshotCommandInput, ExitSnapshotCommandInput, snapshot_enter_permission_paths,
};
pub(crate) use runtime_tools::{SessionRenameToolInput, SessionToolResponse};
pub(crate) use tool_api_inputs::{
    ToolApiHelpInput, ToolApiListInput, ToolApiSearchInput, ToolApiTagsInput,
};

const PLAN_NAMESPACE: &str = "workflow_plan";
const PLAN_KEY_ACTIVE: &str = "active";
const PLAN_RUNTIME_NAMESPACE: &str = "workflow_plan_runtime";
const PLAN_RUNTIME_AUTO_SIGNATURE_KEY: &str = "last_autorun_signature";
const PLAN_RUNTIME_AUTO_CONTINUATIONS_KEY: &str = "autorun_continuations";
const PLAN_AUTORUN_MAX_CONTINUATIONS: u32 = 5;
const PLAN_DISPLAY_CONTRIBUTION_ID: &str = "plan";
const PLAN_REVIEW_DECISION_APPROVE: &str = "Approve";
const PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON: &str = "Approve with autorun on";
const PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF: &str = "Approve with autorun off";
const PLAN_REVIEW_DECISION_APPROVE_REQUESTED: &str = "Approve requested status";
const PLAN_REVIEW_DECISION_APPROVE_REQUESTED_PAUSE: &str =
    "Approve requested status with auto-continue off";
const PLAN_REVIEW_DECISION_KEEP_PLANNING: &str = "Keep in planning";
const PLAN_REVIEW_DECISION_REJECT: &str = "Reject";
const PLAN_REVIEW_DECISION_CANCELLED: &str = "Cancel plan";

#[derive(Debug)]
enum PlanUpdateTarget {
    Plan,
    Step(usize),
    Check {
        step_index: usize,
        check_index: usize,
    },
}

/// What kind of review a plan approval request is for. Plan creation and
/// later phase transitions present slightly different titles and bodies, but
/// share the same decision flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanReviewKind {
    /// The agent just created/replaced the plan and asks for approval to start.
    Creation,
    /// The plan already exists and a status change (active/blocked/completed/...) is being reviewed.
    StatusChange,
}

pub(crate) struct WorkflowPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
    config: OnceLock<WorkflowPluginConfig>,
    workspace_root: OnceLock<PathBuf>,
}

#[derive(Debug, Clone)]
struct AvailableToolRecord {
    name: String,
    summary: String,
    tags: Vec<String>,
    plugin_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct ToolTagRecord {
    tag: String,
    tool_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugin_count: Option<usize>,
}

/// One loaded plugin projected for discovery (plugins_* family).
#[derive(Debug, Clone)]
pub(crate) struct AvailablePluginRecord {
    pub plugin_id: String,
    pub summary: String,
    pub version: String,
    pub tags: Vec<String>,
    pub tools: Vec<String>,
}

impl WorkflowPlugin {
    pub(crate) fn initialize(
        &self,
        ctx: agena_plugin_host::sdk::InitContext,
        config: WorkflowPluginConfig,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<()> {
        validate_tool_discovery_config(&config.tool_discovery)?;
        self.config
            .set(config)
            .map_err(|_| PluginError::internal("workflow plugin config already initialized"))?;
        self.workspace_root.set(ctx.workspace_root).map_err(|_| {
            PluginError::internal("workflow plugin workspace root already initialized")
        })?;
        *self
            .host
            .write()
            .map_err(|_| PluginError::internal("workflow plugin host lock poisoned"))? = Some(host);
        Ok(())
    }

    pub(crate) async fn tool_execute_before_hook(
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
                "the active plan is still in planning; use plan.update or clear the plan before using mutating tools"
                    .to_string(),
            ),
            ..ToolBeforePatch::default()
        }))
    }

    pub(crate) async fn command_execute_before_hook(
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

    pub(crate) async fn agent_stop_hook(
        &self,
        input: agena_plugin_host::AgentStopInput,
    ) -> SdkResult<Option<agena_plugin_host::AgentStopPatch>> {
        if input.stop_hook_active {
            return Ok(None);
        }
        let Some(plan) = self.load_active_plan().await? else {
            let _ = self.sync_plan_display(None).await;
            return Ok(None);
        };
        // The display sync is cosmetic. A failure here must not abort the
        // stop hook, which would hide the plan autorun continuation (and, at
        // the dispatch level, every other plugin's hook run) from the run.
        if let Err(error) = self.sync_plan_display(Some(&plan)).await {
            tracing::warn!(
                target: "agena::workflow",
                plan = %plan.title,
                                "plan display sync failed during agent.stop: {error}"
            );
        }
        if plan.phase != WorkflowPlanPhase::Active || !plan.autorun {
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
        let signature = Self::plan_auto_signature(&plan, step_index)?;
        let mut continuations = self.load_autorun_continuations().await?;
        let same_signature = self
            .load_autorun_signature()
            .await?
            .is_some_and(|current| current == signature);
        if same_signature {
            if continuations >= PLAN_AUTORUN_MAX_CONTINUATIONS {
                tracing::warn!(
                    target: "agena::workflow",
                    plan = %plan.title,
                    step = step_index + 1,
                    "plan autorun stopped after {continuations} consecutive continuations without plan progress; update the plan state to resume"
                );
                return Ok(None);
            }
        } else {
            continuations = 0;
            self.save_autorun_signature(signature.as_str()).await?;
        }
        self.save_autorun_continuations(continuations + 1).await?;
        Ok(Some(agena_plugin_host::AgentStopPatch {
            continue_with_message: Some(Self::autorun_prompt(&plan, step_index, step)),
            reason: Some("workflow plan autorun".to_string()),
        }))
    }
}

fn tags_summary(tags: &[String]) -> String {
    if tags.is_empty() {
        return "untagged".to_string();
    }
    tags.join(", ")
}

fn compact_tool_summary(value: &str, max_chars: usize) -> String {
    let single_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= max_chars {
        return single_line;
    }
    let retained = max_chars.saturating_sub(1);
    let mut compact = single_line.chars().take(retained).collect::<String>();
    compact.push('…');
    compact
}

fn validate_tool_discovery_config(config: &ToolDiscoveryConfig) -> SdkResult<()> {
    for (path, value) in [
        ("list.default_limit", config.list.default_limit),
        ("list.max_limit", config.list.max_limit),
        ("list.max_summary_chars", config.list.max_summary_chars),
        ("search.default_limit", config.search.default_limit),
        ("search.max_limit", config.search.max_limit),
        ("search.max_query_length", config.search.max_query_length),
        ("search.max_summary_chars", config.search.max_summary_chars),
        ("tags.default_limit", config.tags.default_limit),
        ("tags.max_limit", config.tags.max_limit),
    ] {
        if value == 0 {
            return Err(PluginError::internal(format!(
                "tools plugin config `{path}` must be greater than 0"
            )));
        }
    }
    for (path, default_limit, max_limit) in [
        ("list", config.list.default_limit, config.list.max_limit),
        (
            "search",
            config.search.default_limit,
            config.search.max_limit,
        ),
        ("tags", config.tags.default_limit, config.tags.max_limit),
    ] {
        if default_limit > max_limit {
            return Err(PluginError::internal(format!(
                "tools plugin config `{path}.default_limit` must be less than or equal to `{path}.max_limit`"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::workflow_runtime::discovery_text_output;
    use super::{
        AvailableToolRecord, HostRegisteredToolDescriptor, PlanUpdateInput, PlanUpdateTarget,
        ToolApiHelpInput, ToolDescriptor, ToolDiscoveryConfig, WorkflowPlan,
        WorkflowPlanCheckpoint, WorkflowPlanExecutor, WorkflowPlanPhase, WorkflowPlanStep,
        WorkflowPlanStepStatus, WorkflowPlugin, compact_tool_summary,
        validate_tool_discovery_config,
    };
    use agena_plugin_host::sdk::{
        Plugin, PluginErrorKind, PluginKey, ToolDefinition, ToolKey, ToolTag,
    };

    use crate::plugins::provided::shell::ShellPlugin;

    #[test]
    fn discovery_defaults_keep_index_results_compact() {
        let config = ToolDiscoveryConfig::default();
        assert_eq!(config.list.default_limit, 20);
        assert_eq!(config.list.max_limit, 50);
        assert_eq!(config.list.max_summary_chars, 160);
        assert_eq!(config.search.default_limit, 5);
        assert_eq!(config.search.max_limit, 20);
        assert_eq!(config.search.max_summary_chars, 160);
        assert_eq!(config.tags.default_limit, 20);
        assert_eq!(config.tags.max_limit, 50);
        validate_tool_discovery_config(&config).expect("valid discovery defaults");
    }

    #[test]
    fn discovery_config_rejects_zero_and_inverted_limits() {
        let mut config = ToolDiscoveryConfig::default();
        config.tags.max_limit = 0;
        let error = validate_tool_discovery_config(&config).expect_err("zero limit must fail");
        assert!(error.diagnostic.message.contains("tags.max_limit"));

        let mut config = ToolDiscoveryConfig::default();
        config.list.default_limit = config.list.max_limit + 1;
        let error =
            validate_tool_discovery_config(&config).expect_err("inverted list limits must fail");
        assert!(error.diagnostic.message.contains("list.default_limit"));
    }

    #[test]
    fn discovery_summary_is_single_line_and_unicode_safe() {
        assert_eq!(
            compact_tool_summary("  first\n\nsecond\tthird  ", 80),
            "first second third"
        );
        assert_eq!(compact_tool_summary("工具摘要很长", 5), "工具摘要…");
    }

    #[test]
    fn discovery_result_has_one_text_content_channel() {
        let output = discovery_text_output("List tools · 2/3", "Returned 2 of 3 tools.", "a\nb");

        assert_eq!(output.output_text, "a\nb");
        assert_eq!(output.title, "List tools · 2/3");
        assert_eq!(output.summary, "Returned 2 of 3 tools.");
        assert!(output.payload.is_none());
        assert!(output.metadata.is_empty());
        assert!(output.sections.is_empty());
        assert!(output.attachments.is_empty());
    }

    fn registered_tool(namespace: &str, tag: ToolTag) -> HostRegisteredToolDescriptor {
        let plugin = PluginKey::new(namespace, "notes").expect("plugin key");
        let tool_key = ToolKey::new(plugin.clone(), "format").expect("tool key");
        let mut tool: ToolDefinition = serde_json::from_value(serde_json::json!({
            "name": "format"
        }))
        .expect("tool definition");
        tool.tags.push(tag);
        HostRegisteredToolDescriptor {
            plugin,
            tool_key,
            tool,
        }
    }

    #[test]
    fn filtering_available_tools_supports_multiple_tags() {
        let records = vec![
            AvailableToolRecord {
                name: "agena.fs/read".to_string(),
                summary: "Read file".to_string(),
                tags: vec!["query".to_string(), "filesystem".to_string()],
                plugin_id: "agena.fs".to_string(),
            },
            AvailableToolRecord {
                name: "agena.web/search".to_string(),
                summary: "Search web".to_string(),
                tags: vec!["query".to_string(), "network".to_string()],
                plugin_id: "agena.web".to_string(),
            },
        ];

        let filtered = WorkflowPlugin::filter_available_tools_by_tag(
            records,
            Some("query"),
            Some(&["filesystem".to_string()]),
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "agena.fs/read");
    }

    #[test]
    fn duplicate_execution_tool_names_are_rejected_instead_of_picking_one() {
        let tools = vec![
            ToolDescriptor {
                name: "notes.format".to_string(),
                plugin_id: None,
                summary: None,
                help: None,
                examples: Vec::new(),
                input_schema: None,
            },
            ToolDescriptor {
                name: "notes.format".to_string(),
                plugin_id: None,
                summary: None,
                help: None,
                examples: Vec::new(),
                input_schema: None,
            },
        ];

        let error = WorkflowPlugin::resolve_tool_descriptor("notes.format", &tools)
            .expect_err("duplicate compact names must not resolve implicitly");
        assert!(error.diagnostic.message.contains("ambiguous"));
    }

    #[test]
    fn colliding_tool_names_keep_tags_under_their_internal_keys() {
        let visible = HashSet::from([
            "alpha.notes.format".to_string(),
            "beta.notes.format".to_string(),
        ]);
        let tags = WorkflowPlugin::tool_tags_by_visible_name(
            &visible,
            [
                registered_tool("alpha", ToolTag::Query),
                registered_tool("beta", ToolTag::Mutate),
            ],
        );

        assert_eq!(tags["alpha.notes.format"], vec!["query"]);
        assert_eq!(tags["beta.notes.format"], vec!["mutate"]);
        assert!(!tags.contains_key("notes.format"));
    }

    #[test]
    fn execution_tools_require_exact_tool_names() {
        let tools = vec![ToolDescriptor {
            name: "web.fetch".to_string(),
            plugin_id: None,
            summary: None,
            help: None,
            examples: Vec::new(),
            input_schema: None,
        }];

        for requested in ["web.fetch", "agena.web.fetch"] {
            let resolved = WorkflowPlugin::resolve_tool_descriptor(requested, &tools)
                .unwrap_or_else(|error| panic!("{requested} should resolve: {error}"));
            assert_eq!(resolved.name, "web.fetch");
        }

        for invalid in ["web_fetch", "Web.Fetch", " web.fetch", "web.fetch "] {
            let error = WorkflowPlugin::resolve_tool_descriptor(invalid, &tools)
                .expect_err("execution-tool names must resolve exactly");
            assert!(error.diagnostic.message.contains("unknown tool"));
        }
    }

    #[test]
    fn unknown_tool_requires_search_and_help_instead_of_suggestion_guessing() {
        let tools = vec![
            ToolDescriptor {
                name: "shell.run".to_string(),
                plugin_id: None,
                summary: Some("Run one shell process.".to_string()),
                help: None,
                examples: Vec::new(),
                input_schema: None,
            },
            ToolDescriptor {
                name: "shell.logs".to_string(),
                plugin_id: None,
                summary: Some("Read process logs.".to_string()),
                help: None,
                examples: Vec::new(),
                input_schema: None,
            },
        ];

        let error = WorkflowPlugin::resolve_tool_descriptor("process.run", &tools)
            .expect_err("an invented execution-tool name must be rejected");
        assert!(
            error
                .diagnostic
                .message
                .contains("unknown tool 'process.run'")
        );
        assert!(
            error
                .diagnostic
                .message
                .contains("suggestions are not proof")
        );
        assert!(error.diagnostic.message.contains("Do not guess"));
        assert!(error.diagnostic.message.contains("`tools_search`"));
        assert!(error.diagnostic.message.contains("`tools_help`"));
        let data = error
            .diagnostic
            .data
            .expect("unknown-tool recovery must be structured");
        assert_eq!(
            data.pointer("/kind").and_then(serde_json::Value::as_str),
            Some("unknown_execution_tool")
        );
        assert_eq!(
            data.pointer("/recovery/0/function")
                .and_then(serde_json::Value::as_str),
            Some("tools_search")
        );
        assert_eq!(
            data.pointer("/recovery/1/function")
                .and_then(serde_json::Value::as_str),
            Some("tools_help")
        );
        assert_eq!(
            data.pointer("/recovery/2/function")
                .and_then(serde_json::Value::as_str),
            Some("tools_call")
        );
    }

    #[test]
    fn tool_api_parser_preserves_execution_tool_name_bytes() {
        let parsed = ToolApiHelpInput::parse_input(serde_json::json!({
            "tool": " session.rename "
        }))
        .expect("syntactically valid string payload");

        assert_eq!(parsed.tool, " session.rename ");
    }

    #[test]
    fn protocol_functions_cannot_target_themselves() {
        for function in agena_domain::ToolApiFunction::ALL {
            let error = WorkflowPlugin::ensure_execution_tool_target(function.function_name())
                .expect_err("protocol function must not inhabit the execution-tool namespace");
            assert_eq!(error.kind, PluginErrorKind::InvalidParams);
            assert!(
                error
                    .diagnostic
                    .message
                    .contains("cannot inspect or invoke themselves")
            );
        }

        WorkflowPlugin::ensure_execution_tool_target("fs.read")
            .expect("execution-tool names remain valid targets");
    }

    #[test]
    fn shell_help_retry_example_contains_every_ref_backed_required_field() {
        let run = ShellPlugin
            .manifest()
            .tools
            .into_iter()
            .find(|tool| tool.name == "run")
            .expect("shell.run manifest");
        let descriptor = ToolDescriptor {
            name: "shell.run".to_string(),
            plugin_id: None,
            summary: Some("Run one shell process.".to_string()),
            help: None,
            examples: Vec::new(),
            input_schema: Some(run.input_schema()),
        };

        let help = WorkflowPlugin::render_tool_api_help(&descriptor, None);
        let route = help
            .output_text
            .lines()
            .find(|line| line.contains("Call Tool API function `tools_call`"))
            .expect("direct tools_call recovery route");
        assert!(route.contains("\"reads\""), "{route}");
        assert!(route.contains("\"writes\""), "{route}");
        assert!(route.contains("\"network\""), "{route}");
        assert!(
            route.contains(r#""reads":["#) || route.contains(r#""reads": ["#),
            "{route}"
        );
        assert!(route.contains("\"<target>\""), "{route}");
    }

    fn sample_plan() -> WorkflowPlan {
        WorkflowPlan {
            title: "Test plan".to_string(),
            objective: "Objective".to_string(),
            phase: WorkflowPlanPhase::Planning,
            autorun: false,
            document_markdown: String::new(),
            steps: vec![
                WorkflowPlanStep {
                    id: None,
                    title: "First step".to_string(),
                    description: String::new(),
                    executor: WorkflowPlanExecutor::Ai,
                    status: WorkflowPlanStepStatus::Pending,
                    wait_until_ms: None,
                    note: String::new(),
                    checkpoints: vec![
                        WorkflowPlanCheckpoint {
                            id: None,
                            text: "Check A".to_string(),
                            status: WorkflowPlanStepStatus::Pending,
                        },
                        WorkflowPlanCheckpoint {
                            id: None,
                            text: "Check B".to_string(),
                            status: WorkflowPlanStepStatus::Pending,
                        },
                    ],
                },
                WorkflowPlanStep {
                    id: None,
                    title: "Second step".to_string(),
                    description: String::new(),
                    executor: WorkflowPlanExecutor::Human,
                    status: WorkflowPlanStepStatus::Pending,
                    wait_until_ms: None,
                    note: String::new(),
                    checkpoints: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn resolve_step_index_uses_1_based_indices() {
        let plan = sample_plan();

        assert_eq!(WorkflowPlugin::resolve_step_index(&plan, 1).unwrap(), 0);
        assert_eq!(WorkflowPlugin::resolve_step_index(&plan, 2).unwrap(), 1);

        let zero = WorkflowPlugin::resolve_step_index(&plan, 0)
            .expect_err("step 0 must be rejected as not 1-based");
        assert_eq!(
            zero.diagnostic.message,
            "plan step indices are 1-based; `step` must be at least 1"
        );

        let out_of_range = WorkflowPlugin::resolve_step_index(&plan, 3)
            .expect_err("step 3 must be rejected for a 2-step plan");
        assert_eq!(
            out_of_range.diagnostic.message,
            "unknown plan step 3; this plan has 2 step(s)"
        );
    }

    #[test]
    fn resolve_check_index_uses_1_based_indices() {
        let plan = sample_plan();

        assert_eq!(
            WorkflowPlugin::resolve_check_index(&plan.steps[0], 1).unwrap(),
            0
        );
        assert_eq!(
            WorkflowPlugin::resolve_check_index(&plan.steps[0], 2).unwrap(),
            1
        );

        let zero = WorkflowPlugin::resolve_check_index(&plan.steps[0], 0)
            .expect_err("check 0 must be rejected as not 1-based");
        assert_eq!(
            zero.diagnostic.message,
            "plan check indices are 1-based; `check` must be at least 1"
        );

        let out_of_range = WorkflowPlugin::resolve_check_index(&plan.steps[0], 3)
            .expect_err("check 3 must be rejected for a 2-check step");
        assert_eq!(
            out_of_range.diagnostic.message,
            "unknown check 3; this step has 2 check(s)"
        );
    }

    #[test]
    fn validate_plan_update_input_maps_plan_and_step_targets() {
        let plan_update = PlanUpdateInput {
            phase: Some(WorkflowPlanPhase::Active),
            ..Default::default()
        };
        assert!(matches!(
            WorkflowPlugin::validate_plan_update_input(&plan_update).unwrap(),
            PlanUpdateTarget::Plan
        ));

        let step_update = PlanUpdateInput {
            step: Some(2),
            status: Some(WorkflowPlanStepStatus::InProgress),
            ..Default::default()
        };
        assert!(matches!(
            WorkflowPlugin::validate_plan_update_input(&step_update).unwrap(),
            PlanUpdateTarget::Step(2)
        ));

        let check_update = PlanUpdateInput {
            step: Some(1),
            check: Some(2),
            status: Some(WorkflowPlanStepStatus::Completed),
            ..Default::default()
        };
        assert!(matches!(
            WorkflowPlugin::validate_plan_update_input(&check_update).unwrap(),
            PlanUpdateTarget::Check {
                step_index: 1,
                check_index: 2
            }
        ));
    }

    #[test]
    fn plan_config_requires_approval_on_create_by_default() {
        use super::WorkflowPlanConfig;

        assert!(WorkflowPlanConfig::default().require_approval_on_create);
    }

    #[test]
    fn plan_display_content_uses_compact_symbols() {
        let plan = sample_plan();
        assert_eq!(WorkflowPlugin::plan_display_content(&plan), "⏳ 0/2");

        let mut active = sample_plan();
        active.phase = WorkflowPlanPhase::Active;
        active.autorun = true;
        assert_eq!(WorkflowPlugin::plan_display_content(&active), "▶ 0/2 ↻");

        let mut blocked = sample_plan();
        blocked.phase = WorkflowPlanPhase::Blocked;
        assert_eq!(WorkflowPlugin::plan_display_content(&blocked), "⚠ 0/2");

        let mut completed = sample_plan();
        completed.phase = WorkflowPlanPhase::Completed;
        completed.steps[0].status = WorkflowPlanStepStatus::Completed;
        assert_eq!(WorkflowPlugin::plan_display_content(&completed), "✓ 1/2");

        let mut cancelled = sample_plan();
        cancelled.phase = WorkflowPlanPhase::Cancelled;
        assert_eq!(WorkflowPlugin::plan_display_content(&cancelled), "✕ 0/2");

        let mut empty = sample_plan();
        empty.steps.clear();
        assert_eq!(WorkflowPlugin::plan_display_content(&empty), "⏳");

        assert_eq!(
            WorkflowPlugin::plan_phase_symbol(WorkflowPlanPhase::Planning),
            "⏳"
        );
        assert_eq!(
            WorkflowPlugin::plan_phase_symbol(WorkflowPlanPhase::Active),
            "▶"
        );
        assert_eq!(
            WorkflowPlugin::plan_phase_symbol(WorkflowPlanPhase::Blocked),
            "⚠"
        );
        assert_eq!(
            WorkflowPlugin::plan_phase_symbol(WorkflowPlanPhase::Completed),
            "✓"
        );
        assert_eq!(
            WorkflowPlugin::plan_phase_symbol(WorkflowPlanPhase::Cancelled),
            "✕"
        );
    }

    #[test]
    fn creation_review_request_allows_free_text_feedback() {
        use super::PlanReviewKind;

        let plan = sample_plan();
        let request = WorkflowPlugin::phase_review_request(
            &plan,
            WorkflowPlanPhase::Active,
            None,
            None,
            PlanReviewKind::Creation,
        );
        assert_eq!(request.title, "Approve New Plan");
        assert!(request.allow_free_text);
        assert!(request.body_markdown.contains("Proposed Plan"));
        assert!(request.body_markdown.contains("waiting for your approval"));
        let question = request.questions.first().expect("one decision question");
        assert_eq!(question.id, "decision");
        assert!(question.allow_custom);
    }

    #[test]
    fn status_change_review_request_keeps_previous_title() {
        use super::PlanReviewKind;

        let plan = sample_plan();
        let request = WorkflowPlugin::phase_review_request(
            &plan,
            WorkflowPlanPhase::Active,
            None,
            None,
            PlanReviewKind::StatusChange,
        );
        assert_eq!(request.title, "Review Plan Status Change");
    }

    #[test]
    fn review_feedback_decision_classifies_known_labels_and_free_text() {
        for label in [
            super::PLAN_REVIEW_DECISION_APPROVE,
            super::PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON,
            super::PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF,
            super::PLAN_REVIEW_DECISION_APPROVE_REQUESTED,
            super::PLAN_REVIEW_DECISION_APPROVE_REQUESTED_PAUSE,
            super::PLAN_REVIEW_DECISION_KEEP_PLANNING,
            super::PLAN_REVIEW_DECISION_REJECT,
            super::PLAN_REVIEW_DECISION_CANCELLED,
        ] {
            assert!(
                !WorkflowPlugin::is_review_feedback_decision(label),
                "known label must not be treated as feedback: {label}"
            );
        }
        assert!(WorkflowPlugin::is_review_feedback_decision(
            "Please split step 1 into two smaller steps"
        ));
    }

    #[test]
    fn review_decision_extracts_free_text_answer() {
        use std::collections::BTreeMap;

        use agena_plugin_host::sdk::host_api::AskUserResponse;

        let response = AskUserResponse {
            reply: String::new(),
            cancelled: false,
            timed_out: false,
            answers: BTreeMap::from([(
                "decision".to_string(),
                vec!["Make the second step less vague".to_string()],
            )]),
        };
        assert_eq!(
            WorkflowPlugin::review_decision(&response).as_deref(),
            Some("Make the second step less vague")
        );
    }

    #[test]
    fn timed_out_review_response_keeps_plan_in_planning() {
        use std::collections::BTreeMap;

        use agena_plugin_host::sdk::host_api::AskUserResponse;

        // A timed-out ask_user arrives with no answers and no reply text; the
        // runtime resolves the host request with UserInputReplyKind::Timeout.
        let response = AskUserResponse {
            reply: String::new(),
            cancelled: false,
            timed_out: true,
            answers: BTreeMap::new(),
        };
        let decision = WorkflowPlugin::review_decision_from_response(&response);
        assert_eq!(decision, super::PLAN_REVIEW_DECISION_KEEP_PLANNING);

        let mut plan = sample_plan();
        plan.phase = WorkflowPlanPhase::Active;
        WorkflowPlugin::apply_review_decision(
            &mut plan,
            &decision,
            WorkflowPlanPhase::Active,
            None,
            None,
        )
        .expect("keep-in-planning decision applies");
        assert_eq!(plan.phase, WorkflowPlanPhase::Planning);
    }

    #[test]
    fn cancelled_review_response_keeps_plan_in_planning() {
        use agena_plugin_host::sdk::host_api::AskUserResponse;

        let response = AskUserResponse {
            reply: String::new(),
            cancelled: true,
            timed_out: false,
            answers: Default::default(),
        };
        let decision = WorkflowPlugin::review_decision_from_response(&response);
        assert_eq!(decision, super::PLAN_REVIEW_DECISION_KEEP_PLANNING);

        let mut plan = sample_plan();
        plan.phase = WorkflowPlanPhase::Blocked;
        WorkflowPlugin::apply_review_decision(
            &mut plan,
            &decision,
            WorkflowPlanPhase::Blocked,
            None,
            None,
        )
        .expect("keep-in-planning decision applies");
        assert_eq!(plan.phase, WorkflowPlanPhase::Planning);
    }

    #[test]
    fn phase_review_request_relies_on_runtime_default_timeout() {
        use super::PlanReviewKind;

        let plan = sample_plan();
        for kind in [PlanReviewKind::Creation, PlanReviewKind::StatusChange] {
            let request = WorkflowPlugin::phase_review_request(
                &plan,
                WorkflowPlanPhase::Active,
                None,
                None,
                kind,
            );
            assert!(
                request.auto_resolution_ms.is_none(),
                "plan review must not set its own deadline; the runtime default timeout bounds it"
            );
        }
    }

    #[test]
    fn validate_plan_update_input_rejects_invalid_combinations() {
        let check_without_step = PlanUpdateInput {
            check: Some(1),
            status: Some(WorkflowPlanStepStatus::Completed),
            ..Default::default()
        };
        let error = WorkflowPlugin::validate_plan_update_input(&check_without_step)
            .expect_err("check without step must fail");
        assert_eq!(
            error.diagnostic.message,
            "plan.update check updates require `step`"
        );

        let check_without_status = PlanUpdateInput {
            step: Some(1),
            check: Some(1),
            ..Default::default()
        };
        let error = WorkflowPlugin::validate_plan_update_input(&check_without_status)
            .expect_err("check without status must fail");
        assert_eq!(
            error.diagnostic.message,
            "plan.update check updates require `status`"
        );

        let mixed_phase_and_step = PlanUpdateInput {
            phase: Some(WorkflowPlanPhase::Completed),
            step: Some(1),
            status: Some(WorkflowPlanStepStatus::Completed),
            ..Default::default()
        };
        let error = WorkflowPlugin::validate_plan_update_input(&mixed_phase_and_step)
            .expect_err("mixing phase with step/check fields must fail");
        assert_eq!(
            error.diagnostic.message,
            "plan.update must target either the plan itself or a step/check, not both"
        );
    }
}
