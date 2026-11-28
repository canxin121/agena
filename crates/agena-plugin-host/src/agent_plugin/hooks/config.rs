use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;
use serde_json::{Map, Value};

use super::{
    Action, Command, Dialect, Event, Handler, HandlerMode, HookPathOverrides, HookScope, HookSet,
    Matcher, Shell, condition::ToolCondition, http::HttpHook,
};
use crate::agent_plugin::{
    AgentPluginFormat, CLAUDE_MANIFEST, CODEX_MANIFEST, MAX_MANIFEST_BYTES, Manifest, claude_mods,
    deepseek, manifest_path, package_path, read_package_object,
};
use deepseek::{CLAUDE_HOOK_MODULE, CODEX_HOOK_MODULE};

const DEFAULT_HOOKS: &str = "hooks/hooks.json";
const MAX_HANDLERS_PER_EVENT: usize = 128;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

pub(in crate::agent_plugin) fn discover(
    root: &Path,
    format: AgentPluginFormat,
    manifest: &Manifest,
    deepseek_rows: &[(String, serde_yaml::Value)],
    watch: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<String>,
) -> (HookSet, Vec<PathBuf>) {
    let mut reader = Reader {
        root,
        hooks: HookSet::default(),
        modules: Vec::new(),
        scope: HookScope::Package,
        paths: HookPathOverrides::default(),
        watch,
        diagnostics,
    };
    if format == AgentPluginFormat::DeepSeekHarness {
        reader.deepseek(deepseek_rows);
    } else {
        let (dialect, declaration, source) = match format {
            AgentPluginFormat::ClaudeCode => (
                Dialect::Claude,
                manifest.document.get("hooks"),
                CLAUDE_MANIFEST,
            ),
            AgentPluginFormat::AgentPluginsV1 => {
                let extension = manifest
                    .document
                    .get("extensions")
                    .and_then(|value| value.get("com.openai"))
                    .and_then(Value::as_object);
                let (config, source) = if let Some(extension) = extension {
                    (Some(extension), "plugin.json#extensions.com.openai")
                } else {
                    (manifest.codex_overlay.as_ref(), CODEX_MANIFEST)
                };
                (
                    Dialect::Codex,
                    config.and_then(|config| config.get("hooks")),
                    source,
                )
            }
            _ => (
                Dialect::Codex,
                manifest.document.get("hooks"),
                CODEX_MANIFEST,
            ),
        };
        if dialect == Dialect::Claude || declaration.is_none() {
            reader.file(dialect, DEFAULT_HOOKS, false, DEFAULT_TIMEOUT);
        }
        if let Some(declaration) = declaration {
            let entries = declaration
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_else(|| std::slice::from_ref(declaration));
            let all_paths = entries.iter().all(Value::is_string);
            let all_objects = entries.iter().all(Value::is_object);
            if dialect == Dialect::Codex && !all_paths && !all_objects {
                reader.warn(source, "hooks must contain either paths or inline objects");
            } else {
                for entry in entries {
                    match entry {
                        Value::String(path) => reader.file(dialect, path, true, DEFAULT_TIMEOUT),
                        Value::Object(object) => {
                            reader.object(dialect, object, source, Path::new(""), DEFAULT_TIMEOUT)
                        }
                        _ => reader.warn(source, "hook entry must be a path or an inline object"),
                    }
                }
            }
        }
    }
    (reader.hooks, reader.modules)
}

struct Reader<'a> {
    root: &'a Path,
    hooks: HookSet,
    modules: Vec<PathBuf>,
    scope: HookScope,
    paths: HookPathOverrides,
    watch: &'a mut BTreeSet<PathBuf>,
    diagnostics: &'a mut Vec<String>,
}

impl HookSet {
    pub(in crate::agent_plugin) fn add_scoped(
        &mut self,
        root: &Path,
        scope: HookScope,
        source: &str,
        declaration: &serde_yaml::Value,
        diagnostics: &mut Vec<String>,
    ) {
        let object = match serde_json::to_value(declaration) {
            Ok(Value::Object(object)) => object,
            _ => {
                diagnostics.push(format!(
                    "hooks in {source}: expected an object keyed by event"
                ));
                return;
            }
        };
        let mut watch = BTreeSet::new();
        let mut reader = Reader {
            root,
            hooks: std::mem::take(self),
            modules: Vec::new(),
            scope,
            paths: HookPathOverrides::default(),
            watch: &mut watch,
            diagnostics,
        };
        reader.object(
            Dialect::Claude,
            &object,
            source,
            Path::new(""),
            DEFAULT_TIMEOUT,
        );
        *self = reader.hooks;
    }
}

