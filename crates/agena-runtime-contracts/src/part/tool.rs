use std::collections::BTreeMap;

use agena_macros::ToolInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use agena_domain::{
    ExecutionStatus, FilesystemEffects, InteractionNotificationLevel, OperationAuthorization,
    OperationError, OperationUserInput, ProcessShell, RawOutput, ToolInvocation, ToolResultState,
    UserInputQuestion,
};
use agena_tool::{ReadMode, TaskModelSelection};

use agena_domain::TimeRange;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim(
        "command",
        "description",
        "workdir",
        "reads[]",
        "writes[]",
        "network[]"
    ),
    non_empty("command")
)]
#[serde(deny_unknown_fields)]
/// Input of a shell command execution.
pub struct ShellCommandInput {
    pub command: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(path.read, fallback = "")]
    pub workdir: Option<String>,
    /// Files and directories the command may read. Declare only the actual
    /// files/directories affected - never the executables, interpreters, or
    /// tools being invoked (e.g. `node`, `python`, `uv`, `git`, `cargo`) or
    /// their installation directories. Pass an empty array `[]` when the
    /// command reads nothing beyond its executables.
    #[serde(default)]
    #[schemars(example = example_reads())]
    pub reads: Vec<String>,
    /// Files and directories the command may create, modify, or delete.
    /// Declare only the actual files/directories affected - never the
    /// executables, interpreters, or tools being invoked (e.g. `node`,
    /// `python`, `uv`, `git`, `cargo`) or their installation directories.
    /// Pass an empty array `[]` when the command writes nothing.
    #[serde(default)]
    #[schemars(example = example_writes())]
    pub writes: Vec<String>,
    /// Outbound network targets the command may connect to: host names,
    /// `host:port`, or URLs. Pass an empty array `[]` when the command has no
    /// network effect.
    #[serde(default)]
    #[schemars(example = example_network())]
    pub network: Vec<String>,
}

impl ShellCommandInput {
    /// Project the flattened `reads`/`writes` declarations back into the
    /// internal grouped `FilesystemEffects` shape consumed by permission checks.
    pub fn filesystem_effects(&self) -> FilesystemEffects {
        FilesystemEffects {
            read: self.reads.clone(),
            write: self.writes.clone(),
        }
    }
}

fn example_reads() -> Vec<String> {
    vec!["src/lib.rs".to_string()]
}

fn example_writes() -> Vec<String> {
    vec!["target/out.txt".to_string()]
}

