use std::collections::BTreeMap;

use agena_macros::ToolInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use agena_domain::{
    ArtifactRef, ExecutionStatus, FilesystemEffect, InteractionNotificationLevel, NetworkEffect,
    OperationError, ProcessShell, SearchResultItem, TableColumn, TodoItem, ToolInvocation,
    ToolManagedOutput, ToolOutput, ToolResultDisplay, ToolResultState, UserInputQuestion,
};
use agena_tool::{ReadMode, TaskModelSelection};

use super::{AttachmentItem, AttachmentKind, AttachmentSource};
use agena_domain::{StructuredValue, TimeRange};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim(
        "command",
        "description",
        "workdir",
        "filesystem_effects[].path",
        "network_effects[].target"
    ),
    non_empty("command")
)]
pub struct ShellCommandInput {
    pub command: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(path.read, fallback = "")]
    pub workdir: Option<String>,
    /// Filesystem paths the command may read or write. Pass an empty list only
    /// when the command has no filesystem effect beyond entering `workdir`.
    pub filesystem_effects: Vec<FilesystemEffect>,
    /// Outbound network targets the command may connect to. Pass an empty list
    /// when the command has no network effect.
    pub network_effects: Vec<NetworkEffect>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShellMonitorPatternKind {
    Literal,
    #[default]
    Regex,
}

/// Optional completion and capture policy for a managed shell process. Adding
/// this object makes `shell.run` a monitored background invocation and returns
/// the same `process_id` consumed by `shell.list`, `shell.logs` and `shell.stop`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("success_pattern", "failure_pattern", "include_pattern"),
    non_empty_if_present("success_pattern"),
    non_empty_if_present("failure_pattern"),
    non_empty_if_present("include_pattern"),
    minimum("timeout_ms", 1),
    maximum("timeout_ms", 3600000),
    minimum("quiet_period_ms", 1),
    maximum("quiet_period_ms", 3600000),
    minimum("max_buffered_lines", 1),
    maximum("max_buffered_lines", 10000)
)]
#[serde(deny_unknown_fields)]
pub struct ShellMonitorInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_pattern: Option<String>,
    #[serde(default)]
    pub pattern_kind: ShellMonitorPatternKind,
    /// Optional regex selecting which output lines are retained in the buffer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_pattern: Option<String>,
    /// Complete successfully after this many milliseconds without output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_period_ms: Option<u64>,
    /// Overall monitor timeout. Defaults to the command timeout, then five minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Keep running until explicit stop or natural exit, ignoring timeout and
    /// quiet-period completion. Pattern matches still terminate the monitor.
    #[serde(default)]
    pub persistent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_buffered_lines: Option<u32>,
    #[serde(default = "default_true")]
    pub capture_stderr: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