impl Reader<'_> {
    fn warn(&mut self, source: &str, message: impl std::fmt::Display) {
        self.diagnostics
            .push(format!("hooks in {source}: {message}"));
    }

    fn file(&mut self, dialect: Dialect, relative: &str, explicit: bool, timeout: Duration) {
        let path = if explicit {
            manifest_path(self.root, relative)
        } else {
            package_path(self.root, Path::new(relative))
        };
        let path = match path {
            Ok(path) => path,
            Err(error) => return self.warn(relative, error),
        };
        self.watch.insert(path.clone());
        let relative_path = path
            .strip_prefix(self.root)
            .expect("hook path under package root");
        match read_package_object(self.root, relative_path, MAX_MANIFEST_BYTES) {
            Ok(Some(object)) => self.object(
                dialect,
                &object,
                relative,
                relative_path.parent().unwrap_or(Path::new("")),
                timeout,
            ),
            Ok(None) if explicit => self.warn(relative, "hook file does not exist"),
            Ok(None) => {}
            Err(error) => self.warn(relative, error),
        }
    }

    fn object(
        &mut self,
        dialect: Dialect,
        object: &Map<String, Value>,
        source: &str,
        base: &Path,
        timeout: Duration,
    ) {
        let modules = (dialect == Dialect::Claude)
            .then(|| object.get("modules"))
            .flatten();
        if let Some(modules) = modules {
            self.module_entries(source, base, modules);
        }
        let events = match object.get("hooks") {
            Some(Value::Object(events)) => events,
            Some(_) => return self.warn(source, "hooks must be an object keyed by event"),
            None => object,
        };
        for (name, groups) in events {
            if modules.is_some() && object.get("hooks").is_none() && name == "modules" {
                continue;
            }
            let Some(event) = Event::parse(name).filter(|event| {
                !(event.is_elicitation()
                    || event.is_model_switch()
                    || event.is_worktree()
                    || matches!(
                        event,
                        Event::FileChanged
                            | Event::CwdChanged
                            | Event::InstructionsLoaded
                            | Event::MessageDisplay
                            | Event::DirectoryAdded
                            | Event::ConfigChange
                            | Event::Setup
                            | Event::PermissionDenied
                            | Event::TaskCreated
                            | Event::TaskCompleted
                            | Event::PreSkillShellCommand
                            | Event::TeammateIdle
                    ))
                    || dialect == Dialect::Claude
            }) else {
                self.warn(
                    source,
                    format!("event {name} has no Agena lifecycle mapping"),
                );
                continue;
            };
            if event.is_model_attempt() && dialect != Dialect::Codex {
                self.warn(source, format!("event {name} requires Codex hook settings"));
                continue;
            }
            let registered_event =
                if matches!(self.scope, HookScope::Agent(_)) && event == Event::Stop {
                    Event::SubagentStop
                } else {
                    event
                };
            let Some(groups) = groups.as_array() else {
                self.warn(source, format!("{name} must contain matcher groups"));
                continue;
            };
            for group in groups {
                let Some(handlers) = group.get("hooks").and_then(Value::as_array) else {
                    self.warn(source, format!("{name} matcher group needs a hooks array"));
                    continue;
                };
                let matcher = match parse_matcher(dialect, event, group.get("matcher")) {
                    Ok(matcher) => matcher,
                    Err(error) => {
                        self.warn(source, format!("{name}: {error}"));
                        continue;
                    }
                };
                for value in handlers {
                    let event_handlers = self.hooks.0.get(&registered_event);
                    if event_handlers
                        .is_some_and(|handlers| handlers.len() >= MAX_HANDLERS_PER_EVENT)
                    {
                        self.warn(
                            source,
                            format!("{name} exceeds {MAX_HANDLERS_PER_EVENT} handlers"),
                        );
                        break;
                    }
                    match parse_handler(dialect, event, source, &matcher, value, timeout) {
                        Ok(handler) => {
                            let mut handler = handler;
                            handler.scope = self.scope.clone();
                            handler.paths = self.paths.clone();
                            if event == Event::FileChanged
                                && let Some(pattern) = group.get("matcher").and_then(Value::as_str)
                            {
                                handler.watch_names = pattern
                                    .split('|')
                                    .filter(|name| !name.is_empty())
                                    .map(PathBuf::from)
                                    .collect();
                            }
                            handler.once &=
                                matches!(self.scope, HookScope::Skill(_) | HookScope::Command(_));
                            let registered = self.hooks.0.entry(registered_event).or_default();
                            if dialect != Dialect::Claude
                                || !registered.iter().any(|existing| existing.same_as(&handler))
                            {
                                registered.push(handler);
                            }
                        }
                        Err(error) => self.warn(source, format!("{name}: {error}")),
                    }
                }
            }
        }
    }

    fn module_entries(&mut self, source: &str, base: &Path, value: &Value) {
        if self.scope != HookScope::Package {
            self.warn(
                source,
                "Claude mod modules must be declared at package scope",
            );
            return;
        }
        let Some(entries) = value.as_array() else {
            self.warn(source, "modules must be an array of local paths");
            return;
        };
        for entry in entries {
            let path = entry
                .as_str()
                .filter(|path| !path.is_empty() && !path.contains(['\\', '\0']))
                .ok_or_else(|| "expected a local hook module path".to_owned())
                .and_then(|path| local_module_path(self.root, base, Path::new(path)));
            let path = match path {
                Ok(path) => path,
                Err(error) => {
                    self.warn(source, error);
                    continue;
                }
            };
            if !self.modules.contains(&path) {
                if self.modules.len() >= claude_mods::MAX_ENTRIES {
                    self.warn(
                        source,
                        format!("modules exceeds {} entries", claude_mods::MAX_ENTRIES),
                    );
                    break;
                }
                self.watch.insert(path.clone());
                self.modules.push(path);
            }
        }
    }

    fn deepseek(&mut self, rows: &[(String, serde_yaml::Value)]) {
        for (id, row) in rows {
            let dialect = match deepseek::module_name(row) {
                Some(CLAUDE_HOOK_MODULE) => Dialect::Claude,
                Some(CODEX_HOOK_MODULE) => Dialect::Codex,
                _ => continue,
            };
            let Some(config) = row.get("config") else {
                self.warn(id, "DeepSeek hook adapter requires config");
                continue;
            };
            let Some(relative) = config.get("configPath").and_then(serde_yaml::Value::as_str)
            else {
                self.warn(id, "DeepSeek hook adapter requires a literal configPath");
                continue;
            };
            if dialect == Dialect::Claude {
                let path = |key| -> Result<Option<PathBuf>, String> {
                    config
                        .get(key)
                        .map(|value| {
                            value
                                .as_str()
                                .filter(|value| {
                                    !value.is_empty()
                                        && value.len() <= 4096
                                        && !value.contains('\0')
                                })
                                .map(PathBuf::from)
                                .ok_or_else(|| format!("invalid DeepSeek hook {key}"))
                        })
                        .transpose()
                };
                match (path("pluginRoot"), path("projectDir")) {
                    (Ok(plugin_root), Ok(project_dir)) => {
                        self.paths = HookPathOverrides {
                            plugin_root,
                            project_dir,
                        };
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        self.warn(id, error);
                        continue;
                    }
                }
            }
            let timeout = config
                .get("defaultTimeoutMs")
                .and_then(serde_yaml::Value::as_u64)
                .filter(|ms| *ms > 0)
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_TIMEOUT);
            let relative = if relative.starts_with("./") {
                relative.to_owned()
            } else {
                format!("./{relative}")
            };
            self.file(dialect, &relative, true, timeout);
            self.paths = HookPathOverrides::default();
        }
    }
}

