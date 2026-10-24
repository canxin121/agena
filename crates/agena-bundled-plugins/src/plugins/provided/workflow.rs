//! Shared implementation for the agent/tools/interaction/planning/repo/session plugins.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::part::AskUserToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{
    AskUserOption as HostAskUserOption, AskUserQuestion as HostAskUserQuestion, AskUserRequest,
    HostClient, HostDisplayContributeRequest, HostDisplayRemoveRequest, HostGetSessionRequest,
    HostRegisteredToolDescriptor, HostRenameSessionRequest, HostSession, HostStorageDeleteRequest,
    HostStorageGetRequest, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
    ToolDescriptor,
};
use agena_plugin_host::sdk::{
    CommandBeforeInput, CommandBeforeResponse, ContributionKind, PluginDisplayContent,
    PluginDisplayContribution, Result as SdkResult, ToolBeforeInput, ToolBeforePatch,
    ToolInvokeOutput, ToolTag,
};
use agena_runtime_tools::tool::ask_user;
use agena_tool::tool_search::{ToolSearchDocument, search_tools};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod plan_storage;
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ToolListConfig {
    pub(crate) default_limit: u32,
    pub(crate) max_summary_chars: u32,
}

impl Default for ToolListConfig {
    fn default() -> Self {
        Self {
            default_limit: 20,
            max_summary_chars: 160,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WorkflowPlanConfig {
    pub(crate) default_autorun: bool,
    /// Trusted configuration opt-in; a model argument alone cannot waive review.
    pub(crate) allow_unreviewed_activation: bool,
}

impl Default for WorkflowPlanConfig {
    fn default() -> Self {
        Self {
            default_autorun: true,
            allow_unreviewed_activation: false,
        }
    }
}

mod planning_tools;
mod runtime_tools;
mod tool_api_inputs;

pub(crate) use planning_tools::{
    PlanEditInput, PlanGetInput, PlanGetView, PlanPhaseInput, PlanReviewInput, PlanSetInput,
    WorkflowPlan, WorkflowPlanCheckpoint, WorkflowPlanExecutor, WorkflowPlanPhase,
    WorkflowPlanStep, WorkflowPlanStepInput, WorkflowPlanStepStatus,
};
pub(crate) use runtime_tools::{SessionRenameToolInput, SessionToolResponse};
pub(crate) use tool_api_inputs::{ToolApiHelpInput, ToolApiListInput, ToolApiStringBatch};

const PLAN_NAMESPACE: &str = "workflow_plan";
const PLAN_KEY_ACTIVE: &str = "active";
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
enum PlanEditTarget {
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
                "the active plan is still in planning; use plan.edit, plan.phase, or clear the plan before using mutating tools"
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
            if let Err(error) = self.sync_plan_display(None).await {
                tracing::warn!(
                    target: "agena::workflow",
                    diagnostic = %error.diagnostic_message(),
                    "stale plan display could not be cleared during agent.stop"
                );
            }
            return Ok(None);
        };
        // The display sync is cosmetic. A failure here must not abort the
        // stop hook, which would hide the plan autorun continuation (and, at
        // the dispatch level, every other plugin's hook run) from the run.
        if let Err(error) = self.sync_plan_display(Some(&plan)).await {
            tracing::warn!(
                target: "agena::workflow",
                plan = %plan.title,
                diagnostic = %error.diagnostic_message(),
                "plan display sync failed during agent.stop"
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
        Ok(Some(agena_plugin_host::AgentStopPatch {
            continue_with_message: Some(Self::autorun_prompt(
                &plan,
                step_index,
                step,
                input.run_error.as_deref(),
            )),
            reason: Some("workflow plan autorun".to_string()),
        }))
    }

    pub(crate) async fn agent_cancel_hook(
        &self,
        _input: agena_plugin_host::AgentCancelInput,
    ) -> SdkResult<()> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Ok(());
        };
        if plan.autorun {
            plan.autorun = false;
            self.update_active_plan(&mut plan).await?;
        }
        Ok(())
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
        ("list.max_summary_chars", config.list.max_summary_chars),
    ] {
        if value == 0 {
            return Err(PluginError::internal(format!(
                "tools plugin config `{path}` must be greater than 0"
            )));
        }
    }
    Ok(())
}
