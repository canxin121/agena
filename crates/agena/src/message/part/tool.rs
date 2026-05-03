use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::Display;

use super::{
    AttachmentItem, AttachmentKind, AttachmentSource, ExecutionStatus, FileChangeEntry,
    StructuredObject, TimeRange, TodoItem, UserInputQuestion,
};

pub type ToolAttachment = AttachmentItem;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct BashToolInput {
    pub command: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
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

/// Action discriminator for the `monitor` builtin tool.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct SkillRunToolInput {
    /// Skill name or alias.
    pub name: String,
    /// Optional free-form arguments forwarded to the skill body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(tag = "tool", rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum BuiltinToolInput {
    Bash(BashToolInput),
    Read(ReadToolInput),
    ViewFile(ViewFileToolInput),
    ApplyPatch(ApplyPatchToolInput),
    Glob(GlobToolInput),
    Grep(GrepToolInput),
    Task(TaskToolInput),
    ToolSearch(ToolSearchToolInput),
    TodoWrite(TodoWriteToolInput),
    #[serde(rename = "ask_user")]
    AskUser(AskUserToolInput),
    Monitor(MonitorToolInput),
    WebFetch(WebFetchToolInput),
    WebSearch(WebSearchToolInput),
    EnterPlanMode(EnterPlanModeToolInput),
    ExitPlanMode(ExitPlanModeToolInput),
    SkillRun(SkillRunToolInput),
    EnterWorktree(EnterWorktreeToolInput),
    ExitWorktree(ExitWorktreeToolInput),
    CronCreate(CronCreateToolInput),
    CronList(CronListToolInput),
    CronDelete(CronDeleteToolInput),
    ScheduleWakeup(ScheduleWakeupToolInput),
    LspDefinition(LspDefinitionToolInput),
    LspReferences(LspReferencesToolInput),
    LspHover(LspHoverToolInput),
    LspDiagnostics(LspDiagnosticsToolInput),
    NotebookEdit(NotebookEditToolInput),
    PowerShell(PowerShellToolInput),
}

impl BuiltinToolInput {
    /// Stable tool name as serialized in the wire format.
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::Bash(_) => "bash",
            Self::Read(_) => "read",
            Self::ViewFile(_) => "view_file",
            Self::ApplyPatch(_) => "apply_patch",
            Self::Glob(_) => "glob",
            Self::Grep(_) => "grep",
            Self::Task(_) => "task",
            Self::ToolSearch(_) => "tool_search",
            Self::TodoWrite(_) => "todo_write",
            Self::AskUser(_) => "ask_user",
            Self::Monitor(_) => "monitor",
            Self::WebFetch(_) => "web_fetch",
            Self::WebSearch(_) => "web_search",
            Self::EnterPlanMode(_) => "enter_plan_mode",
            Self::ExitPlanMode(_) => "exit_plan_mode",
            Self::SkillRun(_) => "skill_run",
            Self::EnterWorktree(_) => "enter_worktree",
            Self::ExitWorktree(_) => "exit_worktree",
            Self::CronCreate(_) => "cron_create",
            Self::CronList(_) => "cron_list",
            Self::CronDelete(_) => "cron_delete",
            Self::ScheduleWakeup(_) => "schedule_wakeup",
            Self::LspDefinition(_) => "lsp_definition",
            Self::LspReferences(_) => "lsp_references",
            Self::LspHover(_) => "lsp_hover",
            Self::LspDiagnostics(_) => "lsp_diagnostics",
            Self::NotebookEdit(_) => "notebook_edit",
            Self::PowerShell(_) => "powershell",
        }
    }

    /// Convert into a `ToolInvocation::Custom` carrying the tool name and a
    /// `StructuredObject`-encoded payload.
    pub fn into_invocation(self) -> ToolInvocation {
        let name = self.tool_name().to_string();
        let value = serde_json::to_value(&self).unwrap_or(serde_json::Value::Null);
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        object.remove("tool");
        let payload =
            StructuredObject::try_from(serde_json::Value::Object(object)).unwrap_or_default();
        ToolInvocation::Custom {
            name,
            input: payload,
        }
    }

    /// Reconstruct a `BuiltinToolInput` from a `(name, StructuredObject)`
    /// pair, or `None` if the name is not a recognized built-in tool.
    pub fn from_custom(name: &str, payload: &StructuredObject) -> Option<Self> {
        let value: serde_json::Value = payload.clone().into();
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            serde_json::Value::Null => serde_json::Map::new(),
            _ => return None,
        };
        let tag = canonical_builtin_name(name)?;
        object.insert(
            "tool".to_string(),
            serde_json::Value::String(tag.to_string()),
        );
        serde_json::from_value(serde_json::Value::Object(object)).ok()
    }
}