fn example_network() -> Vec<String> {
    vec!["<target>".to_string()]
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
/// How a shell monitor pattern is matched.
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
/// Patterns that determine shell command success or failure.
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
/// Input of the file read tool.
pub struct ReadToolInput {
    /// File or directory path to read. Relative paths are resolved from the
    /// workspace root.
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
/// Input of the glob tool.
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
/// Input of the grep tool.
pub struct GrepToolInput {
    /// Regex pattern to search for.
    #[arg(trim, non_empty)]
    pub pattern: String,
    /// Optional target: a directory to search recursively, or a single file.
    /// Defaults to the workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(trim, non_empty_if_present, path.read, fallback = "")]
    pub path: Option<String>,
    /// Optional glob filter applied before matching lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(trim, non_empty_if_present)]
    pub include: Option<String>,
    /// Include hidden and ignored files that are skipped by default according
    /// to ripgrep-compatible ignore rules.
    #[serde(default)]
    pub include_ignored: bool,
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
/// Input of the task tool.
pub struct TaskToolInput {
    /// Short label for the subtask session.
    pub description: String,
    /// Full instruction payload for the delegated subtask.
    pub prompt: String,
    /// Hard capability boundary for this delegated Agena instance.
    #[serde(default)]
    pub access: TaskAccess,
    /// Run the subtask in the background (default false). When false (default)
    /// the subtask runs inline and this call returns its final result before
    /// the tool call returns. When true, the tool returns immediately with a
    /// task id and the result is delivered as a `system_notification` when the
    /// subtask settles — do not poll tasks.get/tasks.output waiting for it.
    #[serde(default)]
    pub run_in_background: bool,
    /// Optional Skill names or aliases to attach to the delegated subtask's
    /// first user message as immutable Skill references. The child session
    /// receives the resolved Skill instructions as task guidance and should
    /// apply them while completing the task. Use skills appropriate to the
    /// task: for example a read-only review task can attach a review/read-only
    /// skill, an exploration task can attach an explore skill. Unknown names
    /// or aliases are rejected before the subtask starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
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
/// Input of the tool search tool.
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
    "reads[]",
    "writes[]",
    "network[]",
    "process_id"
))]
#[serde(tag = "action", rename_all = "snake_case")]
/// Input of the shell tool.
pub enum ShellToolInput {
    /// Run one process. Set `run_in_background = true` to keep it attached to the session.
    #[input(non_empty("command"))]
    Run {
        #[serde(default)]
        shell: ProcessShell,
        #[serde(flatten)]
        command: Box<ShellCommandInput>,
        /// If true, keep the process attached to the session and return a process id.
        #[serde(default, rename = "run_in_background")]
        run_in_background: bool,
        /// Optional monitor conditions. When present, the invocation is always
        /// managed as a background process regardless of `run_in_background`.
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

/// WebSocket endpoint monitored by the monitor tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("url"), non_empty("url"))]
pub struct MonitorWsInput {
    pub url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protocols: Vec<String>,
}

/// Input of the monitor tool: watch a command's output or a WebSocket feed,
/// emitting each event to the model as a `system_notification` part appended to
/// the launching run (everything-is-a-part: the monitor is its `tool_call`
/// part, every event is a `system_notification` part).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MonitorToolInput {
    /// Start a background monitor. Pass exactly one of `command` or `ws`.
    #[input(exactly_one_of("command", "ws"), trim("command", "description"))]
    Start {
        /// Shell command whose stdout/stderr lines become events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        /// WebSocket feed whose text frames become events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ws: Option<MonitorWsInput>,
        /// Optional timeout in ms; the monitor is killed when it elapses.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        /// Keep the monitor across model turns (default true).
        #[serde(default = "default_true")]
        persistent: bool,
        /// Human-readable description shown in the transcript and activity panel.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        description: String,
    },
    /// Stop a running monitor (kills its command / closes its WebSocket).
    #[input(non_empty("monitor_id"))]
    Stop { monitor_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim(
        "title",
        "kind",
        "body_markdown",
        "questions[].header",
        "questions[].question",
        "questions[].options[].label",
        "questions[].options[].description"
    ),
    min_items("questions", 1),
    max_items("questions", 3),
    max_items("questions[].options", 8),
    max_chars("questions[].header", 12),
    max_chars("body_markdown", 16000),
    minimum("auto_resolution_ms", 60000),
    maximum("auto_resolution_ms", 600000),
    required_unless_present("questions[].allow_custom", "questions[].options"),
    non_empty("questions[].question"),
    non_empty_if_present("questions[].options[].label"),
    distinct_trimmed_within("questions[].options[].label", "questions[]")
)]
/// Input of the ask-user tool.
pub struct AskUserToolInput {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// Optional Markdown body shown in the review dialog. Only the plan
    /// approval review (`kind == "review"`) sets it to the full plan document;
    /// other ask_user requests leave it empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_markdown: String,
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
/// Input of the interaction notify tool.
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

/// Textual patch payload in the agena patch format. Must start with the exact
/// marker line `*** Begin Patch` and end with the exact marker line
/// `*** End Patch`; use `*** Update File:` / `*** Add File:` / `*** Delete File:`
/// directives with `@@` hunks (context lines start with a space, removed lines
/// with `-`, added lines with `+`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(max_chars("patch", 16777216))]
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
/// Input of the web search tool.
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
/// Input of the enter-snapshot tool.
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
/// Input of the exit-snapshot tool.
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
/// Policy applied when a scheduled job misses its fire time.
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
#[input(
    trim("expression", "prompt", "timezone"),
    non_empty("expression", "prompt", "timezone")
)]
/// Input of the cron create tool.
pub struct CronCreateToolInput {
    /// 6-field cron expression: `<sec> <min> <hour> <day-of-month> <month> <day-of-week>`.
    pub expression: String,
    /// Prompt to enqueue when the job fires.
    pub prompt: String,
    /// IANA timezone in which the cron expression is evaluated (for example
    /// `Asia/Shanghai`). This is required so local wall-clock requests cannot
    /// be silently interpreted as UTC.
    pub timezone: String,
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
/// Input of the cron list tool.
pub struct CronListToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("id"), non_empty("id"))]
/// Input of the cron delete tool.
pub struct CronDeleteToolInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("id", "prompt", "expression"), non_empty("id"))]
/// Input of the cron update tool.
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
/// Input of the cron job control tool.
pub struct CronJobControlToolInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(default, deny_unknown_fields)]
/// Input of the cron history tool.
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
#[input(trim("file_path"), non_empty("file_path"))]
/// Input of an LSP position query.
pub struct LspPositionToolInput {
    #[arg(path.read)]
    pub file_path: String,
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("file_path"), non_empty("file_path"))]
/// Input of the LSP definition tool.
pub struct LspDefinitionToolInput {
    #[input(flatten_shape)]
    #[serde(flatten)]
    pub position: LspPositionToolInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("file_path"), non_empty("file_path"))]
