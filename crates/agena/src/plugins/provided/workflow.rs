//! Shared implementation for the catalog/runtime/planning/tasks/repo plugins.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::message::{
    AgentRestoreToolInput, AgentSwitchToolInput, AskUserToolInput, TaskToolInput,
    ToolSearchToolInput,
};
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
    CommandBeforeInput, CommandBeforeResponse, HostCapability, PathRequest, Result as SdkResult,
    ToolBeforeInput, ToolBeforePatch, ToolInvokeOutput, ToolTag,
};
use crate::search::tool_catalog::{ToolCatalogDocument, search_tool_catalog};
use crate::tool::{ToolExecutionView, ToolPayloadExecution, ToolPayloadOutput, ask_user};
use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::plugin::sdk::InitContext;
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
    pub(crate) help: WorkflowToolCatalogHelpConfig,
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
pub(crate) struct WorkflowToolCatalogSearchConfig {
    pub(crate) default_limit: u32,
    pub(crate) max_limit: u32,
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
pub(crate) struct WorkflowToolCatalogHelpConfig {
    pub(crate) include_schema_by_default: bool,
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
        crate::tool::definition::json_schema_for_with_default(WorkflowToolCatalogConfig::default());
    for (pointer, title, description) in [
        (
            "",
            "Tool Catalog Plugin Config",
            "Defaults for tool catalog search and tool help rendering.",
        ),
        (
            "/properties/search",
            "Search",
            "Default search behavior for tools with action=search.",
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
            "/properties/help",
            "Help",
            "Defaults for tools with action=help.",
        ),
        (
            "/properties/help/properties/include_schema_by_default",
            "Include Schema by Default",
            "When enabled, tool help includes the registered input schema unless the caller opts out.",
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
        crate::tool::definition::json_schema_for_with_default(WorkflowPlanConfig::default());
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
mod task_tools;

pub(crate) use catalog_tools::{ToolListInput, ToolsHelpInput, ToolsToolInput};
pub(crate) use planning_tools::{
    PlanGetInput, PlanGetView, PlanSetInput, PlanToolInput, PlanUpdateInput, WorkflowPlan,
    WorkflowPlanCheckpoint, WorkflowPlanExecutor, WorkflowPlanPhase, WorkflowPlanStep,
    WorkflowPlanStepInput, WorkflowPlanStepStatus,
};
pub(crate) use repo_tools::{
    EnterSnapshotCommandInput, ExitSnapshotCommandInput, SnapshotToolInput,
    snapshot_enter_permission_paths,
};
pub(crate) use runtime_tools::{
    AgentToolInput, SessionRenameToolInput, SessionToolInput, SessionToolResponse, UserToolInput,
};
pub(crate) use task_tools::TaskToolActionInput;

#[cfg(test)]
pub(crate) use catalog_tools::resolve_tools_tool_input;
#[cfg(test)]
pub(crate) use planning_tools::resolve_plan_tool_input;
#[cfg(test)]
pub(crate) use repo_tools::ExitSnapshotAction;

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

impl WorkflowPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
            config: OnceLock::new(),
            workspace_root: OnceLock::new(),
        }
    }

