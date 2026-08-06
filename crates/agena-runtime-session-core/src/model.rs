use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

use crate::message::{
    InteractiveRequestPart, Message, MessagePart, PartContent, RequestPart, RuntimeActivity,
};
use agena_domain::{
    ExecutionSelection, ExecutionStatus, ModelRef, PendingInteractiveRequest,
    PendingInteractiveRequestKind, PromptCompactionActivity, PromptTokenUsageSnapshot, Role,
    SessionLifecycleState, SessionRelationKind, SubtaskStatus, TimeRange, ToolInvocation,
    UserInputRequest, WorkflowState,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct SubtaskRuntimeState {
    pub status: SubtaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::Failure>,
}

impl SubtaskRuntimeState {
    pub fn is_empty(&self) -> bool {
        self.status == SubtaskStatus::Created
            && self.started_at_ms.is_none()
            && self.finished_at_ms.is_none()
            && self.failure.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPartRef {
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
pub struct SessionPendingTool {
    pub part: SessionPartRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallRecordState {
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
pub struct ToolCallRecord {
    pub operation_id: String,
    pub call_id: i64,
    pub invocation: ToolInvocation,
    pub advertised_tool_identity: Option<String>,
    pub lifecycle: TimeRange,
    pub state: ToolCallRecordState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPendingInteractiveRequest {
    pub request: SessionPartRef,
    pub tool: SessionPendingTool,
}

/// A pending permission is an unresolved authorization record on the tool
/// Operation itself. `request_id` selects the record; no transcript part is
/// created for the approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPendingPermissionRequest {
    pub request_id: String,
    pub tool: SessionPendingTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionPendingOperation {
    Tool {
        tool: SessionPendingTool,
    },
    Permission {
        pending: SessionPendingPermissionRequest,
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
                pending: SessionPendingPermissionRequest { tool, .. },
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

    fn permission_request(&self) -> Option<&SessionPendingPermissionRequest> {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct WorkflowRuntimeState {
    #[serde(default)]
    pub state: WorkflowState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_operations: Vec<SessionPendingOperation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_tool_calls: Vec<PendingToolCallRuntime>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingToolCallRuntime {
    pub operation_id: String,
    pub call_id: i64,
    pub tool_name: String,
    pub part: SessionPartRef,
}

impl WorkflowRuntimeState {
    pub fn is_empty(&self) -> bool {
        self.state == WorkflowState::Quiescent
            && self.pending_operations.is_empty()
            && self.pending_tool_calls.is_empty()
    }
}

use agena_domain::{PromptCompactionStrategy, PromptCompactionTrigger};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCompactionMessage {
    pub id: i64,
    pub role: Role,
    pub source: agena_domain::MessageSource,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PromptCompactionContent {
    TextSummary {
        summary: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        recent_messages: Vec<PromptCompactionMessage>,
    },
    OpenAiResponses {
        provider_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        adapter_id: Option<String>,
        model_id: String,
        items: Vec<serde_json::Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct PromptCompactionRuntime {
    pub checkpoint_id: String,
    pub compacted_through_message_id: i64,
    pub trigger: PromptCompactionTrigger,
    pub strategy: PromptCompactionStrategy,
    pub content: PromptCompactionContent,
    pub before_tokens: u64,
    pub after_tokens: u64,
    pub created_at_ms: i64,
}

impl PromptCompactionRuntime {
    pub fn is_empty(&self) -> bool {
        self.checkpoint_id.trim().is_empty()
            || match &self.content {
                PromptCompactionContent::TextSummary { summary, .. } => summary.trim().is_empty(),
                PromptCompactionContent::OpenAiResponses { items, .. } => items.is_empty(),
            }
    }

    pub fn activity(&self, generation: u64) -> PromptCompactionActivity {
        PromptCompactionActivity {
            checkpoint_id: self.checkpoint_id.clone(),
            generation,
            compacted_through_message_id: self.compacted_through_message_id,
            trigger: self.trigger,
            strategy: self.strategy,
            before_tokens: self.before_tokens,
            after_tokens: self.after_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct PromptWindowRuntime {
    #[serde(default)]
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<PromptCompactionRuntime>,
    #[serde(default)]
    pub consecutive_compaction_failures: u8,
    #[serde(default)]
    pub auto_compaction_disabled: bool,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub remote_compaction_disabled_models: BTreeSet<String>,
}

impl PromptWindowRuntime {
    pub fn is_empty(&self) -> bool {
        self.generation == 0
            && self
                .compaction
                .as_ref()
                .is_none_or(PromptCompactionRuntime::is_empty)
            && self.consecutive_compaction_failures == 0
            && !self.auto_compaction_disabled
            && self.remote_compaction_disabled_models.is_empty()
    }

    pub fn record_compaction_success(&mut self) {
        self.consecutive_compaction_failures = 0;
        self.auto_compaction_disabled = false;
    }

    pub fn record_compaction_failure(&mut self) {
        self.consecutive_compaction_failures =
            self.consecutive_compaction_failures.saturating_add(1);
        if self.consecutive_compaction_failures >= 3 {
            self.auto_compaction_disabled = true;
        }
    }
}

#[cfg(test)]
mod prompt_window_runtime_tests {
    use super::*;

    #[test]
    fn repeated_failures_disable_auto_compaction_until_success() {
        let mut runtime = PromptWindowRuntime::default();
        runtime.record_compaction_failure();
        runtime.record_compaction_failure();
        assert!(!runtime.auto_compaction_disabled);
        runtime.record_compaction_failure();
        assert!(runtime.auto_compaction_disabled);
        assert_eq!(runtime.consecutive_compaction_failures, 3);
        runtime.record_compaction_success();
        assert!(!runtime.auto_compaction_disabled);
        assert_eq!(runtime.consecutive_compaction_failures, 0);
    }

    #[test]
    fn import_id_rewrite_updates_checkpoint_measurement_and_anchor() {
        let mut runtime = SessionRuntimeState::default();
        runtime.prompt_window.compaction = Some(PromptCompactionRuntime {
            checkpoint_id: "checkpoint".to_owned(),
            compacted_through_message_id: 9,
            trigger: PromptCompactionTrigger::Manual,
            strategy: PromptCompactionStrategy::LocalSummary,
            content: PromptCompactionContent::TextSummary {
                summary: "state".to_owned(),
                recent_messages: vec![PromptCompactionMessage {
                    id: 8,
                    role: Role::User,
                    source: agena_domain::MessageSource::User,
                    text: "recent".to_owned(),
                }],
            },
            before_tokens: 100,
            after_tokens: 20,
            created_at_ms: 1,
        });
        runtime.prompt_tokens.last_successful_assistant_message_id = Some(7);
        runtime.set_provider_anchor(ProviderPromptAnchor {
            provider_id: "p".to_owned(),
            model_id: "m".to_owned(),
            previous_response_id: "r".to_owned(),
            assistant_message_id: 6,
            prompt_window_generation: 1,
            system_fingerprint: String::new(),
            request_options_fingerprint: String::new(),
            provider_request_shape: None,
            transcript_digest: String::new(),
        });

        runtime.rewrite_storage_ids(100, 1_000);
        let checkpoint = runtime.prompt_window.compaction.as_ref().unwrap();
        assert_eq!(checkpoint.compacted_through_message_id, 109);
        let PromptCompactionContent::TextSummary {
            recent_messages, ..
        } = &checkpoint.content
        else {
            panic!("text checkpoint");
        };
        assert_eq!(recent_messages[0].id, 108);
        assert_eq!(
            runtime.prompt_tokens.last_successful_assistant_message_id,
            Some(107)
        );
        assert_eq!(
            runtime
                .provider_anchor("p", "m")
                .unwrap()
                .assistant_message_id,
            106
        );
    }
}

fn prompt_token_usage_snapshot(
    value: &agena_provider::CompletionUsage,
) -> agena_domain::PromptTokenUsageSnapshot {
    agena_domain::PromptTokenUsageSnapshot {
        input_tokens: value.input_tokens,
        output_tokens: value.output_tokens,
        reasoning_tokens: value.reasoning_tokens,
        cache_write_tokens: value.cache_write_tokens,
        cache_read_tokens: value.cache_read_tokens,
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
            .map(agena_domain::PromptTokenUsageSnapshot::prompt_tokens)
    }

    pub fn total_tokens(&self) -> Option<u64> {
        self.last_successful_usage
            .as_ref()
            .map(agena_domain::PromptTokenUsageSnapshot::total_tokens)
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
        usage: &agena_provider::CompletionUsage,
        prompt_window_generation: u64,
        model_context_window_tokens: Option<u32>,
        system_fingerprint: String,
        request_options_fingerprint: String,
        transcript_digest: String,
    ) {
        self.last_successful_usage = Some(prompt_token_usage_snapshot(usage));
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
    pub provider_request_shape: Option<agena_provider::PromptCacheShape>,
    #[serde(default)]
    pub transcript_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct SessionRuntimeState {
    #[serde(default, skip_serializing_if = "WorkflowRuntimeState::is_empty")]
    pub workflow: WorkflowRuntimeState,
    #[serde(default)]
    pub prompt_window: PromptWindowRuntime,
    #[serde(default, skip_serializing_if = "PromptTokenRuntime::is_empty")]
    pub prompt_tokens: PromptTokenRuntime,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_anchors: BTreeMap<String, ProviderPromptAnchor>,
    #[serde(default, skip_serializing_if = "SessionExecutionContext::is_empty")]
    pub execution: SessionExecutionContext,
    #[serde(default, skip_serializing_if = "SubtaskRuntimeState::is_empty")]
    pub subtask: SubtaskRuntimeState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, FromJsonQueryResult)]
pub struct SessionExecutionContext {
    #[serde(flatten)]
    pub selection: ExecutionSelection,
    #[serde(
        default,
        skip_serializing_if = "agena_domain::ExecutionAccess::is_inherit"
    )]
    pub access: agena_domain::ExecutionAccess,
    #[serde(
        default,
        skip_serializing_if = "crate::authorization::PermissionConfig::is_empty"
    )]
    pub effective_permission: crate::authorization::PermissionConfig,
    /// Independent non-escalation boundary inherited from a delegating
    /// parent. It is evaluated as a second policy at authorization time so a
    /// more-specific child rule cannot override a broader parent denial.
    #[serde(
        default,
        skip_serializing_if = "crate::authorization::PermissionConfig::is_empty"
    )]
    pub permission_ceiling: crate::authorization::PermissionConfig,
    /// Hard tool-capability boundary. Unlike a permission Deny this cannot be
    /// approved and stale calls resolve to CapabilityUnavailable.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub capability_denied_tool_names: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_workspace_root: Option<PathBuf>,
}

impl SessionExecutionContext {
    pub fn is_empty(&self) -> bool {
        self.selection.is_empty()
            && self.access.is_inherit()
            && self.effective_permission.is_empty()
            && self.permission_ceiling.is_empty()
            && self.capability_denied_tool_names.is_empty()
            && self.effective_workspace_root.is_none()
    }
}

impl agena_runtime_contracts::ToolSessionContext for SessionExecutionContext {
    fn effective_workspace_root(&self) -> Option<&std::path::Path> {
        self.effective_workspace_root.as_deref()
    }

    fn effective_permission(&self) -> &agena_runtime_contracts::authorization::PermissionConfig {
        &self.effective_permission
    }

    fn permission_ceiling(&self) -> &agena_runtime_contracts::authorization::PermissionConfig {
        &self.permission_ceiling
    }

    fn capability_denied_tool_names(&self) -> &BTreeSet<String> {
        &self.capability_denied_tool_names
    }

    fn execution_access(&self) -> agena_domain::ExecutionAccess {
        self.access
    }

    fn selected_model(&self) -> Option<&str> {
        self.selection.model.as_deref()
    }
}

impl SessionRuntimeState {
    pub fn rewrite_storage_ids(&mut self, message_offset: i64, part_offset: i64) {
        let rewrite_message = |id: &mut i64| {
            if *id > 0 {
                *id = id.saturating_add(message_offset);
            }
        };
        let rewrite_part = |id: &mut i64| {
            if *id > 0 {
                *id = id.saturating_add(part_offset);
            }
        };

        if let Some(compaction) = self.prompt_window.compaction.as_mut() {
            rewrite_message(&mut compaction.compacted_through_message_id);
            if let PromptCompactionContent::TextSummary {
                recent_messages, ..
            } = &mut compaction.content
            {
                for message in recent_messages {
                    rewrite_message(&mut message.id);
                }
            }
        }
        if let Some(message_id) = self
            .prompt_tokens
            .last_successful_assistant_message_id
            .as_mut()
        {
            rewrite_message(message_id);
        }
        for anchor in self.provider_anchors.values_mut() {
            rewrite_message(&mut anchor.assistant_message_id);
        }

        let rewrite_ref = |part_ref: &mut SessionPartRef| {
            rewrite_message(&mut part_ref.message_id);
            rewrite_part(&mut part_ref.part_id);
        };
        for operation in &mut self.workflow.pending_operations {
            match operation {
                SessionPendingOperation::Tool { tool } => rewrite_ref(&mut tool.part),
                SessionPendingOperation::Permission { pending } => {
                    rewrite_ref(&mut pending.tool.part);
                }
                SessionPendingOperation::UserInput { pending } => {
                    rewrite_ref(&mut pending.request);
                    rewrite_ref(&mut pending.tool.part);
                }
            }
        }
        for call in &mut self.workflow.pending_tool_calls {
            rewrite_ref(&mut call.part);
        }
    }

    pub fn provider_anchor_key(provider_id: &str, model_id: &str) -> String {
        format!("{provider_id}/{model_id}")
    }

    pub fn effective_workspace_root(&self) -> Option<&Path> {
        self.execution.effective_workspace_root.as_deref()
    }

    pub fn set_effective_workspace_root(&mut self, path: Option<PathBuf>) {
        self.execution.effective_workspace_root = path;
    }

    pub fn model_override(&self) -> Option<(&str, Option<&str>, &str)> {
        Some((
            self.execution.selection.provider.as_deref()?,
            self.execution.selection.adapter.as_deref(),
            self.execution.selection.model.as_deref()?,
        ))
    }

    pub fn effective_model_ref(&self) -> Result<Option<ModelRef>, agena_domain::IdentifierError> {
        self.execution.selection.model_ref()
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
        usage: &agena_provider::CompletionUsage,
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
    pub relation_kind: SessionRelationKind,
    pub lifecycle_state: SessionLifecycleState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_cutoff_seq_global: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_message_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
    #[serde(skip, default)]
    pub runtime: SessionRuntimeState,
    #[serde(skip, default)]
    approx_bytes: usize,
    #[serde(skip, default)]
    pending_operations: Vec<SessionPendingOperation>,
}

impl Session {
    pub fn new(id: i64, workspace_id: i64, title: impl Into<String>, now: DateTime<Utc>) -> Self {
        Self {
            id,
            parent_id: None,
            depth: 0,
            root_id: id,
            workspace_id,
            title: title.into(),
            version: 1,
            relation_kind: SessionRelationKind::Root,
            lifecycle_state: SessionLifecycleState::Ready,
            source_cutoff_seq_global: None,
            source_message_id: None,
            task_id: None,
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            runtime: SessionRuntimeState::default(),
            approx_bytes: 0,
            pending_operations: Vec::new(),
        }
    }

    pub fn install_projected_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        self.refresh_derived();
    }

    pub fn refresh_derived(&mut self) {
        self.approx_bytes = self.compute_approx_bytes();
        self.pending_operations = self.derive_pending_operations();
        self.runtime.workflow = self.workflow_runtime_snapshot();
    }

    pub fn sync_workflow_state(&mut self) {
        self.refresh_derived();
    }

    fn workflow_runtime_snapshot(&self) -> WorkflowRuntimeState {
        let state = if self.blocked() {
            WorkflowState::Blocked
        } else if self.next_pending_tool().is_some() {
            WorkflowState::ToolPending
        } else {
            WorkflowState::Quiescent
        };

        WorkflowRuntimeState {
            state,
            pending_operations: self.pending_operations.clone(),
            pending_tool_calls: self.pending_tool_runtime_snapshots(),
        }
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

    pub fn workflow_state(&self) -> WorkflowState {
        self.workflow_runtime_snapshot().state
    }

    pub fn runtime(&self) -> &SessionRuntimeState {
        &self.runtime
    }

    /// Delegated-task semantics come exclusively from immutable lineage.
    pub const fn is_subagent(&self) -> bool {
        self.relation_kind.is_subagent()
    }

    pub fn apply_persisted_metadata(&mut self, persisted: &Session) {
        self.id = persisted.id;
        self.parent_id = persisted.parent_id;
        self.depth = persisted.depth;
        self.root_id = persisted.root_id;
        self.workspace_id = persisted.workspace_id;
        self.title = persisted.title.clone();
        self.version = persisted.version;
        self.relation_kind = persisted.relation_kind;
        self.lifecycle_state = persisted.lifecycle_state;
        self.source_cutoff_seq_global = persisted.source_cutoff_seq_global;
        self.source_message_id = persisted.source_message_id;
        self.task_id = persisted.task_id.clone();
        self.created_at = persisted.created_at;
        self.updated_at = persisted.updated_at;
        self.runtime = persisted.runtime.clone();
    }

    pub fn blocked(&self) -> bool {
        self.pending_operations
            .iter()
            .any(SessionPendingOperation::is_blocking_request)
    }

    pub fn next_call_id(&self) -> i64 {
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
                Some(PartContent::Activity(RuntimeActivity::Interaction(request))) => {
                    request_matches(request, request_id)
                }
                _ => false,
            })
    }

    fn pending_request<'a, T>(
        &'a self,
        request_part: &SessionPartRef,
        extract_request: impl FnOnce(&'a RequestPart) -> Option<&'a T>,
    ) -> Option<&'a T> {
        let part = self.part(request_part)?;
        let PartContent::Activity(RuntimeActivity::Interaction(request)) = part.content.as_ref()?
        else {
            return None;
        };
        extract_request(request)
    }

    pub fn find_pending_user_input_by_request_id(
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

    pub fn has_replied_user_input_request(&self, request_id: &str) -> bool {
        self.has_replied_request(request_id, |request, request_id| match request {
            RequestPart::UserInput(request) => {
                request.request.request_id == request_id && request.reply.is_some()
            }
        })
    }

    pub fn has_finished_operation(&self, operation_id: &str) -> bool {
        self.messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .any(|part| {
                part.operation_id.as_deref() == Some(operation_id)
                    && matches!(
                        part.content.as_ref(),
                        Some(PartContent::Activity(RuntimeActivity::Operation(_)))
                    )
                    && matches!(
                        part.status,
                        ExecutionStatus::Completed
                            | ExecutionStatus::PolicyDenied
                            | ExecutionStatus::UserDeclined
                            | ExecutionStatus::CapabilityUnavailable
                            | ExecutionStatus::ToolUnavailable
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

    pub fn user_input_request_for_operation(
        &self,
        operation_id: &str,
        sequence_index: usize,
    ) -> Option<InteractiveRequestPart<UserInputRequest, agena_domain::UserInputReply>> {
        self.messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .filter(|part| part.operation_id.as_deref() == Some(operation_id))
            .filter_map(|part| match part.content.as_ref() {
                Some(PartContent::Activity(RuntimeActivity::Interaction(
                    RequestPart::UserInput(request),
                ))) => Some(request.clone()),
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
        bytes
    }

    pub fn last_conversation_message(&self) -> Option<&Message> {
        self.messages.last()
    }

    pub fn last_assistant_text(&self) -> Option<String> {
        self.last_assistant_text_after(None)
    }

    pub fn last_assistant_text_after(&self, message_id: Option<i64>) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .find(|message| {
                message.role == Role::Assistant
                    && message_id.is_none_or(|message_id| message.id > message_id)
            })
            .map(Message::visible_text_lossy)
            .filter(|text| !text.trim().is_empty())
    }

    pub fn aggregate_usage(&self) -> agena_provider::CompletionUsage {
        let mut total = agena_provider::CompletionUsage::default();
        for usage in self
            .messages
            .iter()
            .filter_map(|message| message.usage.as_ref())
        {
            total.add_assign(usage);
        }
        total
    }

    pub fn find_pending_permission_by_request_id(
        &self,
        request_id: &str,
    ) -> Option<SessionPendingPermissionRequest> {
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

    pub fn has_replied_permission_request(&self, request_id: &str) -> bool {
        self.messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .filter_map(|part| match part.content.as_ref() {
                Some(PartContent::Activity(RuntimeActivity::Operation(operation))) => {
                    Some(operation)
                }
                _ => None,
            })
            .any(|operation| {
                operation
                    .authorization
                    .find(request_id)
                    .is_some_and(|permission| permission.reply.is_some())
            })
    }

    /// Permission actions durably approved for this exact message operation.
    ///
    /// A provider call id alone is not an authorization identity: providers
    /// may reuse one in a later turn. The owning assistant message therefore
    /// participates in the lookup. Callers must still evaluate any current
    /// permission checks whose actions are absent from this returned set; a
    /// plugin/catalog change cannot widen an earlier user approval.
    pub fn operation_permission_approved_actions(
        &self,
        assistant_message_id: i64,
        operation_id: &str,
    ) -> Vec<agena_domain::PermissionAction> {
        self.messages
            .iter()
            .filter(|message| message.id == assistant_message_id)
            .flat_map(|message| &message.parts)
            .find_map(|part| {
                if part.operation_id.as_deref() != Some(operation_id) {
                    return None;
                }
                let Some(PartContent::Activity(RuntimeActivity::Operation(operation))) =
                    part.content.as_ref()
                else {
                    return None;
                };
                let mut approved = Vec::new();
                for permission in &operation.authorization.permissions {
                    let Some(reply) = permission.reply.as_ref() else {
                        continue;
                    };
                    if !matches!(
                        reply.kind,
                        agena_domain::PermissionReplyKind::AllowOnce
                            | agena_domain::PermissionReplyKind::AllowAlways
                    ) {
                        continue;
                    }
                    let actions = if permission.request.requested_actions.is_empty() {
                        std::slice::from_ref(&permission.request.action)
                    } else {
                        permission.request.requested_actions.as_slice()
                    };
                    for action in actions {
                        if !approved.contains(action) {
                            approved.push(action.clone());
                        }
                    }
                }
                Some(approved)
            })
            .unwrap_or_default()
    }

    fn derive_pending_operations(&self) -> Vec<SessionPendingOperation> {
        #[derive(Default)]
        struct PendingRequestParts {
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

                if let Some(PartContent::Activity(RuntimeActivity::Interaction(
                    RequestPart::UserInput(_),
                ))) = part.content.as_ref()
                {
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
            }

            for (part_index, part) in message.parts.iter().enumerate() {
                if part.status != ExecutionStatus::Pending {
                    continue;
                }

                let Some(operation_id) = part.operation_id.as_deref() else {
                    continue;
                };
                let Some(PartContent::Activity(RuntimeActivity::Operation(operation))) =
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

                if let Some(permission) = operation.authorization.awaiting().next() {
                    operations.push(SessionPendingOperation::Permission {
                        pending: SessionPendingPermissionRequest {
                            request_id: permission.request.request_id.clone(),
                            tool,
                        },
                    });
                    continue;
                }

                if let Some(request_parts) = request_parts_by_operation.get(operation_id)
                    && let Some(request) = request_parts.user_input.as_ref()
                {
                    operations.push(SessionPendingOperation::UserInput {
                        pending: SessionPendingInteractiveRequest {
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

    pub fn next_pending_tool(&self) -> Option<SessionPendingTool> {
        self.pending_operations
            .iter()
            .find_map(|pending| pending.queued_tool().cloned())
    }

    pub fn pending_tools(&self) -> Vec<SessionPendingTool> {
        self.pending_operations
            .iter()
            .filter_map(|pending| pending.queued_tool().cloned())
            .collect()
    }

    /// Finds the tool part (Pending or InProgress) whose operation call id
    /// matches `call_id`.
    ///
    /// `pending_tools()` only derives from `Pending` parts, but a tool is
    /// moved to `InProgress` before it executes. Interactive host callbacks
    /// that run during execution (`ask`, plan review) must therefore look up
    /// the executing part directly by call id instead of relying on the
    /// pending-operation projection.
    pub fn pending_tool_by_call_id(&self, call_id: i64) -> Option<SessionPendingTool> {
        for (message_index, message) in self.messages.iter().enumerate() {
            if message.role != Role::Assistant {
                continue;
            }
            for (part_index, part) in message.parts.iter().enumerate() {
                if !matches!(
                    part.status,
                    ExecutionStatus::Pending | ExecutionStatus::InProgress
                ) {
                    continue;
                }
                let Some(PartContent::Activity(RuntimeActivity::Operation(operation))) =
                    part.content.as_ref()
                else {
                    continue;
                };
                if operation.call_id != call_id {
                    continue;
                }
                return Some(SessionPendingTool {
                    part: SessionPartRef::new(message_index, message, part_index, part),
                });
            }
        }
        None
    }

    pub fn resolve_part_ref(&self, part_ref: &SessionPartRef) -> Option<SessionPartRef> {
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

    pub fn part(&self, part_ref: &SessionPartRef) -> Option<&MessagePart> {
        let resolved = self.resolve_part_ref(part_ref)?;
        self.messages
            .get(resolved.message_index)?
            .parts
            .get(resolved.part_index)
    }

    pub fn part_mut(&mut self, part_ref: &SessionPartRef) -> Option<&mut MessagePart> {
        let resolved = self.resolve_part_ref(part_ref)?;
        self.messages
            .get_mut(resolved.message_index)?
            .parts
            .get_mut(resolved.part_index)
    }

    pub fn pending_tool_execution(
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
            PartContent::Activity(RuntimeActivity::Operation(operation)) => operation,
            _ => return None,
        };

        Some((
            operation.call_id,
            &operation.invocation,
            &operation.lifecycle,
        ))
    }

    pub fn pending_tool_record(&self, pending: &SessionPendingTool) -> Option<ToolCallRecord> {
        let part = self.part(&pending.part)?;
        let state = ToolCallRecordState::from_execution_status(part.status)?;
        let operation_id = part.operation_id.clone()?;
        let operation = match part.content.as_ref()? {
            PartContent::Activity(RuntimeActivity::Operation(operation)) => operation,
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

    pub fn pending_permission_request(
        &self,
        pending: &SessionPendingPermissionRequest,
    ) -> Option<&agena_domain::PermissionRequest> {
        let part = self.part(&pending.tool.part)?;
        let PartContent::Activity(RuntimeActivity::Operation(operation)) = part.content.as_ref()?
        else {
            return None;
        };
        operation
            .authorization
            .find(pending.request_id.as_str())
            .filter(|permission| permission.reply.is_none())
            .map(|permission| &permission.request)
    }

    pub fn pending_user_input_request(
        &self,
        pending: &SessionPendingInteractiveRequest,
    ) -> Option<&UserInputRequest> {
        self.pending_request(&pending.request, |request| match request {
            RequestPart::UserInput(InteractiveRequestPart { request, .. }) => Some(request),
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
                        | ExecutionStatus::PolicyDenied
                        | ExecutionStatus::UserDeclined
                        | ExecutionStatus::CapabilityUnavailable
                        | ExecutionStatus::ToolUnavailable
                        | ExecutionStatus::Failed
                        | ExecutionStatus::Cancelled
                ) && matches!(
                    part.content.as_ref(),
                    Some(PartContent::Activity(RuntimeActivity::Operation(_)))
                )
            })
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
        PartContent::Activity(RuntimeActivity::Operation(tool)) => Some(tool.call_id),
        _ => None,
    })
}

// NOTE: `SessionEventType` and `SessionEventRecord` have been removed. The
// unified `crate::event::EventKind` and `crate::event::DomainEvent` types
// are the only event shapes the system carries.