pub struct ReadToolInput {
    /// File or directory path to read. Relative paths are resolved from the
    /// workspace root.
    #[serde(alias = "path")]
    #[arg(trim, non_empty, path.read)]
    pub file_path: String,
    /// 1-based offset for file lines or directory entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    /// Maximum number of lines or directory entries to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// How to render the target: `text`, `attachment`, or `auto`.
    #[serde(default)]
    pub mode: ReadMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
pub struct GlobToolInput {
    /// Glob pattern to match.
    #[arg(trim, non_empty)]
    pub pattern: String,
    /// Optional base path. Defaults to the workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(trim, non_empty_if_present, path.read, fallback = "")]
    pub path: Option<String>,
    /// Number of matching paths to skip before returning results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    /// Maximum paths to return. Defaults to 200 and cannot exceed 1000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Include dependency, VCS, and build-output directories that are skipped
    /// by default (`.git`, `node_modules`, `target`, `dist`, and caches).
    #[serde(default)]
    pub include_ignored: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
pub struct GrepToolInput {
    /// Regex pattern to search for.
    #[arg(trim, non_empty)]
    pub pattern: String,
    /// Optional base path. Defaults to the workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(trim, non_empty_if_present, path.read, fallback = "")]
    pub path: Option<String>,
    /// Optional glob filter applied before matching lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(trim, non_empty_if_present)]
    pub include: Option<String>,
}

/// Input for the delegated `task` subagent command.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskAccess {
    #[default]
    Inherit,
    ReadOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("description", "prompt", "task_id"),
    non_empty("description", "prompt"),
    non_empty_if_present("task_id"),
    minimum("timeout_ms", 1),
    minimum("max_tokens", 1),
    minimum("max_cost_microusd", 1)
)]
#[serde(deny_unknown_fields)]
pub struct TaskToolInput {
    /// Short label for the subtask session.
    pub description: String,
    /// Full instruction payload for the delegated subtask.
    pub prompt: String,
    /// Hard capability boundary for this delegated Agena instance.
    #[serde(default)]
    pub access: TaskAccess,
    /// Resume an existing subtask session instead of creating a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Optional model and mode overrides. Explicit values take precedence over
    /// the parent session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<TaskModelSelection>,
    /// Overall task timeout. A timeout cancels the child execution and returns
    /// a structured `timed_out` task result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Cumulative child-completion token budget. This includes prompt,
    /// output, reasoning and cache token accounting reported by the route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Cumulative child-completion cost ceiling in USD micro-units (one
    /// millionth of a USD). Integer micro-units avoid a floating-point value
    /// becoming a durable budget boundary; for example, 250000 means $0.25.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_microusd: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("query"), non_empty("query"))]
#[serde(deny_unknown_fields)]
pub struct ToolSearchToolInput {
    /// Search text used to rank matching tool names and descriptions.
    #[serde(default)]
    pub query: String,
    /// Maximum number of search results to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Action discriminator for the internal shell-process tool payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim(
    "command",
    "description",
    "workdir",
    "filesystem_effects[].path",
    "network_effects[].target",
    "process_id"
))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ShellToolInput {
    /// Run one process. Set `background = true` to keep it attached to the session.
    #[input(non_empty("command"))]
    Run {
        #[serde(default)]
        shell: ProcessShell,
        #[serde(flatten)]
        command: Box<ShellCommandInput>,
        /// If true, keep the process attached to the session and return a process id.
        #[serde(default)]
        background: bool,
        /// Optional monitor conditions. When present, the invocation is always
        /// managed as a background process regardless of `background`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        monitor: Option<ShellMonitorInput>,
    },
    /// List every active or recently-finished background process in this session.
    #[input(default_when_empty = true)]
    List {},
    /// Read buffered logs from a background process; optionally block waiting for new events.
    #[input(non_empty("process_id"))]
    Logs {
        process_id: String,
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
    /// Terminate a running background process.
    #[input(non_empty("process_id"))]
    Stop { process_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim(
        "title",
        "body_markdown",
        "kind",
        "submit_label",
        "cancel_label",
        "questions[].id",
        "questions[].header",
        "questions[].question",
        "questions[].options[].label",
        "questions[].options[].description",
        "questions[].options[].preview_markdown"
    ),
    min_items("questions", 1),
    max_items("questions", 3),
    max_items("questions[].options", 8),
    max_chars("questions[].header", 12),
    max_chars("questions[].options[].preview_markdown", 16000),
    minimum("auto_resolution_ms", 60000),
    maximum("auto_resolution_ms", 600000),
    required_unless_present("questions[].allow_custom", "questions[].options"),
    non_empty("questions[].id", "questions[].question"),
    non_empty_if_present("questions[].options[].label"),
    distinct_trimmed("questions[].id"),
    distinct_trimmed_within("questions[].options[].label", "questions[]")
)]
pub struct AskUserToolInput {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_markdown: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub submit_label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cancel_label: String,
    /// Automatically continue without an answer after this many milliseconds.
    /// Values are limited to 60 seconds through 10 minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_resolution_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<UserInputQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("title", "body_markdown"),
    non_empty("body_markdown"),
    max_chars("title", 80),
    max_chars("body_markdown", 16000)
)]
pub struct InteractionNotifyToolInput {
    /// Short heading displayed in the transcript notification card.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Markdown notification body. This tool never waits for a reply.
    pub body_markdown: String,
    /// Visual severity used by the TUI notification card.
    #[serde(default)]
    pub level: InteractionNotificationLevel,
}

/// Textual patch payload in the agena patch format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
pub struct ApplyPatchToolInput {
    /// Unified patch text to apply to the workspace.
    pub patch: String,
}

