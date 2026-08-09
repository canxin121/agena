//! Core session data model types.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use agena_domain::{
    ExecutionSelection, ModelRef, PromptCompactionActivity, PromptTokenUsageSnapshot, Role,
    SessionLifecycleState, SessionRelationKind, SubtaskStatus, WorkflowState,
};
use agena_storage::store::{Part, PartRole, PartState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Runtime state of a subtask.
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
/// Reference to a part inside a session's parts projection.
///
/// Parts are flat in v2 — there is no message nesting — so a part reference is
/// a position into [`Session::parts`] plus the part's stable id. The index is
/// not serialized; consumers resolve by `part_id` when the index is stale.
pub struct SessionPartRef {
    #[serde(default, skip_serializing)]
    pub part_index: usize,
    pub part_id: i64,
}

impl SessionPartRef {
    fn new(part_index: usize, part: &Part) -> Self {
        Self {
            part_index,
            part_id: part.part_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A pending tool operation inside a session.
pub struct SessionPendingTool {
    pub part: SessionPartRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A pending interactive request in a session.
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
/// A pending operation in a session.
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Runtime state of the session workflow.
pub struct WorkflowRuntimeState {
    #[serde(default)]
    pub state: WorkflowState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_operations: Vec<SessionPendingOperation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_tool_calls: Vec<PendingToolCallRuntime>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime state of a pending tool call.
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
/// A message included in a prompt compaction summary.
pub struct PromptCompactionMessage {
    pub id: i64,
    pub role: Role,
    pub source: agena_domain::MessageSource,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Content of a prompt compaction.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime state of a prompt compaction.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Runtime state of the prompt window.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Runtime state of prompt token accounting.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Provider-side anchor for prompt continuation.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Full runtime state of a session.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Execution context of a session.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A session with its runtime state.
///
/// The transcript is carried as a **parts projection cache**
/// ([`Session::parts`]): the full ordered list of v2 parts
/// (`agena_storage::store::Part`) in `(created_at_ms, part_id)` order. The
/// aggregate only reads this projection for execution; every write goes
/// through the storage facade. Derived state (pending operations, workflow
/// snapshot, approx bytes) is recomputed from the parts on
/// [`Session::install_projected_parts`] / [`Session::refresh_derived`].
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
    /// Parts projection cache, in `(created_at_ms, part_id)` order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    #[serde(skip, default)]
    pub runtime: SessionRuntimeState,
    #[serde(skip, default)]
    approx_bytes: usize,
    #[serde(skip, default)]
    pending_operations: Vec<SessionPendingOperation>,
}

/// True for a compaction checkpoint: the run marker emitted by
/// `start_run("compaction", …)` — `kind == "run"` with
/// `content.run_kind == "compaction"` (engine.rs maps the compaction run kind
/// to an assistant marker and stamps `content.run_kind`).
fn is_compaction_marker(part: &Part) -> bool {
    part.kind == "run" && part.content.get("run_kind") == Some(&serde_json::json!("compaction"))
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
            parts: Vec::new(),
            runtime: SessionRuntimeState::default(),
            approx_bytes: 0,
            pending_operations: Vec::new(),
        }
    }

    /// Install a fresh parts projection (the ordered transcript as loaded
    /// from the facade) and recompute all derived state from it.
    pub fn install_projected_parts(&mut self, parts: Vec<Part>) {
        self.parts = parts;
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
                let part = self.part(&tool.part)?;
                let operation_id = part
                    .content
                    .get("operation_id")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| part.part_id.to_string());
                let tool_name = part
                    .content
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                Some(PendingToolCallRuntime {
                    operation_id,
                    call_id: part.part_id,
                    tool_name,
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

    // --- Parts projection reads (the A6 execution contract) ----------------

    /// The full ordered parts projection cache.
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// The active model window: parts strictly after the last compaction
    /// checkpoint. A compaction checkpoint is the run marker emitted by
    /// `start_run("compaction", …)` — `kind == "run"` with
    /// `content.run_kind == "compaction"` — and the marker itself is excluded
    /// from the window; with no compaction checkpoint the window is the full
    /// projection. Cheap slice view — parts are already ordered, so the
    /// window is a contiguous suffix (13.4).
    pub fn active_window_parts(&self) -> &[Part] {
        match self.parts.iter().rposition(is_compaction_marker) {
            Some(index) => &self.parts[index + 1..],
            None => &self.parts[..],
        }
    }

    /// The last part in the projection (transcript tail).
    pub fn last_part(&self) -> Option<&Part> {
        self.parts.last()
    }

    /// The last run marker (`kind == "run"`).
    pub fn last_run_marker(&self) -> Option<&Part> {
        self.parts.iter().rev().find(|part| part.kind == "run")
    }

    /// The last user-authored content part (role `user`, excluding run
    /// markers).
    pub fn last_user_part(&self) -> Option<&Part> {
        self.parts
            .iter()
            .rev()
            .find(|part| part.role == PartRole::User && part.kind != "run")
    }

    /// Pending tool calls: `tool_call` parts still awaiting a result — not yet
    /// `completed`/`cancelled` and without a `tool_result` referencing them.
    pub fn pending_tool_calls(&self) -> impl Iterator<Item = &Part> {
        self.parts
            .iter()
            .filter(|part| self.pending_tool_call(part))
    }

    /// Pending interaction parts (`kind == "interaction"`, in-flight state).
    pub fn pending_interactions(&self) -> impl Iterator<Item = &Part> {
        self.parts
            .iter()
            .filter(|part| part.kind == "interaction" && part.state.is_in_flight())
    }

    /// The latest pending interaction part, if any.
    pub fn pending_interaction(&self) -> Option<&Part> {
        self.parts
            .iter()
            .rev()
            .find(|part| part.kind == "interaction" && part.state.is_in_flight())
    }

    /// Every `tool_result` part in the projection.
    pub fn tool_results(&self) -> impl Iterator<Item = &Part> {
        self.parts.iter().filter(|part| part.kind == "tool_result")
    }

    /// Whether a `tool_result` part references `call` via `parent_part_id`.
    pub fn tool_result_for(&self, call: &Part) -> Option<&Part> {
        self.parts
            .iter()
            .find(|part| part.kind == "tool_result" && part.parent_part_id == Some(call.part_id))
    }

    /// Whether `part` is a tool call still awaiting its result (not
    /// `completed`/`cancelled` and without a paired `tool_result`).
    fn pending_tool_call(&self, part: &Part) -> bool {
        part.kind == "tool_call"
            && part.state != PartState::Completed
            && part.state != PartState::Cancelled
            && self.tool_result_for(part).is_none()
    }

    // --- Derived execution state over the parts projection -----------------

    /// Whether the session is gated on a pending interaction (permission or
    /// user input) that blocks further execution.
    pub fn blocked(&self) -> bool {
        self.pending_operations
            .iter()
            .any(SessionPendingOperation::is_blocking_request)
    }

    /// The next tool-call id to allocate. In v2 the engine assigns call ids
    /// from part ids, so this is the highest part id in the projection plus
    /// one.
    pub fn next_call_id(&self) -> i64 {
        self.parts
            .iter()
            .map(|part| part.part_id)
            .max()
            .unwrap_or(0)
            + 1
    }

    pub fn approx_bytes(&self) -> usize {
        self.approx_bytes
    }

    /// Aggregate token usage mirrored into the parts projection.
    ///
    /// The authoritative v2 accounting lives in the separate `UsageRecord`
    /// store (facade `usage_stats`); this projection sums any `usage` object
    /// the engine embeds in part content (e.g. run markers) so the execution
    /// aggregate reads the same window it is executing against. Returns zero
    /// when no part carries a `usage` payload.
    pub fn aggregate_usage(&self) -> agena_provider::CompletionUsage {
        let mut total = agena_provider::CompletionUsage::default();
        for part in &self.parts {
            if let Some(usage) = usage_from_part_content(&part.content) {
                total.add_assign(&usage);
            }
        }
        total
    }

    /// Derive the pending operations from the parts projection:
    ///
    /// - `tool_call` parts not yet `completed`/`cancelled` and without a
    ///   paired `tool_result` become [`SessionPendingOperation::Tool`];
    /// - `interaction` parts in flight become
    ///   [`SessionPendingOperation::Permission`] when their content
    ///   `kind == "permission"`, otherwise [`SessionPendingOperation::UserInput`].
    fn derive_pending_operations(&self) -> Vec<SessionPendingOperation> {
        let mut operations = Vec::new();
        for (index, part) in self.parts.iter().enumerate() {
            match part.kind.as_str() {
                "tool_call" if self.pending_tool_call(part) => {
                    operations.push(SessionPendingOperation::Tool {
                        tool: SessionPendingTool {
                            part: SessionPartRef::new(index, part),
                        },
                    });
                }
                "interaction" if part.state.is_in_flight() => {
                    let part_ref = SessionPartRef::new(index, part);
                    if interaction_kind(part) == Some("permission") {
                        operations.push(SessionPendingOperation::Permission {
                            pending: SessionPendingPermissionRequest {
                                request_id: interaction_request_id(part),
                                tool: SessionPendingTool { part: part_ref },
                            },
                        });
                    } else {
                        operations.push(SessionPendingOperation::UserInput {
                            pending: SessionPendingInteractiveRequest {
                                request: part_ref.clone(),
                                tool: SessionPendingTool { part: part_ref },
                            },
                        });
                    }
                }
                _ => {}
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

    /// The pending tool whose part id matches `part_id` (call ids are part
    /// ids in v2). Interactive host callbacks that run during execution look
    /// up the executing part directly by id instead of relying on the
    /// pending-operation projection.
    pub fn pending_tool_by_part_id(&self, part_id: i64) -> Option<SessionPendingTool> {
        self.parts
            .iter()
            .enumerate()
            .find(|(_, part)| part.part_id == part_id && self.pending_tool_call(part))
            .map(|(index, part)| SessionPendingTool {
                part: SessionPartRef::new(index, part),
            })
    }

    // --- Part references ---------------------------------------------------

    pub fn resolve_part_ref(&self, part_ref: &SessionPartRef) -> Option<SessionPartRef> {
        if let Some(part) = self.parts.get(part_ref.part_index)
            && part.part_id == part_ref.part_id
        {
            return Some(part_ref.clone());
        }
        self.parts
            .iter()
            .enumerate()
            .find(|(_, part)| part.part_id == part_ref.part_id)
            .map(|(index, part)| SessionPartRef::new(index, part))
    }

    pub fn part(&self, part_ref: &SessionPartRef) -> Option<&Part> {
        let resolved = self.resolve_part_ref(part_ref)?;
        self.parts.get(resolved.part_index)
    }

    /// Mutable access to a projected part. Call [`Session::refresh_derived`]
    /// after mutating so derived state stays consistent.
    pub fn part_mut(&mut self, part_ref: &SessionPartRef) -> Option<&mut Part> {
        let resolved = self.resolve_part_ref(part_ref)?;
        self.parts.get_mut(resolved.part_index)
    }

    // --- Text helpers ------------------------------------------------------

    pub fn last_assistant_text(&self) -> Option<String> {
        self.parts
            .iter()
            .rev()
            .filter(|part| part.role == PartRole::Assistant)
            .find_map(text_from_part)
            .filter(|text| !text.trim().is_empty())
    }

    fn compute_approx_bytes(&self) -> usize {
        let mut bytes = 160 + self.title.len();
        bytes = bytes.saturating_add(self.parts.len().saturating_mul(96));
        for part in &self.parts {
            bytes = bytes
                .saturating_add(part.summary.as_ref().map_or(0, |value| value.len()))
                .saturating_add(
                    part.rendered_markdown
                        .as_ref()
                        .map_or(0, |value| value.len()),
                )
                .saturating_add(text_from_part(part).map_or(0, |text| text.len()));
        }
        bytes
    }
}

fn interaction_kind(part: &Part) -> Option<&str> {
    part.content.get("kind").and_then(serde_json::Value::as_str)
}

fn interaction_request_id(part: &Part) -> String {
    part.content
        .get("request_id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| part.part_id.to_string())
}

/// Best-effort text rendering of a part for `last_assistant_text` /
/// `compute_approx_bytes`: the `text` content field when present, else the
/// summary.
fn text_from_part(part: &Part) -> Option<String> {
    part.content
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .filter(|text| !text.trim().is_empty())
        .or_else(|| part.summary.clone())
}

/// Parse a `usage` object embedded in part content (run markers, assistant
/// parts) into a provider `CompletionUsage`. Tolerant: any subset of fields
/// parses (serde defaults fill the rest); empty objects are ignored.
fn usage_from_part_content(content: &serde_json::Value) -> Option<agena_provider::CompletionUsage> {
    let usage = content.get("usage")?;
    let parsed: agena_provider::CompletionUsage = serde_json::from_value(usage.clone()).ok()?;
    (parsed.requests > 0 || parsed.own_total_tokens() > 0).then_some(parsed)
}

#[cfg(test)]
mod parts_projection_tests {
    use super::*;
    use agena_storage::store::PartVisibility;
    use serde_json::json;

    /// Build a v2 part. Content follows the 4.1.1 shapes: run markers carry
    /// `{"run_kind": ...}`, text/think carry `{"text": ...}`, tool calls
    /// `{"name", "input"}`, tool results `{"output", "ok"}`, interactions
    /// `{"kind", "prompt", ...}`.
    fn part(
        part_id: i64,
        kind: &str,
        role: PartRole,
        state: PartState,
        content: serde_json::Value,
    ) -> Part {
        Part {
            part_id,
            kind: kind.to_owned(),
            role,
            state,
            content,
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: (kind != "run").then_some(1),
            origin_session_id: 1,
            revision: 1,
            started_at_ms: part_id,
            finished_at_ms: state.is_terminal().then_some(part_id),
            created_at_ms: part_id,
            updated_at_ms: part_id,
            provider_state: None,
        }
    }

    fn session_with(parts: Vec<Part>) -> Session {
        let mut session = Session::new(1, 1, "t", chrono::Utc::now());
        session.install_projected_parts(parts);
        session
    }

    #[test]
    fn install_projected_parts_replaces_projection_and_refreshes_derived() {
        let mut session = Session::new(1, 1, "t", chrono::Utc::now());
        assert!(session.parts().is_empty());
        assert!(!session.blocked());
        assert_eq!(session.workflow_state(), WorkflowState::Quiescent);

        session.install_projected_parts(vec![part(
            1,
            "interaction",
            PartRole::Runtime,
            PartState::Pending,
            json!({"kind": "ask_user", "prompt": "continue?"}),
        )]);
        assert_eq!(session.parts().len(), 1);
        assert!(session.blocked());
        assert_eq!(session.workflow_state(), WorkflowState::Blocked);
    }

    #[test]
    fn run_marker_and_user_part_are_projections_over_kind_and_role() {
        let session = session_with(vec![
            part(
                1,
                "run",
                PartRole::Runtime,
                PartState::Completed,
                json!({"run_kind": "user_send"}),
            ),
            part(
                2,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "hello"}),
            ),
            part(
                3,
                "run",
                PartRole::Assistant,
                PartState::InProgress,
                json!({"run_kind": "continue"}),
            ),
            part(
                4,
                "text",
                PartRole::Assistant,
                PartState::Completed,
                json!({"text": "hi"}),
            ),
            part(
                5,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "more"}),
            ),
        ]);

        assert_eq!(session.last_run_marker().map(|p| p.part_id), Some(3));
        assert_eq!(session.last_user_part().map(|p| p.part_id), Some(5));
        assert_eq!(session.last_part().map(|p| p.part_id), Some(5));
        assert_eq!(session.parts().len(), 5);
    }

    #[test]
    fn pending_tool_calls_exclude_paired_and_terminal_calls() {
        let paired = part(
            10,
            "tool_call",
            PartRole::Assistant,
            PartState::InProgress,
            json!({"name": "fs.read", "input": {}}),
        );
        let unpaired = part(
            11,
            "tool_call",
            PartRole::Assistant,
            PartState::Pending,
            json!({"name": "fs.write", "input": {}}),
        );
        let terminal = part(
            12,
            "tool_call",
            PartRole::Assistant,
            PartState::Completed,
            json!({"name": "done", "input": {}}),
        );
        let mut result = part(
            13,
            "tool_result",
            PartRole::Tool,
            PartState::Completed,
            json!({"output": "ok", "ok": true}),
        );
        result.parent_part_id = Some(paired.part_id);

        let session = session_with(vec![paired, unpaired, terminal, result]);

        let pending: Vec<i64> = session.pending_tool_calls().map(|p| p.part_id).collect();
        assert_eq!(pending, vec![11]);
        assert_eq!(
            session
                .tool_result_for(&session.parts()[0])
                .map(|p| p.part_id),
            Some(13),
            "fs.read is paired with its tool_result"
        );
        assert_eq!(session.tool_results().count(), 1);
        // Highest part id + 1 is the next call id (call ids are part ids).
        assert_eq!(session.next_call_id(), 14);
        assert_eq!(session.workflow_state(), WorkflowState::ToolPending);
    }

    #[test]
    fn pending_interactions_gate_the_session_until_answered() {
        let ask = part(
            20,
            "interaction",
            PartRole::Runtime,
            PartState::Pending,
            json!({"kind": "ask_user", "prompt": "?"}),
        );
        let answered = part(
            21,
            "interaction",
            PartRole::Runtime,
            PartState::Completed,
            json!({"kind": "plan_review", "prompt": "x", "reply": "yes"}),
        );

        let session = session_with(vec![ask, answered]);
        let pending: Vec<i64> = session.pending_interactions().map(|p| p.part_id).collect();
        assert_eq!(pending, vec![20]);
        assert_eq!(session.pending_interaction().map(|p| p.part_id), Some(20));
        assert!(session.blocked());
    }

    #[test]
    fn derive_pending_operations_maps_parts_to_tool_permission_and_user_input() {
        let tool = part(
            30,
            "tool_call",
            PartRole::Assistant,
            PartState::Pending,
            json!({"name": "fs.write", "input": {}}),
        );
        let ask = part(
            31,
            "interaction",
            PartRole::Runtime,
            PartState::Pending,
            json!({"kind": "ask_user", "prompt": "proceed?"}),
        );
        let permission = part(
            32,
            "interaction",
            PartRole::Runtime,
            PartState::Pending,
            json!({"kind": "permission", "prompt": "allow?", "request_id": "req-1"}),
        );

        let session = session_with(vec![tool, ask, permission]);
        let ops = &session.pending_operations;

        assert_eq!(ops.len(), 3);
        match &ops[0] {
            SessionPendingOperation::Tool { tool } => assert_eq!(tool.part.part_id, 30),
            _ => panic!("expected Tool op"),
        }
        match &ops[1] {
            SessionPendingOperation::UserInput { pending } => {
                assert_eq!(pending.request.part_id, 31)
            }
            _ => panic!("expected UserInput op"),
        }
        match &ops[2] {
            SessionPendingOperation::Permission { pending } => {
                assert_eq!(pending.request_id, "req-1");
                assert_eq!(pending.tool.part.part_id, 32);
            }
            _ => panic!("expected Permission op"),
        }

        let tool_refs = session.pending_tools();
        assert_eq!(tool_refs.len(), 1);
        assert_eq!(
            session.next_pending_tool().map(|t| t.part.part_id),
            Some(30)
        );
        assert_eq!(
            session.pending_tool_by_part_id(30).map(|t| t.part.part_id),
            Some(30)
        );
        assert!(session.pending_tool_by_part_id(999).is_none());
    }

    #[test]
    fn aggregate_usage_sums_usage_embedded_in_part_content() {
        let with_usage = part(
            50,
            "run",
            PartRole::Assistant,
            PartState::Completed,
            json!({
                "run_kind": "continue",
                "usage": {"requests": 1, "input_tokens": 100, "output_tokens": 20, "reasoning_tokens": 5}
            }),
        );
        let plain = part(
            51,
            "text",
            PartRole::Assistant,
            PartState::Completed,
            json!({"text": "no usage here"}),
        );

        let session = session_with(vec![with_usage, plain.clone()]);
        let usage = session.aggregate_usage();
        assert_eq!(usage.requests, 1);
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.reasoning_tokens, 5);

        let empty = session_with(vec![plain]);
        assert!(!empty.aggregate_usage().has_own_usage());
    }

    #[test]
    fn part_refs_resolve_by_id_when_the_index_is_stale() {
        let mut session = session_with(vec![
            part(
                1,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "a"}),
            ),
            part(
                2,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "b"}),
            ),
        ]);

        let current = SessionPartRef::new(0, &session.parts()[0]);
        assert_eq!(session.part(&current).map(|p| p.part_id), Some(1));

        let stale = SessionPartRef {
            part_index: 99,
            part_id: 2,
        };
        assert_eq!(session.part(&stale).map(|p| p.part_id), Some(2));
        if let Some(mut_part) = session.part_mut(&stale) {
            mut_part.summary = Some("updated".to_owned());
        }
        assert_eq!(
            session.part(&stale).and_then(|p| p.summary.as_deref()),
            Some("updated")
        );
    }

    #[test]
    fn last_assistant_text_takes_the_newest_assistant_text_part() {
        let session = session_with(vec![
            part(
                1,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "hello"}),
            ),
            part(
                2,
                "think",
                PartRole::Assistant,
                PartState::Completed,
                json!({"text": "thinking"}),
            ),
            part(
                3,
                "text",
                PartRole::Assistant,
                PartState::Completed,
                json!({"text": "hi there"}),
            ),
        ]);
        assert_eq!(session.last_assistant_text().as_deref(), Some("hi there"));
    }

    #[test]
    fn active_window_parts_without_compaction_returns_all_parts() {
        let session = session_with(vec![
            part(
                1,
                "run",
                PartRole::Runtime,
                PartState::Completed,
                json!({"run_kind": "user_send"}),
            ),
            part(
                2,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "hello"}),
            ),
            part(
                3,
                "text",
                PartRole::Assistant,
                PartState::Completed,
                json!({"text": "hi"}),
            ),
        ]);

        let window = session.active_window_parts();
        assert_eq!(window.len(), 3);
        assert_eq!(window[0].part_id, 1);
        assert_eq!(window[2].part_id, 3);
    }

    #[test]
    fn active_window_parts_with_one_compaction_returns_only_parts_after_it() {
        let session = session_with(vec![
            part(
                1,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "old 1"}),
            ),
            part(
                2,
                "run",
                PartRole::Assistant,
                PartState::Completed,
                json!({"run_kind": "compaction", "summary": "cp"}),
            ),
            part(
                3,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "new 1"}),
            ),
            part(
                4,
                "text",
                PartRole::Assistant,
                PartState::Completed,
                json!({"text": "new 2"}),
            ),
        ]);

        let window = session.active_window_parts();
        assert_eq!(window.len(), 2);
        assert_eq!(window[0].part_id, 3);
        assert_eq!(window[1].part_id, 4);
    }

    #[test]
    fn active_window_parts_with_two_compactions_returns_parts_after_the_last() {
        let session = session_with(vec![
            part(
                1,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "old 1"}),
            ),
            part(
                2,
                "run",
                PartRole::Assistant,
                PartState::Completed,
                json!({"run_kind": "compaction", "summary": "cp1"}),
            ),
            part(
                3,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "mid"}),
            ),
            part(
                4,
                "run",
                PartRole::Assistant,
                PartState::Completed,
                json!({"run_kind": "compaction", "summary": "cp2"}),
            ),
            part(
                5,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "new"}),
            ),
        ]);

        let window = session.active_window_parts();
        assert_eq!(window.len(), 1);
        assert_eq!(window[0].part_id, 5);
    }

    #[test]
    fn active_window_parts_with_compaction_at_the_end_is_empty() {
        let session = session_with(vec![
            part(
                1,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "old"}),
            ),
            part(
                2,
                "run",
                PartRole::Assistant,
                PartState::Completed,
                json!({"run_kind": "compaction", "summary": "cp"}),
            ),
        ]);

        assert!(session.active_window_parts().is_empty());
    }

    #[test]
    fn active_window_parts_ignores_unrelated_run_markers() {
        // A background/steer run marker is NOT a compaction checkpoint; the
        // window must still span it.
        let session = session_with(vec![
            part(
                1,
                "run",
                PartRole::Assistant,
                PartState::Completed,
                json!({"run_kind": "background"}),
            ),
            part(
                2,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "hello"}),
            ),
            part(
                3,
                "run",
                PartRole::Assistant,
                PartState::Completed,
                json!({"run_kind": "compaction", "summary": "cp"}),
            ),
            part(
                4,
                "text",
                PartRole::User,
                PartState::Completed,
                json!({"text": "new"}),
            ),
        ]);

        let window = session.active_window_parts();
        assert_eq!(window.len(), 1);
        assert_eq!(window[0].part_id, 4);
    }
}
// NOTE: `SessionEventType` and `SessionEventRecord` have been removed. The
// unified `crate::event::EventKind` and `crate::event::DomainEvent` types
// are the only event shapes the system carries.
