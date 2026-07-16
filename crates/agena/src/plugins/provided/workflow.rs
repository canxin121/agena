//! Shared implementation for the agent/tools/interaction/planning/repo/session/tasks plugins.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::message::{AgentSwitchToolInput, AskUserToolInput, TaskToolInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{
    AskUserOption as HostAskUserOption, AskUserQuestion as HostAskUserQuestion, AskUserRequest,
    HostAgentRestoreRequest, HostAgentRestoreResponse, HostAgentSwitchRequest,
    HostAgentSwitchResponse, HostClient, HostEnterSnapshotRequest, HostExitSnapshotRequest,
    HostGetSessionRequest, HostRegisteredToolDescriptor, HostRenameSessionRequest, HostSession,
    HostStatuslineContributeRequest, HostStatuslineRemoveRequest, HostStorageDeleteRequest,
    HostStorageGetRequest, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
    RunSubtaskModelSelection, RunSubtaskRequest, RunSubtaskStatus, ToolDescriptor,
};
use crate::plugin::sdk::{
    CommandBeforeInput, CommandBeforeResponse, PathRequest, Result as SdkResult, ToolBeforeInput,
    ToolBeforePatch, ToolInvokeOutput, ToolTag,
};
use crate::search::tool_search::{ToolSearchDocument, search_tools};
use crate::tool::{ToolExecutionView, ToolPayloadExecution, ToolPayloadOutput, ask_user};
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
    pub(crate) search: ToolSearchConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ToolSearchConfig {
    pub(crate) default_limit: u32,
    pub(crate) max_limit: u32,
    pub(crate) max_query_length: u32,
}

impl Default for ToolSearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 50,
            max_limit: 100,
            max_query_length: 512,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WorkflowPlanConfig {
    pub(crate) default_autorun: bool,
    pub(crate) allow_direct_approval: bool,
}

impl Default for WorkflowPlanConfig {
    fn default() -> Self {
        Self {
            default_autorun: true,
            allow_direct_approval: true,
        }
    }
}