/// Input of the LSP references tool.
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
/// Input of the LSP hover tool.
pub struct LspHoverToolInput {
    #[input(flatten_shape)]
    #[serde(flatten)]
    pub position: LspPositionToolInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("file_path"), non_empty("file_path"))]
/// Input of the LSP diagnostics tool.
pub struct LspDiagnosticsToolInput {
    #[arg(path.read)]
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A message part representing a tool operation.
///
/// The durable record stores invocation identity and exactly one raw result.
/// Model and human presentations are runtime projections and never fields on
/// this type.
pub struct OperationPart {
    pub call_id: i64,
    pub invocation: ToolInvocation,
    #[serde(default, skip_serializing_if = "OperationAuthorization::is_empty")]
    pub authorization: OperationAuthorization,
    #[serde(default, skip_serializing_if = "OperationUserInput::is_empty")]
    pub user_input: OperationUserInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<RawOutput>,
    #[serde(default)]
    pub state: ToolResultState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationError>,
    /// Invocation and runtime-control metadata. Raw result metadata belongs
    /// inside `output.metadata` so result facts have one storage location.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub lifecycle: TimeRange,
}

const PROVIDER_ONLY_METADATA_KEY: &str = "provider_only";
const ADVERTISED_TOOL_IDENTITY_METADATA_KEY: &str = "advertised_tool_identity";
const PROVIDER_RAW_METADATA_KEY: &str = "agena.provider_raw";
/// Marker that an operation was launched into the background (a monitored
/// shell process or a delegated task) and must keep rendering as in-progress
/// on the transcript part until the background work actually finishes. The
/// value is a serialized [`BackgroundOperation`]; the session layer stamps it
/// at tool-success time, and the runtime's completion bridge reads it back to
/// terminalize the part when the process/task settles.
pub(crate) const BACKGROUND_OPERATION_METADATA_KEY: &str = "agena.background";
/// Durable claim that a background operation's completion has already been
/// notified to the model. Set on the launching tool part's operation metadata
/// once the notification run is committed, so a re-delivered completion signal
/// (e.g. a repeated `SessionMetaUpdated`) is a no-op — the agena analog of
/// Claude Code's atomic `notified` claim (`I4e`).
pub const NOTIFIED_METADATA_KEY: &str = "agena.notified";

/// Which background work a launched-but-unfinished operation corresponds to,
/// used to correlate the transcript part with the completion signal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BackgroundOperation {
    pub kind: String,
    pub id: String,
}

impl OperationPart {
    pub fn pending(call_id: i64, invocation: ToolInvocation, lifecycle: TimeRange) -> Self {
        Self {
            call_id,
            invocation,
            authorization: OperationAuthorization::default(),
            user_input: OperationUserInput::default(),
            output: None,
            state: ToolResultState::Pending,
            error: None,
            metadata: BTreeMap::new(),
            lifecycle,
        }
    }

