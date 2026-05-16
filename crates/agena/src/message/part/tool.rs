use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::Display;

use super::{
    AttachmentItem, AttachmentKind, AttachmentSource, ExecutionStatus, StructuredObject,
    StructuredValue, TimeRange, TodoItem, UserInputQuestion,
};

pub type ToolAttachment = AttachmentItem;

/// Filesystem access mode declared by a tool invocation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FilesystemAccess {
    Read,
    Write,
    ReadWrite,
}

impl FilesystemAccess {
    pub const fn includes_read(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite)
    }

    pub const fn includes_write(self) -> bool {
        matches!(self, Self::Write | Self::ReadWrite)
    }
}

/// One path a command may read, write, or both.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct FilesystemEffect {
    /// File or directory path affected by the command. For shell tools,
    /// relative paths are resolved from the command working directory.
    pub path: String,
    pub access: FilesystemAccess,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct BashToolInput {
    pub command: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    /// Filesystem paths the command may read or write. Pass an empty list only
    /// when the command has no filesystem effect beyond entering `workdir`.
    pub filesystem_effects: Vec<FilesystemEffect>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ReadToolInput {
    pub file_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ViewFileToolInput {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct GlobToolInput {
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct GrepToolInput {
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TaskSubagentType {
    Explore,
    Implement,
    Verify,
}

impl TaskSubagentType {
    pub const fn guidance(self) -> &'static str {
        match self {
            Self::Explore => {
                "Focus on understanding the codebase, collecting evidence, and reporting findings without making edits."
            }
            Self::Implement => {
                "Own the requested code changes, adapt to concurrent edits, and avoid reverting unrelated work."
            }
            Self::Verify => {
                "Validate behavior with targeted checks, look for regressions, and summarize remaining risks."
            }
        }
    }

    pub fn apply_prompt_guidance(self, prompt: &str) -> String {
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            format!("Profile guidance: {}", self.guidance())
        } else {
            format!(
                "Profile guidance: {}\n\nDelegated task:\n{}",
                self.guidance(),
                trimmed
            )
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TaskToolInput {
    pub description: String,
    pub prompt: String,
    pub subagent_type: TaskSubagentType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ToolSearchToolInput {
    #[serde(default)]
    pub query: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TodoWriteToolInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<TodoItem>,
}

/// Stream channel reported by the `monitor` tool for each captured event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MonitorStream {
    Stdout,
    Stderr,
}

/// Lifecycle state of a registered monitor.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MonitorStatus {
    /// The child process is running and the buffer is being filled.
    Running,
    /// The child exited normally (use `exit_code` for the precise code).
    Exited,
    /// The runner aborted the child after exceeding `timeout_ms`.
    TimedOut,
    /// `stop` action terminated the child.
    Stopped,
    /// The child failed to start or crashed before producing output.
    Failed,
}

/// One captured event line from a monitored process.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct MonitorEvent {
    /// Monotonic sequence number scoped to a single monitor (starts at 1).
    pub seq: u64,
    pub stream: MonitorStream,
    /// Wall-clock timestamp in milliseconds since the Unix epoch.
    pub ts_ms: i64,
    /// Captured line, without the trailing newline. Lossy UTF-8 if needed.
    pub line: String,
}

/// Action discriminator for the `monitor` tool payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MonitorToolInput {
    /// Spawn a new background monitor and return its id.
    Start {
        /// Shell command to execute. Runs under `/bin/sh -lc` (or `cmd /c` on Windows).
        command: String,
        /// One-line human-readable description (shown in events / metadata).
        #[serde(default)]
        description: String,
        /// Optional working directory; defaults to workspace root.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workdir: Option<String>,
        /// Filesystem paths the command may read or write. Relative paths are
        /// resolved from the monitor working directory.
        filesystem_effects: Vec<FilesystemEffect>,
        /// Auto-kill after this many ms when not persistent. Default 300000, max 3_600_000.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        /// If true, the monitor runs until explicitly stopped or the session ends.
        #[serde(default)]
        persistent: bool,
        /// Optional regex applied to each line; only matching lines are kept.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        include_pattern: Option<String>,
        /// Ring buffer size in lines (default 1000, max 10_000).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_buffered_lines: Option<u32>,
        /// Whether stderr lines are captured. Default true.
        #[serde(default = "default_capture_stderr")]
        capture_stderr: bool,
    },
    /// List every active or recently-finished monitor in this session.
    List {},
    /// Read events from a monitor; optionally block waiting for new events.
    Read {
        monitor_id: String,
        /// Return only events with `seq > since_seq`. Default 0.
        #[serde(default)]
        since_seq: u64,
        /// Max events to return. Default 200, max 2000.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
        /// If positive, block up to this many ms waiting for new events when none are available.
        #[serde(default)]
        wait_ms: u64,
    },
    /// Terminate a running monitor.
    Stop { monitor_id: String },
}

fn default_capture_stderr() -> bool {
    true
}

/// Lightweight summary record returned by `list` / embedded inside other outputs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitorSummary {
    pub monitor_id: String,
    pub command: String,
    pub description: String,
    pub status: MonitorStatus,
    pub persistent: bool,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<i64>,
    pub buffered_lines: u32,
    pub last_seq: u64,
    pub dropped_lines: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct AskUserToolInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<UserInputQuestion>,
}

pub type RequestUserInputToolInput = AskUserToolInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ApplyPatchToolInput {
    pub patch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct WebFetchToolInput {
    /// Absolute URL to fetch.  HTTP is upgraded to HTTPS.
    pub url: String,
    /// Optional follow-up instruction; when present, the fetched markdown
    /// is summarized by the session's default LLM provider before being
    /// returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct WebSearchToolInput {
    pub query: String,
    /// Restrict results to these domains; empty means no restriction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_domains: Vec<String>,
    /// Drop results from these domains.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
pub struct EnterPlanModeToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
pub struct ExitPlanModeToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
pub struct WorkflowPromptToolInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
pub struct GetGoalToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
pub struct ClearGoalToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct CreateGoalToolInput {
    pub objective: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpdateGoalStatus {
    Active,
    Paused,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct UpdateGoalToolInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    pub status: UpdateGoalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
pub struct EnterWorktreeToolInput {
    /// Optional name; when absent a slug is generated from the timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Path of an already-existing worktree to enter.  Mutually
    /// exclusive with `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ExitWorktreeToolInput {
    /// "keep" leaves the worktree on disk; "remove" deletes it.
    pub action: String,
    /// Required `true` when `action = "remove"` and the worktree has
    /// uncommitted changes / unpushed commits.
    #[serde(default)]
    pub discard_changes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct CronCreateToolInput {
    /// 6-field cron expression: `<sec> <min> <hour> <day-of-month> <month> <day-of-week>`.
    pub expression: String,
    /// Prompt to enqueue when the job fires.
    pub prompt: String,
    #[serde(default = "default_cron_max_age")]
    pub max_age_days: u32,
}

fn default_cron_max_age() -> u32 {
    7
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
pub struct CronListToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct CronDeleteToolInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ScheduleWakeupToolInput {
    pub delay_seconds: u32,
    pub prompt: String,
    /// Short reason logged for telemetry / shown back to the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct LspDefinitionToolInput {
    pub file_path: String,
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct LspReferencesToolInput {
    pub file_path: String,
    pub line: u32,
    pub character: u32,
    #[serde(default = "default_true")]
    pub include_declaration: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct LspHoverToolInput {
    pub file_path: String,
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct LspDiagnosticsToolInput {
    pub file_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotebookEditMode {
    Replace,
    Insert,
    Delete,
}

impl NotebookEditMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Insert => "insert",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotebookCellType {
    Code,
    Markdown,
}

impl NotebookCellType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Markdown => "markdown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct NotebookEditToolInput {
    pub notebook_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_number: Option<u32>,
    #[serde(default)]
    pub new_source: String,
    pub edit_mode: NotebookEditMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_type: Option<NotebookCellType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct PowerShellToolInput {
    pub command: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    /// Filesystem paths the command may read or write. Pass an empty list only
    /// when the command has no filesystem effect beyond entering `workdir`.
    pub filesystem_effects: Vec<FilesystemEffect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInvocation {
    pub entry_name: String,
    pub input: StructuredObject,
}

impl PluginInvocation {
    pub fn from_tool_invocation(invocation: &ToolInvocation) -> Self {
        Self {
            entry_name: invocation.name.clone(),
            input: invocation.input.clone(),
        }
    }
}

/// A dynamic tool invocation: stable name + structured payload. Shipped tools
/// and user/plugin-supplied tools share this shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolInvocation {
    pub name: String,
    #[serde(default)]
    pub input: StructuredObject,
}

impl ToolInvocation {
    pub fn new(name: impl Into<String>, input: StructuredObject) -> Self {
        Self {
            name: name.into(),
            input,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolResultBlock {
    Text {
        text: String,
    },
    Image {
        mime: String,
        url: String,
    },
    Audio {
        mime: String,
        url: String,
    },
    ResourceLink {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    EmbeddedResource {
        uri: String,
        mime: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base64: Option<String>,
    },
    File {
        url: String,
        filename: String,
        mime: String,
    },
}

impl ToolResultBlock {
    pub fn to_attachment_item(&self) -> Option<AttachmentItem> {
        match self {
            Self::Text { .. } => None,
            Self::Image { mime, url } | Self::Audio { mime, url } => Some(AttachmentItem {
                kind: AttachmentKind::detect(mime.as_str(), Some(url.as_str())),
                mime: mime.clone(),
                source: attachment_source_from_location(url.as_str())?,
                filename: filename_hint(url.as_str()),
                title: None,
                size_bytes: None,
                sha256: None,
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            }),
            Self::ResourceLink { uri, title } => Some(AttachmentItem {
                kind: AttachmentKind::detect("", Some(uri.as_str())),
                mime: String::new(),
                source: attachment_source_from_location(uri.as_str())?,
                filename: filename_hint(uri.as_str()),
                title: title.clone(),
                size_bytes: None,
                sha256: None,
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            }),
            Self::EmbeddedResource {
                uri, mime, base64, ..
            } => {
                let source = if let Some(base64) = base64.as_ref() {
                    AttachmentSource::Base64 {
                        data: base64.clone(),
                    }
                } else {
                    attachment_source_from_location(uri.as_str())?
                };

                Some(AttachmentItem {
                    kind: AttachmentKind::detect(mime.as_str(), Some(uri.as_str())),
                    mime: mime.clone(),
                    source,
                    filename: filename_hint(uri.as_str()),
                    title: None,
                    size_bytes: None,
                    sha256: None,
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: None,
                })
            }
            Self::File {
                url,
                filename,
                mime,
            } => Some(AttachmentItem {
                kind: AttachmentKind::detect(mime.as_str(), Some(filename.as_str())),
                mime: mime.clone(),
                source: attachment_source_from_location(url.as_str())?,
                filename: non_empty(filename.as_str()),
                title: None,
                size_bytes: None,
                sha256: None,
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolOutput {
    #[serde(default, skip_serializing_if = "StructuredObject::is_empty")]
    pub payload: StructuredObject,
}

impl ToolOutput {
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }

    pub fn from_json_payload(payload: Option<&serde_json::Value>) -> Result<Self, String> {
        match payload {
            None | Some(serde_json::Value::Null) => Ok(Self::default()),
            Some(value) => Ok(Self {
                payload: StructuredObject::try_from(value.clone())?,
            }),
        }
    }

    pub fn to_json_payload(&self) -> Option<serde_json::Value> {
        (!self.payload.is_empty()).then(|| serde_json::Value::from(self.payload.clone()))
    }

    pub fn content_blocks(&self) -> Vec<ToolResultBlock> {
        let Some(blocks) = self
            .payload
            .get("content_blocks")
            .and_then(StructuredValue::as_array)
        else {
            return Vec::new();
        };

        blocks
            .iter()
            .filter_map(|block| {
                let value = serde_json::Value::from(block.clone());
                serde_json::from_value(value).ok()
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ToolExecutionPart {
    Pending {
        call_id: i64,
        invocation: ToolInvocation,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        #[serde(default)]
        lifecycle: TimeRange,
    },
    InProgress {
        call_id: i64,
        invocation: ToolInvocation,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        output_text: String,
        #[serde(default)]
        lifecycle: TimeRange,
    },
    Completed {
        call_id: i64,
        invocation: ToolInvocation,
        #[serde(default)]
        output_text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<ToolResultBlock>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<ToolAttachment>,
        #[serde(default, skip_serializing_if = "ToolOutput::is_empty")]
        details: ToolOutput,
        #[serde(default)]
        lifecycle: TimeRange,
    },
    Failed {
        call_id: i64,
        invocation: ToolInvocation,
        error_message: String,
        #[serde(default)]
        output_text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<ToolResultBlock>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<ToolAttachment>,
        #[serde(default, skip_serializing_if = "ToolOutput::is_empty")]
        details: ToolOutput,
        #[serde(default)]
        lifecycle: TimeRange,
    },
}

impl ToolExecutionPart {
    pub const fn status(&self) -> ExecutionStatus {
        match self {
            Self::Pending { .. } => ExecutionStatus::Pending,
            Self::InProgress { .. } => ExecutionStatus::InProgress,
            Self::Completed { .. } => ExecutionStatus::Completed,
            Self::Failed { .. } => ExecutionStatus::Failed,
        }
    }

    pub fn call_id(&self) -> i64 {
        match self {
            Self::Pending { call_id, .. }
            | Self::InProgress { call_id, .. }
            | Self::Completed { call_id, .. }
            | Self::Failed { call_id, .. } => *call_id,
        }
    }

    pub fn invocation(&self) -> &ToolInvocation {
        match self {
            Self::Pending { invocation, .. }
            | Self::InProgress { invocation, .. }
            | Self::Completed { invocation, .. }
            | Self::Failed { invocation, .. } => invocation,
        }
    }

    pub fn lifecycle(&self) -> &TimeRange {
        match self {
            Self::Pending { lifecycle, .. }
            | Self::InProgress { lifecycle, .. }
            | Self::Completed { lifecycle, .. }
            | Self::Failed { lifecycle, .. } => lifecycle,
        }
    }

    pub fn lifecycle_mut(&mut self) -> &mut TimeRange {
        match self {
            Self::Pending { lifecycle, .. }
            | Self::InProgress { lifecycle, .. }
            | Self::Completed { lifecycle, .. }
            | Self::Failed { lifecycle, .. } => lifecycle,
        }
    }

    /// `Pending` carries no output yet; the other states all expose an
    /// `output_text` accumulator.
    pub fn output_text(&self) -> Option<&str> {
        match self {
            Self::Pending { .. } => None,
            Self::InProgress { output_text, .. }
            | Self::Completed { output_text, .. }
            | Self::Failed { output_text, .. } => Some(output_text.as_str()),
        }
    }

    /// `Pending` and `InProgress` have no `title` of their own — they share
    /// the title field; `Completed` / `Failed` drop it.
    pub fn title(&self) -> Option<&str> {
        match self {
            Self::Pending { title, .. } | Self::InProgress { title, .. } => Some(title.as_str()),
            Self::Completed { .. } | Self::Failed { .. } => None,
        }
    }

    /// Only `Failed` carries an error message.
    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Failed { error_message, .. } => Some(error_message.as_str()),
            _ => None,
        }
    }

    pub fn append_output_delta(&mut self, delta: &str) -> bool {
        match self {
            Self::Pending {
                call_id,
                invocation,
                title,
                lifecycle,
            } => {
                *self = Self::InProgress {
                    call_id: *call_id,
                    invocation: invocation.clone(),
                    title: title.clone(),
                    output_text: delta.to_string(),
                    lifecycle: lifecycle.clone(),
                };
                true
            }
            Self::InProgress { output_text, .. }
            | Self::Completed { output_text, .. }
            | Self::Failed { output_text, .. } => {
                output_text.push_str(delta);
                true
            }
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn filename_hint(value: &str) -> Option<String> {
    value
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn attachment_source_from_location(value: &str) -> Option<AttachmentSource> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with("data:") {
        return Some(AttachmentSource::DataUrl {
            url: trimmed.to_owned(),
        });
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Some(AttachmentSource::Url {
            url: trimmed.to_owned(),
        });
    }

    if trimmed.starts_with("file://")
        || trimmed.starts_with('/')
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
    {
        return Some(AttachmentSource::LocalPath {
            path: trimmed.to_owned(),
        });
    }

    Some(AttachmentSource::Url {
        url: trimmed.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn task_subagent_type_guidance_wraps_prompt() {
        let prompt = TaskSubagentType::Verify.apply_prompt_guidance("run focused checks");

        assert!(prompt.contains("Validate behavior with targeted checks"));
        assert!(prompt.contains("run focused checks"));
    }

    #[test]
    fn task_subagent_type_serializes_as_snake_case() {
        let value = serde_json::to_string(&TaskSubagentType::Implement)
            .expect("task subagent type should serialize");
        assert_eq!(value, "\"implement\"");
    }

    #[test]
    fn tool_result_block_converts_resource_links_into_attachments() {
        let block = ToolResultBlock::ResourceLink {
            uri: "https://example.com/report.pdf".to_string(),
            title: Some("report".to_string()),
        };

        let attachment = block
            .to_attachment_item()
            .expect("resource link should become attachment");
        assert_eq!(attachment.kind, AttachmentKind::Pdf);
        assert_eq!(attachment.title.as_deref(), Some("report"));
        assert_eq!(
            attachment.source,
            AttachmentSource::Url {
                url: "https://example.com/report.pdf".to_string(),
            }
        );
    }

    #[test]
    fn tool_attachment_aliases_attachment_item_shape() {
        let attachment = ToolAttachment {
            kind: AttachmentKind::Image,
            mime: "image/png".to_string(),
            source: AttachmentSource::Url {
                url: "https://example.com/image.png".to_string(),
            },
            filename: Some("image.png".to_string()),
            title: None,
            size_bytes: Some(16),
            sha256: None,
            width: Some(2),
            height: Some(3),
            duration_ms: None,
            page_count: None,
        };

        assert_eq!(attachment.kind, AttachmentKind::Image);
        assert_eq!(attachment.filename.as_deref(), Some("image.png"));
        assert_eq!(attachment.width, Some(2));
        assert_eq!(attachment.height, Some(3));
    }

    #[test]
    fn empty_tool_output_serializes_without_details() {
        assert_eq!(
            serde_json::to_value(ToolOutput::default()).expect("tool output should serialize"),
            json!({})
        );

        let part = ToolExecutionPart::Completed {
            call_id: 7,
            invocation: ToolInvocation::new("plugin.example", StructuredObject::default()),
            output_text: String::new(),
            blocks: Vec::new(),
            attachments: Vec::new(),
            details: ToolOutput::default(),
            lifecycle: TimeRange::default(),
        };
        let serialized = serde_json::to_value(part).expect("tool execution part should serialize");

        assert_eq!(serialized["state"], "completed");
        assert!(serialized.get("details").is_none());
    }

    #[test]
    fn tool_output_round_trips_generic_payload_and_blocks() {
        let payload = json!({
            "content_blocks": [
                {
                    "type": "text",
                    "text": "done"
                },
                {
                    "type": "resource_link",
                    "uri": "https://example.com/report.pdf",
                    "title": "report"
                }
            ],
            "metadata": {
                "plugin": "example"
            },
            "count": 2
        });

        let output = ToolOutput::from_json_payload(Some(&payload))
            .expect("generic object payload should decode");

        assert_eq!(output.to_json_payload(), Some(payload));
        assert_eq!(
            output.content_blocks(),
            vec![
                ToolResultBlock::Text {
                    text: "done".to_string()
                },
                ToolResultBlock::ResourceLink {
                    uri: "https://example.com/report.pdf".to_string(),
                    title: Some("report".to_string())
                }
            ]
        );
    }

    #[test]
    fn tool_output_rejects_source_tagged_shapes() {
        let err = serde_json::from_value::<ToolOutput>(json!({
            "source": "custom",
            "output": {
                "name": "plugin.example",
                "payload": {
                    "text": "hello"
                }
            }
        }))
        .expect_err("source-tagged output should not deserialize as the neutral model");

        assert!(err.to_string().contains("unknown field"));
    }
}
