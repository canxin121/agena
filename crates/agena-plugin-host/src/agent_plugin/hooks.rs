//! Translate foreign hook declarations and results at Agena's typed lifecycle boundary.

mod background;
mod condition;
mod config;
mod dispatch;
pub(crate) mod file_watch;
mod http;
mod mods;
pub(super) mod monitor;
mod process;
mod scope;

pub(crate) use dispatch::ToolProjection;
pub use dispatch::{project_claude_mod_tool_input, project_transcript_tool};

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use agena_plugin_sdk::{HookHandlerDefinition, HookSubscription, PluginManifest};
use regex::Regex;
use serde_json::Value;

pub(super) use config::discover;
pub(super) use mods::{diagnose_mod_events, register_mod_events};
pub(super) use process::{HookRunner, ModProcessOptions, run_mod_process};

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum HookScope {
    #[default]
    Package,
    Agent(String),
    Skill(PathBuf),
    Command(String),
}

macro_rules! events {
    ($($event:ident => $subscription:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum Event { $($event),+ }

        impl Event {
            const ALL: &'static [Self] = &[$(Self::$event),+];

            fn parse(name: &str) -> Option<Self> {
                match name {
                    $(stringify!($event) => Some(Self::$event),)+
                    _ => None,
                }
            }

            fn name(self) -> &'static str {
                match self { $(Self::$event => stringify!($event),)+ }
            }

            fn subscription(self) -> HookSubscription {
                match self { $(Self::$event => HookSubscription::$subscription,)+ }
            }
        }
    };
}

events! {
    PreToolUse => TOOL_BEFORE,
    PostToolUse => TOOL_AFTER,
    PostToolUseFailure => TOOL_FAILURE,
    PostToolBatch => TOOL_BATCH_AFTER,
    PermissionRequest => PERMISSION_REQUEST,
    PermissionDenied => TOOL_FAILURE,
    Notification => NOTIFICATION,
    UserPromptSubmit => USER_PROMPT_SUBMIT,
    UserPromptExpansion => USER_PROMPT_EXPANSION,
    SessionStart => SESSION_START,
    Setup => IMPORTED_SESSION_SETUP,
    SessionEnd => SESSION_END,
    FileChanged => SESSION_FILE_CHANGED,
    CwdChanged => SESSION_CWD_CHANGED,
    InstructionsLoaded => IMPORTED_INSTRUCTIONS_LOADED,
    MessageDisplay => IMPORTED_MESSAGE_DISPLAY,
    ConfigChange => IMPORTED_CONFIG_CHANGE,
    DirectoryAdded => IMPORTED_DIRECTORY_ADDED,
    Stop => AGENT_STOP,
    StopFailure => AGENT_STOP,
    Interrupt => AGENT_CANCEL,
    PreCompact => COMPACTION_BEFORE,
    PostCompact => COMPACTION_AFTER,
    SubagentStart => SUBAGENT_START,
    SubagentStop => AGENT_STOP,
    TeammateIdle => AGENT_STOP,
    TaskCreated => IMPORTED_TASK_BEFORE,
    TaskCompleted => IMPORTED_TASK_BEFORE,
    Elicitation => IMPORTED_MCP_ELICITATION,
    ElicitationResult => IMPORTED_MCP_ELICITATION_RESULT,
    PreSkillShellCommand => IMPORTED_SKILL_SHELL_BEFORE,
    PreModelSwitch => IMPORTED_MODEL_SWITCH_BEFORE,
    PostModelSwitch => IMPORTED_MODEL_SWITCH_AFTER,
    PreModelRequest => IMPORTED_MODEL_ATTEMPT_BEFORE,
    PostModelRequest => IMPORTED_MODEL_ATTEMPT_AFTER,
    WorktreeCreate => WORKTREE_CREATE,
    WorktreeRemove => WORKTREE_REMOVE,
}

