//! Shared implementation for the catalog/runtime/planning/tasks/repo plugins.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::message::{AgentSwitchToolInput, AskUserToolInput, TaskToolInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{
    AskUserOption as HostAskUserOption, AskUserQuestion as HostAskUserQuestion, AskUserRequest,
    HostAgentRestoreRequest, HostAgentRestoreResponse, HostAgentSwitchRequest,
    HostAgentSwitchResponse, HostClient, HostEnterSnapshotRequest, HostExitSnapshotRequest,
    HostGetSessionRequest, HostRenameSessionRequest, HostSession, HostStatuslineContributeRequest,
    HostStatuslineRemoveRequest, HostStorageDeleteRequest, HostStorageGetRequest, HostStorageScope,
    HostStorageSetRequest, HostStorageVisibility, SpawnSubtaskRequest, ToolDescriptor,
};
use crate::plugin::sdk::{
    CommandBeforeInput, CommandBeforeResponse, PathRequest, Result as SdkResult, ToolBeforeInput,
    ToolBeforePatch, ToolInvokeOutput, ToolTag,
};
use crate::search::tool_catalog::{ToolCatalogDocument, search_tool_catalog};
use crate::tool::{ToolExecutionView, ToolPayloadExecution, ToolPayloadOutput, ask_user};
use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod workflow_plan;
mod workflow_runtime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WorkflowPluginConfig {
    pub(crate) tool_catalog: WorkflowToolCatalogConfig,
    pub(crate) plan: WorkflowPlanConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WorkflowToolCatalogConfig {
    pub(crate) search: WorkflowToolCatalogSearchConfig,
}

impl Default for WorkflowToolCatalogConfig {
    fn default() -> Self {
        Self {
            search: WorkflowToolCatalogSearchConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WorkflowToolCatalogSearchConfig {
    pub(crate) default_limit: u32,
    pub(crate) max_limit: u32,
    pub(crate) max_query_length: u32,
}

impl Default for WorkflowToolCatalogSearchConfig {
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

pub(crate) fn tool_catalog_plugin_config_schema() -> serde_json::Value {
    let mut schema =
        crate::tool::definition::json_schema_for_default(WorkflowToolCatalogConfig::default());
    for (pointer, title, description) in [
        (
            "",
            "Tool Catalog Plugin Config",
            "Defaults for tool catalog search behavior.",
        ),
        (
            "/properties/search",
            "Search",
            "Default behavior for the catalog search tool.",
        ),
        (
            "/properties/search/properties/default_limit",
            "Default Limit",
            "Number of tool search results returned when the caller omits limit.",
        ),
        (
            "/properties/search/properties/max_limit",
            "Max Limit",
            "Upper bound enforced for tool catalog search results.",
        ),
        (
            "/properties/search/properties/max_query_length",
            "Max Query Length",
            "Upper bound enforced for the catalog search query length.",
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

mod catalog_tools;
mod planning_tools;
mod repo_tools;
mod runtime_tools;

pub(crate) use catalog_tools::{
    CatalogSearchInput, ToolCallInput, ToolListInput, ToolTagsInput, ToolsHelpInput,
};
pub(crate) use planning_tools::{
    PlanGetInput, PlanGetView, PlanSetInput, PlanUpdateInput, WorkflowPlan, WorkflowPlanCheckpoint,
    WorkflowPlanExecutor, WorkflowPlanPhase, WorkflowPlanStep, WorkflowPlanStepInput,
    WorkflowPlanStepStatus,
};
pub(crate) use repo_tools::{
    EnterSnapshotCommandInput, ExitSnapshotCommandInput, snapshot_enter_permission_paths,
};
pub(crate) use runtime_tools::{SessionRenameToolInput, SessionToolResponse};

const PLAN_NAMESPACE: &str = "workflow_plan";
const PLAN_KEY_ACTIVE: &str = "active";
const PLAN_RUNTIME_NAMESPACE: &str = "workflow_plan_runtime";
const PLAN_RUNTIME_AUTO_SIGNATURE_KEY: &str = "last_autorun_signature";
const PLAN_STATUSLINE_SEGMENT_ID: &str = "plan";
const TOOL_CATALOG_RUNTIME_NAMESPACE: &str = "workflow_tool_catalog_runtime";
const TOOL_CATALOG_HELP_PREFLIGHTS_KEY: &str = "help_preflights";
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
    /// Host storage provides no atomic read-modify-write operation. Serialize
    /// preflight updates so concurrent gateway calls cannot lose a grant.
    help_preflight_lock: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone)]
struct CatalogToolRecord {
    name: String,
    summary: String,
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct CatalogTagRecord {
    tag: String,
    tool_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HelpedToolsState {
    /// A help result authorizes exactly one subsequent call of the same
    /// catalog target. Keeping this state in session-private storage lets a
    /// model inspect another tool in between without losing the preflight,
    /// while consuming it removes any ambiguity about whether a later call
    /// was covered by stale help.
    #[serde(default)]
    ready_tools: BTreeMap<String, u32>,
}

impl HelpedToolsState {
    fn grant(&mut self, tool_name: &str) {
        let grants = self.ready_tools.entry(tool_name.to_string()).or_default();
        *grants = grants.saturating_add(1);
    }

    fn consume(&mut self, tool_name: &str) -> bool {
        let Some(grants) = self.ready_tools.get_mut(tool_name) else {
            return false;
        };
        *grants = grants.saturating_sub(1);
        if *grants == 0 {
            self.ready_tools.remove(tool_name);
        }
        true
    }
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
    use super::{CatalogToolRecord, HelpedToolsState, ToolDescriptor, WorkflowPlugin};

    #[test]
    fn each_help_preflight_authorizes_exactly_one_call_of_its_target() {
        let mut state = HelpedToolsState::default();
        state.grant("web.search");

        assert!(state.consume("web.search"));
        assert!(
            !state.consume("web.search"),
            "a second call must obtain help again"
        );
        assert!(
            !state.consume("fs.glob"),
            "help for one tool must never authorize another"
        );

        state.grant("fs.glob");
        assert!(state.consume("fs.glob"));

        state.grant("web.search");
        state.grant("web.search");
        assert!(state.consume("web.search"));
        assert!(
            state.consume("web.search"),
            "each additional help result must authorize one additional call"
        );
    }

    #[test]
    fn filter_catalog_records_supports_multiple_tags() {
        let records = vec![
            CatalogToolRecord {
                name: "agena.fs/read".to_string(),
                summary: "Read file".to_string(),
                tags: vec!["read_only".to_string(), "filesystem_read".to_string()],
            },
            CatalogToolRecord {
                name: "agena.web/search".to_string(),
                summary: "Search web".to_string(),
                tags: vec!["read_only".to_string(), "network".to_string()],
            },
        ];

        let filtered = WorkflowPlugin::filter_catalog_records_by_tag(
            records,
            Some("read_only"),
            Some(&["filesystem_read".to_string()]),
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "agena.fs/read");
    }

    #[test]
    fn duplicate_gateway_tool_names_are_rejected_instead_of_picking_one() {
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
    fn gateway_requires_dotted_catalog_target_names() {
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

        let error = WorkflowPlugin::resolve_tool_descriptor("web_fetch", &tools)
            .expect_err("underscore aliases must not silently rewrite catalog target names");
        assert!(error.message.contains("unknown tool"));
    }
}
