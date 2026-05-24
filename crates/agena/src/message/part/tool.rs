use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::Display;

use super::{
    AttachmentItem, AttachmentKind, AttachmentSource, ExecutionStatus, StructuredObject,
    StructuredValue, TimeRange, TodoItem, UserInputQuestion,
};

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

/// One outbound network target a command may access.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct NetworkEffect {
    /// Absolute URL or `host[:port]` target. Shell tools must declare every
    /// remote endpoint they may connect to; pass an empty list when the
    /// command has no network effect.
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ShellCommandInput {
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
    /// Outbound network targets the command may connect to. Pass an empty list
    /// when the command has no network effect.
    pub network_effects: Vec<NetworkEffect>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ReadToolInput {
    pub file_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default)]
    pub mode: ReadMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReadMode {
    Text,
    Attachment,
    #[default]
    Auto,
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
#[serde(deny_unknown_fields)]
pub struct ToolSearchToolInput {
    #[serde(default)]
    pub query: String,
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
        #[serde(flatten)]
        command: ShellCommandInput,
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
pub struct AgentSwitchToolInput {
    /// Target agent profile. Omit or pass an empty string to clear the
    /// explicit runtime agent selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Push the current agent so `agent_restore` can return to it later.
    #[serde(default)]
    pub push_previous: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
pub struct AgentRestoreToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
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
    /// Short reason logged for diagnostics / shown back to the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct LspPositionToolInput {
    pub file_path: String,
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct LspDefinitionToolInput {
    #[serde(flatten)]
    pub position: LspPositionToolInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct LspReferencesToolInput {
    #[serde(flatten)]
    pub position: LspPositionToolInput,
    #[serde(default = "default_true")]
    pub include_declaration: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct LspHoverToolInput {
    #[serde(flatten)]
    pub position: LspPositionToolInput,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInvocation {
    pub entry_name: String,
    pub plugin_name: Option<String>,
    pub input: StructuredObject,
}

impl PluginInvocation {
    pub fn from_tool_invocation(invocation: &ToolInvocation) -> Self {
        Self {
            entry_name: invocation.name.clone(),
            plugin_name: invocation.plugin_name.clone(),
            input: invocation.input.clone(),
        }
    }
}

/// A dynamic tool invocation: stable name + structured payload. Shipped tools
/// and user/plugin-supplied tools share this shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolInvocation {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
    #[serde(default)]
    pub input: StructuredObject,
}

impl ToolInvocation {
    pub fn new(name: impl Into<String>, input: StructuredObject) -> Self {
        Self {
            name: name.into(),
            plugin_name: None,
            input,
        }
    }

    pub fn with_plugin_name(
        name: impl Into<String>,
        plugin_name: impl Into<String>,
        input: StructuredObject,
    ) -> Self {
        Self {
            name: name.into(),
            plugin_name: Some(plugin_name.into()),
            input,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OperationBlock {
    Text {
        text: String,
    },
    Markdown {
        text: String,
    },
    Json {
        value: serde_json::Value,
    },
    Table {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        columns: Vec<TableColumn>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rows: Vec<Vec<serde_json::Value>>,
    },
    Log {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stream: Option<String>,
        text: String,
    },
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stdout: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stderr: Option<String>,
    },
    Diff {
        diff: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    FileChanges {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        changes: Vec<super::FileChangeEntry>,
    },
    SearchResults {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        results: Vec<SearchResultItem>,
    },
    Citation {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snippet: Option<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
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
    Media {
        mime_type: String,
        artifact: ArtifactRef,
    },
    Checklist {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<TodoItem>,
    },
    NestedTask {
        task_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        status: ExecutionStatus,
    },
    Progress {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        percent: Option<f32>,
    },
    Custom {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        value: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableColumn {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResultItem {
    pub title: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRef {
    pub uri: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl OperationBlock {
    pub fn to_attachment_item(&self) -> Option<AttachmentItem> {
        match self {
            Self::Text { .. }
            | Self::Markdown { .. }
            | Self::Json { .. }
            | Self::Table { .. }
            | Self::Log { .. }
            | Self::Command { .. }
            | Self::Diff { .. }
            | Self::FileChanges { .. }
            | Self::SearchResults { .. }
            | Self::Citation { .. }
            | Self::Checklist { .. }
            | Self::NestedTask { .. }
            | Self::Progress { .. }
            | Self::Custom { .. } => None,
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
            Self::ResourceLink {
                uri,
                title,
                mime_type,
            } => Some(AttachmentItem {
                kind: AttachmentKind::detect(
                    mime_type.as_deref().unwrap_or(""),
                    Some(uri.as_str()),
                ),
                mime: mime_type.clone().unwrap_or_default(),
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
            Self::Media {
                mime_type,
                artifact,
            } => Some(AttachmentItem {
                kind: AttachmentKind::detect(mime_type.as_str(), artifact.name.as_deref()),
                mime: mime_type.clone(),
                source: attachment_source_from_location(artifact.uri.as_str())?,
                filename: artifact
                    .name
                    .clone()
                    .or_else(|| filename_hint(artifact.uri.as_str())),
                title: artifact.name.clone(),
                size_bytes: artifact.size_bytes,
                sha256: artifact.sha256.clone(),
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            }),
        }
    }

    pub fn text_value(&self) -> Option<&str> {
        match self {
            Self::Text { text }
            | Self::Markdown { text }
            | Self::Log { text, .. }
            | Self::Diff { diff: text, .. } => Some(text.as_str()),
            _ => None,
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

    pub fn content_blocks(&self) -> Vec<OperationBlock> {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelVisibleOutput {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

impl ModelVisibleOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            attachments: Vec::new(),
            truncated: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationPart {
    pub call_id: i64,
    pub invocation: ToolInvocation,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default)]
    pub model_output: ModelVisibleOutput,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<OperationBlock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
    #[serde(default, skip_serializing_if = "ToolOutput::is_empty")]
    pub details: ToolOutput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
    #[serde(default)]
    pub lifecycle: TimeRange,
}

impl OperationPart {
    pub fn pending(
        call_id: i64,
        invocation: ToolInvocation,
        title: impl Into<String>,
        lifecycle: TimeRange,
    ) -> Self {
        Self {
            call_id,
            invocation,
            title: title.into(),
            summary: String::new(),
            model_output: ModelVisibleOutput::default(),
            blocks: Vec::new(),
            artifacts: Vec::new(),
            attachments: Vec::new(),
            details: ToolOutput::default(),
            structured: None,
            metadata: BTreeMap::new(),
            error: None,
            raw: None,
            lifecycle,
        }
    }

    pub fn completed(
        call_id: i64,
        invocation: ToolInvocation,
        output_text: impl Into<String>,
        blocks: Vec<OperationBlock>,
        attachments: Vec<AttachmentItem>,
        details: ToolOutput,
        lifecycle: TimeRange,
    ) -> Self {
        let output_text = output_text.into();
        let structured = details.to_json_payload();
        Self {
            call_id,
            invocation,
            title: String::new(),
            summary: output_text.clone(),
            model_output: ModelVisibleOutput {
                text: output_text,
                attachments: attachments.clone(),
                truncated: None,
            },
            blocks,
            artifacts: Vec::new(),
            attachments,
            details,
            structured,
            metadata: BTreeMap::new(),
            error: None,
            raw: None,
            lifecycle,
        }
    }

    pub fn failed(
        call_id: i64,
        invocation: ToolInvocation,
        error_message: impl Into<String>,
        output_text: impl Into<String>,
        blocks: Vec<OperationBlock>,
        attachments: Vec<AttachmentItem>,
        details: ToolOutput,
        lifecycle: TimeRange,
    ) -> Self {
        let error_message = error_message.into();
        let output_text = output_text.into();
        let structured = details.to_json_payload();
        Self {
            call_id,
            invocation,
            title: String::new(),
            summary: if error_message.trim().is_empty() {
                output_text.clone()
            } else {
                error_message.clone()
            },
            model_output: ModelVisibleOutput {
                text: output_text,
                attachments: attachments.clone(),
                truncated: None,
            },
            blocks,
            artifacts: Vec::new(),
            attachments,
            details,
            structured,
            metadata: BTreeMap::new(),
            error: Some(OperationError {
                message: error_message,
                code: None,
            }),
            raw: None,
            lifecycle,
        }
    }

    pub fn call_id(&self) -> i64 {
        self.call_id
    }

    pub fn invocation(&self) -> &ToolInvocation {
        &self.invocation
    }

    pub fn lifecycle(&self) -> &TimeRange {
        &self.lifecycle
    }

    pub fn lifecycle_mut(&mut self) -> &mut TimeRange {
        &mut self.lifecycle
    }

    pub fn output_text(&self) -> Option<&str> {
        (!self.model_output.text.is_empty()).then_some(self.model_output.text.as_str())
    }

    pub fn title(&self) -> Option<&str> {
        (!self.title.is_empty()).then_some(self.title.as_str())
    }

    pub fn error_message(&self) -> Option<&str> {
        self.error.as_ref().map(|error| error.message.as_str())
    }

    pub fn status(&self) -> ExecutionStatus {
        if self.error.is_some() {
            ExecutionStatus::Failed
        } else if self.lifecycle.end_ms.is_some() {
            ExecutionStatus::Completed
        } else if self.model_output.text.trim().is_empty() {
            ExecutionStatus::Pending
        } else {
            ExecutionStatus::InProgress
        }
    }

    pub fn append_output_delta(&mut self, delta: &str) -> bool {
        self.model_output.text.push_str(delta);
        if self.summary.is_empty() {
            self.summary.push_str(delta);
        }
        if let Some(OperationBlock::Text { text }) = self.blocks.last_mut() {
            text.push_str(delta);
        } else {
            self.blocks.push(OperationBlock::Text {
                text: delta.to_string(),
            });
        }
        true
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