    pub fn completed(
        call_id: i64,
        invocation: ToolInvocation,
        output: RawOutput,
        lifecycle: TimeRange,
    ) -> Self {
        Self {
            call_id,
            invocation,
            authorization: OperationAuthorization::default(),
            user_input: OperationUserInput::default(),
            output: (!output.is_empty()).then_some(output),
            state: ToolResultState::Completed,
            error: None,
            metadata: BTreeMap::new(),
            lifecycle,
        }
    }

    pub fn failed(
        call_id: i64,
        invocation: ToolInvocation,
        failure: agena_failure::Failure,
        output: RawOutput,
        lifecycle: TimeRange,
    ) -> Self {
        Self {
            call_id,
            invocation,
            authorization: OperationAuthorization::default(),
            user_input: OperationUserInput::default(),
            output: (!output.is_empty()).then_some(output),
            state: ToolResultState::Failed,
            error: Some(OperationError { failure }),
            metadata: BTreeMap::new(),
            lifecycle,
        }
    }

    pub fn policy_denied(
        call_id: i64,
        invocation: ToolInvocation,
        output: RawOutput,
        lifecycle: TimeRange,
    ) -> Self {
        Self::non_execution(
            call_id,
            invocation,
            ToolResultState::PolicyDenied,
            output,
            lifecycle,
        )
    }

    pub fn user_declined(
        call_id: i64,
        invocation: ToolInvocation,
        output: RawOutput,
        lifecycle: TimeRange,
    ) -> Self {
        Self::non_execution(
            call_id,
            invocation,
            ToolResultState::UserDeclined,
            output,
            lifecycle,
        )
    }

    pub fn capability_unavailable(
        call_id: i64,
        invocation: ToolInvocation,
        output: RawOutput,
        lifecycle: TimeRange,
    ) -> Self {
        Self::non_execution(
            call_id,
            invocation,
            ToolResultState::CapabilityUnavailable,
            output,
            lifecycle,
        )
    }

    pub fn tool_unavailable(
        call_id: i64,
        invocation: ToolInvocation,
        output: RawOutput,
        lifecycle: TimeRange,
    ) -> Self {
        Self::non_execution(
            call_id,
            invocation,
            ToolResultState::ToolUnavailable,
            output,
            lifecycle,
        )
    }

    fn non_execution(
        call_id: i64,
        invocation: ToolInvocation,
        state: ToolResultState,
        output: RawOutput,
        lifecycle: TimeRange,
    ) -> Self {
        Self {
            call_id,
            invocation,
            authorization: OperationAuthorization::default(),
            user_input: OperationUserInput::default(),
            output: (!output.is_empty()).then_some(output),
            state,
            error: None,
            metadata: BTreeMap::new(),
            lifecycle,
        }
    }

    pub fn set_provider_only(&mut self, value: bool) {
        if value {
            self.metadata.insert(
                PROVIDER_ONLY_METADATA_KEY.to_string(),
                serde_json::Value::Bool(true),
            );
        } else {
            self.metadata.remove(PROVIDER_ONLY_METADATA_KEY);
        }
    }