/// Returns `Some(canonical_tag)` if `name` matches a built-in tool.
pub fn canonical_builtin_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "bash" => "bash",
        "read" => "read",
        "view_file" => "view_file",
        "apply_patch" => "apply_patch",
        "glob" => "glob",
        "grep" => "grep",
        "task" => "task",
        "tool_search" => "tool_search",
        "todo_write" => "todo_write",
        "ask_user" => "ask_user",
        "monitor" => "monitor",
        "web_fetch" => "web_fetch",
        "web_search" => "web_search",
        "enter_plan_mode" => "enter_plan_mode",
        "exit_plan_mode" => "exit_plan_mode",
        "skill_run" => "skill_run",
        "enter_worktree" => "enter_worktree",
        "exit_worktree" => "exit_worktree",
        "cron_create" => "cron_create",
        "cron_list" => "cron_list",
        "cron_delete" => "cron_delete",
        "schedule_wakeup" => "schedule_wakeup",
        "lsp_definition" => "lsp_definition",
        "lsp_references" => "lsp_references",
        "lsp_hover" => "lsp_hover",
        "lsp_diagnostics" => "lsp_diagnostics",
        "notebook_edit" => "notebook_edit",
        "powershell" => "powershell",
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInvocation {
    pub entry_name: String,
    pub input: StructuredObject,
}

