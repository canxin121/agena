use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

use crate::{
    message::{
        ExecutionStatus, Message, MessagePart, MessageStatus, PartContent, PermissionRequestPart,
        TimeRange, ToolExecutionPart, ToolInvocation, UserInputRequest, UserInputRequestPart,
    },
    role::Role,
};

pub(crate) const MESSAGE_TAG_PROMPT_COMPACTED: &str = "prompt_compacted";
pub(crate) const MESSAGE_TAG_PROMPT_SUMMARY: &str = "prompt_summary";
pub(crate) const MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED: &str = "attachment_payload_stripped";
pub(crate) const MESSAGE_TAG_TOOL_RESULT_PRUNED: &str = "tool_result_pruned";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPartRef {
    #[serde(default, skip_serializing)]
    pub message_index: usize,
    #[serde(default, skip_serializing)]
    pub part_index: usize,
    pub message_id: i64,
    pub part_id: i64,
}

impl SessionPartRef {
    fn new(message_index: usize, message: &Message, part_index: usize, part: &MessagePart) -> Self {
        Self {
            message_index,
            part_index,
            message_id: message.id,
            part_id: part.id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPendingTool {
    pub part: SessionPartRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPendingPermission {
    pub request: SessionPartRef,
    pub tool: SessionPendingTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPendingUserInput {
    pub request: SessionPartRef,
    pub tool: SessionPendingTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SessionPendingOperation {
    Tool { tool: SessionPendingTool },
    Permission { pending: SessionPendingPermission },
    UserInput { pending: SessionPendingUserInput },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Idle,
    AwaitingModel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct TurnRuntimeState {
    #[serde(default)]
    pub status: SessionRuntimeStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) pending_operations: Vec<SessionPendingOperation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) pending_tool_calls: Vec<PendingToolCallRuntime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_window_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_event_seq: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionRuntimeStatus {
    #[default]
    Idle,
    AwaitingModel,
    RunningTool,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingToolCallRuntime {
    pub operation_id: String,
    pub call_id: i64,
    pub tool_name: String,
    pub(crate) part: SessionPartRef,
}

impl TurnRuntimeState {
    pub fn is_empty(&self) -> bool {
        self.status == SessionRuntimeStatus::Idle
            && self.pending_operations.is_empty()
            && self.pending_tool_calls.is_empty()
            && self.model_provider_id.is_none()
            && self.model_id.is_none()
            && self.model_variant.is_none()
            && self.prompt_cache_key.is_none()
            && self.prompt_window_generation.is_none()
            && self.latest_event_seq.is_none()
    }

    pub fn clear_active_request(&mut self) {
        self.model_provider_id = None;
        self.model_id = None;
        self.model_variant = None;
        self.prompt_cache_key = None;
        self.prompt_window_generation = None;
    }

    pub fn record_model_request(
        &mut self,
        provider_id: String,
        model_id: String,
        model_variant: Option<String>,
        prompt_cache_key: String,
        prompt_window_generation: u64,
    ) {
        self.status = SessionRuntimeStatus::AwaitingModel;
        self.model_provider_id = Some(provider_id);
        self.model_id = Some(model_id);
        self.model_variant = model_variant.filter(|value| !value.trim().is_empty());
        self.prompt_cache_key = Some(prompt_cache_key);
        self.prompt_window_generation = Some(prompt_window_generation);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct PromptWindowRuntime {
    #[serde(default)]
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct PromptTokenUsageSnapshot {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
}

impl PromptTokenUsageSnapshot {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
    }
}

impl From<&crate::message::MessageUsage> for PromptTokenUsageSnapshot {
    fn from(value: &crate::message::MessageUsage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            reasoning_tokens: value.reasoning_tokens,
            cache_write_tokens: value.cache_write_tokens,
            cache_read_tokens: value.cache_read_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct PromptTokenRuntime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_usage: Option<PromptTokenUsageSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_assistant_message_id: Option<i64>,
    #[serde(default)]
    pub prompt_window_generation: u64,
    #[serde(default)]
    pub system_fingerprint: String,
    #[serde(default)]
    pub request_options_fingerprint: String,
    #[serde(default)]
    pub transcript_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_context_window_tokens: Option<u32>,
}

impl PromptTokenRuntime {
    pub fn is_empty(&self) -> bool {
        self.last_successful_usage.is_none()
            && self.last_successful_assistant_message_id.is_none()
            && self.prompt_window_generation == 0
            && self.system_fingerprint.is_empty()
            && self.request_options_fingerprint.is_empty()
            && self.transcript_digest.is_empty()
            && self.model_context_window_tokens.is_none()
    }

    pub fn total_tokens(&self) -> Option<u64> {
        self.last_successful_usage
            .as_ref()
            .map(PromptTokenUsageSnapshot::total_tokens)
    }

    pub fn matches_request(
        &self,
        prompt_window_generation: u64,
        system_fingerprint: &str,
        request_options_fingerprint: &str,
    ) -> bool {
        self.last_successful_usage.is_some()
            && self.last_successful_assistant_message_id.is_some()
            && self.prompt_window_generation == prompt_window_generation
            && self.system_fingerprint == system_fingerprint
            && self.request_options_fingerprint == request_options_fingerprint
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_success(
        &mut self,
        assistant_message_id: i64,
        usage: &crate::message::MessageUsage,
        prompt_window_generation: u64,
        model_context_window_tokens: Option<u32>,
        system_fingerprint: String,
        request_options_fingerprint: String,
        transcript_digest: String,
    ) {
        self.last_successful_usage = Some(PromptTokenUsageSnapshot::from(usage));
        self.last_successful_assistant_message_id = Some(assistant_message_id);
        self.prompt_window_generation = prompt_window_generation;
        self.model_context_window_tokens = model_context_window_tokens;
        self.system_fingerprint = system_fingerprint;
        self.request_options_fingerprint = request_options_fingerprint;
        self.transcript_digest = transcript_digest;
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    #[default]
    Active,
    Paused,
    BudgetLimited,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalSteeringKind {
    ObjectiveUpdated,
    BudgetLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct GoalSteeringState {
    pub goal_id: i64,
    pub kind: GoalSteeringKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct GoalRuntimeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_steering: Option<GoalSteeringState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_limit_reported_goal_id: Option<i64>,
}

impl GoalRuntimeState {
    pub fn is_empty(&self) -> bool {
        self.pending_steering.is_none() && self.budget_limit_reported_goal_id.is_none()
    }

    pub fn pending_steering(&self) -> Option<&GoalSteeringState> {
        self.pending_steering.as_ref()
    }

    pub fn set_pending_steering(&mut self, goal_id: i64, kind: GoalSteeringKind) {
        self.pending_steering = Some(GoalSteeringState { goal_id, kind });
    }

    pub fn clear_pending_steering(&mut self) {
        self.pending_steering = None;
    }

    pub fn budget_limit_reported(&self, goal_id: i64) -> bool {
        self.budget_limit_reported_goal_id == Some(goal_id)
    }

    pub fn mark_budget_limit_reported(&mut self, goal_id: i64) {
        self.budget_limit_reported_goal_id = Some(goal_id);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct SessionGoal {
    pub id: i64,
    pub session_id: i64,
    pub objective: String,
    #[serde(default)]
    pub status: GoalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    #[serde(default)]
    pub tokens_used: u64,
    #[serde(default)]
    pub time_used_seconds: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct ProviderPromptAnchor {
    pub provider_id: String,
    pub model_id: String,
    pub previous_response_id: String,
    pub assistant_message_id: i64,
    #[serde(default)]
    pub prompt_window_generation: u64,
    #[serde(default)]
    pub system_fingerprint: String,
    #[serde(default)]
    pub request_options_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_request_shape: Option<crate::provider::PromptCacheShape>,
    #[serde(default)]
    pub transcript_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct SessionRuntimeState {
    #[serde(default, skip_serializing_if = "TurnRuntimeState::is_empty")]
    pub turn: TurnRuntimeState,
    #[serde(default)]
    pub prompt_window: PromptWindowRuntime,
    #[serde(default, skip_serializing_if = "PromptTokenRuntime::is_empty")]
    pub prompt_tokens: PromptTokenRuntime,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_anchors: BTreeMap<String, ProviderPromptAnchor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loaded_deferred_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "SessionExecutionContext::is_empty")]
    pub execution: SessionExecutionContext,
    #[serde(default, skip_serializing_if = "GoalRuntimeState::is_empty")]
    pub goal: GoalRuntimeState,
    /// When `Some`, the session is in plan mode: writes/mutating tools
    /// are blocked, the LLM is expected to draft its plan into the
    /// referenced file, and `ExitPlanMode` then asks the user to approve
    /// it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct SessionExecutionContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<crate::agent::AgentMode>,
    #[serde(default)]
    pub agent_hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt_override: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::PermissionConfig::is_empty"
    )]
    pub agent_permission: crate::agent::PermissionConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_variant: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::AgentRunConfig::is_empty"
    )]
    pub agent_run: crate::agent::AgentRunConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_workspace_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

impl SessionExecutionContext {
    pub fn is_empty(&self) -> bool {
        self.agent_profile.is_none()
            && self.agent_mode.is_none()
            && !self.agent_hidden
            && self.agent_color.is_none()
            && self.active_skill_name.is_none()
            && self.system_prompt_override.is_none()
            && self.allowed_tools.is_empty()
            && self.agent_permission.is_empty()
            && self.model_provider_id.is_none()
            && self.model_id.is_none()
            && self.model_variant.is_none()
            && self.agent_run.is_empty()
            && self.effective_workspace_root.is_none()
            && self.task_id.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanState {
    /// Absolute path to the plan markdown file.
    pub file_path: PathBuf,
    /// Slug used in the path (e.g. "fluffy-dancing-comet").
    pub slug: String,
    pub started_at: DateTime<Utc>,
}

impl SessionRuntimeState {
    pub fn provider_anchor_key(provider_id: &str, model_id: &str) -> String {
        format!("{provider_id}/{model_id}")
    }

    pub fn effective_workspace_root(&self) -> Option<&Path> {
        self.execution.effective_workspace_root.as_deref()
    }

    pub fn set_effective_workspace_root(&mut self, path: Option<PathBuf>) {
        self.execution.effective_workspace_root = path;
    }

    pub fn allowed_tools(&self) -> &[String] {
        self.execution.allowed_tools.as_slice()
    }

    pub fn set_allowed_tools(&mut self, allowed_tools: Vec<String>) {
        let mut deduped = allowed_tools
            .into_iter()
            .map(|tool| tool.trim().to_string())
            .filter(|tool| !tool.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        deduped.sort();
        self.execution.allowed_tools = deduped;
    }

    pub fn model_override(&self) -> Option<(&str, &str)> {
        Some((
            self.execution.model_provider_id.as_deref()?,
            self.execution.model_id.as_deref()?,
        ))
    }

    pub fn model_variant_override(&self) -> Option<&str> {
        self.execution.model_variant.as_deref()
    }

    pub fn set_model_override(&mut self, provider_id: Option<String>, model_id: Option<String>) {
        self.execution.model_provider_id = provider_id.filter(|value| !value.trim().is_empty());
        self.execution.model_id = model_id.filter(|value| !value.trim().is_empty());
    }

    pub fn set_model_variant_override(&mut self, variant: Option<String>) {
        self.execution.model_variant = variant.filter(|value| !value.trim().is_empty());
    }

    pub fn provider_anchor(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Option<&ProviderPromptAnchor> {
        self.provider_anchors
            .get(Self::provider_anchor_key(provider_id, model_id).as_str())
    }

    pub fn set_provider_anchor(&mut self, anchor: ProviderPromptAnchor) {
        self.provider_anchors.insert(
            Self::provider_anchor_key(anchor.provider_id.as_str(), anchor.model_id.as_str()),
            anchor,
        );
    }

    pub fn clear_provider_anchor(&mut self, provider_id: &str, model_id: &str) {
        self.provider_anchors
            .remove(Self::provider_anchor_key(provider_id, model_id).as_str());
    }

    pub fn clear_provider_anchors(&mut self) {
        self.provider_anchors.clear();
    }

    pub fn clear_prompt_tokens(&mut self) {
        self.prompt_tokens.clear();
    }

    pub fn loaded_deferred_tools(&self) -> &[String] {
        self.loaded_deferred_tools.as_slice()
    }

    pub fn record_loaded_deferred_tools(&mut self, loaded_tools: &[String]) {
        let mut merged = self
            .loaded_deferred_tools
            .iter()
            .map(String::as_str)
            .filter(|name| !name.trim().is_empty())
            .map(ToOwned::to_owned)
            .collect::<BTreeSet<_>>();

        merged.extend(
            loaded_tools
                .iter()
                .map(String::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned),
        );

        self.loaded_deferred_tools = merged.into_iter().collect();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_prompt_tokens(
        &mut self,
        assistant_message_id: i64,
        usage: &crate::message::MessageUsage,
        prompt_window_generation: u64,
        model_context_window_tokens: Option<u32>,
        system_fingerprint: String,
        request_options_fingerprint: String,
        transcript_digest: String,
    ) {
        self.prompt_tokens.record_success(
            assistant_message_id,
            usage,
            prompt_window_generation,
            model_context_window_tokens,
            system_fingerprint,
            request_options_fingerprint,
            transcript_digest,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionListRequest {
    #[serde(default)]
    pub offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// When `false` (the default) the listing hides subagent sessions —
    /// they belong to the runtime and clutter user-facing UIs. Set to
    /// `true` to surface every session, regardless of `is_subagent`.
    #[serde(default)]
    pub include_subagents: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub depth: i64,
    pub root_id: i64,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    #[serde(default)]
    pub is_subagent: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: u64,
    pub child_session_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<SessionGoal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
pub struct Session {
    pub id: i64,
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub depth: i64,
    #[serde(default)]
    pub root_id: i64,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    #[serde(default)]
    pub is_subagent: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<SessionGoal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
    #[serde(skip, default)]
    pub(crate) runtime: SessionRuntimeState,
    #[serde(skip, default)]
    approx_bytes: usize,
    #[serde(skip, default)]
    pending_operations: Vec<SessionPendingOperation>,
}

impl Session {
    pub fn new(id: i64, workspace_id: i64, title: impl Into<String>, now: DateTime<Utc>) -> Self {
        let mut session = Self {
            id,
            parent_id: None,
            depth: 0,
            root_id: id,
            workspace_id,
            title: title.into(),
            version: 1,
            is_subagent: false,
            created_at: now,
            updated_at: now,
            goal: None,
            messages: Vec::new(),
            runtime: SessionRuntimeState::default(),
            approx_bytes: 0,
            pending_operations: Vec::new(),
        };
        session.refresh_derived();
        session
    }

    pub fn with_messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self.refresh_derived();
        self
    }

    pub(crate) fn replace_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        self.refresh_derived();
    }

    pub(crate) fn refresh_derived(&mut self) {
        self.approx_bytes = self.compute_approx_bytes();
        self.pending_operations = self.derive_pending_operations();
    }

    pub(crate) fn sync_runtime_turn_state(&mut self) {
        self.refresh_derived();
        let previous = self.runtime.turn.clone();
        self.runtime.turn = self.turn_runtime_snapshot(previous);
    }

    fn turn_runtime_snapshot(&self, previous: TurnRuntimeState) -> TurnRuntimeState {
        let status = if self.blocked() {
            SessionRuntimeStatus::Blocked
        } else if self.next_pending_tool().is_some() {
            SessionRuntimeStatus::RunningTool
        } else if self.should_run_model() {
            SessionRuntimeStatus::AwaitingModel
        } else {
            SessionRuntimeStatus::Idle
        };

        let mut snapshot = TurnRuntimeState {
            status,
            pending_operations: self.pending_operations.clone(),
            pending_tool_calls: self.pending_tool_runtime_snapshots(),
            latest_event_seq: previous.latest_event_seq,
            ..TurnRuntimeState::default()
        };

        if status == SessionRuntimeStatus::AwaitingModel {
            snapshot.model_provider_id = previous.model_provider_id;
            snapshot.model_id = previous.model_id;
            snapshot.model_variant = previous.model_variant;
            snapshot.prompt_cache_key = previous.prompt_cache_key;
            snapshot.prompt_window_generation = previous.prompt_window_generation;
        }

        snapshot
    }

    fn pending_tool_runtime_snapshots(&self) -> Vec<PendingToolCallRuntime> {
        self.pending_operations
            .iter()
            .filter_map(|pending| {
                let tool = match pending {
                    SessionPendingOperation::Tool { tool }
                    | SessionPendingOperation::Permission {
                        pending: SessionPendingPermission { tool, .. },
                    }
                    | SessionPendingOperation::UserInput {
                        pending: SessionPendingUserInput { tool, .. },
                    } => tool,
                };
                let (call_id, invocation, _) = self.pending_tool_execution(tool)?;
                let part = self.part(&tool.part)?;
                let operation_id = part.operation_id.clone()?;
                Some(PendingToolCallRuntime {
                    operation_id,
                    call_id,
                    tool_name: tool_invocation_name(invocation),
                    part: tool.part.clone(),
                })
            })
            .collect()
    }

    pub fn status(&self) -> SessionStatus {
        if self.should_run_model() {
            SessionStatus::AwaitingModel
        } else {
            SessionStatus::Idle
        }
    }

    pub fn runtime(&self) -> &SessionRuntimeState {
        &self.runtime
    }

    pub(crate) fn apply_persisted_metadata(&mut self, persisted: &Session) {
        self.id = persisted.id;
        self.parent_id = persisted.parent_id;
        self.depth = persisted.depth;
        self.root_id = persisted.root_id;
        self.workspace_id = persisted.workspace_id;
        self.title = persisted.title.clone();
        self.version = persisted.version;
        self.is_subagent = persisted.is_subagent;
        self.created_at = persisted.created_at;
        self.updated_at = persisted.updated_at;
        self.goal = persisted.goal.clone();
        self.runtime = persisted.runtime.clone();
    }

    pub fn blocked(&self) -> bool {
        self.pending_operations.iter().any(|pending| {
            matches!(
                pending,
                SessionPendingOperation::Permission { .. }
                    | SessionPendingOperation::UserInput { .. }
            )
        })
    }

    pub(crate) fn next_call_id(&self) -> i64 {
        self.messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .filter_map(extract_call_id)
            .max()
            .unwrap_or(0)
            + 1
    }

    pub fn approx_bytes(&self) -> usize {
        self.approx_bytes
    }

    pub(crate) fn find_pending_user_input_by_request_id(
        &self,
        request_id: &str,
    ) -> Option<SessionPendingUserInput> {
        self.pending_operations.iter().find_map(|pending| {
            let SessionPendingOperation::UserInput { pending } = pending else {
                return None;
            };
            self.pending_user_input_request(pending)
                .filter(|request| request.request_id == request_id)
                .map(|_| pending.clone())
        })
    }

    fn compute_approx_bytes(&self) -> usize {
        serde_json::to_vec(&(
            self.id,
            self.parent_id,
            self.workspace_id,
            &self.title,
            self.version,
            self.created_at,
            self.updated_at,
            &self.messages,
            &self.runtime,
        ))
        .map(|bytes| bytes.len())
        .unwrap_or_else(|_| {
            self.messages
                .iter()
                .map(Message::as_text_lossy)
                .map(|text| text.len())
                .sum()
        })
    }

    fn should_run_model(&self) -> bool {
        matches!(
            self.last_conversation_message()
                .map(|message| (message.role, message.state)),
            Some((Role::User | Role::Tool, _))
                | Some((
                    Role::Assistant,
                    MessageStatus::Pending | MessageStatus::InProgress
                ))
        )
    }

    pub(crate) fn last_conversation_message(&self) -> Option<&Message> {
        self.messages
            .iter()
            .rev()
            .find(|message| !message.metadata.has_tag(MESSAGE_TAG_PROMPT_SUMMARY))
    }

    pub(crate) fn find_pending_permission_by_request_id(
        &self,
        request_id: &str,
    ) -> Option<SessionPendingPermission> {
        self.pending_operations.iter().find_map(|pending| {
            let SessionPendingOperation::Permission { pending } = pending else {
                return None;
            };
            self.pending_permission_request(pending)
                .filter(|request| request.request_id == request_id)
                .map(|_| pending.clone())
        })
    }

    #[cfg(test)]
    pub(crate) fn pending_operations(&self) -> &[SessionPendingOperation] {
        self.pending_operations.as_slice()
    }

    fn derive_pending_operations(&self) -> Vec<SessionPendingOperation> {
        let mut operations = Vec::new();
        let completed_tool_operations = self.completed_tool_operations();

        for (message_index, message) in self.messages.iter().enumerate() {
            if message.role != Role::Assistant {
                continue;
            }

            let mut permission_parts = HashMap::new();
            let mut user_input_parts = HashMap::new();

            for (part_index, part) in message.parts.iter().enumerate() {
                if part.status != ExecutionStatus::Pending {
                    continue;
                }

                let Some(operation_id) = part.operation_id.as_deref() else {
                    continue;
                };

                match part.content.as_ref() {
                    Some(PartContent::PermissionRequest(_)) => {
                        permission_parts.insert(
                            operation_id,
                            SessionPartRef::new(message_index, message, part_index, part),
                        );
                    }
                    Some(PartContent::UserInputRequest(_)) => {
                        user_input_parts.insert(
                            operation_id,
                            SessionPartRef::new(message_index, message, part_index, part),
                        );
                    }
                    _ => {}
                }
            }

            for (part_index, part) in message.parts.iter().enumerate() {
                if part.status != ExecutionStatus::Pending {
                    continue;
                }

                let Some(operation_id) = part.operation_id.as_deref() else {
                    continue;
                };
                let Some(PartContent::ToolExecution(ToolExecutionPart::Pending { .. })) =
                    part.content.as_ref()
                else {
                    continue;
                };
                if completed_tool_operations.contains(operation_id) {
                    continue;
                }

                let tool = SessionPendingTool {
                    part: SessionPartRef::new(message_index, message, part_index, part),
                };

                if let Some(request) = permission_parts.get(operation_id) {
                    operations.push(SessionPendingOperation::Permission {
                        pending: SessionPendingPermission {
                            request: request.clone(),
                            tool,
                        },
                    });
                    continue;
                }

                if let Some(request) = user_input_parts.get(operation_id) {
                    operations.push(SessionPendingOperation::UserInput {
                        pending: SessionPendingUserInput {
                            request: request.clone(),
                            tool,
                        },
                    });
                    continue;
                }

                operations.push(SessionPendingOperation::Tool { tool });
            }
        }

        operations
    }

    pub(crate) fn next_pending_tool(&self) -> Option<SessionPendingTool> {
        self.pending_operations.iter().find_map(|pending| {
            let SessionPendingOperation::Tool { tool } = pending else {
                return None;
            };
            Some(tool.clone())
        })
    }

    pub(crate) fn pending_tools(&self) -> Vec<SessionPendingTool> {
        self.pending_operations
            .iter()
            .filter_map(|pending| match pending {
                SessionPendingOperation::Tool { tool } => Some(tool.clone()),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn part(&self, part_ref: &SessionPartRef) -> Option<&MessagePart> {
        let message = self.messages.get(part_ref.message_index)?;
        if message.id != part_ref.message_id {
            return None;
        }

        let part = message.parts.get(part_ref.part_index)?;
        (part.id == part_ref.part_id).then_some(part)
    }

    pub(crate) fn part_mut(&mut self, part_ref: &SessionPartRef) -> Option<&mut MessagePart> {
        let message = self.messages.get_mut(part_ref.message_index)?;
        if message.id != part_ref.message_id {
            return None;
        }

        let part = message.parts.get_mut(part_ref.part_index)?;
        (part.id == part_ref.part_id).then_some(part)
    }

    pub(crate) fn pending_tool_execution(
        &self,
        pending: &SessionPendingTool,
    ) -> Option<(i64, &ToolInvocation, &TimeRange)> {
        let part = self.part(&pending.part)?;
        let (call_id, invocation, lifecycle) = match part.content.as_ref()? {
            PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id,
                invocation,
                lifecycle,
                ..
            })
            | PartContent::ToolExecution(ToolExecutionPart::InProgress {
                call_id,
                invocation,
                lifecycle,
                ..
            }) => (call_id, invocation, lifecycle),
            _ => return None,
        };

        Some((*call_id, invocation, lifecycle))
    }

    pub(crate) fn pending_permission_request(
        &self,
        pending: &SessionPendingPermission,
    ) -> Option<&crate::permission::PermissionRequest> {
        let part = self.part(&pending.request)?;
        let PartContent::PermissionRequest(PermissionRequestPart { request, .. }) =
            part.content.as_ref()?
        else {
            return None;
        };

        Some(request)
    }

    pub(crate) fn pending_user_input_request(
        &self,
        pending: &SessionPendingUserInput,
    ) -> Option<&UserInputRequest> {
        let part = self.part(&pending.request)?;
        let PartContent::UserInputRequest(UserInputRequestPart { request, .. }) =
            part.content.as_ref()?
        else {
            return None;
        };

        Some(request)
    }

    fn completed_tool_operations(&self) -> HashSet<&str> {
        self.messages
            .iter()
            .filter(|message| message.role == Role::Tool)
            .flat_map(|message| message.parts.iter())
            .filter_map(|part| part.operation_id.as_deref())
            .collect()
    }
}

fn tool_invocation_name(invocation: &ToolInvocation) -> String {
    let ToolInvocation { name, .. } = invocation;
    name.clone()
}

fn extract_call_id(part: &MessagePart) -> Option<i64> {
    part.content.as_ref().and_then(|content| match content {
        PartContent::ToolExecution(tool) => match tool {
            ToolExecutionPart::Pending { call_id, .. }
            | ToolExecutionPart::InProgress { call_id, .. }
            | ToolExecutionPart::Completed { call_id, .. }
            | ToolExecutionPart::Failed { call_id, .. } => Some(*call_id),
        },
        _ => None,
    })
}

// NOTE: `SessionEventType` and `SessionEventRecord` have been removed. The
// unified `crate::event::EventKind` and `crate::event::DomainEvent` types
// are the only event shapes the system carries.

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::message::{
        ExecutionStatus, FirstPartyToolInput, MessageMetadata, MessagePart, MessageStatus,
        PartContent, TimeRange, TodoWriteToolInput, ToolExecutionPart, UserInputQuestion,
    };
    use crate::permission::{PermissionAction, PermissionRequest};
    use crate::role::Role;

    use super::*;

    #[test]
    fn find_pending_permission_by_request_id_handles_multiple_requests() {
        let session = Session::new(1, 1, "multi-permission", Utc::now()).with_messages(vec![
            assistant_message(
                11,
                vec![
                    pending_tool_part(101, 11, "op-1", 1),
                    pending_permission_part(102, 11, "op-1", "perm-1"),
                    pending_tool_part(103, 11, "op-2", 2),
                    pending_permission_part(104, 11, "op-2", "perm-2"),
                ],
            ),
        ]);

        let pending = session
            .find_pending_permission_by_request_id("perm-2")
            .expect("second pending permission should resolve");

        assert_eq!(session.pending_operations().len(), 2);
        assert_eq!(pending.request.part_id, 104);
        assert_eq!(pending.tool.part.part_id, 103);
        assert!(session.blocked());
    }

    #[test]
    fn find_pending_user_input_by_request_id_handles_multiple_requests() {
        let session =
            Session::new(1, 1, "multi-input", Utc::now()).with_messages(vec![assistant_message(
                21,
                vec![
                    pending_tool_part(201, 21, "op-1", 1),
                    pending_user_input_part(202, 21, "op-1", "input-1"),
                    pending_tool_part(203, 21, "op-2", 2),
                    pending_user_input_part(204, 21, "op-2", "input-2"),
                ],
            )]);

        let pending = session
            .find_pending_user_input_by_request_id("input-2")
            .expect("second pending user input should resolve");

        assert_eq!(session.pending_operations().len(), 2);
        assert_eq!(pending.request.part_id, 204);
        assert_eq!(pending.tool.part.part_id, 203);
        assert!(session.blocked());
    }

    #[test]
    fn sync_runtime_turn_state_records_blocked_pending_tools() {
        let mut session =
            Session::new(1, 1, "blocked", Utc::now()).with_messages(vec![assistant_message(
                11,
                vec![
                    pending_tool_part(101, 11, "op-1", 7),
                    pending_permission_part(102, 11, "op-1", "perm-1"),
                ],
            )]);

        session.sync_runtime_turn_state();

        assert_eq!(session.runtime.turn.status, SessionRuntimeStatus::Blocked);
        assert_eq!(session.runtime.turn.pending_operations.len(), 1);
        assert_eq!(session.runtime.turn.pending_tool_calls.len(), 1);
        assert_eq!(
            session.runtime.turn.pending_tool_calls[0].operation_id,
            "op-1"
        );
        assert_eq!(session.runtime.turn.pending_tool_calls[0].call_id, 7);
        assert_eq!(
            session.runtime.turn.pending_tool_calls[0].tool_name,
            "todo_write"
        );
    }

    #[test]
    fn sync_runtime_turn_state_preserves_active_model_request() {
        let mut session = Session::new(1, 1, "awaiting", Utc::now())
            .with_messages(vec![Message::prompt_text(Role::User, "hello")]);
        session.runtime.turn.record_model_request(
            "openai".to_owned(),
            "gpt-5".to_owned(),
            Some("high".to_owned()),
            "cache-key".to_owned(),
            42,
        );

        session.sync_runtime_turn_state();

        assert_eq!(
            session.runtime.turn.status,
            SessionRuntimeStatus::AwaitingModel
        );
        assert_eq!(
            session.runtime.turn.model_provider_id.as_deref(),
            Some("openai")
        );
        assert_eq!(session.runtime.turn.model_id.as_deref(), Some("gpt-5"));
        assert_eq!(session.runtime.turn.model_variant.as_deref(), Some("high"));
        assert_eq!(
            session.runtime.turn.prompt_cache_key.as_deref(),
            Some("cache-key")
        );
        assert_eq!(session.runtime.turn.prompt_window_generation, Some(42));
    }

    #[test]
    fn pending_assistant_message_keeps_session_awaiting_model() {
        let mut assistant = assistant_message(31, vec![]);
        assistant.state = MessageStatus::Pending;

        let session = Session::new(1, 1, "awaiting-model", Utc::now())
            .with_messages(vec![Message::prompt_text(Role::User, "hello"), assistant]);

        assert_eq!(session.status(), SessionStatus::AwaitingModel);
    }

    fn assistant_message(id: i64, mut parts: Vec<MessagePart>) -> Message {
        for (index, part) in parts.iter_mut().enumerate() {
            part.part_index = index as i32;
        }
        Message {
            id,
            role: Role::Assistant,
            state: MessageStatus::Completed,
            parts,
            created_at: Utc::now(),
            metadata: MessageMetadata::default(),
            usage: None,
            finish: None,
        }
    }

    fn pending_tool_part(
        part_id: i64,
        message_id: i64,
        operation_id: &str,
        call_id: i64,
    ) -> MessagePart {
        let invocation = FirstPartyToolInput::TodoWrite(TodoWriteToolInput { items: Vec::new() })
            .into_invocation();
        let mut part = MessagePart::with_content(
            part_id,
            message_id,
            Utc::now(),
            ExecutionStatus::Pending,
            PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id,
                invocation,
                title: format!("tool {operation_id}"),
                lifecycle: TimeRange::default(),
            }),
        );
        part.operation_id = Some(operation_id.to_string());
        part
    }

    fn pending_permission_part(
        part_id: i64,
        message_id: i64,
        operation_id: &str,
        request_id: &str,
    ) -> MessagePart {
        let mut part = MessagePart::with_content(
            part_id,
            message_id,
            Utc::now(),
            ExecutionStatus::Pending,
            PartContent::PermissionRequest(PermissionRequestPart::pending(PermissionRequest {
                request_id: request_id.to_string(),
                session_id: Some(1),
                action: PermissionAction::BuiltinTool {
                    tool_name: "todo_write".to_string(),
                    qualifier: None,
                },
                reason: format!("need permission for {operation_id}"),
                explanation: "matched static permission policy".to_string(),
                source: Some("static_policy".to_string()),
                scope: None,
                operator: None,
                risk: crate::permission::PermissionRiskLevel::Medium,
                trace: vec![crate::permission::DecisionTraceStep {
                    source_kind: crate::permission::PolicySourceKind::StaticPolicy,
                    summary: "matched static permission policy".to_string(),
                    source: Some("static_policy".to_string()),
                    scope: None,
                    operator: None,
                }],
                created_at: Utc::now(),
            })),
        );
        part.operation_id = Some(operation_id.to_string());
        part
    }

    fn pending_user_input_part(
        part_id: i64,
        message_id: i64,
        operation_id: &str,
        request_id: &str,
    ) -> MessagePart {
        let mut part = MessagePart::with_content(
            part_id,
            message_id,
            Utc::now(),
            ExecutionStatus::Pending,
            PartContent::UserInputRequest(UserInputRequestPart::pending(UserInputRequest {
                request_id: request_id.to_string(),
                session_id: Some(1),
                questions: vec![UserInputQuestion {
                    id: "answer".to_string(),
                    header: "Answer".to_string(),
                    question: format!("question for {operation_id}"),
                    options: Vec::new(),
                    multiple: false,
                    allow_custom: true,
                }],
                created_at: Utc::now(),
            })),
        );
        part.operation_id = Some(operation_id.to_string());
        part
    }
}