impl Event {
    fn matcher_field(self) -> Option<&'static str> {
        match self {
            Self::PreToolUse
            | Self::PreSkillShellCommand
            | Self::PostToolUse
            | Self::PostToolUseFailure
            | Self::PermissionRequest
            | Self::PermissionDenied => Some("tool_name"),
            Self::SessionStart | Self::ConfigChange | Self::DirectoryAdded => Some("source"),
            Self::Setup => Some("trigger"),
            Self::Notification => Some("notification_type"),
            Self::SessionEnd => Some("reason"),
            Self::FileChanged => Some("file_path"),
            Self::InstructionsLoaded => Some("load_reason"),
            Self::Elicitation | Self::ElicitationResult => Some("mcp_server_name"),
            Self::UserPromptExpansion => Some("command_name"),
            Self::PreModelSwitch | Self::PostModelSwitch => Some("to_model"),
            Self::StopFailure => Some("error"),
            Self::PreCompact | Self::PostCompact => Some("trigger"),
            Self::SubagentStart | Self::SubagentStop => Some("agent_type"),
            Self::TeammateIdle => Some("teammate_name"),
            _ => None,
        }
    }

    fn can_block_on_failure(self, dialect: Dialect) -> bool {
        if dialect == Dialect::Codex {
            return matches!(
                self,
                Self::PreToolUse
                    | Self::PostToolUse
                    | Self::PermissionRequest
                    | Self::UserPromptSubmit
                    | Self::SessionStart
                    | Self::SubagentStart
                    | Self::Stop
                    | Self::SubagentStop
                    | Self::PreCompact
                    | Self::PostCompact
                    | Self::Interrupt
            );
        }
        matches!(
            self,
            Self::PreToolUse
                | Self::PostToolBatch
                | Self::PermissionRequest
                | Self::UserPromptSubmit
                | Self::UserPromptExpansion
                | Self::PreCompact
                | Self::TaskCreated
                | Self::TaskCompleted
                | Self::Elicitation
                | Self::ElicitationResult
                | Self::PreSkillShellCommand
                | Self::PreModelSwitch
                | Self::ConfigChange
        )
    }

    fn is_elicitation(self) -> bool {
        matches!(self, Self::Elicitation | Self::ElicitationResult)
    }

    fn is_model_switch(self) -> bool {
        matches!(self, Self::PreModelSwitch | Self::PostModelSwitch)
    }

    fn is_worktree(self) -> bool {
        matches!(self, Self::WorktreeCreate | Self::WorktreeRemove)
    }

    fn is_model_attempt(self) -> bool {
        matches!(self, Self::PreModelRequest | Self::PostModelRequest)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dialect {
    Claude,
    Codex,
}

fn codex_shell_aliases(launch: agena_plugin_sdk::ShellLaunchMode) -> &'static [&'static str] {
    use agena_plugin_sdk::ShellLaunchMode;
    match launch {
        ShellLaunchMode::Open => &[
            "bash",
            "shell",
            "shell_command",
            "exec_command",
            "open",
            "agena.shell.open",
        ],
        ShellLaunchMode::Spawn => &[
            "bash",
            "shell",
            "shell_command",
            "exec_command",
            "spawn",
            "agena.shell.spawn",
        ],
        ShellLaunchMode::Watch => &[
            "bash",
            "shell",
            "shell_command",
            "exec_command",
            "watch",
            "agena.shell.watch",
        ],
    }
}

#[derive(Clone, Copy, Default)]
enum CodexShellHook<'a> {
    #[default]
    Default,
    Launch(agena_plugin_sdk::ShellLaunchMode),
    Skip,
    Completed(&'a agena_plugin_sdk::ShellProcessCompletion),
}

struct HookInput<'a> {
    fields: Value,
    claude_mcp: Option<&'a ClaudeMcpTool>,
    codex_tool_cwd: Option<&'a str>,
    codex_shell_hook: CodexShellHook<'a>,
    skip_scope: Option<HookScope>,
    canonical_model: Option<&'a str>,
    process_cwd: Option<&'a std::path::Path>,
}

impl From<Value> for HookInput<'_> {
    fn from(fields: Value) -> Self {
        Self {
            fields,
            claude_mcp: None,
            codex_tool_cwd: None,
            codex_shell_hook: CodexShellHook::Default,
            skip_scope: None,
            canonical_model: None,
            process_cwd: None,
        }
    }
}

impl<'a> HookInput<'a> {
    fn with_codex_tool_cwd(mut self, cwd: Option<&'a str>) -> Self {
        self.codex_tool_cwd = cwd;
        self
    }

