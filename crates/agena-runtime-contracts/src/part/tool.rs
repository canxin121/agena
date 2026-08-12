use std::collections::BTreeMap;

use agena_macros::ToolInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use agena_domain::{
    ArtifactRef, ExecutionStatus, FilesystemEffects, InteractionNotificationLevel, NetworkEffect,
    OperationAuthorization, OperationError, OperationUserInput, ProcessShell, ToolInvocation,
    ToolManagedOutput, ToolOutput, ToolPresentationSection, ToolResultDisplay, ToolResultState,
    UserInputQuestion,
};
use agena_tool::{ReadMode, TaskModelSelection, normalize_tool_summary, normalize_tool_title};

use super::AttachmentItem;
use agena_domain::{StructuredValue, TimeRange};

#[derive(Debug, Clone, Serialize, PartialEq, Eq, JsonSchema, ToolInput)]
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

impl<'de> Deserialize<'de> for ShellCommandInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            command: String,
            #[serde(default)]
            description: String,
            #[serde(default)]
            timeout_ms: Option<u64>,
            #[serde(default)]
            workdir: Option<String>,
            #[serde(default)]
            reads: Vec<String>,
            #[serde(default)]
            writes: Vec<String>,
            #[serde(default)]
            network: Vec<String>,
            #[serde(default)]
            filesystem_effects: Option<FilesystemEffects>,
            #[serde(default)]
            network_effects: Option<Vec<NetworkEffect>>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let mut reads = wire.reads;
        let mut writes = wire.writes;
        if let Some(effects) = wire.filesystem_effects {
            reads.extend(effects.read);
            writes.extend(effects.write);
        }
        let mut network = wire.network;
        if network.is_empty()
            && let Some(legacy) = wire.network_effects
        {
            network = legacy.into_iter().map(|effect| effect.target).collect();
        }
        Ok(Self {
            command: wire.command,
            description: wire.description,
            timeout_ms: wire.timeout_ms,
            workdir: wire.workdir,
            reads,
            writes,
            network,
        })
    }
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
#[input(trim("expression", "prompt"), non_empty("expression", "prompt"))]
/// Input of the cron create tool.
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
#[input(trim("prompt", "reason"), non_empty("prompt"))]
/// Input of the schedule wakeup tool.
pub struct ScheduleWakeupToolInput {
    pub delay_seconds: u32,
    pub prompt: String,
    /// Short reason logged for diagnostics / shown back to the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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

/// Interpret an output payload's optional message-presentation blocks as
/// unified v2 [`agena_domain::ViewBlock`]s. The output value itself belongs to
/// `agena-domain`; this adapter parses the legacy `content_blocks` JSON shape
/// (text/markdown/json/table/log/command/diff/file_changes/search_results/
/// media/custom) into the single ViewBlock render contract.
pub fn tool_output_content_blocks(
    output: &agena_domain::ToolOutput,
) -> Vec<agena_domain::ViewBlock> {
    let Some(blocks) = output
        .payload
        .get("content_blocks")
        .and_then(StructuredValue::as_array)
    else {
        return Vec::new();
    };

    blocks
        .iter()
        .filter_map(|block| json_block_to_view_block(block).ok())
        .collect()
}

fn json_block_to_view_block(value: &StructuredValue) -> Result<agena_domain::ViewBlock, String> {
    let json = serde_json::Value::from(value.clone());
    let object = json
        .as_object()
        .ok_or_else(|| "block must be an object".to_owned())?;
    let kind = object
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("text");
    let text = |key: &str| {
        object
            .get(key)
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    Ok(match kind {
        "markdown" => agena_domain::ViewBlock::Markdown {
            id: None,
            text: text("text"),
        },
        "json" => agena_domain::ViewBlock::Json {
            id: None,
            value: object
                .get("value")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        },
        "log" => agena_domain::ViewBlock::Log {
            id: None,
            stream: match object.get("stream").and_then(|value| value.as_str()) {
                Some("stderr") => agena_domain::CommandOutputStream::Stderr,
                _ => agena_domain::CommandOutputStream::Stdout,
            },
            text: text("text"),
        },
        "command" => agena_domain::ViewBlock::Command {
            id: None,
            command: text("command"),
            cwd: object
                .get("cwd")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
            exit_code: object
                .get("exit_code")
                .and_then(|value| value.as_i64())
                .map(|code| code as i32),
            stdout: object
                .get("stdout")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned(),
            stderr: object
                .get("stderr")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned(),
        },
        "diff" => agena_domain::ViewBlock::Diff {
            id: None,
            diff: text("diff"),
            language: object
                .get("language")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
        },
        "file_changes" => agena_domain::ViewBlock::FileChanges {
            id: None,
            changes: object
                .get("changes")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .unwrap_or_default(),
        },
        "table" => agena_domain::ViewBlock::Table {
            id: None,
            columns: object
                .get("columns")
                .and_then(|value| value.as_array())
                .map(|columns| {
                    columns
                        .iter()
                        .filter_map(|column| {
                            column.as_str().map(str::to_owned).or_else(|| {
                                column
                                    .get("label")
                                    .and_then(|label| label.as_str())
                                    .map(str::to_owned)
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            rows: object
                .get("rows")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .unwrap_or_default(),
        },
        "search_results" => {
            let items = object
                .get("results")
                .and_then(|value| value.as_array())
                .map(|results| {
                    results
                        .iter()
                        .filter_map(|item| {
                            let title = item
                                .get("title")
                                .and_then(|value| value.as_str())
                                .unwrap_or_default()
                                .to_owned();
                            let url = item
                                .get("url")
                                .and_then(|value| value.as_str())
                                .or_else(|| item.get("uri").and_then(|value| value.as_str()))
                                .unwrap_or_default()
                                .to_owned();
                            let snippet = item
                                .get("snippet")
                                .and_then(|value| value.as_str())
                                .map(str::to_owned);
                            (!title.is_empty() || !url.is_empty()).then_some({
                                agena_domain::WebSearchResult {
                                    title,
                                    url,
                                    snippet,
                                }
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            agena_domain::ViewBlock::SearchResults {
                id: None,
                items,
                total: None,
            }
        }
        "media" | "image" | "audio" | "resource_link" | "embedded_resource" | "file" => {
            let uri = object
                .get("uri")
                .and_then(|value| value.as_str())
                .or_else(|| object.get("url").and_then(|value| value.as_str()))
                .unwrap_or_default()
                .to_owned();
            let mime = object
                .get("mime")
                .and_then(|value| value.as_str())
                .or_else(|| object.get("mime_type").and_then(|value| value.as_str()))
                .unwrap_or_default()
                .to_owned();
            let name = object
                .get("filename")
                .and_then(|value| value.as_str())
                .or_else(|| object.get("title").and_then(|value| value.as_str()))
                .map(str::to_owned);
            agena_domain::ViewBlock::Media {
                id: None,
                artifact: agena_domain::ArtifactRef {
                    uri,
                    mime,
                    name,
                    size_bytes: None,
                    sha256: None,
                },
            }
        }
        "citation" => agena_domain::ViewBlock::Markdown {
            id: None,
            text: {
                let title = object
                    .get("title")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                let snippet = object
                    .get("snippet")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                let uri = object
                    .get("uri")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                format!("{title}\n\n{snippet}\n\n{uri}").trim().to_owned()
            },
        },
        // Legacy checklist/nested_task/progress and arbitrary custom blocks keep
        // their identity as a Custom ViewBlock.
        other => agena_domain::ViewBlock::Custom {
            id: None,
            kind: other.to_owned(),
            schema: serde_json::Value::Null,
            presentation: std::collections::BTreeMap::new(),
        },
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// Output of a tool visible to the model.
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
/// Envelope carrying the result of a tool execution.
pub struct ToolResultEnvelope {
    #[serde(default, skip_serializing_if = "ToolResultState::is_pending")]
    pub state: ToolResultState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<agena_domain::ViewBlock>,
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
        title: String,
        summary: String,
        output_text: String,
        blocks: Vec<agena_domain::ViewBlock>,
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
                title,
                summary,
                sections: Vec::new(),
            },
            attachments,
            error: None,
            metadata: BTreeMap::new(),
            raw: None,
        }
    }

    pub fn failed(
        failure: agena_failure::Failure,
        blocks: Vec<agena_domain::ViewBlock>,
        attachments: Vec<AttachmentItem>,
        details: &ToolOutput,
    ) -> Self {
        let truncated = details.is_model_truncated();
        let user_summary = failure.user.fallback.clone();
        let model_output = model_visible_failure_text(&failure);
        let human_summary = normalize_tool_summary(user_summary.clone());
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
                summary: human_summary,
                sections: Vec::new(),
            },
            attachments,
            error: Some(OperationError { failure }),
            metadata: BTreeMap::new(),
            raw: None,
        }
    }

    fn non_execution(
        state: ToolResultState,
        output_text: String,
        blocks: Vec<agena_domain::ViewBlock>,
        details: &ToolOutput,
    ) -> Self {
        debug_assert!(matches!(
            state,
            ToolResultState::PolicyDenied
                | ToolResultState::UserDeclined
                | ToolResultState::CapabilityUnavailable
                | ToolResultState::ToolUnavailable
        ));
        let human_summary = normalize_tool_summary(&output_text);
        Self {
            state,
            structured: details.to_json_payload(),
            content: blocks,
            model_preview: ModelVisibleOutput {
                text: output_text.clone(),
                attachments: Vec::new(),
                truncated: false,
            },
            managed_outputs: Vec::new(),
            display: ToolResultDisplay {
                title: String::new(),
                summary: human_summary,
                sections: Vec::new(),
            },
            attachments: Vec::new(),
            error: None,
            metadata: BTreeMap::new(),
            raw: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A message part representing a tool operation.
pub struct OperationPart {
    pub call_id: i64,
    pub invocation: ToolInvocation,
    #[serde(default, skip_serializing_if = "OperationAuthorization::is_empty")]
    pub authorization: OperationAuthorization,
    #[serde(default, skip_serializing_if = "OperationUserInput::is_empty")]
    pub user_input: OperationUserInput,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "ToolOutput::is_empty")]
    pub details: ToolOutput,
    #[serde(default, skip_serializing_if = "ToolResultEnvelope::is_empty")]
    pub result: ToolResultEnvelope,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
    #[serde(default)]
    pub lifecycle: TimeRange,
}

/// Complete, producer-owned presentation and result data for one Operation.
///
/// Keeping these fields together prevents callers from creating a completed
/// Operation with detailed output but no concise title/summary contract.
#[derive(Debug, Clone, PartialEq)]
pub struct OperationCompletion {
    pub title: String,
    pub summary: String,
    pub output_text: String,
    pub blocks: Vec<agena_domain::ViewBlock>,
    pub attachments: Vec<AttachmentItem>,
    pub details: ToolOutput,
}

impl OperationCompletion {
    pub fn new(
        title: impl Into<String>,
        summary: impl Into<String>,
        output_text: impl Into<String>,
        blocks: Vec<agena_domain::ViewBlock>,
        attachments: Vec<AttachmentItem>,
        details: ToolOutput,
    ) -> Self {
        Self {
            title: title.into(),
            summary: summary.into(),
            output_text: output_text.into(),
            blocks,
            attachments,
            details,
        }
    }
}

const PROVIDER_ONLY_METADATA_KEY: &str = "provider_only";
const LEGACY_PROVIDER_NATIVE_ONLY_METADATA_KEY: &str = "provider_native_only";
const ADVERTISED_TOOL_IDENTITY_METADATA_KEY: &str = "advertised_tool_identity";
/// Marker that an operation was launched into the background (a monitored
/// shell process or a delegated task) and must keep rendering as in-progress
/// on the transcript part until the background work actually finishes. The
/// value is a serialized [`BackgroundOperation`]; the session layer stamps it
/// at tool-success time, and the runtime's completion bridge reads it back to
/// terminalize the part when the process/task settles.
pub(crate) const BACKGROUND_OPERATION_METADATA_KEY: &str = "agena.background";

/// Which background work a launched-but-unfinished operation corresponds to,
/// used to correlate the transcript part with the completion signal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BackgroundOperation {
    pub kind: String,
    pub id: String,
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
            authorization: OperationAuthorization::default(),
            user_input: OperationUserInput::default(),
            title: normalize_tool_title(title.into()),
            summary: String::new(),
            artifacts: Vec::new(),
            details: ToolOutput::default(),
            result: ToolResultEnvelope::default(),
            metadata: BTreeMap::new(),
            error: None,
            raw: None,
            lifecycle,
        }
    }

    pub fn completed(
        call_id: i64,
        invocation: ToolInvocation,
        completion: OperationCompletion,
        lifecycle: TimeRange,
    ) -> Self {
        let OperationCompletion {
            title,
            summary,
            output_text,
            blocks,
            attachments,
            details,
        } = completion;
        let title = normalize_tool_title(title);
        let summary = normalize_tool_summary(summary);
        let result = ToolResultEnvelope::completed(
            title.clone(),
            summary.clone(),
            output_text.clone(),
            blocks.clone(),
            attachments.clone(),
            &details,
        );
        Self {
            call_id,
            invocation,
            authorization: OperationAuthorization::default(),
            user_input: OperationUserInput::default(),
            title,
            summary,
            artifacts: Vec::new(),
            details,
            result,
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
        blocks: Vec<agena_domain::ViewBlock>,
        attachments: Vec<AttachmentItem>,
        details: ToolOutput,
        lifecycle: TimeRange,
    ) -> Self {
        let result = ToolResultEnvelope::failed(
            failure.clone(),
            blocks.clone(),
            attachments.clone(),
            &details,
        );
        let user_summary = normalize_tool_summary(&failure.user.fallback);
        Self {
            call_id,
            invocation,
            authorization: OperationAuthorization::default(),
            user_input: OperationUserInput::default(),
            title: String::new(),
            summary: user_summary,
            artifacts: Vec::new(),
            details,
            result,
            metadata: BTreeMap::new(),
            error: Some(OperationError { failure }),
            raw: None,
            lifecycle,
        }
    }

    pub fn policy_denied(
        call_id: i64,
        invocation: ToolInvocation,
        output_text: impl Into<String>,
        blocks: Vec<agena_domain::ViewBlock>,
        details: ToolOutput,
        lifecycle: TimeRange,
    ) -> Self {
        Self::non_execution(
            call_id,
            invocation,
            ToolResultState::PolicyDenied,
            output_text.into(),
            blocks,
            details,
            lifecycle,
        )
    }

    pub fn user_declined(
        call_id: i64,
        invocation: ToolInvocation,
        output_text: impl Into<String>,
        blocks: Vec<agena_domain::ViewBlock>,
        details: ToolOutput,
        lifecycle: TimeRange,
    ) -> Self {
        Self::non_execution(
            call_id,
            invocation,
            ToolResultState::UserDeclined,
            output_text.into(),
            blocks,
            details,
            lifecycle,
        )
    }

    pub fn capability_unavailable(
        call_id: i64,
        invocation: ToolInvocation,
        output_text: impl Into<String>,
        blocks: Vec<agena_domain::ViewBlock>,
        details: ToolOutput,
        lifecycle: TimeRange,
    ) -> Self {
        Self::non_execution(
            call_id,
            invocation,
            ToolResultState::CapabilityUnavailable,
            output_text.into(),
            blocks,
            details,
            lifecycle,
        )
    }

    pub fn tool_unavailable(
        call_id: i64,
        invocation: ToolInvocation,
        output_text: impl Into<String>,
        blocks: Vec<agena_domain::ViewBlock>,
        details: ToolOutput,
        lifecycle: TimeRange,
    ) -> Self {
        Self::non_execution(
            call_id,
            invocation,
            ToolResultState::ToolUnavailable,
            output_text.into(),
            blocks,
            details,
            lifecycle,
        )
    }

    fn non_execution(
        call_id: i64,
        invocation: ToolInvocation,
        state: ToolResultState,
        output_text: String,
        blocks: Vec<agena_domain::ViewBlock>,
        details: ToolOutput,
        lifecycle: TimeRange,
    ) -> Self {
        let result =
            ToolResultEnvelope::non_execution(state, output_text.clone(), blocks.clone(), &details);
        Self {
            call_id,
            invocation,
            authorization: OperationAuthorization::default(),
            user_input: OperationUserInput::default(),
            title: String::new(),
            summary: normalize_tool_summary(&output_text),
            artifacts: Vec::new(),
            details,
            result,
            metadata: BTreeMap::new(),
            error: None,
            raw: None,
            lifecycle,
        }
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = normalize_tool_title(title.into());
        self.result.display.title = self.title.clone();
    }

    pub fn set_summary(&mut self, summary: impl Into<String>) {
        self.summary = agena_tool::normalize_tool_summary(summary.into());
        self.result.display.summary = self.summary.clone();
    }

    /// Set canonical named result sections. These are intentionally kept out
    /// of `blocks`: blocks are a rendering compatibility projection, while
    /// sections are the durable presentation contract for Activity clients.
    pub fn set_presentation_sections(&mut self, sections: Vec<ToolPresentationSection>) {
        self.result.display.sections = sections;
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

    /// Stamp the background-work correlation marker on this operation. The
    /// storage part state stays `InProgress` (so the transcript keeps showing
    /// the spinner) while `status()` still reports terminal via the lifecycle
    /// end — the provider pairing and the running spinner are independent.
    pub fn set_background_operation(&mut self, background: &BackgroundOperation) {
        self.metadata.insert(
            BACKGROUND_OPERATION_METADATA_KEY.to_string(),
            serde_json::to_value(background).expect("background marker is always JSON serializable"),
        );
    }

    /// The background-work correlation marker, when this operation was
    /// launched into the background and has not been terminalized yet.
    pub fn background_operation(&self) -> Option<BackgroundOperation> {
        self.metadata
            .get(BACKGROUND_OPERATION_METADATA_KEY)
            .cloned()
            .and_then(|value| serde_json::from_value::<BackgroundOperation>(value).ok())
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
        (!self.result.model_preview.text.is_empty())
            .then_some(self.result.model_preview.text.as_str())
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
        if self.result.state == ToolResultState::PolicyDenied {
            ExecutionStatus::PolicyDenied
        } else if self.result.state == ToolResultState::UserDeclined {
            ExecutionStatus::UserDeclined
        } else if self.result.state == ToolResultState::CapabilityUnavailable {
            ExecutionStatus::CapabilityUnavailable
        } else if self.result.state == ToolResultState::ToolUnavailable {
            ExecutionStatus::ToolUnavailable
        } else if self.error.is_some() {
            ExecutionStatus::Failed
        } else if self.lifecycle.end_ms.is_some() {
            ExecutionStatus::Completed
        } else if self.result.model_preview.text.trim().is_empty() {
            ExecutionStatus::Pending
        } else {
            ExecutionStatus::InProgress
        }
    }

    /// Append a streamed delta to the model-visible preview. The human-facing
    /// detail is derived at render time from the compact result, so streaming
    /// only ever grows the flat preview (bounded at completion); it is not
    /// persisted per-delta.
    pub fn append_output_delta(&mut self, delta: &str) -> bool {
        self.result.state = ToolResultState::Running;
        self.result.model_preview.text.push_str(delta);
        true
    }
}

/// The same bounded, sanitized failure detail shown to the user is the tool's
/// model-visible result. Closed category prose such as "the plugin failed"
/// discards the only actionable information and causes the model to diagnose
/// a provider outage instead of reacting to the real tool result.
fn model_visible_failure_text(failure: &agena_failure::Failure) -> String {
    let detail = failure.user.fallback.trim();
    if !detail.is_empty() {
        return detail.to_owned();
    }
    failure
        .model
        .as_ref()
        .map(agena_failure::ModelFeedback::message)
        .unwrap_or_else(|| "Tool execution failed without diagnostic details.".to_owned())
}

#[cfg(test)]
mod operation_part_tests {
    use super::OperationPart;
    use agena_domain::{TimeRange, ToolInvocation};

    fn operation() -> OperationPart {
        OperationPart::pending(
            1,
            ToolInvocation::new("shell", agena_domain::StructuredObject::default()),
            "Run process",
            TimeRange {
                start_ms: 0,
                end_ms: None,
            },
        )
    }

    #[test]
    fn streamed_delta_only_grows_the_flat_model_preview() {
        let mut op = operation();
        assert!(op.append_output_delta("building "));
        assert!(op.append_output_delta("thing\n"));
        assert_eq!(op.result.model_preview.text, "building thing\n");
        assert_eq!(op.result.state, agena_domain::ToolResultState::Running);
    }
}

#[cfg(test)]
mod failure_projection_tests {
    use super::{ToolResultEnvelope, model_visible_failure_text};
    use agena_domain::ToolOutput;
    use agena_failure::{
        Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, ModelFeedback,
        RecoveryDirective, RetryDirective, UserPresentation,
    };

    #[test]
    fn failed_tool_model_output_uses_the_sanitized_real_result() {
        let detail = "field `questions` requires at least 1 item";
        let failure = Failure::new(
            FailureCode::new("plugin.invalid_input"),
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            FailureImpact::OperationFailed,
            UserPresentation::validated("plugin-invalid-input", detail),
        )
        .with_model_feedback(ModelFeedback::plugin_failure());

        assert_eq!(model_visible_failure_text(&failure), detail);
        let result =
            ToolResultEnvelope::failed(failure, Vec::new(), Vec::new(), &ToolOutput::default());
        assert_eq!(result.display.summary, detail);
        assert_eq!(result.model_preview.text, detail);
        assert!(!result.model_preview.text.contains("The plugin failed"));
    }
}
