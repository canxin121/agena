//! # agena-tool
//!
//! Provider-independent tool contracts.
//!
//! Concrete executors, built-in tools, plugin hosts, and permission policy
//! implementations belong in adapter/runtime crates rather than this crate.
//!
//! ## What lives here
//!
//! - **Tool descriptors** — normalization helpers ([`normalize_tool_title`],
//!   [`normalize_tool_summary`], [`compose_tool_title`]) and
//!   [`invocation_call_summary`].
//! - **Execution contracts** — [`PreparedToolInvocation`],
//!   [`ToolPermissionCheck`], [`ToolExecutionSummary`], [`ToolRuntimeEvent`],
//!   and the runtime event sink.
//! - **Shell** — [`shell`] provides [`ShellRequest`] / [`ShellOutput`] and
//!   [`ShellError`]; [`shell_analysis`] analyzes command shapes.
//! - **Search** — [`code_search`] and [`tool_search`] locate code and tools.
//! - **Value types** — [`ReadMode`], [`SnapshotBackend`],
//!   [`ToolAvailability`], patch operations, and cron summaries.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

pub use agena_domain::ToolPresentationSection;
pub mod tool_activity;
use agena_domain::{
    CommandBeginEvent, CommandEndEvent, CommandOutputDeltaEvent, PermissionAction,
    PermissionDecision, ToolInvocation, ToolPermissionContract,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub use tool_activity::{
    RenderContext, RenderError, ToolActivityEvent, ToolActivityResult, ToolHumanRenderer,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Durable Operation titles are compact scan labels, not result previews.
/// Detailed output belongs in `summary`, sections, and the expanded Activity.
pub const TOOL_TITLE_MAX_DISPLAY_WIDTH: usize = 96;

/// Durable Operation summaries are compact result statements. They are
/// intentionally much smaller than model-visible output so transcript clients
/// can render them without loading or inspecting the full result.
pub const TOOL_SUMMARY_MAX_DISPLAY_WIDTH: usize = 120;

/// Normalize a tool-provided title at the shared runtime boundary.
///
/// Whitespace is collapsed so streamed or plugin titles cannot accidentally
/// become multi-line transcript content. Titles that are genuinely long are
/// bounded by terminal display width and retain an ellipsis; ordinary titles
/// remain readable without a fixed, prematurely small UI cutoff.
pub fn normalize_tool_title(title: impl AsRef<str>) -> String {
    normalize_tool_presentation_line(title, TOOL_TITLE_MAX_DISPLAY_WIDTH)
}

/// Normalize a tool-provided result summary at the shared runtime boundary.
///
/// Tools remain responsible for the summary's meaning. This function only
/// enforces the one-line, bounded storage contract; it never derives a summary
/// from `output_text`.
pub fn normalize_tool_summary(summary: impl AsRef<str>) -> String {
    normalize_tool_presentation_line(summary, TOOL_SUMMARY_MAX_DISPLAY_WIDTH)
}

/// Compose the durable Operation title from the execution-tool name and the
/// concise, call-specific summary produced for this invocation
/// ("fs.read · Read README.md", "tools.search · Search tools · filesystem").
/// The bare tool name is returned when no summary is available, and the
/// composed value is bounded by the title contract.
pub fn compose_tool_title(tool_name: impl AsRef<str>, summary: impl AsRef<str>) -> String {
    let tool_name = tool_name.as_ref().trim();
    let summary = summary.as_ref().trim();
    if summary.is_empty() || summary == tool_name {
        return normalize_tool_title(tool_name);
    }
    if tool_name.is_empty() {
        return normalize_tool_title(summary);
    }
    normalize_tool_title(format!("{tool_name} · {summary}"))
}

/// Pick the single most informative string argument of a tool invocation to
/// use as a call-start title summary ("README.md", "cargo test", "filesystem").
/// Returns an empty string when the input carries no obvious subject so the
/// caller can fall back to the bare tool name.
pub fn invocation_call_summary(input: &serde_json::Value) -> String {
    const PREFERRED_KEYS: &[&str] = &[
        "tool",
        "command",
        "description",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "title",
        "expression",
        "notebook_path",
        "process_id",
        "task_id",
        "function",
        "model",
        "id",
        "name",
    ];
    for key in PREFERRED_KEYS {
        if let Some(value) = input.get(*key).and_then(serde_json::Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }
    // Provider-native envelopes nest the real target under `input`.
    if let Some(inner) = input.get("input").and_then(serde_json::Value::as_object)
        && let Some(value) = inner.get("tool").and_then(serde_json::Value::as_str)
    {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    String::new()
}

fn normalize_tool_presentation_line(value: impl AsRef<str>, max_width: usize) -> String {
    let normalized = value
        .as_ref()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if UnicodeWidthStr::width(normalized.as_str()) <= max_width {
        return normalized;
    }

    let content_width = max_width.saturating_sub(1);
    let mut width = 0_usize;
    let mut bounded = String::new();
    for grapheme in normalized.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width.saturating_add(grapheme_width) > content_width {
            break;
        }
        bounded.push_str(grapheme);
        width = width.saturating_add(grapheme_width);
    }
    bounded = bounded.trim_end().to_owned();
    bounded.push('…');
    bounded
}

#[cfg(test)]
mod tool_title_tests {
    use super::{
        TOOL_SUMMARY_MAX_DISPLAY_WIDTH, TOOL_TITLE_MAX_DISPLAY_WIDTH, compose_tool_title,
        invocation_call_summary, normalize_tool_summary, normalize_tool_title,
    };
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn composed_titles_join_the_tool_name_and_call_summary() {
        assert_eq!(
            compose_tool_title("fs.read", "Read README.md"),
            "fs.read · Read README.md"
        );
        assert_eq!(
            compose_tool_title("tools.search", "Search tools · filesystem"),
            "tools.search · Search tools · filesystem"
        );
    }

    #[test]
    fn composed_titles_fall_back_to_the_bare_tool_name() {
        assert_eq!(compose_tool_title("shell.run", ""), "shell.run");
        assert_eq!(compose_tool_title("shell.run", "   "), "shell.run");
        assert_eq!(compose_tool_title("fs.read", "fs.read"), "fs.read");
        assert_eq!(compose_tool_title("", ""), "");
    }

    #[test]
    fn invocation_call_summary_prefers_the_most_informative_argument() {
        assert_eq!(
            invocation_call_summary(&serde_json::json!({"path": "README.md"})),
            "README.md"
        );
        assert_eq!(
            invocation_call_summary(&serde_json::json!({"command": "cargo test", "timeout_ms": 5})),
            "cargo test"
        );
        assert_eq!(
            invocation_call_summary(&serde_json::json!({"query": "filesystem"})),
            "filesystem"
        );
        assert_eq!(
            invocation_call_summary(
                &serde_json::json!({"tool": "fs.write", "input": {"path": "notes.txt"}})
            ),
            "fs.write"
        );
        assert_eq!(invocation_call_summary(&serde_json::json!({})), "");
    }

    #[test]
    fn normal_titles_are_preserved_and_whitespace_is_collapsed() {
        assert_eq!(
            normalize_tool_title("  Read   crates/agena-domain/src/activity.rs  "),
            "Read crates/agena-domain/src/activity.rs"
        );
    }

    #[test]
    fn genuinely_long_titles_are_width_bounded_with_an_ellipsis() {
        let title = format!("Inspect {}", "很长的标题".repeat(20));
        let bounded = normalize_tool_title(title);

        assert!(bounded.ends_with('…'));
        assert!(UnicodeWidthStr::width(bounded.as_str()) <= TOOL_TITLE_MAX_DISPLAY_WIDTH);
    }

    #[test]
    fn summaries_are_single_line_and_defensively_bounded() {
        let summary = format!("  42 matches\n{}  ", "in many files ".repeat(20));
        let bounded = normalize_tool_summary(summary);

        assert!(bounded.starts_with("42 matches in many files"));
        assert!(bounded.ends_with('…'));
        assert!(UnicodeWidthStr::width(bounded.as_str()) <= TOOL_SUMMARY_MAX_DISPLAY_WIDTH);
    }
}

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
/// Kind of a patch operation.
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
/// Whether a snapshot backend is available and why.
pub struct SnapshotBackendSupport {
    pub backend: SnapshotBackend,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
/// Capabilities of the available snapshot backends.
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
    pub summary: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<ToolPresentationSection>,
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
    pub last_run_failure: Option<agena_failure::UserProblem>,
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
    pub failure: Option<agena_failure::UserProblem>,
}

/// A permission decision attached to one tool access action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermissionCheck {
    pub action: PermissionAction,
    pub decision: PermissionDecision,
    /// The tool's full permission contract: shell/interactive/read_only/task
    /// flags plus declared path/network specs. The decision pipeline reads
    /// these directly; never tool tags (tags are metadata for discovery/UI).
    pub contract: ToolPermissionContract,
}

impl ToolPermissionCheck {
    /// Whether the contract is path-scoped (declares concrete path specs).
    pub fn is_path_scoped(&self) -> bool {
        !self.contract.input_paths.is_empty() || !self.contract.path_access.is_empty()
    }
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
    /// Fully resolved environment after `shell.env` and `command.before`
    /// hooks. Carrying it with the prepared command prevents execution from
    /// re-entering synchronous plugin hooks on a blocking worker.
    pub env: std::collections::HashMap<String, String>,
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
/// Request to execute a shell command.
pub struct ShellRequest {
    pub command: Vec<String>,
    pub cwd: std::path::PathBuf,
    pub env: std::collections::HashMap<String, String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
/// Output of a shell command execution.
pub struct ShellOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub aggregated_output: String,
    pub duration: Duration,
    pub timed_out: bool,
}

/// Runtime-side presentation signals emitted while a process-backed tool is
/// running. The session runtime owns delivery (and decides whether the
/// signals are ephemeral or durable); this crate only exposes a small callback
/// contract so stdout/stderr can be observed without waiting for the child
/// process to exit.
#[derive(Debug, Clone)]
pub enum ToolRuntimeEvent {
    CommandBegin(CommandBeginEvent),
    CommandOutputDelta(CommandOutputDeltaEvent),
    CommandEnd(CommandEndEvent),
}

/// Sink receiving tool runtime events.
pub type ToolRuntimeEventSink = Arc<dyn Fn(ToolRuntimeEvent) + Send + Sync>;

#[derive(Debug, thiserror::Error)]
/// Error from shell command execution.
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
        ToolOutputTruncationPolicy, ToolPresentationSection,
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
            env: std::collections::HashMap::new(),
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
            summary: "README.md · 1 file".to_owned(),
            output_text: "content".to_owned(),
            sections: vec![ToolPresentationSection {
                title: "Result".to_owned(),
                text: "content".to_owned(),
            }],
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
    fn execution_summary_requires_the_result_summary_contract() {
        let error = serde_json::from_value::<ToolExecutionSummary>(serde_json::json!({
            "title": "legacy",
            "output_text": "output"
        }))
        .expect_err("summary is a required execution-result field");
        assert!(error.to_string().contains("summary"));
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
