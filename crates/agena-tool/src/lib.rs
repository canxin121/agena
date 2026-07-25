//! Provider-independent tool contracts.
//!
//! Concrete executors, built-in tools, plugin hosts, and permission policy
//! implementations belong in adapter/runtime crates rather than this crate.

use std::collections::BTreeMap;
use std::time::Duration;

use agena_domain::{PermissionAction, PermissionDecision, ToolInvocation};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod code_search;
pub mod shell;
pub mod shell_analysis;
pub mod tool_search;

/// Rendering strategy for the built-in read tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReadMode {
    Text,
    Attachment,
    #[default]
    Auto,
}

/// Optional provider/model selection overrides for a delegated task.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TaskModelSelection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

/// One file-level change produced by the apply-patch tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedFileChange {
    pub path: String,
    pub kind: PatchOpKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_path: Option<String>,
}

/// Stable result metadata emitted after an apply-patch operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyPatchExecution {
    pub operation_id: String,
    pub files: Vec<AppliedFileChange>,
    pub before_hash: String,
    pub after_hash: String,
    pub inverse_patch: String,
    pub diff: String,
    pub progress: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOpKind {
    Add,
    Update,
    Delete,
    Move,
}

/// Model-facing builtin-tool availability profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinToolProfile {
    Full,
    ReadOnly,
    NoTask,
}

impl BuiltinToolProfile {
    pub fn infer(model_id: Option<&str>) -> Self {
        let Some(model_id) = model_id else {
            return Self::Full;
        };
        let lowered = model_id.to_ascii_lowercase();
        if lowered.contains("readonly") || lowered.contains("read_only") {
            return Self::ReadOnly;
        }
        if lowered.contains("no-task") || lowered.contains("chat") {
            return Self::NoTask;
        }
        Self::Full
    }
}

/// Snapshot backend selected by the concrete repository/snapshot adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotBackend {
    Rift,
    Git,
}

impl AsRef<str> for SnapshotBackend {
    fn as_ref(&self) -> &str {
        match self {
            Self::Rift => "rift",
            Self::Git => "git",
        }
    }
}

impl std::fmt::Display for SnapshotBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone)]
pub struct SnapshotBackendSupport {
    pub backend: SnapshotBackend,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct SnapshotBackendCapabilities {
    pub preferred_backend: Option<SnapshotBackend>,
    pub git: SnapshotBackendSupport,
    pub rift: SnapshotBackendSupport,
}

impl SnapshotBackendCapabilities {
    pub fn for_backend(&self, backend: SnapshotBackend) -> &SnapshotBackendSupport {
        match backend {
            SnapshotBackend::Rift => &self.rift,
            SnapshotBackend::Git => &self.git,
        }
    }
}

/// Presentation-neutral availability result for one builtin tool.
#[derive(Debug, Clone)]
pub struct ToolAvailability {
    pub tool_name: String,
    pub enabled: bool,
    pub reason: String,
}

/// Runtime-neutral summary of one completed tool execution.
///
/// Concrete executors may attach core transcript parts or file attachments;
/// those remain outside this contract. This value carries only the stable
/// textual presentation and metadata that a runtime/application boundary can
/// forward without depending on message or UI types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolExecutionSummary {
    pub title: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ToolAttachmentSummary>,
}

/// Attachment metadata that can cross the executor/runtime boundary without
/// carrying the plugin SDK's concrete attachment source type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolAttachmentSummary {
    pub kind: String,
    pub mime: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hint: Option<String>,
}

/// Stable model-facing summary for one scheduled cron job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobSummary {
    pub id: String,
    pub kind: String,
    pub expression: Option<String>,
    pub at: Option<String>,
    pub prompt: String,
    pub next_fire_at: Option<String>,
    pub last_fired_at: Option<String>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub misfire_policy: String,
    #[serde(default)]
    pub retry_max_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<String>,
    #[serde(default)]
    pub run_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_error: Option<String>,
}

/// Stable history entry emitted by `cron.history`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronRunSummary {
    pub job_id: String,
    pub triggered_at: String,
    pub finished_at: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_for: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// A permission decision attached to one tool access action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermissionCheck {
    pub action: PermissionAction,
    pub decision: PermissionDecision,
}

/// Invocation after tool lookup/presentation has prepared it for execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedToolInvocation {
    pub invocation: ToolInvocation,
    pub title_override: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

/// Shell invocation after path/working-directory preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedShellCommand {
    pub command: String,
    pub cwd: std::path::PathBuf,
}