pub(crate) fn tool_discovery_config_schema() -> serde_json::Value {
    let mut schema =
        crate::tool::definition::json_schema_for_default(ToolDiscoveryConfig::default());
    for (pointer, title, description) in [
        (
            "",
            "Tool Discovery Settings",
            "Defaults for listing and searching available Agena execution tools.",
        ),
        (
            "/properties/search",
            "Search",
            "Default behavior for execution-tool search.",
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

pub(crate) fn planning_plugin_config_schema() -> serde_json::Value {
    let mut schema =
        crate::tool::definition::json_schema_for_default(WorkflowPlanConfig::default());
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
    ToolApiCallInput, ToolApiHelpInput, ToolApiListInput, ToolApiSearchInput, ToolApiTagsInput,
};

const PLAN_NAMESPACE: &str = "workflow_plan";
const PLAN_KEY_ACTIVE: &str = "active";
const PLAN_RUNTIME_NAMESPACE: &str = "workflow_plan_runtime";
const PLAN_RUNTIME_AUTO_SIGNATURE_KEY: &str = "last_autorun_signature";
const PLAN_STATUSLINE_SEGMENT_ID: &str = "plan";
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
    Step(String),
    Check {
        step_id: String,
        checkpoint_id: String,
    },
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
}

#[derive(Debug, Clone, Serialize)]
struct ToolTagRecord {
    tag: String,
    tool_count: usize,
}

impl WorkflowPlugin {
    pub(crate) fn initialize(
        &self,
        ctx: crate::plugin::sdk::InitContext,
        config: WorkflowPluginConfig,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<()> {
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
        let signature = Self::plan_auto_signature(&plan, step_index, step)?;
        if self
            .load_autorun_signature()
            .await?
            .is_some_and(|current| current == signature)
        {
            return Ok(None);
        }
        self.save_autorun_signature(signature.as_str()).await?;
        Ok(Some(crate::plugin::AgentStopPatch {
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{
        AvailableToolRecord, HostRegisteredToolDescriptor, ToolApiHelpInput, ToolDescriptor,
        WorkflowPlugin,
    };
    use crate::plugin::sdk::{PluginErrorCode, PluginKey, ToolDefinition, ToolKey, ToolTag};

    fn registered_tool(namespace: &str, tag: ToolTag) -> HostRegisteredToolDescriptor {
        let plugin = PluginKey::new(namespace, "notes").expect("plugin key");
        let tool_key = ToolKey::new(plugin.clone(), "format").expect("tool key");
        let mut tool: ToolDefinition = serde_json::from_value(serde_json::json!({
            "name": "format"
        }))
        .expect("tool definition");
        tool.permissions.tags.push(tag);
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
                tags: vec!["read_only".to_string(), "filesystem_read".to_string()],
            },
            AvailableToolRecord {
                name: "agena.web/search".to_string(),
                summary: "Search web".to_string(),
                tags: vec!["read_only".to_string(), "network".to_string()],
            },
        ];

        let filtered = WorkflowPlugin::filter_available_tools_by_tag(
            records,
            Some("read_only"),
            Some(&["filesystem_read".to_string()]),
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "agena.fs/read");
    }

    #[test]
    fn duplicate_execution_tool_names_are_rejected_instead_of_picking_one() {
        let tools = vec![
            ToolDescriptor {
                name: "notes.format".to_string(),
                summary: None,
                help: None,
                examples: Vec::new(),
                input_schema: None,
            },
            ToolDescriptor {
                name: "notes.format".to_string(),
                summary: None,
                help: None,
                examples: Vec::new(),
                input_schema: None,
            },
        ];

        let error = WorkflowPlugin::resolve_tool_descriptor("notes.format", &tools)
            .expect_err("duplicate compact names must not resolve implicitly");
        assert!(error.message.contains("ambiguous"));
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
                registered_tool("alpha", ToolTag::ReadOnly),
                registered_tool("beta", ToolTag::Mutating),
            ],
        );

        assert_eq!(tags["alpha.notes.format"], vec!["read_only"]);
        assert_eq!(tags["beta.notes.format"], vec!["mutating"]);
        assert!(!tags.contains_key("notes.format"));
    }

    #[test]
    fn execution_tools_require_exact_tool_names() {
        let tools = vec![ToolDescriptor {
            name: "web.fetch".to_string(),
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
            assert!(error.message.contains("unknown tool"));
        }
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
    fn schema_rejection_embeds_help_and_direct_retry_routing() {
        let descriptor = ToolDescriptor {
            name: "fs.read".to_string(),
            summary: Some("Read workspace files.".to_string()),
            help: Some("Read a file with file_path.".to_string()),
            examples: vec![r#"{"file_path":"README.md"}"#.to_string()],
            input_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string", "minLength": 1}
                },
                "required": ["file_path"],
                "additionalProperties": false
            })),
        };

        let error = WorkflowPlugin::invalid_tool_input_with_embedded_help(
            &descriptor,
            "$: missing required property 'file_path'",
        );

        assert_eq!(error.code, PluginErrorCode::InvalidParams);
        assert!(error.message.contains("the tool was not run"));
        assert!(
            error
                .message
                .contains("A separate `tools_help` call is unnecessary")
        );
        assert!(error.message.contains("Tool help for `fs.read`"));
        assert!(error.message.contains("Usage:"));
        assert!(error.message.contains("Read a file with file_path."));
        let data = error.data.expect("structured embedded help");
        assert_eq!(
            data.pointer("/kind").and_then(serde_json::Value::as_str),
            Some("tool_input_rejected_with_help")
        );
        assert_eq!(
            data.pointer("/tool").and_then(serde_json::Value::as_str),
            Some("fs.read")
        );
        assert_eq!(
            data.pointer("/retry/function")
                .and_then(serde_json::Value::as_str),
            Some("tools_call")
        );
        assert_eq!(
            data.pointer("/help/input_schema/required/0")
                .and_then(serde_json::Value::as_str),
            Some("file_path")
        );
    }
}