/// Input for the built-in `web_fetch` tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("url", "prompt"), non_empty("url"))]
pub struct WebFetchToolInput {
    /// Absolute URL to fetch. HTTP is upgraded to HTTPS.
    pub url: String,
    /// Optional follow-up instruction used to prioritize the most relevant
    /// excerpts from the fetched page in the returned text output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// Input for the built-in `web_search` tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("query", "allowed_domains[]", "blocked_domains[]"),
    non_empty("query")
)]
pub struct WebSearchToolInput {
    /// Search query text.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default, ToolInput)]
#[input(
    trim("name", "path"),
    non_empty_if_present("name", "path"),
    conflicts_with("name", "path")
)]
pub struct EnterSnapshotToolInput {
    /// Optional name; when absent a slug is generated from the timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Path of an already-existing snapshot to enter. Mutually
    /// exclusive with `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("action"), non_empty("action"))]
pub struct ExitSnapshotToolInput {
    /// "keep" leaves the snapshot on disk; "remove" deletes it.
    pub action: String,
    /// Required `true` when `action = "remove"` and the snapshot has
    /// uncommitted changes / unpushed commits.
    #[serde(default)]
    pub discard_changes: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CronMisfirePolicyInput {
    Skip,
    #[default]
    RunOnceNow,
    Reschedule,
}

/// Bounded exponential retry settings for a cron delivery. `max_attempts`
/// includes the initial attempt, so the default permits two retries after the
/// normal delivery attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CronRetryPolicyInput {
    #[schemars(range(min = 1, max = 20))]
    pub max_attempts: u32,
    #[schemars(range(min = 1, max = 3600))]
    pub initial_delay_seconds: u32,
    #[schemars(range(min = 1, max = 86400))]
    pub max_delay_seconds: u32,
    #[schemars(range(min = 1, max = 10))]
    pub multiplier: u32,
}

impl Default for CronRetryPolicyInput {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay_seconds: 15,
            max_delay_seconds: 300,
            multiplier: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("expression", "prompt"), non_empty("expression", "prompt"))]
pub struct CronCreateToolInput {
    /// 6-field cron expression: `<sec> <min> <hour> <day-of-month> <month> <day-of-week>`.
    pub expression: String,
    /// Prompt to enqueue when the job fires.
    pub prompt: String,
    #[serde(default = "default_cron_max_age")]
    pub max_age_days: u32,
    /// What to do after a restart when this fire is materially overdue.
    #[serde(default)]
    pub misfire_policy: CronMisfirePolicyInput,
    #[serde(default)]
    pub retry_policy: CronRetryPolicyInput,
}

fn default_cron_max_age() -> u32 {
    7
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default, ToolInput)]
pub struct CronListToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("id"), non_empty("id"))]
pub struct CronDeleteToolInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("id", "prompt", "expression"), non_empty("id"))]
pub struct CronUpdateToolInput {
    pub id: String,
    /// Optional replacement prompt. At least one update field is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Optional replacement cron expression. Valid only for cron jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// Optional replacement retention period. Valid only for cron jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
    /// Optional replacement recovery policy. Valid only for cron jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub misfire_policy: Option<CronMisfirePolicyInput>,
    /// Optional replacement bounded retry policy. Valid only for cron jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<CronRetryPolicyInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("id"), non_empty("id"))]
pub struct CronJobControlToolInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(default, deny_unknown_fields)]
pub struct CronHistoryToolInput {
    /// Restrict history to one job. Omitting it returns newest records across
    /// all retained jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default = "default_cron_history_limit")]
    #[schemars(range(min = 1, max = 200))]
    pub limit: u32,
}

const fn default_cron_history_limit() -> u32 {
    50
}

impl Default for CronHistoryToolInput {
    fn default() -> Self {
        Self {
            id: None,
            limit: default_cron_history_limit(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("prompt", "reason"), non_empty("prompt"))]
pub struct ScheduleWakeupToolInput {
    pub delay_seconds: u32,
    pub prompt: String,
    /// Short reason logged for diagnostics / shown back to the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("file_path"), non_empty("file_path"))]
pub struct LspPositionToolInput {
    #[arg(path.read)]
    pub file_path: String,
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("file_path"), non_empty("file_path"))]
pub struct LspDefinitionToolInput {
    #[input(flatten_shape)]
    #[serde(flatten)]
    pub position: LspPositionToolInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("file_path"), non_empty("file_path"))]
pub struct LspReferencesToolInput {
    #[input(flatten_shape)]
    #[serde(flatten)]
    pub position: LspPositionToolInput,
    #[serde(default = "default_true")]
    pub include_declaration: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("file_path"), non_empty("file_path"))]