/// Maximum-character policy applied by concrete tool-output truncators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolOutputTruncationPolicy {
    pub max_chars: usize,
}

impl Default for ToolOutputTruncationPolicy {
    fn default() -> Self {
        Self {
            max_chars: usize::MAX,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ShellRequest {
    pub command: Vec<String>,
    pub cwd: std::path::PathBuf,
    pub env: std::collections::HashMap<String, String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ShellOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub aggregated_output: String,
    pub duration: Duration,
    pub timed_out: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("command cancelled")]
    Cancelled,
    #[error("invalid shell request: {0}")]
    InvalidRequest(String),
    #[error("failed to spawn child process: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("failed to wait for child process: {0}")]
    Wait(#[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        BuiltinToolProfile, PreparedShellCommand, SnapshotBackend, SnapshotBackendCapabilities,
        SnapshotBackendSupport, ToolAttachmentSummary, ToolAvailability, ToolExecutionSummary,
        ToolOutputTruncationPolicy,
    };

    #[test]
    fn truncation_policy_defaults_to_unbounded_output() {
        assert_eq!(ToolOutputTruncationPolicy::default().max_chars, usize::MAX);
    }

    #[test]
    fn prepared_shell_command_keeps_command_and_directory() {
        let command = PreparedShellCommand {
            command: "echo ok".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
        };
        assert_eq!(command.command, "echo ok");
        assert_eq!(command.cwd, std::path::Path::new("/tmp"));
    }

    #[test]
    fn builtin_profile_inference_is_provider_independent() {
        assert_eq!(BuiltinToolProfile::infer(None), BuiltinToolProfile::Full);
        assert_eq!(
            BuiltinToolProfile::infer(Some("chat-readonly")),
            BuiltinToolProfile::ReadOnly
        );
        assert_eq!(
            BuiltinToolProfile::infer(Some("model-no-task")),
            BuiltinToolProfile::NoTask
        );
    }

    #[test]
    fn snapshot_capabilities_select_the_requested_backend() {
        let capabilities = SnapshotBackendCapabilities {
            preferred_backend: Some(SnapshotBackend::Rift),
            git: SnapshotBackendSupport {
                backend: SnapshotBackend::Git,
                available: false,
                detail: "missing git".to_owned(),
            },
            rift: SnapshotBackendSupport {
                backend: SnapshotBackend::Rift,
                available: true,
                detail: "ready".to_owned(),
            },
        };
        assert!(capabilities.for_backend(SnapshotBackend::Rift).available);
        assert!(!capabilities.for_backend(SnapshotBackend::Git).available);
    }

    #[test]
    fn availability_value_carries_only_presentation_neutral_fields() {
        let value = ToolAvailability {
            tool_name: "read".to_owned(),
            enabled: true,
            reason: "read-only profile".to_owned(),
        };
        assert!(value.enabled);
        assert_eq!(value.tool_name, "read");
    }

    #[test]
    fn execution_summary_round_trips_without_core_types() {
        let value = ToolExecutionSummary {
            title: "Read README".to_owned(),
            output_text: "content".to_owned(),
            payload: Some(serde_json::json!({"kind": "read"})),
            metadata: BTreeMap::from([(String::from("path"), String::from("README.md"))]),
            attachments: Vec::new(),
        };
        let json = serde_json::to_value(&value).expect("serialize execution summary");
        let decoded: ToolExecutionSummary =
            serde_json::from_value(json).expect("deserialize execution summary");
        assert_eq!(decoded, value);
    }

    #[test]
    fn execution_summary_accepts_legacy_payload_without_attachments() {
        let decoded: ToolExecutionSummary = serde_json::from_value(serde_json::json!({
            "title": "legacy",
            "output_text": "output"
        }))
        .expect("decode summary without optional attachment metadata");
        assert!(decoded.attachments.is_empty());
    }

    #[test]
    fn attachment_summary_has_a_stable_wire_shape() {
        let value = ToolAttachmentSummary {
            kind: "file".to_owned(),
            mime: "text/plain".to_owned(),
            label: "README.md".to_owned(),
            size_bytes: Some(12),
            source_hint: Some("README.md".to_owned()),
        };
        assert_eq!(
            serde_json::to_value(value).expect("serialize attachment summary"),
            serde_json::json!({
                "kind": "file",
                "mime": "text/plain",
                "label": "README.md",
                "size_bytes": 12,
                "source_hint": "README.md"
            })
        );
    }
}