fn local_module_path(root: &Path, base: &Path, path: &Path) -> Result<PathBuf, String> {
    let mut relative = base.to_owned();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => relative.push(name),
            Component::ParentDir if relative.pop() => {}
            _ => return Err(format!("hook module {path:?} is outside its package")),
        }
    }
    if !claude_mods::is_module(&relative) {
        return Err(format!("unsupported hook module extension in {path:?}"));
    }
    package_path(root, &relative)
}

fn parse_matcher(dialect: Dialect, event: Event, value: Option<&Value>) -> Result<Matcher, String> {
    if event.matcher_field().is_none() {
        return Ok(Matcher::All);
    }
    let pattern = match value {
        None | Some(Value::Null) => return Ok(Matcher::All),
        Some(Value::String(pattern)) => pattern.trim(),
        Some(_) => return Err("matcher must be a string".into()),
    };
    if matches!(pattern, "" | "*") {
        return Ok(Matcher::All);
    }
    let narrow = matches!(event, Event::FileChanged | Event::StopFailure);
    if dialect == Dialect::Claude
        && pattern.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || b"_|".contains(&byte)
                || (!narrow && b"- ,".contains(&byte))
        })
    {
        return Ok(Matcher::Exact(
            pattern
                .split(['|', ','])
                .map(|part| part.trim().to_owned())
                .collect(),
        ));
    }
    regex::RegexBuilder::new(pattern)
        .size_limit(256 * 1024)
        .build()
        .map(Matcher::Regex)
        .map_err(|_| "unsupported matcher regular expression".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawHandler {
    #[serde(rename = "type", default = "command_kind")]
    kind: String,
    command: Option<String>,
    #[serde(alias = "command_windows")]
    command_windows: Option<String>,
    args: Option<Vec<String>>,
    shell: Option<String>,
    #[serde(alias = "timeoutSec", alias = "timeout_sec")]
    timeout: Option<f64>,
    #[serde(alias = "status_message")]
    status_message: Option<String>,
    #[serde(alias = "additional_context_limit")]
    additional_context_limit: Option<usize>,
    #[serde(rename = "async", default)]
    background: bool,
    #[serde(default)]
    async_rewake: bool,
    #[serde(default)]
    once: bool,
    #[serde(rename = "if")]
    condition: Option<String>,
    on_failure: Option<String>,
    url: Option<String>,
    prompt: Option<String>,
    server: Option<String>,
    tool: Option<String>,
    input: Option<Value>,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    allowed_env_vars: Vec<String>,
}

fn command_kind() -> String {
    "command".into()
}

fn parse_handler(
    dialect: Dialect,
    event: Event,
    source: &str,
    matcher: &Matcher,
    value: &Value,
    default_timeout: Duration,
) -> Result<Handler, String> {
    let raw = RawHandler::deserialize(value).map_err(|_| "invalid hook handler fields")?;
    if event == Event::Setup && raw.kind != "command" {
        return Err("Setup requires a Claude command hook".into());
    }
    if event.is_worktree() && !matches!(raw.kind.as_str(), "command" | "http") {
        return Err("worktree events require a command or HTTP hook".into());
    }
    let action = match raw.kind.as_str() {
        "command" => Action::Command(command(
            dialect,
            raw.command,
            raw.command_windows,
            raw.args,
            raw.shell.as_deref(),
        )?),
        "http" if dialect == Dialect::Claude => Action::Http(HttpHook::new(
            raw.url.as_deref(),
            raw.headers,
            raw.allowed_env_vars,
        )?),
        "prompt" | "agent" if dialect == Dialect::Claude => {
            let prompt = raw
                .prompt
                .filter(|value| !value.trim().is_empty() && value.len() <= 32 * 1024)
                .ok_or("a model hook needs a nonempty prompt of at most 32 KiB")?;
            if raw.kind == "agent" {
                if !matches!(
                    event,
                    Event::PreToolUse
                        | Event::PreSkillShellCommand
                        | Event::PostToolUse
                        | Event::PostToolUseFailure
                        | Event::UserPromptSubmit
                        | Event::TaskCreated
                        | Event::TaskCompleted
                        | Event::Stop
                        | Event::SubagentStop
                        | Event::TeammateIdle
                ) {
                    return Err("this event does not support agent hooks".into());
                }
                Action::Agent(prompt)
            } else {
                Action::Prompt(prompt)
            }
        }
        "mcp_tool" if dialect == Dialect::Codex && event == Event::SessionEnd => {
            return Err("Codex SessionEnd cannot call MCP tools".into());
        }
        "mcp_tool" if dialect == Dialect::Codex && raw.server.as_deref() == Some("codex_apps") => {
            return Err("Codex account connector hooks are excluded".into());
        }
        "mcp_tool" => Action::Mcp {
            server: raw
                .server
                .filter(|value| !value.trim().is_empty() && !value.chars().any(char::is_control))
                .ok_or("an MCP hook needs a server name")?,
            tool: raw
                .tool
                .filter(|value| !value.trim().is_empty() && !value.chars().any(char::is_control))
                .ok_or("an MCP hook needs a tool name")?,
            input: match raw.input {
                Some(value @ Value::Object(_)) => value,
                None => Value::Object(Default::default()),
                _ => return Err("MCP hook input must be an object".into()),
            },
        },
        _ => {
            return Err(format!("handler type {:?} has no Agena adapter", raw.kind));
        }
    };
    let mode = match (raw.background, raw.async_rewake) {
        (false, false) => HandlerMode::Foreground,
        _ if matches!(
            event,
            Event::Setup | Event::PostModelSwitch | Event::MessageDisplay | Event::ConfigChange
                | Event::DirectoryAdded
        ) || event.is_worktree() =>
        {
            HandlerMode::Foreground
        }
        (_, _) if !matches!(&action, Action::Command(_)) => {
            return Err("async and asyncRewake require a command hook".into());
        }
        (_, true) if dialect == Dialect::Codex => {
            return Err("asyncRewake is a Claude command hook option".into());
        }
        (true, false) if dialect == Dialect::Codex && event == Event::SessionEnd => {
            HandlerMode::Foreground
        }
        (_, true) => HandlerMode::Rewake,
        (true, false) => HandlerMode::Background,
    };
    let timeout = match raw.timeout {
        Some(seconds) if seconds.is_finite() && seconds > 0.0 => {
            Duration::from_secs_f64(seconds.clamp(0.001, 600.0))
        }
        Some(_) => return Err("timeout must be a positive number of seconds".into()),
        None if event.is_model_switch() => default_timeout.min(Duration::from_secs(30)),
        None if event == Event::MessageDisplay => default_timeout.min(Duration::from_secs(10)),
        None if event == Event::DirectoryAdded => Duration::from_secs(600),
        None if matches!(action, Action::Agent(_)) => default_timeout.min(Duration::from_secs(60)),
        None => default_timeout.min(DEFAULT_TIMEOUT),
    };
    let block_on_failure = match raw.on_failure.as_deref() {
        None | Some("continue") => false,
        Some("block") => true,
        Some(_) => return Err("onFailure must be continue or block".into()),
    };
    if dialect == Dialect::Codex
        && block_on_failure
        && (mode != HandlerMode::Foreground || !event.can_block_on_failure(dialect))
    {
        return Err(
            "onFailure:block requires a synchronous Codex hook that supports blocking".into(),
        );
    }
    let condition = raw
        .condition
        .as_deref()
        .map(ToolCondition::parse)
        .transpose()?;
    let additional_context_limit = (dialect == Dialect::Codex
        && matches!(&action, Action::Command(_))
        && matches!(
            event,
            Event::PreToolUse
                | Event::PostToolUse
                | Event::SessionStart
                | Event::UserPromptSubmit
                | Event::SubagentStart
        ))
    .then_some(raw.additional_context_limit.unwrap_or(2_500));
    Ok(Handler {
        dialect,
        source: source.to_owned(),
        matcher: matcher.clone(),
        condition,
        action,
        status: raw
            .status_message
            .filter(|status| !status.trim().is_empty()),
        additional_context_limit,
        timeout,
        block_on_failure,
        mode,
        scope: HookScope::Package,
        once: raw.once,
        paths: HookPathOverrides::default(),
        watch_names: Vec::new(),
    })
}

fn command(
    dialect: Dialect,
    command: Option<String>,
    command_windows: Option<String>,
    args: Option<Vec<String>>,
    shell: Option<&str>,
) -> Result<Command, String> {
    let command = if cfg!(windows) && dialect == Dialect::Codex {
        command_windows.or(command)
    } else {
        command
    }
    .filter(|command| !command.trim().is_empty() && !command.contains('\0'))
    .ok_or("command hook needs a nonempty command")?;
    match args {
        Some(args) if args.iter().all(|arg| !arg.contains('\0')) => Ok(Command::Exec {
            program: command,
            args,
        }),
        Some(_) => Err("command hook arguments contain NUL".into()),
        None => {
            if command.contains("${user_config.") {
                return Err("user_config substitutions in hooks require exec form".into());
            }
            let shell = match shell {
                None => Shell::Default,
                Some("bash") => Shell::Bash,
                Some("powershell") => Shell::PowerShell,
                Some(_) => return Err("unsupported command hook shell".into()),
            };
            Ok(Command::Shell {
                script: command,
                shell,
            })
        }
    }
}