impl PluginInvocation {
    pub fn from_tool_invocation(invocation: &ToolInvocation) -> Self {
        let ToolInvocation::Custom { name, input } = invocation;
        Self {
            entry_name: name.clone(),
            input: input.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ToolInvocation {
    Custom {
        name: String,
        #[serde(default)]
        input: StructuredObject,
    },
}

impl ToolInvocation {
    /// If this invocation names a built-in tool, decode its input into a
    /// strongly-typed [`BuiltinToolInput`].
    pub fn as_builtin(&self) -> Option<BuiltinToolInput> {
        let Self::Custom { name, input } = self;
        BuiltinToolInput::from_custom(name, input)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum BuiltinToolOutput {
    Bash {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    Read {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        loaded_paths: Vec<String>,
    },
    ViewFile {
        path: String,
        kind: AttachmentKind,
        mime: String,
        size_bytes: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page_count: Option<u32>,
    },
    ApplyPatch {
        operation_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        changes: Vec<FileChangeEntry>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_hash: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_hash: Option<String>,
        inverse_patch: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        diff: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        progress: Vec<String>,
    },
    Glob {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
    },
    Grep {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        matches: Option<u32>,
    },
    Task {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_provider_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
    },
    ToolSearch {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        results: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        loaded_tools: Vec<String>,
    },
    TodoWrite {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<TodoItem>,
    },
    #[serde(rename = "ask_user")]
    AskUser {
        #[serde(
            default,
            deserialize_with = "crate::message::part::activity::deserialize_user_input_answers",
            skip_serializing_if = "crate::message::part::activity::user_input_answers_is_empty"
        )]
        answers: BTreeMap<String, Vec<String>>,
    },
    Monitor {
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        monitor_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<MonitorStatus>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        events: Vec<MonitorEvent>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        monitors: Vec<MonitorSummary>,
        /// Highest seq returned in `events` (or current `last_seq` for `start`/`list`/`stop`).
        #[serde(default)]
        last_seq: u64,
        /// True when the monitor still has buffered or future events past `last_seq`.
        #[serde(default)]
        has_more: bool,
        /// Total lines dropped due to ring-buffer eviction (cumulative).
        #[serde(default)]
        dropped_lines: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
    },
    WebFetch {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        markdown: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(default)]
        truncated: bool,
        #[serde(default)]
        cached: bool,
        status: u16,
    },
    WebSearch {
        query: String,
        backend: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        results: Vec<WebSearchHit>,
    },
    EnterPlanMode {
        plan_path: String,
        slug: String,
    },
    ExitPlanMode {
        approved: bool,
        plan_path: String,
    },
    SkillRun {
        name: String,
        body_chars: usize,
        allowed_tools: Vec<String>,
    },
    EnterWorktree {
        path: String,
        branch: String,
    },
    ExitWorktree {
        action: String,
        path: String,
    },
    CronCreate {
        id: String,
        next_fire_at: Option<String>,
    },
    CronList {
        jobs: Vec<CronJobSummary>,
    },
    CronDelete {
        id: String,
        removed: bool,
    },
    ScheduleWakeup {
        id: String,
        next_fire_at: String,
    },
    LspDefinition {
        /// Pretty-printed `path:line:col` locations the symbol resolves
        /// to (1-based line/col so the LLM can paste them straight to
        /// the user).
        locations: Vec<String>,
    },
    LspReferences {
        locations: Vec<String>,
    },
    LspHover {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contents: Option<String>,
    },
    LspDiagnostics {
        /// Each entry: `path:line:col [severity] message`.
        entries: Vec<String>,
    },
    NotebookEdit {
        path: String,
        edit_mode: String,
        cell_index: u32,
        cell_count: u32,
    },
    PowerShell {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobSummary {
    pub id: String,
    pub kind: String,
    pub expression: Option<String>,
    pub at: Option<String>,
    pub prompt: String,
    pub next_fire_at: Option<String>,
    pub last_fired_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchHit {
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

impl BuiltinToolOutput {
    /// Stable tool name as serialized in the wire format.
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::Bash { .. } => "bash",
            Self::Read { .. } => "read",
            Self::ViewFile { .. } => "view_file",
            Self::ApplyPatch { .. } => "apply_patch",
            Self::Glob { .. } => "glob",
            Self::Grep { .. } => "grep",
            Self::Task { .. } => "task",
            Self::ToolSearch { .. } => "tool_search",
            Self::TodoWrite { .. } => "todo_write",
            Self::AskUser { .. } => "ask_user",
            Self::Monitor { .. } => "monitor",
            Self::WebFetch { .. } => "web_fetch",
            Self::WebSearch { .. } => "web_search",
            Self::EnterPlanMode { .. } => "enter_plan_mode",
            Self::ExitPlanMode { .. } => "exit_plan_mode",
            Self::SkillRun { .. } => "skill_run",
            Self::EnterWorktree { .. } => "enter_worktree",
            Self::ExitWorktree { .. } => "exit_worktree",
            Self::CronCreate { .. } => "cron_create",
            Self::CronList { .. } => "cron_list",
            Self::CronDelete { .. } => "cron_delete",
            Self::ScheduleWakeup { .. } => "schedule_wakeup",
            Self::LspDefinition { .. } => "lsp_definition",
            Self::LspReferences { .. } => "lsp_references",
            Self::LspHover { .. } => "lsp_hover",
            Self::LspDiagnostics { .. } => "lsp_diagnostics",
            Self::NotebookEdit { .. } => "notebook_edit",
            Self::PowerShell { .. } => "powershell",
        }
    }

    /// Convert into a [`CustomToolOutput`] carrying the tool name and a
    /// `StructuredObject` payload (the inner fields, without the `tool` tag).
    pub fn into_custom_output(self) -> CustomToolOutput {
        let name = self.tool_name().to_string();
        let value = serde_json::to_value(&self).unwrap_or(serde_json::Value::Null);
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        object.remove("tool");
        let payload =
            StructuredObject::try_from(serde_json::Value::Object(object)).unwrap_or_default();
        CustomToolOutput { name, payload }
    }

    /// Reverse of [`into_custom_output`]: try to decode a `CustomToolOutput`
    /// emitted by a built-in tool back into the strongly-typed enum.
    pub fn from_custom(output: &CustomToolOutput) -> Option<Self> {
        let tag = canonical_builtin_name(&output.name)?;
        let value: serde_json::Value = output.payload.clone().into();
        let mut object = match value {
            serde_json::Value::Object(map) => map,
            serde_json::Value::Null => serde_json::Map::new(),
            _ => return None,
        };
        object.insert(
            "tool".to_string(),
            serde_json::Value::String(tag.to_string()),
        );
        serde_json::from_value(serde_json::Value::Object(object)).ok()
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolOutput {
    pub server: String,
    pub tool: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_blocks: Vec<ToolResultBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<StructuredObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomToolOutput {
    pub name: String,
    #[serde(default)]
    pub payload: StructuredObject,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ToolOutput {
    None,
    Mcp {
        output: McpToolOutput,
    },
    Custom {
        output: CustomToolOutput,
    },
    Unknown {
        name: String,
        #[serde(default)]
        payload: StructuredObject,
    },
}

impl ToolOutput {
    /// If this output is a `Custom` carrying a built-in tool's payload,
    /// decode it into a strongly-typed [`BuiltinToolOutput`].
    pub fn as_builtin(&self) -> Option<BuiltinToolOutput> {
        match self {
            Self::Custom { output } => BuiltinToolOutput::from_custom(output),
            _ => None,
        }
    }
}

impl Default for ToolOutput {
    fn default() -> Self {
        Self::None
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
        #[serde(default)]
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
        #[serde(default)]
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

    pub fn append_output_delta(&mut self, delta: &str) -> bool {
        match self {
            Self::Pending {
                call_id,
                invocation,
                title,
                lifecycle,
            } => {
                *self = Self::InProgress {
                    call_id: call_id.clone(),
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
}