    pub fn is_provider_only(&self) -> bool {
        self.metadata
            .get(PROVIDER_ONLY_METADATA_KEY)
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

    /// Stamp the background-work correlation marker on this operation. The
    /// storage part state stays `InProgress` (so the transcript keeps showing
    /// the spinner) while `status()` still reports terminal via the lifecycle
    /// end — the provider pairing and the running spinner are independent.
    pub fn set_background_operation(&mut self, background: &BackgroundOperation) {
        match serde_json::to_value(background) {
            Ok(value) => {
                self.metadata
                    .insert(BACKGROUND_OPERATION_METADATA_KEY.to_string(), value);
            }
            Err(error) => tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "serialize a background-operation marker",
                    &error,
                ),
                "background-operation marker was not persisted"
            ),
        }
    }

    /// The background-work correlation marker, when this operation was
    /// launched into the background and has not been terminalized yet.
    pub fn background_operation(&self) -> Option<BackgroundOperation> {
        let value = self.metadata.get(BACKGROUND_OPERATION_METADATA_KEY)?;
        match serde_json::from_value::<BackgroundOperation>(value.clone()) {
            Ok(operation) => Some(operation),
            Err(error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "decode a persisted background-operation marker",
                        &error,
                    ),
                    "persisted background-operation marker is malformed"
                );
                None
            }
        }
    }

    /// Atomically claim that this operation's completion has been notified to
    /// the model (see [`NOTIFIED_METADATA_KEY`]).
    pub fn set_notified(&mut self) {
        self.metadata.insert(
            NOTIFIED_METADATA_KEY.to_string(),
            serde_json::Value::Bool(true),
        );
    }

    /// Whether this operation's completion has already been notified.
    pub fn is_notified(&self) -> bool {
        self.metadata
            .get(NOTIFIED_METADATA_KEY)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
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

    pub fn raw_output(&self) -> Option<&RawOutput> {
        self.output.as_ref()
    }

    pub fn raw_output_mut(&mut self) -> &mut RawOutput {
        self.output.get_or_insert_with(RawOutput::default)
    }

    pub fn set_provider_raw(&mut self, raw: Option<serde_json::Value>) {
        let output = self.raw_output_mut();
        match raw {
            Some(raw) => {
                output
                    .metadata
                    .insert(PROVIDER_RAW_METADATA_KEY.to_owned(), raw);
            }
            None => {
                output.metadata.remove(PROVIDER_RAW_METADATA_KEY);
            }
        }
        if output.is_empty() {
            self.output = None;
        }
    }

    pub fn provider_raw(&self) -> Option<&serde_json::Value> {
        self.output
            .as_ref()?
            .metadata
            .get(PROVIDER_RAW_METADATA_KEY)
    }

    /// Best-effort model-visible text carved from the single payload. The
    /// authoritative per-tool model projection lives in the provider layer;
    /// this accessor only serves lossy fallbacks (text summaries, plain-text
    /// transcripts).
    pub fn output_text(&self) -> Option<&str> {
        let output = self.output.as_ref()?;
        if !output.text.is_empty() {
            return Some(output.text.as_str());
        }
        output.payload.as_ref().and_then(|payload| {
            payload
                .as_str()
                .or_else(|| payload.get("text").and_then(serde_json::Value::as_str))
        })
    }

    pub fn title(&self) -> Option<&str> {
        (!self.invocation.name.is_empty()).then_some(self.invocation.name.as_str())
    }

    pub fn error_message(&self) -> Option<&str> {
        self.error.as_ref().map(OperationError::user_message)
    }

    pub fn status(&self) -> ExecutionStatus {
        if self.state == ToolResultState::PolicyDenied {
            ExecutionStatus::PolicyDenied
        } else if self.state == ToolResultState::UserDeclined {
            ExecutionStatus::UserDeclined
        } else if self.state == ToolResultState::CapabilityUnavailable {
            ExecutionStatus::CapabilityUnavailable
        } else if self.state == ToolResultState::ToolUnavailable {
            ExecutionStatus::ToolUnavailable
        } else if self.state == ToolResultState::Failed {
            ExecutionStatus::Failed
        } else if self.state == ToolResultState::Cancelled {
            ExecutionStatus::Cancelled
        } else if self.error.is_some() {
            ExecutionStatus::Failed
        } else if self.state == ToolResultState::Completed {
            ExecutionStatus::Completed
        } else if self.state == ToolResultState::Running {
            ExecutionStatus::InProgress
        } else if self.lifecycle.end_ms.is_some() {
            ExecutionStatus::Completed
        } else {
            ExecutionStatus::Pending
        }
    }
}

#[cfg(test)]
mod operation_part_tests {
    use super::OperationPart;
    use agena_domain::{StructuredObject, TimeRange, ToolInvocation, ToolResultState};

    fn operation() -> OperationPart {
        OperationPart::pending(
            5,
            ToolInvocation::new("fs.read", StructuredObject::default()),
            TimeRange {
                start_ms: 100,
                end_ms: Some(200),
            },
        )
    }