    fn with_codex_shell_hook(mut self, terminal: CodexShellHook<'a>) -> Self {
        self.codex_shell_hook = terminal;
        self
    }
}

struct ClaudeMcpTool {
    tool_name: String,
    provenance: Value,
}

impl ClaudeMcpTool {
    fn apply(&self, fields: &mut Value) {
        fields["tool_name"] = serde_json::json!(self.tool_name);
        fields["mcp_server"] = self.provenance.clone();
    }
}

#[derive(Clone, Debug)]
enum Matcher {
    All,
    Exact(Vec<String>),
    Regex(Regex),
}

impl Matcher {
    fn matches(&self, value: &str) -> bool {
        match self {
            Self::All => true,
            Self::Exact(options) => options.iter().any(|option| option == value),
            Self::Regex(regex) => regex.is_match(value),
        }
    }

    fn same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::All, Self::All) => true,
            (Self::Exact(left), Self::Exact(right)) => left == right,
            (Self::Regex(left), Self::Regex(right)) => left.as_str() == right.as_str(),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Shell {
    Default,
    Bash,
    PowerShell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Exec { program: String, args: Vec<String> },
    Shell { script: String, shell: Shell },
}

#[derive(Clone, Debug)]
struct Handler {
    source: String,
    dialect: Dialect,
    matcher: Matcher,
    condition: Option<condition::ToolCondition>,
    action: Action,
    status: Option<String>,
    additional_context_limit: Option<usize>,
    timeout: Duration,
    block_on_failure: bool,
    mode: HandlerMode,
    scope: HookScope,
    once: bool,
    paths: HookPathOverrides,
    watch_names: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct HookPathOverrides {
    plugin_root: Option<PathBuf>,
    project_dir: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum HandlerMode {
    #[default]
    Foreground,
    Background,
    Rewake,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Command(Command),
    Http(http::HttpHook),
    Prompt(String),
    Agent(String),
    Mcp {
        server: String,
        tool: String,
        input: Value,
    },
}

impl Handler {
    fn same_as(&self, other: &Self) -> bool {
        self.dialect == other.dialect
            && self.matcher.same_as(&other.matcher)
            && self.condition == other.condition
            && self.action == other.action
            && self.timeout == other.timeout
            && self.block_on_failure == other.block_on_failure
            && self.mode == other.mode
            && self.scope == other.scope
            && self.once == other.once
            && self.paths == other.paths
            && self.watch_names == other.watch_names
    }
}

fn sequential_hook_budget<'a>(handlers: impl Iterator<Item = &'a Handler>) -> Duration {
    handlers.fold(Duration::from_secs(2), |budget, handler| {
        budget.saturating_add(handler.timeout)
    })
}

#[derive(Clone, Debug, Default)]
pub(super) struct HookSet(BTreeMap<Event, Vec<Handler>>);

impl HookSet {
    pub(super) fn len(&self) -> usize {
        self.0.values().map(Vec::len).sum()
    }

    pub(super) fn directory_added_timeout(&self) -> Duration {
        sequential_hook_budget(self.0.get(&Event::DirectoryAdded).into_iter().flatten())
            .saturating_add(Duration::from_secs(1))
    }

    fn claude_stop_handlers(&self) -> impl Iterator<Item = &Handler> {
        [Event::Stop, Event::SubagentStop, Event::TeammateIdle]
            .into_iter()
            .filter_map(move |event| self.0.get(&event))
            .flatten()
            .filter(|handler| handler.dialect == Dialect::Claude)
    }

    pub(super) fn has_claude_stop_hooks(&self) -> bool {
        self.claude_stop_handlers().next().is_some()
    }

    pub(crate) fn claude_lifecycle_timeout(&self, display: bool) -> Duration {
        let event = if display {
            Event::MessageDisplay
        } else {
            Event::ConfigChange
        };
        sequential_hook_budget(self.0.get(&event).into_iter().flatten())
            .saturating_add(Duration::from_secs(1))
    }

    pub(crate) fn worktree_timeout(&self, create: bool) -> Duration {
        let event = if create {
            Event::WorktreeCreate
        } else {
            Event::WorktreeRemove
        };
        sequential_hook_budget(self.0.get(&event).into_iter().flatten())
            .saturating_add(Duration::from_secs(1))
    }

