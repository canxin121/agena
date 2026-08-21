//! Core session data model types.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use agena_domain::{
    ExecutionSelection, ModelRef, PromptCompactionActivity, PromptTokenUsageSnapshot, Role,
    SessionLifecycleState, SessionRelationKind, SubtaskStatus, WorkflowState,
};
use agena_storage::store::{Part, PartRole};

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
    /// Process-local carrier for plugin scope routing. This is deliberately
    /// excluded from persisted execution config; the owning Session binds its
    /// current id after creation/hydration.
    #[serde(skip, default)]
    pub scope_session_id: Option<i64>,
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
    fn session_id(&self) -> Option<i64> {
        self.scope_session_id
    }

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

/// Decode the [`OperationPart`] carried by a `tool_call` part's content: the
/// canonical content is the flattened [`ToolCallContent`] shape, so the
/// operation lives under the `extra["operation"]` bucket `operation_from_tool_call`
/// reads back. Returns `None` for non-tool parts or undecodable payloads.
fn operation_from_part(part: &Part) -> Option<agena_runtime_contracts::part::OperationPart> {
    if part.kind != "tool_call" {
        return None;
    }
    let tool_call =
        agena_runtime_contracts::part_content::ToolCallContent::try_from(&part.content).ok()?;
    Some(agena_runtime_contracts::part_content::operation_from_tool_call(&tool_call))
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
        };
        session.bind_runtime_scope();
        session
    }

    pub fn bind_runtime_scope(&mut self) {
        self.runtime.execution.scope_session_id = Some(self.id);
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
            WorkflowState::AwaitingInteraction
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
                let operation = operation_from_part(part);
                let call_id = operation
                    .as_ref()
                    .map(|operation| operation.call_id)
                    .filter(|call_id| *call_id != 0)
                    .unwrap_or(part.part_id);
                let operation_id = operation
                    .as_ref()
                    .and_then(|operation| operation.metadata.get("agena.operation_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| call_id.to_string());
                let tool_name = operation
                    .as_ref()
                    .map(|operation| operation.invocation.name.as_str())
                    .filter(|name| !name.trim().is_empty())
                    .or_else(|| part.content.get("name").and_then(serde_json::Value::as_str))
                    .unwrap_or_default()
                    .to_owned();
                Some(PendingToolCallRuntime {
                    operation_id,
                    call_id,
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
        self.bind_runtime_scope();
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

    /// Pending tool calls are represented by the state of the single
    /// `tool_call` part. Results never live in a second persisted part.
    pub fn pending_tool_calls(&self) -> impl Iterator<Item = &Part> {
        self.parts
            .iter()
            .filter(|part| self.pending_tool_call(part))
    }

    /// Whether `part` carries an unanswered user-input ask: an in-flight
    /// `tool_call` operation whose `user_input` bucket has an awaiting record.
    fn pending_user_input_part(&self, part: &Part) -> bool {
        if !part.state.is_in_flight() {
            return false;
        }
        part.kind == "tool_call"
            && operation_from_part(part)
                .map(|operation| operation.user_input.awaiting().next().is_some())
                .unwrap_or(false)
    }

    /// Pending user-input asks carried by in-flight `tool_call` operations.
    pub fn pending_interactions(&self) -> impl Iterator<Item = &Part> {
        self.parts
            .iter()
            .filter(|part| self.pending_user_input_part(part))
    }

    /// The latest pending user-input ask, if any.
    pub fn pending_interaction(&self) -> Option<&Part> {
        self.parts
            .iter()
            .rev()
            .find(|part| self.pending_user_input_part(part))
    }

    /// Whether `part` is an in-flight tool call still awaiting its result.
    fn pending_tool_call(&self, part: &Part) -> bool {
        part.kind == "tool_call" && part.state.is_in_flight()
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
    /// - `tool_call` parts not yet `completed`/`cancelled` become
    ///   [`SessionPendingOperation::Tool`]. Every
    ///   unresolved authorization record on that operation also becomes a
    ///   [`SessionPendingOperation::Permission`], and a suspended tool whose
    ///   operation carries an awaiting `user_input` record is also a
    ///   [`SessionPendingOperation::UserInput`] — each ask lives inside the
    ///   operation;
    fn derive_pending_operations(&self) -> Vec<SessionPendingOperation> {
        let mut operations = Vec::new();
        for (index, part) in self.parts.iter().enumerate() {
            match part.kind.as_str() {
                "tool_call" if self.pending_tool_call(part) => {
                    let tool = SessionPendingTool {
                        part: SessionPartRef::new(index, part),
                    };
                    operations.push(SessionPendingOperation::Tool { tool: tool.clone() });
                    if let Some(operation) = operation_from_part(part) {
                        for permission in operation.authorization.awaiting() {
                            operations.push(SessionPendingOperation::Permission {
                                pending: SessionPendingPermissionRequest {
                                    request_id: permission.request.request_id.clone(),
                                    tool: tool.clone(),
                                },
                            });
                        }
                        if operation.user_input.awaiting().next().is_some() {
                            operations.push(SessionPendingOperation::UserInput {
                                pending: SessionPendingInteractiveRequest {
                                    request: tool.part.clone(),
                                    tool: tool.clone(),
                                },
                            });
                        }
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
                .saturating_add(text_from_part(part).map_or(0, |text| text.len()));
        }
        bytes
    }
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
    use agena_storage::store::PartState;
    use agena_storage::store::PartVisibility;
    use serde_json::json;

    /// Build a part using the current durable content shapes: run markers carry
    /// `{"run_kind": ...}`, text/think carry `{"text": ...}`, and a
    /// `tool_call` carries invocation, lifecycle, output, authorization, and
    /// user-input state in one strict content object.
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

    /// Project an [`OperationPart`] onto the canonical flat `tool_call` content
    /// (single source: invocation identity + one payload; no operation bucket).
    fn tool_call_content(
        operation: &agena_runtime_contracts::part::OperationPart,
    ) -> serde_json::Value {
        agena_runtime_contracts::part_content::ToolCallContent {
            name: operation.invocation.name.clone(),
            plugin: operation.invocation.plugin_name.clone(),
            input: serde_json::Value::from(operation.invocation.input.clone()),
            tool_api_call: operation.invocation.tool_api_call.clone(),
            call_id: operation.call_id,
            state: operation.state,
            authorization: operation.authorization.clone(),
            user_input: operation.user_input.clone(),
            output: operation.output.clone(),
            error: operation.error.clone(),
            metadata: operation.metadata.clone(),
            lifecycle: operation.lifecycle.clone(),
        }
        .as_value()
    }

    #[test]
    fn install_projected_parts_replaces_projection_and_refreshes_derived() {
        let mut session = Session::new(1, 1, "t", chrono::Utc::now());
        assert!(session.parts().is_empty());
        assert!(!session.blocked());
        assert_eq!(session.workflow_state(), WorkflowState::Quiescent);

        session.install_projected_parts(vec![tool_call_with_ask(1, "host-input:1:1:0", false)]);
        assert_eq!(session.parts().len(), 1);
        assert!(session.blocked());
        assert_eq!(session.workflow_state(), WorkflowState::AwaitingInteraction);
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
    fn pending_tool_calls_are_defined_only_by_the_tool_call_state() {
        let running = part(
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
        let completed = part(
            12,
            "tool_call",
            PartRole::Assistant,
            PartState::Completed,
            json!({"name": "done", "input": {}}),
        );
        let failed = part(
            14,
            "tool_call",
            PartRole::Assistant,
            PartState::Failed,
            json!({"name": "failed", "input": {}}),
        );
        let cancelled = part(
            15,
            "tool_call",
            PartRole::Assistant,
            PartState::Cancelled,
            json!({"name": "cancelled", "input": {}}),
        );
        let session = session_with(vec![running, unpaired, completed, failed, cancelled]);

        let pending: Vec<i64> = session.pending_tool_calls().map(|p| p.part_id).collect();
        assert_eq!(pending, vec![10, 11]);
        // Highest part id + 1 is the next call id (call ids are part ids).
        assert_eq!(session.next_call_id(), 16);
        assert_eq!(session.workflow_state(), WorkflowState::ToolPending);
    }

    #[test]
    fn pending_tool_runtime_snapshot_uses_operation_protocol_identity() {
        let mut operation = agena_runtime_contracts::part::OperationPart::pending(
            7,
            agena_domain::ToolInvocation::new(
                "tools_search",
                agena_domain::StructuredObject::default(),
            ),
            agena_domain::TimeRange::default(),
        );
        operation.metadata.insert(
            "agena.operation_id".to_owned(),
            serde_json::json!("call_tools_search_7"),
        );
        let pending = part(
            42,
            "tool_call",
            PartRole::Assistant,
            PartState::InProgress,
            tool_call_content(&operation),
        );
        let session = session_with(vec![pending]);
        let snapshot = session
            .runtime
            .workflow
            .pending_tool_calls
            .first()
            .expect("pending tool runtime snapshot");
        assert_eq!(snapshot.operation_id, "call_tools_search_7");
        assert_eq!(snapshot.call_id, 7);
        assert_eq!(snapshot.tool_name, "tools_search");
        assert_eq!(snapshot.part.part_id, 42);
    }

    /// A `tool_call` part whose operation carries one user-input request,
    /// awaiting (or answered when `answered` is set).
    fn tool_call_with_ask(part_id: i64, request_id: &str, answered: bool) -> Part {
        let mut operation = agena_runtime_contracts::part::OperationPart::pending(
            part_id,
            agena_domain::ToolInvocation::new(
                "plan.set",
                agena_domain::StructuredObject::default(),
            ),
            agena_domain::TimeRange {
                start_ms: 1,
                end_ms: None,
            },
        );
        let request = agena_domain::UserInputRequest {
            request_id: request_id.to_owned(),
            session_id: Some(1),
            title: "Approve New Plan".to_owned(),
            body_markdown: String::new(),
            kind: agena_domain::UserInputKind::Review,
            source: agena_domain::UserInputSource::Host,
            auto_resolution_ms: None,
            presented_at: None,
            questions: Vec::new(),
            created_at: chrono::Utc::now(),
        };
        operation.user_input.push_pending(request);
        if answered {
            operation.user_input.record_reply(
                agena_domain::UserInputReply {
                    request_id: request_id.to_owned(),
                    kind: agena_domain::UserInputReplyKind::Submit,
                    answers: Default::default(),
                    reason: None,
                },
                1234,
            );
        }
        part(
            part_id,
            "tool_call",
            PartRole::Assistant,
            PartState::Pending,
            tool_call_content(&operation),
        )
    }

    /// A `tool_call` part whose operation carries one permission request,
    /// awaiting (or answered when `answered` is set).
    fn tool_call_with_permission(part_id: i64, request_id: &str, answered: bool) -> Part {
        let mut operation = agena_runtime_contracts::part::OperationPart::pending(
            part_id,
            agena_domain::ToolInvocation::new("fs.read", agena_domain::StructuredObject::default()),
            agena_domain::TimeRange {
                start_ms: 1,
                end_ms: None,
            },
        );
        let request = agena_domain::PermissionRequest {
            request_id: request_id.to_owned(),
            session_id: Some(1),
            action: agena_domain::PermissionAction::Tool {
                tool_name: "fs.read".to_owned(),
                qualifier: None,
            },
            related_actions: Vec::new(),
            requested_actions: Vec::new(),
            reason: "confirm read".to_owned(),
            explanation: String::new(),
            source: None,
            scope: None,
            operator: None,
            trace: Vec::new(),
            created_at: chrono::Utc::now(),
        };
        operation.authorization.push_pending(request);
        if answered {
            operation.authorization.record_reply(
                agena_domain::PermissionReply {
                    request_id: request_id.to_owned(),
                    kind: agena_domain::PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
                1234,
            );
        }
        part(
            part_id,
            "tool_call",
            PartRole::Assistant,
            PartState::Pending,
            tool_call_content(&operation),
        )
    }

    #[test]
    fn pending_interactions_gate_the_session_until_answered() {
        let ask = tool_call_with_ask(20, "host-input:1:1:0", false);
        let answered = tool_call_with_ask(21, "host-input:1:1:1", true);

        let session = session_with(vec![ask, answered]);
        let pending: Vec<i64> = session.pending_interactions().map(|p| p.part_id).collect();
        assert_eq!(pending, vec![20]);
        assert_eq!(session.pending_interaction().map(|p| p.part_id), Some(20));
        assert!(session.blocked());
    }

    #[test]
    fn pending_operation_permissions_gate_the_session_until_answered() {
        let awaiting = tool_call_with_permission(22, "host-permission:1:22", false);
        let answered = tool_call_with_permission(23, "host-permission:1:23", true);

        let session = session_with(vec![awaiting, answered]);
        assert_eq!(session.pending_operations.len(), 3);
        match &session.pending_operations[0] {
            SessionPendingOperation::Tool { tool } => assert_eq!(tool.part.part_id, 22),
            other => panic!("expected pending tool, got {other:?}"),
        }
        match &session.pending_operations[1] {
            SessionPendingOperation::Permission { pending } => {
                assert_eq!(pending.request_id, "host-permission:1:22");
                assert_eq!(pending.tool.part.part_id, 22);
            }
            other => panic!("expected pending permission, got {other:?}"),
        }
        match &session.pending_operations[2] {
            SessionPendingOperation::Tool { tool } => assert_eq!(tool.part.part_id, 23),
            other => panic!("expected answered tool to remain queued, got {other:?}"),
        }
        assert!(session.blocked());
        assert_eq!(session.workflow_state(), WorkflowState::AwaitingInteraction);
    }

    #[test]
    fn derive_pending_operations_maps_tool_and_user_input_asks() {
        let tool = part(
            30,
            "tool_call",
            PartRole::Assistant,
            PartState::Pending,
            json!({"name": "fs.write", "input": {}}),
        );
        // A suspended tool whose operation carries an awaiting user-input
        // record is both a pending Tool and a pending UserInput (one tool_call
        // activity == one ask in the single-activity shape).
        let ask = tool_call_with_ask(31, "host-input:1:1:0", false);
        let session = session_with(vec![tool, ask]);
        let ops = &session.pending_operations;

        assert_eq!(ops.len(), 3);
        match &ops[0] {
            SessionPendingOperation::Tool { tool } => assert_eq!(tool.part.part_id, 30),
            _ => panic!("expected Tool op"),
        }
        match &ops[1] {
            SessionPendingOperation::Tool { tool } => assert_eq!(tool.part.part_id, 31),
            _ => panic!("expected Tool op for the suspended ask tool"),
        }
        match &ops[2] {
            SessionPendingOperation::UserInput { pending } => {
                assert_eq!(pending.request.part_id, 31);
                assert_eq!(pending.tool.part.part_id, 31);
            }
            _ => panic!("expected UserInput op for the suspended ask tool"),
        }

        let tool_refs = session.pending_tools();
        assert_eq!(tool_refs.len(), 2);
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
