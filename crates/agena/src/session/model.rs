use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

use crate::{
    execution_prefs::ExecutionSelection,
    message::{
        ExecutionStatus, InteractiveRequestPart, Message, MessagePart, MessageStatus, PartContent,
        PendingInteractiveRequest, PendingInteractiveRequestKind, RequestPart, TimeRange,
        ToolInvocation, UserInputRequest,
    },
    model::ModelRef,
    role::Role,
    session::history::RunSource,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolCallRecordState {
    Queued,
    Running,
}

impl ToolCallRecordState {
    fn from_execution_status(status: ExecutionStatus) -> Option<Self> {
        match status {
            ExecutionStatus::Pending => Some(Self::Queued),
            ExecutionStatus::InProgress => Some(Self::Running),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolCallRecord {
    pub operation_id: String,
    pub call_id: i64,
    pub invocation: ToolInvocation,
    pub advertised_tool_identity: Option<String>,
    pub lifecycle: TimeRange,
    pub state: ToolCallRecordState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionPendingInteractiveRequest {
    pub request: SessionPartRef,
    pub tool: SessionPendingTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SessionPendingOperation {
    Tool {
        tool: SessionPendingTool,
    },
    Permission {
        pending: SessionPendingInteractiveRequest,
    },
    UserInput {
        pending: SessionPendingInteractiveRequest,
    },
}

impl SessionPendingOperation {
    fn tool(&self) -> &SessionPendingTool {
        match self {
            Self::Tool { tool }
            | Self::Permission {
                pending: SessionPendingInteractiveRequest { tool, .. },
            }
            | Self::UserInput {
                pending: SessionPendingInteractiveRequest { tool, .. },
            } => tool,
        }
    }

    fn queued_tool(&self) -> Option<&SessionPendingTool> {
        match self {
            Self::Tool { tool } => Some(tool),
            Self::Permission { .. } | Self::UserInput { .. } => None,
        }
    }

    fn is_blocking_request(&self) -> bool {
        matches!(self, Self::Permission { .. } | Self::UserInput { .. })
    }

    fn permission_request(&self) -> Option<&SessionPendingInteractiveRequest> {
        match self {
            Self::Permission { pending } => Some(pending),
            Self::Tool { .. } | Self::UserInput { .. } => None,
        }
    }

    fn user_input_request(&self) -> Option<&SessionPendingInteractiveRequest> {
        match self {
            Self::UserInput { pending } => Some(pending),
            Self::Tool { .. } | Self::Permission { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Idle,
    AwaitingModel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct RunRuntimeState {
    #[serde(default)]
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) pending_operations: Vec<SessionPendingOperation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) pending_tool_calls: Vec<PendingToolCallRuntime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_adapter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<RunSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_window_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_event_seq: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunStatus {
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

impl RunRuntimeState {
    pub fn is_empty(&self) -> bool {
        self.status == RunStatus::Idle
            && self.pending_operations.is_empty()
            && self.pending_tool_calls.is_empty()
            && self.model_provider_id.is_none()
            && self.model_adapter_id.is_none()
            && self.model_id.is_none()
            && self.model_thinking_mode.is_none()
            && self.model_speed_mode.is_none()
            && self.model_verbosity.is_none()
            && self.model_parallel_tool_calls.is_none()
            && self.source.is_none()
            && self.prompt_cache_key.is_none()
            && self.prompt_window_generation.is_none()
            && self.latest_event_seq.is_none()
    }

    pub fn clear_active_request(&mut self) {
        self.model_provider_id = None;
        self.model_adapter_id = None;
        self.model_id = None;
        self.model_thinking_mode = None;
        self.model_speed_mode = None;
        self.model_verbosity = None;
        self.model_parallel_tool_calls = None;
        self.source = None;
        self.prompt_cache_key = None;
        self.prompt_window_generation = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_run_request(
        &mut self,
        source: RunSource,
        provider_id: String,
        adapter_id: Option<String>,
        model_id: String,
        model_thinking_mode: Option<String>,
        model_speed_mode: Option<String>,
        model_verbosity: Option<String>,
        model_parallel_tool_calls: Option<bool>,
        prompt_cache_key: String,
        prompt_window_generation: u64,
    ) {
        self.status = RunStatus::AwaitingModel;
        self.model_provider_id = Some(provider_id);
        self.model_adapter_id = adapter_id.filter(|value| !value.trim().is_empty());
        self.model_id = Some(model_id);
        self.model_thinking_mode = model_thinking_mode.filter(|value| !value.trim().is_empty());
        self.model_speed_mode = model_speed_mode.filter(|value| !value.trim().is_empty());
        self.model_verbosity = model_verbosity.filter(|value| !value.trim().is_empty());
        self.model_parallel_tool_calls = model_parallel_tool_calls;
        self.source = Some(source);
        self.prompt_cache_key = Some(prompt_cache_key);
        self.prompt_window_generation = Some(prompt_window_generation);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptCompactionStrategy {
    #[default]
    LocalAgent,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct PromptCompactionRuntime {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_start_message_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compacted_at_message_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compacted_by_message_id: Option<i64>,
    #[serde(default)]
    pub strategy: PromptCompactionStrategy,
    #[serde(default)]
    pub created_at_ms: i64,
}

impl PromptCompactionRuntime {
    pub fn is_empty(&self) -> bool {
        self.summary.trim().is_empty()
            && self.tail_start_message_id.is_none()
            && self.compacted_at_message_id.is_none()
            && self.compacted_by_message_id.is_none()
            && self.strategy == PromptCompactionStrategy::LocalAgent
            && self.created_at_ms == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct PromptWindowRuntime {
    #[serde(default)]
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<PromptCompactionRuntime>,
}

impl PromptWindowRuntime {
    pub fn is_empty(&self) -> bool {
        self.generation == 0
            && self
                .compaction
                .as_ref()
                .is_none_or(PromptCompactionRuntime::is_empty)
    }
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
    pub fn prompt_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
    }

    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens()
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

    pub fn prompt_tokens(&self) -> Option<u64> {
        self.last_successful_usage
            .as_ref()
            .map(PromptTokenUsageSnapshot::prompt_tokens)
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
    #[serde(default, skip_serializing_if = "RunRuntimeState::is_empty")]
    pub run: RunRuntimeState,
    #[serde(default)]
    pub prompt_window: PromptWindowRuntime,
    #[serde(default, skip_serializing_if = "PromptTokenRuntime::is_empty")]
    pub prompt_tokens: PromptTokenRuntime,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_anchors: BTreeMap<String, ProviderPromptAnchor>,
    #[serde(default, skip_serializing_if = "SessionExecutionContext::is_empty")]
    pub execution: SessionExecutionContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct SessionExecutionContext {
    #[serde(flatten)]
    pub selection: ExecutionSelection,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_stack: Vec<Option<String>>,
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
    pub effective_permission: crate::agent::PermissionConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_workspace_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

impl SessionExecutionContext {
    pub fn is_empty(&self) -> bool {
        self.selection.is_empty()
            && self.agent_stack.is_empty()
            && self.active_skill_name.is_none()
            && self.system_prompt_override.is_none()
            && self.allowed_tools.is_empty()
            && self.effective_permission.is_empty()
            && self.effective_workspace_root.is_none()
            && self.task_id.is_none()
    }
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

    pub fn model_override(&self) -> Option<(&str, Option<&str>, &str)> {
        Some((
            self.execution.selection.provider.as_deref()?,
            self.execution.selection.adapter.as_deref(),
            self.execution.selection.model.as_deref()?,
        ))
    }

    pub fn effective_model_ref(&self) -> Result<Option<ModelRef>, crate::model::IdentifierError> {
        if let Some(model) = self.execution.selection.model_ref()? {
            return Ok(Some(model));
        }

        let Some(provider_id) = self.run.model_provider_id.as_deref() else {
            return Ok(None);
        };
        let Some(model_id) = self.run.model_id.as_deref() else {
            return Ok(None);
        };

        match self.run.model_adapter_id.as_deref() {
            Some(adapter_id) => {
                ModelRef::try_new_with_adapter(provider_id, adapter_id, model_id).map(Some)
            }
            None => ModelRef::try_new(provider_id, model_id).map(Some),
        }
    }

    pub fn model_thinking_mode_override(&self) -> Option<&str> {
        self.execution.selection.thinking_mode.as_deref()
    }

    pub fn model_speed_mode_override(&self) -> Option<&str> {
        self.execution.selection.speed_mode.as_deref()
    }

    pub fn model_verbosity_override(&self) -> Option<&str> {
        self.execution.selection.verbosity.as_deref()
    }

    pub fn model_parallel_tool_calls_override(&self) -> Option<bool> {
        self.execution.selection.parallel_tool_calls
    }

    pub fn set_model_override(
        &mut self,
        provider_id: Option<String>,
        adapter_id: Option<String>,
        model_id: Option<String>,
    ) {
        self.execution
            .selection
            .set_model_override(provider_id, adapter_id, model_id);
    }

    pub fn set_model_mode_overrides(
        &mut self,
        thinking_mode: Option<String>,
        speed_mode: Option<String>,
        verbosity: Option<String>,
        parallel_tool_calls: Option<bool>,
    ) {
        self.execution.selection.set_model_mode_overrides(
            thinking_mode,
            speed_mode,
            verbosity,
            parallel_tool_calls,
        );
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

    pub(crate) fn sync_runtime_run_state(&mut self) {
        self.refresh_derived();
        let previous = self.runtime.run.clone();
        self.runtime.run = self.run_runtime_snapshot(previous);
    }

    fn run_runtime_snapshot(&self, previous: RunRuntimeState) -> RunRuntimeState {
        let status = if self.blocked() {
            RunStatus::Blocked
        } else if self.next_pending_tool().is_some() {
            RunStatus::RunningTool
        } else if self.should_run_model() {
            RunStatus::AwaitingModel
        } else {
            RunStatus::Idle
        };

        let mut snapshot = RunRuntimeState {
            status,
            pending_operations: self.pending_operations.clone(),
            pending_tool_calls: self.pending_tool_runtime_snapshots(),
            latest_event_seq: previous.latest_event_seq,
            ..RunRuntimeState::default()
        };

        if status == RunStatus::AwaitingModel {
            snapshot.model_provider_id = previous.model_provider_id;
            snapshot.model_adapter_id = previous.model_adapter_id;
            snapshot.model_id = previous.model_id;
            snapshot.model_thinking_mode = previous.model_thinking_mode;
            snapshot.model_speed_mode = previous.model_speed_mode;
            snapshot.model_verbosity = previous.model_verbosity;
            snapshot.model_parallel_tool_calls = previous.model_parallel_tool_calls;
            snapshot.source = previous.source;
            snapshot.prompt_cache_key = previous.prompt_cache_key;
            snapshot.prompt_window_generation = previous.prompt_window_generation;
        }

        snapshot
    }

    fn pending_tool_runtime_snapshots(&self) -> Vec<PendingToolCallRuntime> {
        self.pending_operations
            .iter()
            .filter_map(|pending| {
                let tool = pending.tool();
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
        self.runtime = persisted.runtime.clone();
    }

    pub fn blocked(&self) -> bool {
        self.pending_operations
            .iter()
            .any(SessionPendingOperation::is_blocking_request)
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

    fn find_pending_request_by_id<P: Clone>(
        &self,
        request_id: &str,
        extract_pending: impl Fn(&SessionPendingOperation) -> Option<&P>,
        request_matches: impl Fn(&Self, &P, &str) -> bool,
    ) -> Option<P> {
        self.pending_operations
            .iter()
            .find_map(|pending_operation| {
                let pending = extract_pending(pending_operation)?;
                request_matches(self, pending, request_id).then(|| pending.clone())
            })
    }

    fn has_replied_request(
        &self,
        request_id: &str,
        request_matches: impl Fn(&RequestPart, &str) -> bool,
    ) -> bool {
        self.messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .any(|part| match part.content.as_ref() {
                Some(PartContent::Request(request)) => request_matches(request, request_id),
                _ => false,
            })
    }

    fn pending_request<'a, T>(
        &'a self,
        request_part: &SessionPartRef,
        extract_request: impl FnOnce(&'a RequestPart) -> Option<&'a T>,
    ) -> Option<&'a T> {
        let part = self.part(request_part)?;
        let PartContent::Request(request) = part.content.as_ref()? else {
            return None;
        };
        extract_request(request)
    }

    pub(crate) fn find_pending_user_input_by_request_id(
        &self,
        request_id: &str,
    ) -> Option<SessionPendingInteractiveRequest> {
        self.find_pending_request_by_id(
            request_id,
            SessionPendingOperation::user_input_request,
            |session, pending, request_id| {
                session
                    .pending_user_input_request(pending)
                    .is_some_and(|request| request.request_id == request_id)
            },
        )
    }

    pub(crate) fn has_replied_user_input_request(&self, request_id: &str) -> bool {
        self.has_replied_request(request_id, |request, request_id| match request {
            RequestPart::UserInput(request) => {
                request.request.request_id == request_id && request.reply.is_some()
            }
            _ => false,
        })
    }

    pub(crate) fn has_finished_operation(&self, operation_id: &str) -> bool {
        self.messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .any(|part| {
                part.operation_id.as_deref() == Some(operation_id)
                    && matches!(part.content.as_ref(), Some(PartContent::Operation(_)))
                    && matches!(
                        part.status,
                        ExecutionStatus::Completed
                            | ExecutionStatus::Failed
                            | ExecutionStatus::Cancelled
                    )
            })
    }

    pub fn pending_interactive_requests(&self) -> Vec<PendingInteractiveRequest> {
        let mut seen = HashSet::new();
        let mut requests = Vec::new();

        for pending in &self.pending_operations {
            match pending {
                SessionPendingOperation::Permission { pending } => {
                    let Some(request) = self.pending_permission_request(pending).cloned() else {
                        continue;
                    };
                    let key = format!(
                        "{:?}:{}",
                        PendingInteractiveRequestKind::Permission,
                        request.request_id
                    );
                    if seen.insert(key) {
                        requests.push(PendingInteractiveRequest::from(request));
                    }
                }
                SessionPendingOperation::UserInput { pending } => {
                    let Some(request) = self.pending_user_input_request(pending).cloned() else {
                        continue;
                    };
                    let key = format!(
                        "{:?}:{}",
                        PendingInteractiveRequestKind::UserInput,
                        request.request_id
                    );
                    if seen.insert(key) {
                        requests.push(PendingInteractiveRequest::from(request));
                    }
                }
                SessionPendingOperation::Tool { .. } => {}
            }
        }

        requests
    }

    pub(crate) fn user_input_request_for_operation(
        &self,
        operation_id: &str,
        sequence_index: usize,
    ) -> Option<InteractiveRequestPart<UserInputRequest, crate::message::UserInputReply>> {
        self.messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .filter(|part| part.operation_id.as_deref() == Some(operation_id))
            .filter_map(|part| match part.content.as_ref() {
                Some(PartContent::Request(RequestPart::UserInput(request))) => {
                    Some(request.clone())
                }
                _ => None,
            })
            .nth(sequence_index)
    }

    fn compute_approx_bytes(&self) -> usize {
        let mut bytes = 160 + self.title.len();
        bytes = bytes.saturating_add(self.messages.len().saturating_mul(96));
        for message in &self.messages {
            bytes = bytes
                .saturating_add(message.as_text_lossy().len())
                .saturating_add(message.parts.len().saturating_mul(64))
                .saturating_add(message.metadata.model_provider_id.len())
                .saturating_add(message.metadata.model_id.len());
            for part in &message.parts {
                bytes = bytes
                    .saturating_add(part.name.as_ref().map_or(0, |value| value.len()))
                    .saturating_add(part.summary.as_ref().map_or(0, |value| value.len()))
                    .saturating_add(part.operation_id.as_ref().map_or(0, |value| value.len()));
            }
            if let Some(usage) = message.usage.as_ref() {
                bytes = bytes.saturating_add(std::mem::size_of_val(usage));
            }
        }
        bytes = bytes
            .saturating_add(
                self.runtime
                    .execution
                    .allowed_tools
                    .iter()
                    .map(String::len)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.runtime
                    .run
                    .prompt_cache_key
                    .as_ref()
                    .map_or(0, String::len),
            );
        bytes
    }

    fn should_run_model(&self) -> bool {
        let Some(message) = self.last_conversation_message() else {
            return false;
        };
        message.role == Role::User
            || message_has_completed_operation(message)
            || matches!(
                (message.role, message.state),
                (
                    Role::Assistant,
                    MessageStatus::Pending | MessageStatus::InProgress
                )
            )
    }

    pub(crate) fn last_conversation_message(&self) -> Option<&Message> {
        self.messages.last()
    }

    pub(crate) fn find_pending_permission_by_request_id(
        &self,
        request_id: &str,
    ) -> Option<SessionPendingInteractiveRequest> {
        self.find_pending_request_by_id(
            request_id,
            SessionPendingOperation::permission_request,
            |session, pending, request_id| {
                session
                    .pending_permission_request(pending)
                    .is_some_and(|request| request.request_id == request_id)
            },
        )
    }

    pub(crate) fn has_replied_permission_request(&self, request_id: &str) -> bool {
        self.has_replied_request(request_id, |request, request_id| match request {
            RequestPart::Permission(request) => {
                request.request.request_id == request_id && request.reply.is_some()
            }
            _ => false,
        })
    }

    fn derive_pending_operations(&self) -> Vec<SessionPendingOperation> {
        #[derive(Default)]
        struct PendingRequestParts {
            permission: Option<SessionPartRef>,
            user_input: Option<SessionPartRef>,
        }

        let mut operations = Vec::new();
        let completed_tool_operations = self.completed_tool_operations();

        for (message_index, message) in self.messages.iter().enumerate() {
            if message.role != Role::Assistant {
                continue;
            }

            let mut request_parts_by_operation: HashMap<&str, PendingRequestParts> = HashMap::new();

            for (part_index, part) in message.parts.iter().enumerate() {
                if part.status != ExecutionStatus::Pending {
                    continue;
                }

                let Some(operation_id) = part.operation_id.as_deref() else {
                    continue;
                };

                match part.content.as_ref() {
                    Some(PartContent::Request(RequestPart::Permission(_))) => {
                        request_parts_by_operation
                            .entry(operation_id)
                            .or_default()
                            .permission = Some(SessionPartRef::new(
                            message_index,
                            message,
                            part_index,
                            part,
                        ));
                    }
                    Some(PartContent::Request(RequestPart::UserInput(_))) => {
                        request_parts_by_operation
                            .entry(operation_id)
                            .or_default()
                            .user_input = Some(SessionPartRef::new(
                            message_index,
                            message,
                            part_index,
                            part,
                        ));
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
                if !matches!(part.content.as_ref(), Some(PartContent::Operation(_))) {
                    continue;
                }
                if completed_tool_operations.contains(operation_id) {
                    continue;
                }

                let tool = SessionPendingTool {
                    part: SessionPartRef::new(message_index, message, part_index, part),
                };

                if let Some(request_parts) = request_parts_by_operation.get(operation_id) {
                    if let Some(request) = request_parts.permission.as_ref() {
                        operations.push(SessionPendingOperation::Permission {
                            pending: SessionPendingInteractiveRequest {
                                request: request.clone(),
                                tool,
                            },
                        });
                        continue;
                    }

                    if let Some(request) = request_parts.user_input.as_ref() {
                        operations.push(SessionPendingOperation::UserInput {
                            pending: SessionPendingInteractiveRequest {
                                request: request.clone(),
                                tool,
                            },
                        });
                        continue;
                    }
                }

                operations.push(SessionPendingOperation::Tool { tool });
            }
        }

        operations
    }

    pub(crate) fn next_pending_tool(&self) -> Option<SessionPendingTool> {
        self.pending_operations
            .iter()
            .find_map(|pending| pending.queued_tool().cloned())
    }

    pub(crate) fn pending_tools(&self) -> Vec<SessionPendingTool> {
        self.pending_operations
            .iter()
            .filter_map(|pending| pending.queued_tool().cloned())
            .collect()
    }

    pub(crate) fn resolve_part_ref(&self, part_ref: &SessionPartRef) -> Option<SessionPartRef> {
        if let Some(message) = self.messages.get(part_ref.message_index)
            && message.id == part_ref.message_id
            && let Some(part) = message.parts.get(part_ref.part_index)
            && part.id == part_ref.part_id
        {
            return Some(SessionPartRef::new(
                part_ref.message_index,
                message,
                part_ref.part_index,
                part,
            ));
        }

        self.messages
            .iter()
            .enumerate()
            .find(|(_, message)| message.id == part_ref.message_id)
            .and_then(|(message_index, message)| {
                message
                    .parts
                    .iter()
                    .enumerate()
                    .find(|(_, part)| part.id == part_ref.part_id)
                    .map(|(part_index, part)| {
                        SessionPartRef::new(message_index, message, part_index, part)
                    })
            })
    }

    pub(crate) fn part(&self, part_ref: &SessionPartRef) -> Option<&MessagePart> {
        let resolved = self.resolve_part_ref(part_ref)?;
        self.messages
            .get(resolved.message_index)?
            .parts
            .get(resolved.part_index)
    }

    pub(crate) fn part_mut(&mut self, part_ref: &SessionPartRef) -> Option<&mut MessagePart> {
        let resolved = self.resolve_part_ref(part_ref)?;
        self.messages
            .get_mut(resolved.message_index)?
            .parts
            .get_mut(resolved.part_index)
    }

    pub(crate) fn pending_tool_execution(
        &self,
        pending: &SessionPendingTool,
    ) -> Option<(i64, &ToolInvocation, &TimeRange)> {
        let part = self.part(&pending.part)?;
        if !matches!(
            part.status,
            ExecutionStatus::Pending | ExecutionStatus::InProgress
        ) {
            return None;
        }
        let operation = match part.content.as_ref()? {
            PartContent::Operation(operation) => operation,
            _ => return None,
        };

        Some((
            operation.call_id,
            &operation.invocation,
            &operation.lifecycle,
        ))
    }

    pub(crate) fn pending_tool_record(
        &self,
        pending: &SessionPendingTool,
    ) -> Option<ToolCallRecord> {
        let part = self.part(&pending.part)?;
        let state = ToolCallRecordState::from_execution_status(part.status)?;
        let operation_id = part.operation_id.clone()?;
        let operation = match part.content.as_ref()? {
            PartContent::Operation(operation) => operation,
            _ => return None,
        };

        Some(ToolCallRecord {
            operation_id,
            call_id: operation.call_id,
            invocation: operation.invocation.clone(),
            advertised_tool_identity: operation.advertised_tool_identity().map(ToOwned::to_owned),
            lifecycle: operation.lifecycle.clone(),
            state,
        })
    }

    pub(crate) fn pending_permission_request(
        &self,
        pending: &SessionPendingInteractiveRequest,
    ) -> Option<&crate::permission::PermissionRequest> {
        self.pending_request(&pending.request, |request| match request {
            RequestPart::Permission(InteractiveRequestPart { request, .. }) => Some(request),
            _ => None,
        })
    }

    pub(crate) fn pending_user_input_request(
        &self,
        pending: &SessionPendingInteractiveRequest,
    ) -> Option<&UserInputRequest> {
        self.pending_request(&pending.request, |request| match request {
            RequestPart::UserInput(InteractiveRequestPart { request, .. }) => Some(request),
            _ => None,
        })
    }

    fn completed_tool_operations(&self) -> HashSet<&str> {
        self.messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .filter(|part| {
                matches!(
                    part.status,
                    ExecutionStatus::Completed
                        | ExecutionStatus::Failed
                        | ExecutionStatus::Cancelled
                )
            })
            .filter_map(|part| part.operation_id.as_deref())
            .collect()
    }
}

fn message_has_completed_operation(message: &Message) -> bool {
    message.parts.iter().any(|part| {
        matches!(
            part.status,
            ExecutionStatus::Completed | ExecutionStatus::Failed | ExecutionStatus::Cancelled
        ) && matches!(
            part.content.as_ref(),
            Some(PartContent::Operation(operation)) if !operation.is_provider_native_only()
        )
    })
}

fn tool_invocation_name(invocation: &ToolInvocation) -> String {
    let ToolInvocation { name, .. } = invocation;
    name.clone()
}

fn extract_call_id(part: &MessagePart) -> Option<i64> {
    part.content.as_ref().and_then(|content| match content {
        PartContent::Operation(tool) => Some(tool.call_id),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;
    use crate::message::{OperationPart, StructuredObject, TimeRange, ToolOutput};

    fn user_message(id: i64, text: &str) -> Message {
        let mut message = Message::prompt_text(Role::User, text);
        message.id = id;
        if let Some(part) = message.parts.first_mut() {
            part.id = id * 100 + 1;
            part.message_id = id;
        }
        message
    }

    fn assistant_tool_message(
        id: i64,
        status: ExecutionStatus,
        operation_id: &str,
        part_id: i64,
    ) -> Message {
        let invocation = ToolInvocation::new(
            "process.run",
            StructuredObject::try_from(json!({ "command": "date" })).expect("tool input"),
        );
        let mut message = Message::prompt_parts(
            Role::Assistant,
            vec![PartContent::Operation(OperationPart::completed(
                7,
                invocation,
                String::new(),
                Vec::new(),
                Vec::new(),
                ToolOutput::default(),
                TimeRange::default(),
            ))],
        );
        message.id = id;
        if let Some(part) = message.parts.first_mut() {
            part.id = part_id;
            part.message_id = id;
            part.operation_id = Some(operation_id.to_string());
            part.status = status;
        }
        message
    }

    #[test]
    fn cancelled_operation_suppresses_stale_pending_tool() {
        let now = Utc::now();
        let session = Session::new(1, 1, "session", now).with_messages(vec![
            user_message(1, "run date"),
            assistant_tool_message(2, ExecutionStatus::Pending, "call_date_1", 201),
            assistant_tool_message(3, ExecutionStatus::Cancelled, "call_date_1", 301),
        ]);

        assert!(
            session.pending_tools().is_empty(),
            "cancelled tool operations should not leave a stale pending tool"
        );
        assert!(
            !session.blocked(),
            "cancelled tool operations should not surface as blocked"
        );
    }

    #[test]
    fn cancelled_operation_keeps_model_continuation_eligible() {
        let now = Utc::now();
        let session = Session::new(1, 1, "session", now).with_messages(vec![
            user_message(1, "run date"),
            assistant_tool_message(2, ExecutionStatus::Cancelled, "call_date_1", 201),
        ]);

        assert!(
            session.should_run_model(),
            "cancelled tool operations should still count as terminal tool results"
        );
    }
}

// NOTE: `SessionEventType` and `SessionEventRecord` have been removed. The
// unified `crate::event::EventKind` and `crate::event::DomainEvent` types
// are the only event shapes the system carries.