    pub(crate) fn model_switch_timeout(&self, before: bool) -> Duration {
        let event = if before {
            Event::PreModelSwitch
        } else {
            Event::PostModelSwitch
        };
        sequential_hook_budget(self.0.get(&event).into_iter().flatten())
            .saturating_add(Duration::from_secs(1))
    }

    pub(crate) fn model_attempt_timeout(&self, before: bool) -> Duration {
        let event = if before {
            Event::PreModelRequest
        } else {
            Event::PostModelRequest
        };
        sequential_hook_budget(self.0.get(&event).into_iter().flatten())
            .saturating_add(Duration::from_secs(1))
    }

    pub(super) fn watch_filenames(&self) -> impl Iterator<Item = &PathBuf> {
        self.0
            .get(&Event::FileChanged)
            .into_iter()
            .flatten()
            .flat_map(|handler| &handler.watch_names)
    }
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(in crate::agent_plugin) fn agent_timeout(&self, method: &str) -> Option<Duration> {
        if method == agena_plugin_sdk::rpc::method::HOOK_IMPORTED_CLAUDE_STOP {
            return self
                .claude_stop_handlers()
                .filter_map(Handler::agent_timeout)
                .max();
        }
        let subscription = method.strip_prefix("hooks/")?;
        self.0
            .iter()
            .filter(|(event, _)| event.subscription().names().contains(&subscription))
            .flat_map(|(_, handlers)| handlers)
            .filter_map(Handler::agent_timeout)
            .max()
    }

    pub(super) fn register(&self, manifest: &mut PluginManifest) {
        if self.0.contains_key(&Event::InstructionsLoaded) {
            manifest.hooks.insert(HookSubscription::SESSION_END);
        }
        if self.0.contains_key(&Event::FileChanged) {
            manifest.hooks.insert(
                HookSubscription::SESSION_START
                    | HookSubscription::SESSION_END
                    | HookSubscription::USER_PROMPT_SUBMIT,
            );
        }
        if self.0.values().flatten().any(|handler| {
            handler.mode != HandlerMode::Foreground || handler.scope != HookScope::Package
        }) {
            manifest.hooks.insert(HookSubscription::SESSION_END);
        }
        if self
            .0
            .values()
            .flatten()
            .any(|handler| matches!(handler.scope, HookScope::Skill(_)))
        {
            manifest
                .hooks
                .insert(HookSubscription::TOOL_AFTER | HookSubscription::SUBAGENT_START);
        }
        for (&event, handlers) in &self.0 {
            let subscription = event.subscription();
            manifest.hooks.insert(subscription);
            let name = subscription.names()[0];
            for (index, handler) in handlers.iter().enumerate() {
                manifest.hook_handlers.push(HookHandlerDefinition {
                    id: format!("{}-{index}", event.name()),
                    event: name.to_owned(),
                    summary: handler
                        .status
                        .clone()
                        .unwrap_or_else(|| format!("{} hook", event.name())),
                    help: format!(
                        "Runs a {} hook declared in {}.",
                        match handler.dialect {
                            Dialect::Claude => "Claude Code",
                            Dialect::Codex => "Codex",
                        },
                        handler.source
                    ),
                    translations: Default::default(),
                    priority: 0,
                    filters: Default::default(),
                    is_async: event == Event::InstructionsLoaded
                        || handler.mode != HandlerMode::Foreground,
                });
            }
        }
    }
}

impl Handler {
    fn agent_timeout(&self) -> Option<Duration> {
        matches!(self.action, Action::Agent(_)).then(|| self.timeout.min(Duration::from_secs(120)))
    }
}

#[derive(Default)]
struct Effects {
    display_content: Option<String>,
    block: Option<String>,
    permission_ask: Option<String>,
    context: Vec<String>,
    input: Option<Value>,
    output: Option<Value>,
    initial_message: Option<String>,
    session_title: Option<String>,
    reload_skills: bool,
    watch_paths: Option<Vec<PathBuf>>,
    elicitation: Option<agena_plugin_sdk::McpElicitationReply>,
    stop_skill: Option<String>,
    model_switch: agena_plugin_sdk::ImportedModelSwitchPatch,
    extra_codex_rollout_budget_units: f64,
    worktree: Option<agena_plugin_sdk::WorktreeHookOutput>,
}