pub struct LspHoverToolInput {
    #[input(flatten_shape)]
    #[serde(flatten)]
    pub position: LspPositionToolInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("file_path"), non_empty("file_path"))]
pub struct LspDiagnosticsToolInput {
    #[arg(path.read)]
    pub file_path: String,
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
        changes: Vec<agena_domain::FileChangeRecord>,
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

/// Interpret an output payload's optional message-presentation blocks.
/// The output value itself belongs to `agena-domain`; only this conversion
/// depends on core's message `OperationBlock` representation.
pub fn tool_output_content_blocks(output: &agena_domain::ToolOutput) -> Vec<OperationBlock> {
    let Some(blocks) = output
        .payload
        .get("content_blocks")
        .and_then(StructuredValue::as_array)
    else {
        return Vec::new();
    };

    blocks
        .iter()
        .filter_map(|block| serde_json::from_value(serde_json::Value::from(block.clone())).ok())
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelVisibleOutput {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl ModelVisibleOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            attachments: Vec::new(),
            truncated: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.attachments.is_empty() && !self.truncated
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolResultEnvelope {
    #[serde(default, skip_serializing_if = "ToolResultState::is_pending")]
    pub state: ToolResultState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<OperationBlock>,
    #[serde(default, skip_serializing_if = "ModelVisibleOutput::is_empty")]
    pub model_preview: ModelVisibleOutput,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_outputs: Vec<ToolManagedOutput>,
    #[serde(default, skip_serializing_if = "ToolResultDisplay::is_empty")]
    pub display: ToolResultDisplay,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationError>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

impl ToolResultEnvelope {
    pub fn is_empty(&self) -> bool {
        ToolResultState::is_pending(&self.state)
            && self.structured.is_none()
            && self.content.is_empty()
            && self.model_preview.is_empty()
            && self.managed_outputs.is_empty()
            && self.display.is_empty()
            && self.attachments.is_empty()
            && self.error.is_none()
            && self.metadata.is_empty()
            && self.raw.is_none()
    }

    pub fn completed(
        output_text: String,
        blocks: Vec<OperationBlock>,
        attachments: Vec<AttachmentItem>,
        details: &ToolOutput,
    ) -> Self {
        let truncated = details.is_model_truncated();
        Self {
            state: ToolResultState::Completed,
            structured: details.to_json_payload(),
            content: blocks,
            model_preview: ModelVisibleOutput {
                text: output_text.clone(),
                attachments: attachments.clone(),
                truncated,
            },
            managed_outputs: details.managed_outputs.clone(),
            display: ToolResultDisplay {
                title: String::new(),
                summary: output_text,
            },
            attachments,
            error: None,
            metadata: BTreeMap::new(),
            raw: None,
        }
    }

    pub fn failed(
        failure: agena_failure::Failure,
        blocks: Vec<OperationBlock>,
        attachments: Vec<AttachmentItem>,
        details: &ToolOutput,
    ) -> Self {
        let truncated = details.is_model_truncated();
        let user_summary = failure.user.fallback.clone();
        let model_output = failure
            .model
            .as_ref()
            .map(|feedback| feedback.message())
            .unwrap_or_else(|| {
                "The tool failed because of an internal system error. Try an alternative approach."
                    .to_owned()
            });
        Self {
            state: ToolResultState::Failed,
            structured: details.to_json_payload(),
            content: blocks,
            model_preview: ModelVisibleOutput {
                text: model_output,
                attachments: attachments.clone(),
                truncated,
            },
            managed_outputs: details.managed_outputs.clone(),
            display: ToolResultDisplay {
                title: String::new(),
                summary: user_summary,
            },
            attachments,
            error: Some(OperationError { failure }),
            metadata: BTreeMap::new(),
            raw: None,
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
    #[serde(default, skip_serializing_if = "ToolResultEnvelope::is_empty")]
    pub result: ToolResultEnvelope,
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

const PROVIDER_ONLY_METADATA_KEY: &str = "provider_only";
const LEGACY_PROVIDER_NATIVE_ONLY_METADATA_KEY: &str = "provider_native_only";
const ADVERTISED_TOOL_IDENTITY_METADATA_KEY: &str = "advertised_tool_identity";

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
            result: ToolResultEnvelope::default(),
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
        let truncated = details.is_model_truncated();
        let result = ToolResultEnvelope::completed(
            output_text.clone(),
            blocks.clone(),
            attachments.clone(),
            &details,
        );
        Self {
            call_id,
            invocation,
            title: String::new(),
            summary: output_text.clone(),
            model_output: ModelVisibleOutput {
                text: output_text,
                attachments: attachments.clone(),
                truncated,
            },
            blocks,
            artifacts: Vec::new(),
            attachments,
            details,
            result,
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
        failure: agena_failure::Failure,
        blocks: Vec<OperationBlock>,
        attachments: Vec<AttachmentItem>,
        details: ToolOutput,
        lifecycle: TimeRange,
    ) -> Self {
        let structured = details.to_json_payload();
        let truncated = details.is_model_truncated();
        let result = ToolResultEnvelope::failed(
            failure.clone(),
            blocks.clone(),
            attachments.clone(),
            &details,
        );
        let user_summary = failure.user.fallback.clone();
        let model_output = failure
            .model
            .as_ref()
            .map(|feedback| feedback.message())
            .unwrap_or_else(|| {
                "The tool failed because of an internal system error. Try an alternative approach."
                    .to_owned()
            });
        Self {
            call_id,
            invocation,
            title: String::new(),
            summary: user_summary,
            model_output: ModelVisibleOutput {
                text: model_output,
                attachments: attachments.clone(),
                truncated,
            },
            blocks,
            artifacts: Vec::new(),
            attachments,
            details,
            result,
            structured,
            metadata: BTreeMap::new(),
            error: Some(OperationError { failure }),
            raw: None,
            lifecycle,
        }
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
        self.result.display.title = self.title.clone();
    }

    pub fn set_provider_only(&mut self, value: bool) {
        if value {
            self.metadata.insert(
                PROVIDER_ONLY_METADATA_KEY.to_string(),
                serde_json::Value::Bool(true),
            );
            self.metadata
                .remove(LEGACY_PROVIDER_NATIVE_ONLY_METADATA_KEY);
        } else {
            self.metadata.remove(PROVIDER_ONLY_METADATA_KEY);
            self.metadata
                .remove(LEGACY_PROVIDER_NATIVE_ONLY_METADATA_KEY);
        }
    }

    pub fn is_provider_only(&self) -> bool {
        self.metadata
            .get(PROVIDER_ONLY_METADATA_KEY)
            .or_else(|| self.metadata.get(LEGACY_PROVIDER_NATIVE_ONLY_METADATA_KEY))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    pub fn set_advertised_tool_identity(&mut self, identity: impl Into<String>) {
        self.metadata.insert(
            ADVERTISED_TOOL_IDENTITY_METADATA_KEY.to_string(),
            serde_json::Value::String(identity.into()),
        );
    }

    pub fn advertised_tool_identity(&self) -> Option<&str> {
        self.metadata
            .get(ADVERTISED_TOOL_IDENTITY_METADATA_KEY)
            .and_then(serde_json::Value::as_str)
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
        if !self.result.model_preview.text.is_empty() {
            Some(self.result.model_preview.text.as_str())
        } else {
            (!self.model_output.text.is_empty()).then_some(self.model_output.text.as_str())
        }
    }

    pub fn title(&self) -> Option<&str> {
        (!self.title.is_empty()).then_some(self.title.as_str())
    }

    pub fn error_message(&self) -> Option<&str> {
        self.result
            .error
            .as_ref()
            .or(self.error.as_ref())
            .map(OperationError::user_message)
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
        self.result.state = ToolResultState::Running;
        self.result.model_preview.text.push_str(delta);
        if self.summary.is_empty() {
            self.summary.push_str(delta);
        }
        if self.result.display.summary.is_empty() {
            self.result.display.summary.push_str(delta);
        }
        if let Some(OperationBlock::Text { text }) = self.blocks.last_mut() {
            text.push_str(delta);
        } else {
            self.blocks.push(OperationBlock::Text {
                text: delta.to_string(),
            });
        }
        if let Some(OperationBlock::Text { text }) = self.result.content.last_mut() {
            text.push_str(delta);
        } else {
            self.result.content.push(OperationBlock::Text {
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