    fn config(&self) -> SdkResult<&WorkflowPluginConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::new("workflow plugin invoked before init"))
    }

    async fn invoke_tools_usage(&self) -> SdkResult<ToolInvokeOutput> {
        Ok(Self::invoke_tool_catalog_usage())
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
        let response = self
            .host()?
            .rename_session(HostRenameSessionRequest {
                session_id: None,
                title: input.title.clone(),
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
        self.clear_autorun_signature().await?;
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
        self.clear_autorun_signature().await?;
        self.sync_plan_statusline(None).await?;
        Ok(())
    }

    async fn load_autorun_signature(&self) -> SdkResult<Option<String>> {
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

    async fn save_autorun_signature(&self, signature: &str) -> SdkResult<()> {
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

    async fn clear_autorun_signature(&self) -> SdkResult<()> {
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
                            "plan check {}.{} requires non-empty text",
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
                                format!("step_{}_check_{}", step_index + 1, checkpoint_index + 1)
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
        objective: &str,
        title: Option<&str>,
        document_markdown: Option<&str>,
        steps: &[WorkflowPlanStepInput],
        autorun: Option<bool>,
        previous: Option<&WorkflowPlan>,
    ) -> SdkResult<WorkflowPlan> {
        let objective = Self::validate_plan_objective(objective)?;
        let title = title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| Self::default_plan_title(&objective));
        let autorun = match autorun {
            Some(value) => value,
            None => previous
                .map(|plan| plan.autorun)
                .unwrap_or(self.config()?.plan.default_autorun),
        };
        Ok(WorkflowPlan {
            title,
            objective,
            phase: WorkflowPlanPhase::Planning,
            autorun,
            document_markdown: document_markdown
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_default(),
            steps: Self::normalize_plan_steps(steps)?,
        })
    }

    fn plan_phase_label(phase: WorkflowPlanPhase) -> &'static str {
        match phase {
            WorkflowPlanPhase::Planning => "planning",
            WorkflowPlanPhase::Active => "active",
            WorkflowPlanPhase::Blocked => "blocked",
            WorkflowPlanPhase::Completed => "completed",
            WorkflowPlanPhase::Cancelled => "cancelled",
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
            .or_else(|| {
                Self::parse_1_based_index_hint(checkpoint_id, &["check", "checkpoint", "cp", "c"])
            })
            .filter(|index| *index < step.checkpoints.len())
    }

    fn plan_step_identifier_hint(step: &WorkflowPlanStep, index: usize) -> String {
        format!("step_id={} (step {})", step.id, index + 1)
    }

    fn checkpoint_identifier_hint(checkpoint: &WorkflowPlanCheckpoint, index: usize) -> String {
        format!("check_id={} (check {})", checkpoint.id, index + 1)
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

        let metadata = vec![format!(
            "Autorun: {}",
            if plan.autorun { "on" } else { "off" }
        )];
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
        if total_steps == 0 {
            return format!(
                "plan:{} autorun:{}",
                Self::plan_phase_label(plan.phase),
                if plan.autorun { "on" } else { "off" }
            );
        }
        format!(
            "plan:{} steps:{}/{} autorun:{}",
            Self::plan_phase_label(plan.phase),
            completed_steps,
            total_steps,
            if plan.autorun { "on" } else { "off" }
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

    fn step_goal(step: &WorkflowPlanStep) -> &str {
        let description = step.description.trim();
        if !description.is_empty() {
            description
        } else {
            step.title.trim()
        }
    }

    fn plan_summary_text(plan: &WorkflowPlan) -> String {
        let (completed_steps, total_steps, _, _) = Self::plan_progress_counts(plan);
        let mut parts = vec![format!("phase {}", Self::plan_phase_label(plan.phase))];
        if total_steps > 0 {
            parts.push(format!("steps {completed_steps}/{total_steps}"));
        }
        parts.push(format!(
            "autorun {}",
            if plan.autorun { "on" } else { "off" }
        ));
        parts.join(" | ")
    }

    fn plan_output_text(prefix: &str, plan: &WorkflowPlan) -> String {
        format!("{prefix}\n{}", Self::plan_summary_text(plan))
    }

    fn plan_current_text(plan: &WorkflowPlan) -> String {
        match Self::next_actionable_step(plan) {
            Some((index, step)) => format!(
                "Current step {}: '{}' [{}].\nGoal: {}\nStatus: {}.",
                index + 1,
                step.title,
                Self::plan_step_identifier_hint(step, index),
                Self::step_goal(step),
                Self::plan_step_status_label(step.status)
            ),
            None => "The active plan has no current actionable step.".to_string(),
        }
    }

    fn plan_get_text(plan: &WorkflowPlan, view: PlanGetView) -> String {
        match view {
            PlanGetView::Current => Self::plan_current_text(plan),
            PlanGetView::Summary => Self::plan_summary_text(plan),
            PlanGetView::Full => Self::workflow_plan_markdown(plan),
        }
    }

    fn plan_get_payload(plan: &WorkflowPlan, view: PlanGetView) -> serde_json::Value {
        match Self::next_actionable_step(plan) {
            Some((index, step)) => serde_json::json!({
                "plan": plan,
                "view": view,
                "current_step": step,
                "current_step_index": index,
                "current_step_goal": Self::step_goal(step),
            }),
            None => serde_json::json!({
                "plan": plan,
                "view": view,
                "current_step": serde_json::Value::Null,
                "current_step_goal": serde_json::Value::Null,
            }),
        }
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
                        "check {}.{} ('{}') is still {}",
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

    fn normalized_optional_identifier(
        value: Option<&str>,
        field_name: &str,
    ) -> SdkResult<Option<String>> {
        match value.map(str::trim) {
            Some("") => Err(PluginError::invalid_params(format!(
                "{field_name} must not be empty when provided"
            ))),
            Some(value) => Ok(Some(value.to_string())),
            None => Ok(None),
        }
    }

    fn validate_plan_update_input(input: &PlanUpdateInput) -> SdkResult<PlanUpdateTarget> {
        let phase_update_requested =
            input.phase.is_some() || input.autorun.is_some() || input.summary.is_some();
        let step_id = Self::normalized_optional_identifier(input.step_id.as_deref(), "step_id")?;
        let checkpoint_id =
            Self::normalized_optional_identifier(input.checkpoint_id.as_deref(), "check_id")?;

        if phase_update_requested {
            if step_id.is_some()
                || checkpoint_id.is_some()
                || input.status.is_some()
                || input.wait_until_ms.is_some()
                || input.note.is_some()
            {
                return Err(PluginError::invalid_params(
                    "plan.update must target either the plan itself or a step/check, not both"
                        .to_string(),
                ));
            }
            if input.summary.is_some() && input.phase != Some(WorkflowPlanPhase::Completed) {
                return Err(PluginError::invalid_params(
                    "plan.update summary is only valid when phase is `completed`".to_string(),
                ));
            }
            if input.phase.is_none() && input.autorun.is_none() {
                return Err(PluginError::invalid_params(
                    "plan.update requires `phase` or `autorun` for plan-level updates".to_string(),
                ));
            }
            return Ok(PlanUpdateTarget::Plan);
        }

        let Some(step_id) = step_id else {
            if checkpoint_id.is_some() {
                return Err(PluginError::invalid_params(
                    "plan.update check updates require step_id".to_string(),
                ));
            }
            return Err(PluginError::invalid_params(
                "plan.update requires either `phase` / `autorun` or `step_id`".to_string(),
            ));
        };

        if let Some(checkpoint_id) = checkpoint_id {
            if input.status.is_none() {
                return Err(PluginError::invalid_params(
                    "plan.update check updates require `status`".to_string(),
                ));
            }
            if input.wait_until_ms.is_some() || input.note.is_some() {
                return Err(PluginError::invalid_params(
                    "plan.update check updates do not support `wait_until_ms` or `note`"
                        .to_string(),
                ));
            }
            return Ok(PlanUpdateTarget::Check {
                step_id,
                checkpoint_id,
            });
        }

        if input.status.is_none() && input.wait_until_ms.is_none() && input.note.is_none() {
            return Err(PluginError::invalid_params(
                "plan.update step updates require at least one of `status`, `wait_until_ms`, or `note`"
                    .to_string(),
            ));
        }

        Ok(PlanUpdateTarget::Step(step_id))
    }

    fn plan_phase_requires_approval(phase: WorkflowPlanPhase) -> bool {
        matches!(
            phase,
            WorkflowPlanPhase::Active | WorkflowPlanPhase::Blocked | WorkflowPlanPhase::Completed
        )
    }

    fn plan_phase_is_approved(phase: WorkflowPlanPhase) -> bool {
        matches!(
            phase,
            WorkflowPlanPhase::Active | WorkflowPlanPhase::Blocked | WorkflowPlanPhase::Completed
        )
    }

    fn mark_plan_completed(plan: &mut WorkflowPlan, summary: Option<&str>) -> SdkResult<()> {
        Self::ensure_plan_ready_for_completion(plan)?;
        plan.phase = WorkflowPlanPhase::Completed;
        Self::append_completion_summary(plan, summary);
        Ok(())
    }

    fn validate_plan_phase_change(plan: &WorkflowPlan, phase: WorkflowPlanPhase) -> SdkResult<()> {
        match phase {
            WorkflowPlanPhase::Completed => Self::ensure_plan_ready_for_completion(plan),
            WorkflowPlanPhase::Active | WorkflowPlanPhase::Blocked => {
                if Self::plan_completion_blocker(plan).is_none() {
                    return Err(PluginError::invalid_params(format!(
                        "cannot set plan status to {}: all steps and checks are already complete; reopen a step or check first",
                        Self::plan_phase_label(phase)
                    )));
                }
                Ok(())
            }
            WorkflowPlanPhase::Planning | WorkflowPlanPhase::Cancelled => Ok(()),
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
        Ok(())
    }

    fn plan_auto_signature(
        plan: &WorkflowPlan,
        step_index: usize,
        step: &WorkflowPlanStep,
    ) -> SdkResult<String> {
        let serialized =
            serde_json::to_string(plan).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(format!("{serialized}:{step_index}:{}", step.id))
    }

    fn review_decision(response: &crate::plugin::sdk::host_api::AskUserResponse) -> Option<String> {
        response
            .answers
            .get("decision")
            .and_then(|values| values.first())
            .cloned()
            .or_else(|| {
                response
                    .answers
                    .values()
                    .find_map(|values| values.first().cloned())
            })
            .or_else(|| {
                response
                    .answers
                    .get("reply")
                    .and_then(|values| values.first())
                    .cloned()
            })
            .or_else(|| {
                let reply = response.reply.trim();
                (!reply.is_empty()).then_some(reply.to_string())
            })
    }

    fn phase_review_transition_summary(
        phase: WorkflowPlanPhase,
        effective_autorun: bool,
    ) -> String {
        match phase {
            WorkflowPlanPhase::Active => format!(
                "Move the plan to `active` with autorun {}.",
                if effective_autorun { "on" } else { "off" }
            ),
            WorkflowPlanPhase::Blocked => {
                "Move the plan to `blocked` after review approval.".to_string()
            }
            WorkflowPlanPhase::Completed => {
                "Mark the plan `completed` after review approval.".to_string()
            }
            WorkflowPlanPhase::Planning => {
                "Return the plan to `planning` after review approval.".to_string()
            }
            WorkflowPlanPhase::Cancelled => {
                "Move the plan to `cancelled` after review approval.".to_string()
            }
        }
    }

    fn phase_review_body_markdown(
        plan: &WorkflowPlan,
        phase: WorkflowPlanPhase,
        requested_autorun: Option<bool>,
        completion_summary: Option<&str>,
    ) -> String {
        let effective_autorun = requested_autorun.unwrap_or(plan.autorun);
        let mut sections = vec![
            "## Requested Status Change".to_string(),
            String::new(),
            Self::phase_review_transition_summary(phase, effective_autorun),
        ];
        if phase == WorkflowPlanPhase::Completed
            && let Some(summary) = completion_summary
                .map(str::trim)
                .filter(|value| !value.is_empty())
        {
            sections.push(String::new());
            sections.push("### Completion Summary".to_string());
            sections.push(String::new());
            sections.push(summary.to_string());
        }
        sections.push(String::new());
        sections.push(Self::workflow_plan_markdown(plan));
        sections.join("\n")
    }

    fn phase_review_request(
        plan: &WorkflowPlan,
        phase: WorkflowPlanPhase,
        requested_autorun: Option<bool>,
        completion_summary: Option<&str>,
    ) -> AskUserRequest {
        let requested_auto = requested_autorun.unwrap_or(plan.autorun);
        let mut options = if phase == WorkflowPlanPhase::Active {
            let approve_on = HostAskUserOption {
                label: PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON.to_string(),
                description: "Approve the plan, move it to active, and keep autorun on."
                    .to_string(),
            };
            let approve_off = HostAskUserOption {
                label: PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF.to_string(),
                description: "Approve the plan, move it to active, and keep autorun off."
                    .to_string(),
            };
            if requested_auto {
                vec![approve_on, approve_off]
            } else {
                vec![approve_off, approve_on]
            }
        } else {
            vec![HostAskUserOption {
                label: PLAN_REVIEW_DECISION_APPROVE.to_string(),
                description: match phase {
                    WorkflowPlanPhase::Blocked => {
                        "Approve the plan and move it to blocked.".to_string()
                    }
                    WorkflowPlanPhase::Completed => {
                        "Approve the plan and mark it completed.".to_string()
                    }
                    WorkflowPlanPhase::Planning => {
                        "Approve the plan and return it to planning.".to_string()
                    }
                    WorkflowPlanPhase::Cancelled => "Approve the plan and cancel it.".to_string(),
                    WorkflowPlanPhase::Active => unreachable!(),
                },
            }]
        };
        options.extend([
            HostAskUserOption {
                label: PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string(),
                description: "Return to planning so the plan can be edited further.".to_string(),
            },
            HostAskUserOption {
                label: PLAN_REVIEW_DECISION_REJECT.to_string(),
                description: "Reject the current plan and mark the review as rejected.".to_string(),
            },
            HostAskUserOption {
                label: PLAN_REVIEW_DECISION_CANCELLED.to_string(),
                description: "Cancel the plan entirely and stop work on it.".to_string(),
            },
        ]);
        AskUserRequest {
            title: "Review Plan Status Change".to_string(),
            body_markdown: Self::phase_review_body_markdown(
                plan,
                phase,
                requested_autorun,
                completion_summary,
            ),
            kind: "review".to_string(),
            submit_label: "Submit decision".to_string(),
            cancel_label: "Keep in planning".to_string(),
            questions: vec![HostAskUserQuestion {
                id: "decision".to_string(),
                header: "Decision".to_string(),
                question: format!(
                    "Choose whether this plan should move to {}.",
                    Self::plan_phase_label(phase)
                ),
                options,
                multiple: false,
                allow_custom: false,
            }],
            prompt: String::new(),
            options: Vec::new(),
            allow_free_text: false,
        }
    }

    async fn review_plan_status_transition(
        &self,
        mut plan: WorkflowPlan,
        phase: WorkflowPlanPhase,
        requested_autorun: Option<bool>,
        completion_summary: Option<&str>,
    ) -> SdkResult<ToolInvokeOutput> {
        Self::set_plan_phase(&mut plan, WorkflowPlanPhase::Planning, None)?;
        self.save_active_plan(&plan).await?;

        let response = self
            .host()?
            .ask_user(Self::phase_review_request(
                &plan,
                phase,
                requested_autorun,
                completion_summary,
            ))
            .await?;

        let decision = if response.cancelled {
            PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string()
        } else {
            Self::review_decision(&response)
                .unwrap_or_else(|| PLAN_REVIEW_DECISION_KEEP_PLANNING.to_string())
        };

        match decision.as_str() {
            PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON => {
                Self::set_plan_phase(&mut plan, phase, completion_summary)?;
                plan.autorun = true;
            }
            PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF
            | PLAN_REVIEW_DECISION_APPROVE_REQUESTED_PAUSE => {
                Self::set_plan_phase(&mut plan, phase, completion_summary)?;
                plan.autorun = false;
            }
            PLAN_REVIEW_DECISION_APPROVE | PLAN_REVIEW_DECISION_APPROVE_REQUESTED => {
                Self::set_plan_phase(&mut plan, phase, completion_summary)?;
                if let Some(autorun) = requested_autorun {
                    plan.autorun = autorun;
                }
            }
            PLAN_REVIEW_DECISION_CANCELLED => {
                Self::set_plan_phase(&mut plan, WorkflowPlanPhase::Cancelled, None)?;
            }
            _ => {
                plan.phase = WorkflowPlanPhase::Planning;
            }
        }
        self.save_active_plan(&plan).await?;

        let output_text =
            Self::plan_output_text(format!("Plan review decision: {decision}.").as_str(), &plan);
        let payload = serde_json::json!({
            "plan": plan,
            "decision": decision,
        });
        Ok(ToolInvokeOutput::text(output_text)
            .with_title("plan review")
            .with_payload(payload))
    }

    fn plan_lock_active(plan: &WorkflowPlan) -> bool {
        plan.phase == WorkflowPlanPhase::Planning
    }

    fn tool_allowed_during_planning(input: &ToolBeforeInput) -> bool {
        if input.tool_name == "plan"
            || input.tool_name == "user"
            || input.tool_name == "tools"
            || input.tool_name == "session"
            || input.tool_name == "agent"
        {
            return true;
        }
        if input.tool_name == "task" {
            return TaskToolActionInput::parse_input(input.input.clone()).is_ok_and(|task| {
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
                ToolTag::Mutating | ToolTag::FilesystemWrite | ToolTag::Snapshot
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

    fn autorun_prompt(plan: &WorkflowPlan, step_index: usize, step: &WorkflowPlanStep) -> String {
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
        let pending_checks = step
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
        if !pending_checks.is_empty() {
            lines.push("Pending checks:".to_string());
            lines.extend(pending_checks);
        }
        lines.push(
            "Update the plan state as you make progress. If the next step needs human input, stop and say exactly what is needed.".to_string(),
        );
        lines.push("</plan_context>".to_string());
        lines.join("\n")
    }

    async fn invoke_plan_get(&self, input: &PlanGetInput) -> SdkResult<ToolInvokeOutput> {
        let view = input.view.unwrap_or_default();
        let Some(plan) = self.load_active_plan().await? else {
            let payload = serde_json::json!({
                "plan": serde_json::Value::Null,
                "view": view,
                "current_step": serde_json::Value::Null,
                "current_step_goal": serde_json::Value::Null,
            });
            return Ok(ToolInvokeOutput::text("No plan.")
                .with_title("plan")
                .with_payload(payload));
        };
        let payload = Self::plan_get_payload(&plan, view);
        Ok(ToolInvokeOutput::text(Self::plan_get_text(&plan, view))
            .with_title("plan")
            .with_payload(payload))
    }

    async fn invoke_plan_set(&self, input: &PlanSetInput) -> SdkResult<ToolInvokeOutput> {
        let previous = self.load_active_plan().await?;
        let plan = self.build_plan(
            input.objective.as_str(),
            input.title.as_deref(),
            input.document_markdown.as_deref(),
            input.steps.as_slice(),
            input.autorun,
            previous.as_ref(),
        )?;
        self.save_active_plan(&plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(
            ToolInvokeOutput::text(Self::plan_output_text("Saved the plan.", &plan))
                .with_title("plan")
                .with_payload(payload),
        )
    }

    async fn invoke_plan_update(&self, input: &PlanUpdateInput) -> SdkResult<ToolInvokeOutput> {
        let Some(mut plan) = self.load_active_plan().await? else {
            return Err(PluginError::invalid_params("no active plan to update"));
        };
        let target = Self::validate_plan_update_input(input)?;
        let message = match target {
            PlanUpdateTarget::Plan => {
                let completion_summary = match input.phase {
                    Some(WorkflowPlanPhase::Completed) => input.summary.as_deref(),
                    _ => None,
                };
                let allow_direct_approval = self.config()?.plan.allow_direct_approval;
                if let Some(phase) = input.phase {
                    Self::validate_plan_phase_change(&plan, phase)?;
                    if Self::plan_phase_requires_approval(phase)
                        && !Self::plan_phase_is_approved(plan.phase)
                        && !allow_direct_approval
                    {
                        return self
                            .review_plan_status_transition(
                                plan,
                                phase,
                                input.autorun,
                                completion_summary,
                            )
                            .await;
                    }
                    Self::set_plan_phase(&mut plan, phase, completion_summary)?;
                }
                if let Some(autorun) = input.autorun {
                    plan.autorun = autorun;
                }
                "Updated the plan.".to_string()
            }
            PlanUpdateTarget::Step(step_id) => {
                let Some(step_index) = Self::resolve_plan_step_index(&plan, step_id.as_str())
                else {
                    return Err(PluginError::invalid_params(format!(
                        "unknown plan step '{}'; available steps: {}",
                        step_id,
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
                if let Some(status) = input.status {
                    step.status = status;
                    Self::cascade_terminal_step_status(step, status);
                }
                if let Some(wait_until_ms) = input.wait_until_ms {
                    step.wait_until_ms = Some(wait_until_ms);
                }
                if let Some(note) = input.note.as_deref() {
                    step.note = note.to_string();
                }
                format!("Updated step '{}'.", step.title)
            }
            PlanUpdateTarget::Check {
                step_id,
                checkpoint_id,
            } => {
                let Some(step_index) = Self::resolve_plan_step_index(&plan, step_id.as_str())
                else {
                    return Err(PluginError::invalid_params(format!(
                        "unknown plan step '{}'; available steps: {}",
                        step_id,
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
                        Self::resolve_checkpoint_index(step, checkpoint_id.as_str())
                    else {
                        return Err(PluginError::invalid_params(format!(
                            "unknown check '{}' for step '{}'; available checks: {}",
                            checkpoint_id,
                            step_id,
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
                    checkpoint.status = input
                        .status
                        .expect("validated plan check update requires status");
                    checkpoint.text.clone()
                };
                Self::reconcile_step_status_from_checkpoints(step);
                format!("Updated check '{checkpoint_text}'.")
            }
        };
        self.save_active_plan(&plan).await?;
        let payload = Self::plan_payload(&plan)?;
        Ok(
            ToolInvokeOutput::text(Self::plan_output_text(message.as_str(), &plan))
                .with_title("plan")
                .with_payload(payload),
        )
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

    async fn invoke_snapshot_enter(
        &self,
        args: &EnterSnapshotCommandInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let request = match args {
            EnterSnapshotCommandInput::New { name } => HostEnterSnapshotRequest {
                name: name.clone(),
                path: None,
            },
            EnterSnapshotCommandInput::Existing { path } => HostEnterSnapshotRequest {
                name: None,
                path: Some(path.clone()),
            },
        };
        self.host()?.enter_snapshot(request).await
    }

    async fn invoke_snapshot_exit(
        &self,
        args: &ExitSnapshotCommandInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.host()?
            .exit_snapshot(HostExitSnapshotRequest {
                action: args.exit_action.as_str().to_string(),
                discard_changes: args.discard_changes,
            })
            .await
    }

    async fn permission_snapshot_enter(
        &self,
        args: &EnterSnapshotCommandInput,
    ) -> SdkResult<Vec<PathRequest>> {
        snapshot_enter_permission_paths(self.workspace_root()?, args)
    }

    async fn permission_snapshot_exit(
        &self,
        _args: &ExitSnapshotCommandInput,
    ) -> SdkResult<Vec<PathRequest>> {
        Ok(Vec::new())
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
        let query = input.query.as_str();
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

    async fn invoke_tool_list(&self, input: &ToolListInput) -> SdkResult<ToolInvokeOutput> {
        let mut tools = self.host()?.list_tools().await?;
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        let total = tools.len();
        if let Some(limit) = input.limit {
            tools.truncate(limit.max(1) as usize);
        }
        let verbose = input.verbose.unwrap_or(false);

        let mut lines = vec![format!("Available tool(s): {}/{}.", tools.len(), total)];
        for tool in &tools {
            if verbose {
                let summary = tool
                    .summary
                    .as_deref()
                    .or(tool.description.as_deref())
                    .unwrap_or_default()
                    .trim();
                if summary.is_empty() {
                    lines.push(format!("- {}", tool.name));
                } else {
                    lines.push(format!("- {}: {}", tool.name, summary));
                }
            } else {
                lines.push(format!("- {}", tool.name));
            }
        }

        let payload = serde_json::json!({
            "tools": tools.iter().map(|tool| tool.name.clone()).collect::<Vec<_>>(),
            "total": total,
            "returned": tools.len(),
        });
        Ok(ToolInvokeOutput::text(lines.join("\n"))
            .with_title("Tool list")
            .with_payload(payload)
            .with_metadata("total_tools", total.to_string())
            .with_metadata("returned_tools", tools.len().to_string()))
    }

    async fn invoke_tool_help(&self, input: &ToolsHelpInput) -> SdkResult<ToolInvokeOutput> {
        let requested = input.tool.as_str();
        let config = self.config()?;
        let tools = self.host()?.list_tools().await?;
        let mut exact: Option<&ToolDescriptor> = None;
        let mut case_insensitive: Option<&ToolDescriptor> = None;
        for tool in &tools {
            if tool.name == requested {
                exact = Some(tool);
                break;
            }
            if case_insensitive.is_none() && tool.name.eq_ignore_ascii_case(requested) {
                case_insensitive = Some(tool);
            }
        }
        let Some(descriptor) = exact.or(case_insensitive) else {
            let suggestions = Self::suggest_tool_names(requested, &tools);
            let message = if suggestions.is_empty() {
                format!("unknown tool '{requested}'")
            } else {
                format!(
                    "unknown tool '{requested}'. Did you mean {}?",
                    suggestions
                        .iter()
                        .map(|tool| format!("`{tool}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            return Err(PluginError::invalid_params(message));
        };

        let mut lines = vec![format!("Tool: {}", descriptor.name)];
        if let Some(before_help) = descriptor
            .before_help
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            lines.push("Before help:".to_string());
            lines.push(before_help.to_string());
        }
        lines.push("Usage:".to_string());
        if let Some(schema) = descriptor.input_schema.as_ref() {
            if let Some(arguments) = crate::tool::definition::schema_usage_text(schema) {
                lines.push(arguments);
            } else {
                lines.push("- No input arguments.".to_string());
            }
        } else {
            lines.push("- No input arguments.".to_string());
        }
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
        let declared_examples = descriptor.examples.clone();
        let generated_examples = descriptor
            .input_schema
            .as_ref()
            .map(crate::tool::definition::schema_example_texts)
            .unwrap_or_default();
        if !declared_examples.is_empty() || !generated_examples.is_empty() {
            lines.push("Examples:".to_string());
            let mut seen_examples = HashSet::new();
            if !declared_examples.is_empty() {
                lines.push("Declared examples:".to_string());
                for example in &declared_examples {
                    if seen_examples.insert(example.clone()) {
                        lines.push(format!("- {example}"));
                    }
                }
            }
            if !generated_examples.is_empty() {
                lines.push("Generated examples:".to_string());
                for example in &generated_examples {
                    if seen_examples.insert(example.clone()) {
                        lines.push(format!("- {example}"));
                    }
                }
            }
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
        if let Some(after_help) = descriptor
            .after_help
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            lines.push("After help:".to_string());
            lines.push(after_help.to_string());
        }

        Ok(
            ToolInvokeOutput::text(lines.join("\n"))
                .with_title(format!("{} help", descriptor.name)),
        )
    }

    fn suggest_tool_names(requested: &str, tools: &[ToolDescriptor]) -> Vec<String> {
        let candidate_names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        let mut suggestions = crate::tool::suggest_tool_names(requested, candidate_names, 1);
        if suggestions.is_empty() {
            let catalog = tools
                .iter()
                .cloned()
                .map(Self::tool_search_document_from_descriptor)
                .collect::<Vec<_>>();
            if let Ok(results) = search_tool_catalog(&catalog, requested, 3) {
                for tool in results {
                    if !tool.name.eq_ignore_ascii_case(requested)
                        && !suggestions.contains(&tool.name)
                    {
                        suggestions.push(tool.name);
                    }
                    if suggestions.len() >= 3 {
                        break;
                    }
                }
            }
        }
        suggestions
    }

    fn invoke_tool_catalog_usage() -> ToolInvokeOutput {
        ToolInvokeOutput::text(
            [
                "Tool catalog usage:",
                "Usage:",
                r#"- {"action":"usage"} or {}"#,
                "Examples:",
                r#"- List: {"action":"list"}"#,
                r#"- Search: {"action":"search","query":"web","limit":8}"#,
                r#"- Help: {"action":"help","tool":"web.search"}"#,
                "Notes:",
                "This command only inspects tool help; call the target tool directly to execute it.",
            ]
            .join("\n"),
        )
        .with_title("Tool catalog usage")
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
                    .unwrap_or_else(|| PLAN_REVIEW_DECISION_APPROVE.to_string()),
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

    fn init_test_plugin_with_plan_config(
        default_autorun: bool,
        allow_direct_approval: bool,
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
        plugin
            .initialize(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: PathBuf::from("."),
                    plugin_id: "planning-test".to_string(),
                    host_callback_url: None,
                    host_callback_token: None,
                    config: json!({}),
                    protocol_version: 1,
                },
                WorkflowPluginConfig {
                    tool_catalog: WorkflowToolCatalogConfig::default(),
                    plan: WorkflowPlanConfig {
                        default_autorun,
                        allow_direct_approval,
                    },
                },
                host.clone(),
            )
            .expect("workflow plugin should initialize");
        (plugin, host, runtime)
    }

    fn init_test_plugin(
        default_autorun: bool,
        ask_user_reply: Option<&str>,
    ) -> (
        WorkflowPlugin,
        Arc<WorkflowTestHostClient>,
        tokio::runtime::Runtime,
    ) {
        init_test_plugin_with_plan_config(default_autorun, true, ask_user_reply)
    }

    fn invoke_plan_result(
        runtime: &tokio::runtime::Runtime,
        plugin: &WorkflowPlugin,
        input: serde_json::Value,
    ) -> SdkResult<ToolInvokeOutput> {
        let parsed = PlanToolInput::parse_input(input)?;
        runtime.block_on(parsed.dispatch_tool_invoke(plugin))
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
                "backend": "remote"
            }),
        )
        .expect_err("tools search should reject unknown fields");
        assert!(err.to_string().contains("unknown field 'backend'"));

        let err = resolve_tools_tool_input(json!({
            "action": "search",
            "query": "memory",
            "backend": "remote"
        }))
        .expect_err("tools resolver should preserve unknown-field rejection");
        assert!(err.to_string().contains("unknown field 'backend'"));
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
    fn tools_list_accepts_explicit_action() {
        let (action, action_input) = resolve_tools_tool_input(json!({
            "action": "list",
            "limit": 5,
            "verbose": true
        }))
        .expect("list action should resolve");

        assert_eq!(action, "list");
        let parsed: ToolListInput = serde_json::from_value(action_input).expect("list input");
        assert_eq!(parsed.limit, Some(5));
        assert_eq!(parsed.verbose, Some(true));
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
    fn tools_help_accepts_tool_without_action() {
        let (action, action_input) = resolve_tools_tool_input(json!({
            "tool": "  web.search  ",
            "include_schema": false
        }))
        .expect("tool-only tools input should infer help");

        assert_eq!(action, "help");
        let parsed: ToolsHelpInput = serde_json::from_value(action_input).expect("help input");
        assert_eq!(parsed.tool, "web.search");
        assert_eq!(parsed.include_schema, Some(false));
    }

    #[test]
    fn tools_search_trims_query_before_parse() {
        let (action, action_input) = resolve_tools_tool_input(json!({
            "query": "  web search  ",
            "limit": 2
        }))
        .expect("query-only tools input should infer search");

        assert_eq!(action, "search");
        let parsed: ToolSearchToolInput =
            serde_json::from_value(action_input).expect("search input");
        assert_eq!(parsed.query, "web search");
        assert_eq!(parsed.limit, Some(2));
    }

    #[test]
    fn tools_search_schema_includes_flattened_shape_field_docs() {
        let schema = ToolsToolInput::tool_definition().sanitized_input_schema();
        let usage = crate::tool::definition::schema_usage_text(&schema).expect("tools usage text");
        assert!(usage.contains("Search text used to rank matching tool names and descriptions."));
        assert!(usage.contains("Maximum number of search results to return."));
    }

    #[test]
    fn session_rename_trims_and_validates_title_at_parse_time() {
        let parsed = SessionToolInput::parse_input(json!({
            "action": "rename",
            "title": "  Focused Session  "
        }))
        .expect("session rename should trim title before parse");
        match parsed {
            SessionToolInput::Rename { args } => assert_eq!(args.title, "Focused Session"),
            other => panic!("expected rename variant, got {other:?}"),
        }

        let err = SessionToolInput::parse_input(json!({
            "action": "rename",
            "title": "   "
        }))
        .expect_err("session rename should reject blank title during parse");
        assert!(err.to_string().contains("field `title` must not be empty"));
    }

    #[test]
    fn session_rename_schema_includes_title_parameter() {
        let shape_schema = SessionRenameToolInput::input_schema();
        assert!(
            shape_schema.pointer("/properties/title").is_some(),
            "{shape_schema:#}"
        );

        let tool_schema = SessionToolInput::input_schema();
        assert!(
            tool_schema.pointer("/oneOf/1/properties/title").is_some(),
            "{tool_schema:#}"
        );

        let sanitized = SessionToolInput::tool_definition().sanitized_input_schema();
        assert!(
            sanitized.pointer("/oneOf/1/properties/title").is_some(),
            "{sanitized:#}"
        );
    }

    #[test]
    fn agent_inputs_trim_flattened_shape_fields_at_parse_time() {
        let parsed = AgentToolInput::parse_input(json!({
            "action": "switch",
            "agent": "  reviewer  ",
            "push_previous": true
        }))
        .expect("agent switch should trim agent through flattened shape");
        match parsed {
            AgentToolInput::Switch { args } => {
                assert_eq!(args.agent.as_deref(), Some("reviewer"));
                assert!(args.push_previous);
            }
            other => panic!("expected switch variant, got {other:?}"),
        }
    }

    #[test]
    fn task_input_trims_and_validates_flattened_shape_fields_at_parse_time() {
        let parsed = TaskToolInput::parse_input(json!({
            "description": "  Draft the migration plan  ",
            "prompt": "  Review the current plugin macros and summarize the remaining gaps.  ",
            "subagent_type": "explore",
            "task_id": "  task-42  ",
            "command": "  cargo test -p agena --lib  "
        }))
        .expect("task input should parse");
        assert_eq!(parsed.description, "Draft the migration plan");
        assert_eq!(
            parsed.prompt,
            "Review the current plugin macros and summarize the remaining gaps."
        );
        assert_eq!(parsed.task_id.as_deref(), Some("task-42"));
        assert_eq!(parsed.command.as_deref(), Some("cargo test -p agena --lib"));

        let schema = TaskToolInput::input_schema();
        let usage = crate::tool::definition::schema_usage_text(&schema).expect("task usage text");
        assert!(usage.contains("Short label for the subtask session."));
        assert!(usage.contains("Full instruction payload for the delegated subtask."));
        assert!(usage.contains("Which subagent profile should execute the subtask."));

        let err = TaskToolInput::parse_input(json!({
            "description": "   ",
            "prompt": "   ",
            "subagent_type": "verify"
        }))
        .expect_err("blank task description and prompt should be rejected");
        assert!(
            err.to_string()
                .contains("field `description` must not be empty")
        );
    }

    #[test]
    fn tools_help_ignores_search_only_noise_fields() {
        let (action, action_input) = resolve_tools_tool_input(json!({
            "action": "help",
            "tool": "web.search",
            "query": "web",
            "limit": 10,
            "include_schema": true
        }))
        .expect("help should ignore known search-only fields");

        assert_eq!(action, "help");
        let parsed: ToolsHelpInput = serde_json::from_value(action_input).expect("help input");
        assert_eq!(parsed.tool, "web.search");
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
    fn tools_usage_action_is_declared_and_resolves() {
        let (action, action_input) = ToolsToolInput::resolve_tool(
            "tools",
            json!({
                "action": "usage"
            }),
        )
        .expect("usage action should resolve");

        assert_eq!(action, "usage");
        assert_eq!(action_input, json!({}));

        let schema = ToolsToolInput::tool_definition().sanitized_input_schema();
        let literals = schema_string_literals(&schema, &schema);
        assert!(literals.contains("list"));
        assert!(literals.contains("usage"));
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
        assert!(config.plan.allow_direct_approval);
    }

    #[test]
    fn workflow_plugin_config_accepts_plan_approval_policy() {
        let config: WorkflowPluginConfig = serde_json::from_value(json!({
            "plan": {
                "default_autorun": false,
                "allow_direct_approval": false
            }
        }))
        .expect("workflow config should parse");

        assert!(!config.plan.default_autorun);
        assert!(!config.plan.allow_direct_approval);
    }

    #[test]
    fn allowlisted_workflow_tools_skip_plan_storage_probe_before_init() {
        let plugin = WorkflowPlugin::new();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("workflow test runtime");

        let patch = rt
            .block_on(plugin.tool_execute_before_hook(ToolBeforeInput {
                tool_name: "session".to_string(),
                plugin_name: "runtime-test".to_string(),
                session_id: 1,
                call_id: 1,
                workspace_root: ".".to_string(),
                tags: vec![ToolTag::ReadOnly],
                input: json!({ "action": "get" }),
                title_override: None,
                metadata: Default::default(),
            }))
            .expect("allowlisted workflow tool should not need plan storage");

        assert!(patch.is_none());
    }

    #[test]
    fn task_and_snapshot_tools_declare_plan_storage_capability() {
        let task_definition = TaskToolActionInput::tool_definition();
        assert!(
            task_definition
                .capabilities
                .contains(&HostCapability::PluginStorage)
        );

        let snapshot_definition = SnapshotToolInput::tool_definition();
        assert!(
            snapshot_definition
                .capabilities
                .contains(&HostCapability::PluginStorage)
        );
    }

    #[test]
    fn snapshot_permission_paths_follow_surface_shape() {
        let (plugin, _host, runtime) = init_test_plugin(true, None);

        let existing_paths = runtime
            .block_on(
                SnapshotToolInput::parse_tool(
                    "snapshot",
                    json!({
                    "action": "enter",
                    "target": "existing",
                    "path": "/tmp/existing-snapshot"
                    }),
                )
                .expect("parse existing snapshot")
                .dispatch_permission_paths(&plugin),
            )
            .expect("existing snapshot permission paths");
        assert_eq!(
            existing_paths,
            vec![
                PathRequest::read("/tmp/existing-snapshot"),
                PathRequest::write("/tmp/existing-snapshot"),
            ]
        );

        let new_paths = runtime
            .block_on(
                SnapshotToolInput::parse_tool(
                    "snapshot",
                    json!({
                    "action": "enter",
                    "target": "new",
                    "name": "feature-branch"
                    }),
                )
                .expect("parse new snapshot")
                .dispatch_permission_paths(&plugin),
            )
            .expect("new snapshot permission paths");
        assert_eq!(new_paths.len(), 1);
        assert_eq!(new_paths[0].kind, crate::plugin::sdk::PathKind::Write);
        assert!(
            new_paths[0].path.ends_with("/snapshots") || new_paths[0].path.ends_with("\\snapshots")
        );

        let exit_paths = runtime
            .block_on(
                SnapshotToolInput::parse_tool(
                    "snapshot",
                    json!({
                    "action": "exit",
                    "exit_action": "keep",
                    "discard_changes": false
                    }),
                )
                .expect("parse exit snapshot")
                .dispatch_permission_paths(&plugin),
            )
            .expect("exit snapshot permission paths");
        assert!(exit_paths.is_empty());
    }

    #[test]
    fn snapshot_surface_flattened_enter_shape_reuses_inner_rules_at_parse_time() {
        let parsed = SnapshotToolInput::parse_input(json!({
            "action": "enter",
            "target": "new",
            "name": "  feature-x  "
        }))
        .expect("snapshot enter should parse");
        let SnapshotToolInput::Enter { args } = parsed else {
            panic!("expected enter variant");
        };
        let EnterSnapshotCommandInput::New { name } = args else {
            panic!("expected new target variant");
        };
        assert_eq!(name.as_deref(), Some("feature-x"));

        let parsed = SnapshotToolInput::parse_input(json!({
            "action": "enter",
            "target": "existing",
            "path": "/tmp/existing-snapshot"
        }))
        .expect("snapshot enter existing should parse");
        let SnapshotToolInput::Enter { args } = parsed else {
            panic!("expected enter variant");
        };
        let EnterSnapshotCommandInput::Existing { path } = args else {
            panic!("expected existing target variant");
        };
        assert_eq!(path, "/tmp/existing-snapshot");

        let parsed = SnapshotToolInput::parse_input(json!({
            "action": "exit",
            "exit_action": "remove",
            "discard_changes": true
        }))
        .expect("snapshot exit should parse");
        let SnapshotToolInput::Exit { args } = parsed else {
            panic!("expected exit variant");
        };
        assert_eq!(args.exit_action, ExitSnapshotAction::Remove);
        assert!(args.discard_changes);
    }

    #[test]
    fn snapshot_enter_schema_exposes_target_variants() {
        let schema = SnapshotToolInput::tool_definition().sanitized_input_schema();
        assert!(
            schema.pointer("/oneOf/0/properties/target").is_some(),
            "{schema:#}"
        );
        assert!(
            schema.pointer("/oneOf/0/properties/name").is_some(),
            "{schema:#}"
        );
        assert!(
            schema.pointer("/oneOf/0/properties/path").is_some(),
            "{schema:#}"
        );
    }

    #[test]
    fn plan_create_accepts_model_payload_when_step_title_falls_back_to_description() {
        let plugin = WorkflowPlugin::new();
        let (action, action_input) = resolve_plan_tool_input(json!({
            "action": "set",
            "autorun": false,
            "check_id": "cp1",
            "document_markdown": "# Plan: 尝试 plan 功能\n\n目标：演示并验证 plan 工具可用。",
            "note": "初次创建计划",
            "objective": "尝试一下 plan 功能",
            "phase": "planning",
            "status": "pending",
            "step_id": "step1",
            "steps": [
                {
                    "checks": [
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
        assert_eq!(action, "set");
        let args: PlanSetInput =
            serde_json::from_value(action_input).expect("plan set input should deserialize");

        let plan = plugin
            .build_plan(
                args.objective.as_str(),
                args.title.as_deref(),
                args.document_markdown.as_deref(),
                args.steps.as_slice(),
                args.autorun,
                None,
            )
            .expect("plan should build");

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].title, "创建计划");
        assert_eq!(plan.steps[0].description, "创建计划");
    }

    #[test]
    fn resolve_plan_tool_input_normalizes_canonical_actions() {
        let (trimmed_step_action, trimmed_step_input) = resolve_plan_tool_input(json!({
            "action": "update",
            "step_id": "  step_2  ",
            "status": "blocked",
            "note": "  waiting on review  "
        }))
        .expect("plan.update should trim ids and notes during parse");
        assert_eq!(trimmed_step_action, "update");
        let trimmed_step_args: PlanUpdateInput = serde_json::from_value(trimmed_step_input)
            .expect("plan.update input should deserialize");
        assert_eq!(trimmed_step_args.step_id.as_deref(), Some("step_2"));
        assert_eq!(trimmed_step_args.note.as_deref(), Some("waiting on review"));

        let (status_action, status_input) = resolve_plan_tool_input(json!({
            "action": "update",
            "phase": "active",
            "autorun": true
        }))
        .expect("plan.update should parse canonical fields");
        assert_eq!(status_action, "update");
        let status_args: PlanUpdateInput =
            serde_json::from_value(status_input).expect("status input should deserialize");
        assert_eq!(status_args.phase, Some(WorkflowPlanPhase::Active));
        assert_eq!(status_args.autorun, Some(true));
    }

    #[test]
    fn plan_update_requires_a_target_or_plan_fields() {
        let err = WorkflowPlugin::validate_plan_update_input(&PlanUpdateInput::default())
            .expect_err("plan.update should reject empty updates");
        assert!(
            err.to_string()
                .contains("plan.update requires either `phase` / `autorun` or `step_id`")
        );
    }

    #[test]
    fn plan_nested_shape_fields_trim_and_validate_at_parse_time() {
        let parsed = PlanToolInput::parse_input(json!({
            "action": "update",
            "autorun": true,
            "summary": "  keep moving  "
        }))
        .expect("plan.update should trim summary through flattened shape");
        match parsed {
            PlanToolInput::Update { args } => {
                assert_eq!(args.summary.as_deref(), Some("keep moving"));
                assert_eq!(args.autorun, Some(true));
            }
            other => panic!("expected update variant, got {other:?}"),
        }

        let parsed = PlanToolInput::parse_input(json!({
            "action": "update",
            "step_id": "  step_1  ",
            "check_id": "  cp_1  ",
            "status": "completed"
        }))
        .expect("plan.update should trim ids through flattened shape");
        match parsed {
            PlanToolInput::Update { args } => {
                assert_eq!(args.step_id.as_deref(), Some("step_1"));
                assert_eq!(args.checkpoint_id.as_deref(), Some("cp_1"));
            }
            other => panic!("expected update variant, got {other:?}"),
        }

        let parsed = PlanToolInput::parse_input(json!({
            "action": "update",
            "step_id": "   ",
            "status": "blocked"
        }))
        .expect("plan.update should still parse before semantic validation");
        let err = match parsed {
            PlanToolInput::Update { args } => WorkflowPlugin::validate_plan_update_input(&args)
                .expect_err("plan.update should reject blank step ids"),
            other => panic!("expected update variant, got {other:?}"),
        };
        assert!(err.to_string().contains("step_id"));
    }

    #[test]
    fn user_request_input_validation_runs_at_parse_time() {
        let err = UserToolInput::parse_input(json!({
            "action": "request_input",
            "questions": []
        }))
        .expect_err("user.request_input should reject empty question sets during parse");
        assert!(
            err.to_string()
                .contains("field `questions` requires at least 1 item")
        );
    }

    #[test]
    fn user_request_input_schema_includes_question_parameters() {
        let schema = UserToolInput::tool_definition().sanitized_input_schema();
        assert!(
            schema.pointer("/oneOf/0/properties/questions").is_some(),
            "{schema:#}"
        );

        let model_schema = crate::tool::model_safe_tool_schema(&schema);
        assert!(
            model_schema.pointer("/properties/questions").is_some(),
            "{model_schema:#}"
        );
    }

    #[test]
    fn user_request_input_rejects_too_many_questions_at_parse_time() {
        let err = UserToolInput::parse_input(json!({
            "action": "request_input",
            "questions": [
                { "id": "q1", "question": "One?", "options": [{ "label": "A", "description": "" }] },
                { "id": "q2", "question": "Two?", "options": [{ "label": "A", "description": "" }] },
                { "id": "q3", "question": "Three?", "options": [{ "label": "A", "description": "" }] },
                { "id": "q4", "question": "Four?", "options": [{ "label": "A", "description": "" }] }
            ]
        }))
        .expect_err("user.request_input should reject too many questions during parse");
        assert!(
            err.to_string()
                .contains("field `questions` accepts at most 3 items")
        );
    }

    #[test]
    fn user_request_input_rejects_long_headers_at_parse_time() {
        let err = UserToolInput::parse_input(json!({
            "action": "request_input",
            "questions": [{
                "id": "q1",
                "header": "Header too long",
                "question": "Pick one",
                "options": [{ "label": "A", "description": "" }]
            }]
        }))
        .expect_err("user.request_input should reject long headers during parse");
        assert!(
            err.to_string()
                .contains("field `questions[].header` must be at most 12 characters")
        );
    }

    #[test]
    fn user_request_input_requires_options_or_allow_custom_at_parse_time() {
        let err = UserToolInput::parse_input(json!({
            "action": "request_input",
            "questions": [{
                "id": "q1",
                "question": "Pick one"
            }]
        }))
        .expect_err("user.request_input should reject questions without options or allow_custom during parse");
        assert!(err.to_string().contains(
            "field `questions[].allow_custom` is required unless `questions[].options` is present"
        ));
    }

    #[test]
    fn user_request_input_rejects_duplicate_question_ids_at_parse_time() {
        let err = UserToolInput::parse_input(json!({
            "action": "request_input",
            "questions": [
                { "id": "q1", "question": "One?", "allow_custom": true },
                { "id": " q1 ", "question": "Two?", "allow_custom": true }
            ]
        }))
        .expect_err("duplicate question ids should be rejected during parse");
        assert!(
            err.to_string()
                .contains("field `questions[].id` must not contain duplicate values")
        );
    }

    #[test]
    fn user_request_input_rejects_duplicate_option_labels_per_question_at_parse_time() {
        let err = UserToolInput::parse_input(json!({
            "action": "request_input",
            "questions": [{
                "id": "q1",
                "question": "Pick one",
                "options": [
                    { "label": "A", "description": "" },
                    { "label": " A ", "description": "" }
                ]
            }]
        }))
        .expect_err("duplicate option labels should be rejected during parse");
        assert!(err.to_string().contains(
            "field `questions[].options[].label` must not contain duplicate values within `questions[]`"
        ));
    }

    #[test]
    fn plan_create_overwrites_existing_plan_and_inherits_autorun_when_omitted() {
        let (plugin, _host, runtime) = init_test_plugin(false, None);

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
                "objective": "Initial draft plan.",
                "autorun": true,
                "steps": [
                    {
                        "id": "step_initial",
                        "title": "Write the initial draft",
                        "executor": "ai"
                    }
                ]
            }),
        );

        let overwritten = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
                "objective": "Overwritten draft plan.",
                "steps": [
                    {
                        "id": "step_overwritten",
                        "title": "Write the replacement draft",
                        "executor": "ai"
                    }
                ]
            }),
        );

        let plan = output_plan(&overwritten);
        assert_eq!(plan.objective, "Overwritten draft plan.");
        assert_eq!(plan.phase, WorkflowPlanPhase::Planning);
        assert!(plan.autorun);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].id, "step_overwritten");
        assert!(overwritten.output_text.contains("Saved the plan."));
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
            &PlanToolInput::tool_definition().sanitized_input_schema(),
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
            autorun: false,
            ..WorkflowPlan::default()
        };

        let rendered = WorkflowPlugin::workflow_plan_markdown(&plan);

        assert!(rendered.starts_with("# Plan Demo"));
        assert!(rendered.contains("_Autorun: off_"));
        assert!(!rendered.contains("# Demonstrate creating a plan"));
        assert!(!rendered.contains("Objective:"));
        assert!(!rendered.contains("Phase:"));
    }

    #[test]
    fn workflow_plan_markdown_avoids_repeating_fallback_step_descriptions() {
        let plan = WorkflowPlan {
            title: "Plan Demo".to_string(),
            objective: "Plan Demo".to_string(),
            autorun: true,
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
    fn plan_current_reports_actionable_step_and_goal() {
        let (plugin, _host, runtime) = init_test_plugin(false, None);

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
                "objective": "Track the current actionable step.",
                "title": "Current Step Plan",
                "autorun": false,
                "steps": [
                    {
                        "id": "step_done",
                        "title": "Finish setup",
                        "executor": "ai",
                        "status": "completed"
                    },
                    {
                        "id": "step_focus",
                        "title": "Investigate the failure",
                        "description": "Find the root cause and define the minimal fix.",
                        "executor": "ai"
                    }
                ]
            }),
        );

        let current = invoke_plan(&runtime, &plugin, json!({ "action": "get" }));
        assert!(
            current.output_text.contains(
                "Current step 2: 'Investigate the failure' [step_id=step_focus (step 2)]."
            )
        );
        assert!(
            current
                .output_text
                .contains("Goal: Find the root cause and define the minimal fix.")
        );
        assert!(current.output_text.contains("Status: pending."));

        let payload = current
            .payload
            .expect("current output should include payload");
        assert_eq!(payload["current_step"]["id"], "step_focus");
        assert_eq!(payload["current_step_index"], 1);
        assert_eq!(
            payload["current_step_goal"],
            "Find the root cause and define the minimal fix."
        );
    }

    #[test]
    fn plan_tool_flow_blocks_premature_completion_and_reports_status() {
        let (plugin, host, runtime) = init_test_plugin(false, None);

        let created = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
                "objective": "Test plan creation and retrieval.",
                "title": "Plan Trial",
                "document_markdown": "# Plan Trial\n\n- Test plan creation and retrieval.\n- No code changes required.",
                "autorun": false,
                "steps": [
                    {
                        "id": "step_trial",
                        "title": "Create a minimal draft plan.",
                        "executor": "ai",
                        "note": "Initial trial step",
                        "checks": [
                            {
                                "id": "cp_trial",
                                "text": "Create a minimal draft plan."
                            }
                        ]
                    }
                ]
            }),
        );
        assert!(
            created
                .output_text
                .contains("phase planning | steps 0/1 | autorun off")
        );
        assert!(!created.output_text.contains("# Plan Trial"));
        assert_eq!(
            host.statusline_content(PLAN_STATUSLINE_SEGMENT_ID)
                .as_deref(),
            Some("plan:planning steps:0/1 autorun:off")
        );

        let completion_err = invoke_plan_result(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "completed",
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
                "action": "update",
                "phase": "completed"
            }),
        )
        .expect_err("plan completion should fail while work remains");
        assert!(runtime_completion_err.to_string().contains(
            "cannot complete plan: step 1 ('Create a minimal draft plan.') is still pending"
        ));

        let updated = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
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
        assert!(
            updated
                .output_text
                .contains("phase planning | steps 1/1 | autorun off")
        );
        assert!(
            !updated
                .output_text
                .contains("[x] Create a minimal draft plan.")
        );

        let completed = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "completed",
                "summary": "Minimal trial plan"
            }),
        );
        let completed_plan = output_plan(&completed);
        assert_eq!(completed_plan.phase, WorkflowPlanPhase::Completed);
        assert!(
            completed
                .output_text
                .contains("phase completed | steps 1/1 | autorun off")
        );
        assert!(!completed.output_text.contains("## Completion Summary"));
        assert_eq!(
            host.statusline_content(PLAN_STATUSLINE_SEGMENT_ID)
                .as_deref(),
            Some("plan:completed steps:1/1 autorun:off")
        );

        let current = invoke_plan(&runtime, &plugin, json!({ "action": "get" }));
        assert!(
            current
                .output_text
                .contains("The active plan has no current actionable step.")
        );
        let current_payload = current
            .payload
            .expect("current output should include payload");
        assert!(current_payload["current_step"].is_null());
    }

    #[test]
    fn plan_update_check_accepts_check_index_hints() {
        let (plugin, _host, runtime) = init_test_plugin(false, None);

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
                "objective": "Exercise check matching.",
                "title": "Check Plan",
                "autorun": false,
                "steps": [
                    {
                        "id": "step_review",
                        "title": "Review the plan",
                        "executor": "ai",
                        "checks": [
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
                "action": "update",
                "step_id": "step1",
                "check_id": "cp1",
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
    fn plan_update_rejects_summary_until_completed() {
        let (plugin, _host, runtime) = init_test_plugin(false, None);

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
                "objective": "Exercise summary tolerance.",
                "title": "Summary Plan",
                "autorun": false,
                "steps": [
                    {
                        "id": "step_summary",
                        "title": "Finish the work",
                        "executor": "ai"
                    }
                ]
            }),
        );

        let err = invoke_plan_result(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "blocked",
                "summary": "This should be ignored."
            }),
        )
        .expect_err("summary should only be accepted when completing the plan");
        assert!(
            err.to_string()
                .contains("plan.update summary is only valid when phase is `completed`")
        );
    }

    #[test]
    fn plan_set_status_active_requests_review_when_direct_approval_disabled() {
        let (plugin, host, runtime) = init_test_plugin_with_plan_config(
            false,
            false,
            Some(PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF),
        );

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
                "objective": "Require review before activation.",
                "title": "Review Required Plan",
                "autorun": false,
                "steps": [
                    {
                        "id": "step_review_gate",
                        "title": "Wait for approval",
                        "executor": "ai"
                    }
                ]
            }),
        );

        let reviewed = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "active"
            }),
        );
        let request = host
            .last_ask_user_request()
            .expect("phase change should issue a host review request");
        assert_eq!(request.kind, "review");
        assert!(request.body_markdown.contains("## Requested Status Change"));
        assert!(
            request
                .body_markdown
                .contains("Move the plan to `active` with autorun off.")
        );
        assert!(
            request.questions[0]
                .options
                .iter()
                .any(|option| option.label == PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_ON)
        );
        assert!(
            request.questions[0]
                .options
                .iter()
                .any(|option| option.label == PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF)
        );

        let plan = output_plan(&reviewed);
        assert_eq!(plan.phase, WorkflowPlanPhase::Active);
        assert!(!plan.autorun);
        assert!(
            reviewed
                .output_text
                .contains("Plan review decision: Approve with autorun off.")
        );
    }

    #[test]
    fn plan_set_status_active_keep_planning_review_does_not_error() {
        let (plugin, _host, runtime) = init_test_plugin_with_plan_config(
            false,
            false,
            Some(PLAN_REVIEW_DECISION_KEEP_PLANNING),
        );

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
                "objective": "Stay in planning after review.",
                "title": "Keep Planning Plan",
                "steps": [
                    {
                        "id": "step_keep_planning",
                        "title": "Wait for edits",
                        "executor": "ai"
                    }
                ]
            }),
        );

        let reviewed = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "active"
            }),
        );
        let plan = output_plan(&reviewed);
        assert_eq!(plan.phase, WorkflowPlanPhase::Planning);
        assert!(
            reviewed
                .output_text
                .contains("Plan review decision: Keep in planning.")
        );
    }

    #[test]
    fn approved_plan_can_change_phase_when_direct_approval_disabled() {
        let (plugin, _host, runtime) = init_test_plugin_with_plan_config(
            false,
            false,
            Some(PLAN_REVIEW_DECISION_APPROVE_ACTIVE_AUTORUN_OFF),
        );

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
                "objective": "Move between approved phases after review.",
                "title": "Approved Plan",
                "steps": [
                    {
                        "id": "step_reviewed",
                        "title": "Enter the approved lifecycle",
                        "executor": "ai"
                    }
                ]
            }),
        );

        let activated = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "active"
            }),
        );
        let activated_plan = output_plan(&activated);
        assert_eq!(activated_plan.phase, WorkflowPlanPhase::Active);

        let blocked = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "blocked"
            }),
        );
        let blocked_plan = output_plan(&blocked);
        assert_eq!(blocked_plan.phase, WorkflowPlanPhase::Blocked);
    }

    #[test]
    fn plan_cancel_and_restore_actions_switch_phase_flexibly() {
        let (plugin, host, runtime) = init_test_plugin(false, None);

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
                "objective": "Exercise plan cancellation.",
                "title": "Cancelable Plan",
                "autorun": false,
                "steps": [
                    {
                        "id": "step_cancel",
                        "title": "Pause the plan lifecycle",
                        "executor": "ai"
                    }
                ]
            }),
        );

        let cancelled = invoke_plan(
            &runtime,
            &plugin,
            json!({ "action": "update", "phase": "cancelled" }),
        );
        let cancelled_plan = output_plan(&cancelled);
        assert_eq!(cancelled_plan.phase, WorkflowPlanPhase::Cancelled);
        assert!(
            cancelled
                .output_text
                .contains("phase cancelled | steps 0/1 | autorun off")
        );
        assert_eq!(
            host.statusline_content(PLAN_STATUSLINE_SEGMENT_ID)
                .as_deref(),
            Some("plan:cancelled steps:0/1 autorun:off")
        );

        let restored = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "active",
                "autorun": true
            }),
        );
        let restored_plan = output_plan(&restored);
        assert_eq!(restored_plan.phase, WorkflowPlanPhase::Active);
        assert!(restored_plan.autorun);
        assert!(
            restored
                .output_text
                .contains("phase active | steps 0/1 | autorun on")
        );
        assert_eq!(
            host.statusline_content(PLAN_STATUSLINE_SEGMENT_ID)
                .as_deref(),
            Some("plan:active steps:0/1 autorun:on")
        );

        let reset_to_draft = invoke_plan(
            &runtime,
            &plugin,
            json!({ "action": "update", "phase": "planning" }),
        );
        let reset_plan = output_plan(&reset_to_draft);
        assert_eq!(reset_plan.phase, WorkflowPlanPhase::Planning);

        let completed_cancel = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "cancelled"
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
                "action": "set",
                "objective": "Exercise reopening completed plans.",
                "title": "Reopenable Plan",
                "autorun": false,
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
                "action": "update",
                "step_id": "step_done",
                "status": "completed"
            }),
        );

        let completed = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "completed",
                "summary": "Everything finished."
            }),
        );
        let completed_plan = output_plan(&completed);
        assert_eq!(completed_plan.phase, WorkflowPlanPhase::Completed);

        let restored = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "planning"
            }),
        );
        let restored_plan = output_plan(&restored);
        assert_eq!(restored_plan.phase, WorkflowPlanPhase::Planning);
        assert!(
            restored
                .output_text
                .contains("phase planning | steps 1/1 | autorun off")
        );

        let blocked_err = invoke_plan_result(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "blocked"
            }),
        )
        .expect_err("active phases should require remaining incomplete work");
        assert!(blocked_err.to_string().contains(
            "cannot set plan status to blocked: all steps and checks are already complete"
        ));

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "step_id": "step_done",
                "status": "pending"
            }),
        );

        let blocked = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "blocked"
            }),
        );
        let blocked_plan = output_plan(&blocked);
        assert_eq!(blocked_plan.phase, WorkflowPlanPhase::Blocked);
    }

    #[test]
    fn approved_active_plan_autorun_runs_once() {
        let (plugin, _host, runtime) = init_test_plugin(true, None);

        let created = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
                "objective": "Verify autorun.",
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
        assert!(
            created
                .output_text
                .contains("phase planning | steps 0/1 | autorun on")
        );

        let activated = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "active"
            }),
        );
        let activated_plan = output_plan(&activated);
        assert_eq!(activated_plan.phase, WorkflowPlanPhase::Active);
        assert!(activated_plan.autorun);

        let first_patch = runtime
            .block_on(plugin.agent_stop_hook(crate::plugin::sdk::AgentStopInput {
                session_id: 7,
                stop_hook_active: false,
                last_assistant_message: Some("done".to_string()),
            }))
            .expect("agent_stop should succeed")
            .expect("autorun should continue once");
        assert_eq!(first_patch.reason.as_deref(), Some("workflow plan autorun"));
        assert!(
            first_patch
                .continue_with_message
                .expect("autorun message")
                .contains("Current step 1: Execute the next AI step")
        );

        let second_patch = runtime
            .block_on(plugin.agent_stop_hook(crate::plugin::sdk::AgentStopInput {
                session_id: 7,
                stop_hook_active: false,
                last_assistant_message: Some("done".to_string()),
            }))
            .expect("agent_stop should succeed");
        assert!(second_patch.is_none());
    }

    #[test]
    fn approved_plan_with_autorun_off_does_not_continue() {
        let (plugin, _host, runtime) = init_test_plugin(true, None);

        invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "set",
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
        let activated = invoke_plan(
            &runtime,
            &plugin,
            json!({
                "action": "update",
                "phase": "active",
                "autorun": false
            }),
        );
        let activated_plan = output_plan(&activated);
        assert_eq!(activated_plan.phase, WorkflowPlanPhase::Active);
        assert!(!activated_plan.autorun);

        let patch = runtime
            .block_on(plugin.agent_stop_hook(crate::plugin::sdk::AgentStopInput {
                session_id: 7,
                stop_hook_active: false,
                last_assistant_message: Some("done".to_string()),
            }))
            .expect("agent_stop should succeed");
        assert!(patch.is_none());
    }
}