    #[test]
    fn operation_round_trips_single_source_payload() {
        let mut op = operation();
        op.output = Some(agena_domain::RawOutput {
            payload: Some(serde_json::json!({"preview": "hello", "text": "hello"})),
            ..agena_domain::RawOutput::default()
        });
        op.state = ToolResultState::Completed;
        op.lifecycle.end_ms = Some(300);
        let value = serde_json::to_value(&op).unwrap();
        let back: OperationPart = serde_json::from_value(value).unwrap();
        assert_eq!(back, op);
        assert_eq!(back.output_text(), Some("hello"));
        assert_eq!(back.status(), super::ExecutionStatus::Completed);
    }

    #[test]
    fn operation_status_maps_non_execution_states() {
        let denied = OperationPart::policy_denied(
            1,
            ToolInvocation::new("fs.read", StructuredObject::default()),
            agena_domain::RawOutput::text("denied"),
            TimeRange::default(),
        );
        assert_eq!(denied.status(), super::ExecutionStatus::PolicyDenied);

        let failed = OperationPart::failed(
            2,
            ToolInvocation::new("fs.read", StructuredObject::default()),
            agena_failure::Failure::new(
                agena_failure::FailureCode::new("tool.internal"),
                agena_failure::FailureCategory::Internal,
                agena_failure::FailureResponsibility::System,
                agena_failure::RetryDirective::UseAlternative,
                agena_failure::RecoveryDirective::ChooseAlternative,
                agena_failure::FailureImpact::OperationFailed,
                agena_failure::UserPresentation::new(
                    "tool-internal-failure",
                    "Tool execution failed without diagnostic details.",
                ),
            ),
            agena_domain::RawOutput::default(),
            TimeRange::default(),
        );
        assert_eq!(failed.status(), super::ExecutionStatus::Failed);
        assert!(failed.error_message().is_some());
    }

    #[test]
    fn metadata_helpers_stay_stable() {
        let mut op = operation();
        op.set_notified();
        assert!(op.is_notified());
        op.set_background_operation(&super::BackgroundOperation {
            kind: "shell".into(),
            id: "p-1".into(),
        });
        assert_eq!(
            op.background_operation(),
            Some(super::BackgroundOperation {
                kind: "shell".into(),
                id: "p-1".into(),
            })
        );
        op.set_provider_raw(Some(serde_json::json!({"id": "provider-1"})));
        assert_eq!(
            op.provider_raw().and_then(|raw| raw["id"].as_str()),
            Some("provider-1")
        );
    }
}

#[cfg(test)]
mod current_input_contract_tests {
    use super::{ReadToolInput, ShellCommandInput, TaskToolInput};
    use serde_json::json;

    #[test]
    fn shell_input_rejects_removed_result_and_background_fields() {
        for removed in [
            json!({"filesystem_effects": {"read": [], "write": []}}),
            json!({"background": true}),
        ] {
            let mut input = json!({
                "command": "ls",
                "reads": [],
                "writes": [],
                "network": []
            });
            input
                .as_object_mut()
                .expect("shell input is an object")
                .extend(
                    removed
                        .as_object()
                        .expect("removed fields are an object")
                        .clone(),
                );
            assert!(
                serde_json::from_value::<ShellCommandInput>(input).is_err(),
                "removed shell input fields must be rejected"
            );
        }
    }

    #[test]
    fn task_input_rejects_removed_background_field() {
        let input = json!({
            "description": "inspect",
            "prompt": "inspect the repository",
            "background": true
        });
        assert!(serde_json::from_value::<TaskToolInput>(input).is_err());
    }

    #[test]
    fn read_input_accepts_only_the_canonical_file_path_field() {
        let canonical = json!({"file_path": "README.md"});
        assert!(serde_json::from_value::<ReadToolInput>(canonical).is_ok());

        let removed_alias = json!({"path": "README.md"});
        assert!(serde_json::from_value::<ReadToolInput>(removed_alias).is_err());
    }
}
