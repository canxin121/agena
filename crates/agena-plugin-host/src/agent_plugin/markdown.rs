use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::LazyLock,
};

use agena_plugin_sdk::{
    ApplicableSessions, CommandDefinition, CommandDocs, CommandInvokeInput, CommandTarget,
    ImportedSkillShellRequest, ImportedSkillShellResult, PluginError, PluginKey, ProcessShell,
    SessionKind, SettingsContract, SettingsNode, SettingsNodeKind,
};
use serde_json::Value;

use super::hooks::{HookScope, HookSet};
use super::{
    AgentPluginFormat, DELEGATED_COMMAND_HANDLER, MAX_COMPONENT_ENTRIES, Manifest, manifest_path,
    package_path, read_package_bytes, text_field,
};

const MAX_DOCUMENT_BYTES: u64 = 64 * 1024;
const MAX_COMMAND_ARGUMENTS: usize = 16 * 1024;
const MAX_RENDERED_PROMPT: usize = 128 * 1024;

mod shell;

#[derive(Clone)]
pub(super) struct ImportedCommand {
    pub(super) name: String,
    source: String,
    description: Option<String>,
    usage: Option<String>,
    body: String,
    claude: Option<ClaudeMarkdown>,
    pub(super) execution: CommandExecution,
    pub(super) scope: HookScope,
}

#[derive(Clone)]
struct ClaudeMarkdown {
    shell: Option<String>,
    arguments: Vec<String>,
}

impl ClaudeMarkdown {
    fn new(metadata: &serde_yaml::Value) -> Self {
        let arguments = metadata
            .get("arguments")
            .map(|value| {
                if let Some(text) = value.as_str() {
                    text.split_whitespace().map(ToOwned::to_owned).collect()
                } else if let Some(list) = value.as_sequence() {
                    list.iter()
                        .map(|value| value.as_str().unwrap_or_default().to_owned())
                        .collect()
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default();
        Self {
            shell: metadata
                .get("shell")
                .and_then(serde_yaml::Value::as_str)
                .map(ToOwned::to_owned),
            arguments,
        }
    }
}

#[derive(Clone)]
pub(super) enum CommandExecution {
    Prompt,
    Fork { agent: Option<String> },
    Workflow { name: String },
    DeepSeek(super::deepseek_commands::Command),
}

/// A text-based role that the host may project into its native agent catalog.
#[derive(Clone)]
pub struct AgentPluginRole {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub max_turns: Option<usize>,
    pub model: Option<String>,
    pub output_style: bool,
    pub worktree: bool,
    pub tool_allowlist: Option<BTreeSet<String>>,
    pub tool_denials: BTreeSet<String>,
    pub claude_options: ClaudeAgentOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeAgentMemoryScope {
    User,
    Project,
    Local,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeAgentOptions {
    pub role_name: String,
    pub skills: Vec<String>,
    pub effort: Option<agena_domain::ReasoningEffort>,
    pub omit_claude_md: bool,
    pub auto_compact_window: Option<u64>,
    pub memory: Option<ClaudeAgentMemoryScope>,
    pub background: bool,
}

#[derive(Default)]
pub(super) struct Components {
    pub commands: Vec<ImportedCommand>,
    pub roles: Vec<AgentPluginRole>,
}

pub(super) fn discover(
    root: &Path,
    format: AgentPluginFormat,
    manifest: &Manifest,
    watch: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<String>,
    hooks: &mut HookSet,
) -> Components {
    let mut reader = Reader {
        root,
        watch,
        diagnostics,
        hooks,
    };
    let mut commands = match format {
        AgentPluginFormat::ClaudeCode | AgentPluginFormat::Codex => reader.commands(
            manifest.document.get("commands"),
            format == AgentPluginFormat::ClaudeCode,
        ),
        AgentPluginFormat::AgentPluginsV1 => reader.commands(
            manifest
                .codex_overlay
                .as_ref()
                .and_then(|overlay| overlay.get("commands")),
            false,
        ),
        AgentPluginFormat::DeepSeekHarness => Vec::new(),
    };
    let openai = match format {
        AgentPluginFormat::Codex => Some(&manifest.document),
        AgentPluginFormat::AgentPluginsV1 => manifest
            .document
            .get("extensions")
            .and_then(|extensions| extensions.get("com.openai"))
            .and_then(Value::as_object)
            .or(manifest.codex_overlay.as_ref()),
        _ => None,
    };
    if let Some(onboarding) = openai.and_then(|metadata| metadata.get("onboardingSkill")) {
        match reader.onboarding(onboarding, &commands) {
            Ok(command) => commands.push(command),
            Err(error) => reader
                .diagnostics
                .push(format!("Codex onboardingSkill: {error}")),
        }
    }
    let mut roles = Vec::new();
    if format == AgentPluginFormat::ClaudeCode {
        for (key, folder, style) in [
            ("agents", "agents", false),
            ("outputStyles", "output-styles", true),
        ] {
            for entry in reader.documents(manifest.document.get(key), folder, !style) {
                match reader.role(entry, style) {
                    Ok(role) => roles.push(role),
                    Err(message) => reader.diagnostics.push(message),
                }
            }
        }
    }
    Components { commands, roles }
}

pub(super) fn discover_claude_skills(
    root: &Path,
    directories: &[PathBuf],
    hooks: &mut HookSet,
    diagnostics: &mut Vec<String>,
) -> Vec<ImportedCommand> {
    let mut commands = Vec::new();
    for directory in directories {
        let document = directory.join("SKILL.md");
        let Ok(relative) = document.strip_prefix(root) else {
            continue;
        };
        let source = relative.to_string_lossy();
        let Ok(Some(bytes)) = read_package_bytes(root, relative, 1024 * 1024) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let Ok((metadata, body)) = parse_frontmatter(text, &source) else {
            continue;
        };
        let scope = HookScope::Skill(directory.clone());
        if let Some(declaration) = metadata.get("hooks") {
            hooks.add_scoped(root, scope.clone(), &source, declaration, diagnostics);
        }
        if metadata["user-invocable"].as_bool() == Some(false) {
            continue;
        }
        let default_name = directory
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        let name = metadata["name"].as_str().unwrap_or(default_name);
        let text = |key| metadata[key].as_str().map(ToOwned::to_owned);
        commands.push(ImportedCommand {
            name: command_id(name),
            source: source.into_owned(),
            description: text("description"),
            usage: text("argument-hint"),
            body: body.to_owned(),
            claude: Some(ClaudeMarkdown::new(&metadata)),
            execution: if metadata["context"].as_str() == Some("fork") {
                CommandExecution::Fork {
                    agent: text("agent"),
                }
            } else {
                CommandExecution::Prompt
            },
            scope,
        });
    }
    commands
}

pub(super) fn prepare_model_skill(
    package: &super::AgentPluginDescriptor,
    plugin_id: &PluginKey,
    path: &Path,
    document: &str,
    input: &CommandInvokeInput,
) -> agena_plugin_sdk::Result<Option<PreparedSkillPrompt>> {
    let (metadata, body) = parse_frontmatter(document, &path.display().to_string())
        .map_err(PluginError::invalid_params)?;
    if metadata["disable-model-invocation"].as_bool() == Some(true) {
        return Ok(None);
    }
    let execution = if metadata["context"].as_str() == Some("fork") {
        CommandExecution::Fork {
            agent: metadata["agent"].as_str().map(ToOwned::to_owned),
        }
    } else {
        CommandExecution::Prompt
    };
    let default_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default();
    let command = ImportedCommand {
        name: command_id(metadata["name"].as_str().unwrap_or(default_name)),
        source: path.to_string_lossy().into_owned(),
        scope: HookScope::Skill(path.to_owned()),
        description: None,
        usage: None,
        body: body.to_owned(),
        claude: Some(ClaudeMarkdown::new(&metadata)),
        execution,
    };
    command
        .prepare_prompt(&package.root, &package.name, &plugin_id.to_string(), &package.claude_prompt_options, input)
        .map(Some)
}

struct Document {
    name: String,
    source: String,
    metadata: serde_yaml::Value,
    body: String,
}

impl Document {
    fn field(&self, key: &str) -> Option<&str> {
        self.metadata
            .get(key)
            .and_then(serde_yaml::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }
}

struct Reader<'a> {
    root: &'a Path,
    watch: &'a mut BTreeSet<PathBuf>,
    diagnostics: &'a mut Vec<String>,
    hooks: &'a mut HookSet,
}

impl Reader<'_> {
    fn onboarding(
        &mut self,
        value: &Value,
        commands: &[ImportedCommand],
    ) -> Result<ImportedCommand, String> {
        let path = value.as_str().ok_or("expected a local SKILL.md path")?;
        if Path::new(path)
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            != Some("SKILL.md")
        {
            return Err("expected a local SKILL.md path".into());
        }
        let document = self.read_manifest_doc(path)?;
        let name = ["onboarding", "plugin-onboarding"]
            .into_iter()
            .find(|name| !commands.iter().any(|command| command.name == *name))
            .ok_or("onboarding and plugin-onboarding are already used by commands")?
            .to_owned();
        let description = document.field("description").map(ToOwned::to_owned);
        let usage = document.field("argument-hint").map(ToOwned::to_owned);
        Ok(ImportedCommand {
            scope: HookScope::Command(name.clone()),
            name,
            source: document.source,
            description,
            usage,
            body: document.body,
            claude: None,
            execution: CommandExecution::Prompt,
        })
    }

    fn commands(&mut self, value: Option<&Value>, claude: bool) -> Vec<ImportedCommand> {
        let mut commands = BTreeMap::new();
        let mut add = |reader: &mut Reader<'_>, document: Document, entry: Option<&Value>| {
            let name = command_id(&document.name);
            let execution = if claude && document.field("context") == Some("fork") {
                CommandExecution::Fork {
                    agent: document.field("agent").map(ToOwned::to_owned),
                }
            } else {
                CommandExecution::Prompt
            };
            let command = ImportedCommand {
                scope: HookScope::Command(name.clone()),
                name,
                source: document.source.clone(),
                description: text_field(entry.and_then(|entry| entry.get("description")))
                    .or_else(|| document.field("description").map(ToOwned::to_owned)),
                usage: text_field(entry.and_then(|entry| entry.get("argumentHint")))
                    .or_else(|| document.field("argument-hint").map(ToOwned::to_owned)),
                body: document.body,
                claude: claude.then(|| ClaudeMarkdown::new(&document.metadata)),
                execution,
            };
            if commands.contains_key(&command.name) {
                reader.diagnostics.push(format!(
                    "{}: duplicate command {:?}",
                    command.source, command.name
                ));
            } else {
                if claude && let Some(declaration) = document.metadata.get("hooks") {
                    reader.hooks.add_scoped(
                        reader.root,
                        HookScope::Command(command.name.clone()),
                        &document.source,
                        declaration,
                        reader.diagnostics,
                    );
                }
                commands.insert(command.name.clone(), command);
            }
        };
        match (claude, value) {
            (true, Some(Value::Object(entries))) => {
                for (name, entry) in entries.iter().take(MAX_COMPONENT_ENTRIES) {
                    let document = match (entry.get("source"), entry.get("content")) {
                        (Some(Value::String(source)), None) => {
                            self.read_manifest_doc(source).map(|mut document| {
                                document.name.clone_from(name);
                                document
                            })
                        }
                        (None, Some(Value::String(body))) => {
                            parse_document(name.clone(), format!("manifest commands.{name}"), body)
                        }
                        _ => Err(format!(
                            "command {name:?} requires exactly one source or content string"
                        )),
                    };
                    match document {
                        Ok(document) => add(self, document, Some(entry)),
                        Err(error) => self.diagnostics.push(error),
                    }
                }
            }
            _ => {
                for document in self.documents(value, "commands", false) {
                    add(self, document, None);
                }
            }
        }
        commands.into_values().collect()
    }

    fn documents(&mut self, value: Option<&Value>, folder: &str, nested: bool) -> Vec<Document> {
        let default = Value::String(format!("./{folder}"));
        let value = value.unwrap_or(&default);
        let values = value
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_else(|| std::slice::from_ref(value));
        let mut files = BTreeSet::new();
        let mut pending = Vec::new();
        for value in values {
            let path = value
                .as_str()
                .ok_or_else(|| format!("{folder} requires local paths"))
                .and_then(|path| manifest_path(self.root, path));
            match path {
                Ok(path) => pending.push((path, true)),
                Err(error) => self.diagnostics.push(error),
            }
        }
        let mut visited = BTreeSet::new();
        let mut documents = Vec::new();
        while let Some((path, scan_dir)) = pending.pop() {
            if visited.len() >= MAX_COMPONENT_ENTRIES {
                self.diagnostics
                    .push(format!("too many entries while discovering {folder}"));
                break;
            }
            if !visited.insert(path.clone()) {
                continue;
            }
            if path.is_dir() && scan_dir {
                self.watch.insert(path.clone());
                match std::fs::read_dir(&path) {
                    Ok(entries) => {
                        let mut entries = entries
                            .take(MAX_COMPONENT_ENTRIES)
                            .filter_map(Result::ok)
                            .filter_map(|entry| {
                                let kind = entry.file_type().ok()?;
                                (kind.is_file() || (nested && kind.is_dir()))
                                    .then(|| (entry.path(), nested))
                            })
                            .collect::<Vec<_>>();
                        entries.sort_unstable_by(|a, b| b.0.cmp(&a.0));
                        pending.extend(entries);
                    }
                    Err(error) => self
                        .diagnostics
                        .push(format!("read {folder} directory: {error}")),
                }
                continue;
            }
            if path.extension().is_some_and(|ext| ext == "md") && files.insert(path.clone()) {
                match self.read(&path) {
                    Ok(document) => documents.push(document),
                    Err(error) => self.diagnostics.push(error),
                }
            } else if scan_dir && *value != default {
                self.watch.insert(path.clone());
                self.diagnostics
                    .push(format!("invalid {folder} component {}", path.display()));
            }
        }
        documents
    }

    fn read_manifest_doc(&mut self, source: &str) -> Result<Document, String> {
        let path = manifest_path(self.root, source)?;
        self.read(&path)
    }

    fn read(&mut self, path: &Path) -> Result<Document, String> {
        let relative = path
            .strip_prefix(self.root)
            .map_err(|_| "invalid Markdown component path")?;
        let path = package_path(self.root, relative)?;
        self.watch.insert(path.clone());
        let source = relative.to_string_lossy().into_owned();
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or("invalid Markdown filename")?;
        let bytes = read_package_bytes(self.root, relative, MAX_DOCUMENT_BYTES)?
            .ok_or_else(|| format!("Markdown component {source} is missing"))?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| format!("Markdown component {source} is not UTF-8"))?;
        parse_document(name.to_owned(), source, text)
    }

    fn role(&mut self, document: Document, output_style: bool) -> Result<AgentPluginRole, String> {
        // Claude ignores permissionMode and mcpServers for packaged subagents.
        let mut claude_options = ClaudeAgentOptions::default();
        if !output_style {
            let metadata = &document.metadata;
            if let Some(value) = metadata.get("skills") {
                let skills = value
                    .as_sequence()
                    .filter(|skills| skills.len() <= MAX_COMPONENT_ENTRIES)
                    .ok_or_else(|| format!("{}: skills must be a list", document.source))?;
                for skill in skills {
                    let name = skill
                        .as_str()
                        .filter(|name| !name.is_empty() && name.len() <= 256)
                        .ok_or_else(|| format!("{}: invalid skill name", document.source))?;
                    claude_options.skills.push(name.to_owned());
                }
            }
            if let Some(value) = metadata.get("effort") {
                claude_options.effort = serde_yaml::from_value(value.clone())
                    .ok()
                    .filter(|effort| *effort != agena_domain::ReasoningEffort::Minimal);
                if claude_options.effort.is_none() {
                    self.diagnostics.push(format!("{}: invalid effort", document.source));
                }
            }
            claude_options.omit_claude_md = metadata["omitClaudeMd"].as_bool() == Some(true);
            if let Some(value) = metadata.get("autoCompactWindow") {
                claude_options.auto_compact_window = value.as_u64().filter(|window| *window >= 100_000);
                if claude_options.auto_compact_window.is_none() {
                    self.diagnostics.push(format!("{}: autoCompactWindow must be an integer of at least 100000", document.source));
                }
            }
            if let Some(value) = metadata.get("memory") {
                claude_options.memory = match value.as_str() {
                    Some("user") => Some(ClaudeAgentMemoryScope::User),
                    Some("project") => Some(ClaudeAgentMemoryScope::Project),
                    Some("local") => Some(ClaudeAgentMemoryScope::Local),
                    _ => {
                        self.diagnostics.push(format!(
                            "{}: memory must be \"user\", \"project\", or \"local\"",
                            document.source
                        ));
                        None
                    }
                };
            }
            if let Some(value) = metadata.get("background") {
                match value.as_bool() {
                    Some(background) => claude_options.background = background,
                    None => self.diagnostics.push(format!(
                        "{}: background must be a boolean",
                        document.source
                    )),
                }
            }
        }
        let model = document
            .field("model")
            .map(str::trim)
            .filter(|model| !model.is_empty() && *model != "inherit")
            .map(ToOwned::to_owned);
        let name = document
            .field("name")
            .unwrap_or(&document.name)
            .replace(':', ".")
            .to_ascii_lowercase();
        claude_options.role_name = name.clone();
        let instructions = document
            .body
            .replace("${CLAUDE_PLUGIN_ROOT}", &self.root.to_string_lossy());
        let worktree = match document.metadata.get("isolation") {
            None => false,
            Some(value) if !output_style && value.as_str() == Some("worktree") => true,
            Some(_) => {
                return Err(format!(
                    "{}: unsupported isolation setting",
                    document.source
                ));
            }
        };
        let tool_allowlist = self.role_tools(&document, "tools", false)?;
        let tool_denials = self
            .role_tools(&document, "disallowedTools", true)?
            .unwrap_or_default();
        if !output_style && let Some(declaration) = document.metadata.get("hooks") {
            self.hooks.add_scoped(
                self.root,
                HookScope::Agent(name.clone()),
                &document.source,
                declaration,
                self.diagnostics,
            );
        }
        Ok(AgentPluginRole {
            name,
            description: document.field("description").unwrap_or_default().to_owned(),
            max_turns: document
                .metadata
                .get("maxTurns")
                .and_then(serde_yaml::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value != 0),
            instructions,
            model,
            output_style,
            worktree,
            tool_allowlist,
            tool_denials,
            claude_options,
        })
    }

    fn role_tools(
        &mut self,
        document: &Document,
        field: &str,
        denying: bool,
    ) -> Result<Option<BTreeSet<String>>, String> {
        use serde_yaml::Value as Yaml;
        let names = match document.metadata.get(field) {
            None | Some(Yaml::Null) => return Ok(None),
            Some(Yaml::String(names)) => names.split(',').collect::<Vec<_>>(),
            Some(Yaml::Sequence(names)) => names
                .iter()
                .map(Yaml::as_str)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| format!("{}: {field} must contain tool names", document.source))?,
            Some(_) => return Err(format!("{}: {field} must list tool names", document.source)),
        };
        let mut resolved = BTreeSet::new();
        for name in names
            .into_iter()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            let tools: &[&str] = match name {
                "Glob" | "LS" => &["agena.fs.glob"],
                "Grep" => &["agena.fs.grep"],
                "Read" if denying => {
                    &["agena.fs.read", "agena.fs.read_many", "agena.notebook.read"]
                }
                "Read" => &["agena.fs.read", "agena.notebook.read"],
                "NotebookRead" | "ReadNotebook" => &["agena.notebook.read"],
                "Write" if denying => &["agena.fs.write", "agena.fs.apply_patch"],
                "Write" => &["agena.fs.write"],
                "Edit" | "MultiEdit" if denying => &[
                    "agena.fs.replace",
                    "agena.fs.apply_patch",
                    "agena.notebook.edit",
                    "agena.notebook.edit_cell",
                ],
                "Edit" | "MultiEdit" => &["agena.fs.replace"],
                "NotebookEdit" | "NotebookEditCell" if denying => {
                    &["agena.notebook.edit", "agena.notebook.edit_cell"]
                }
                "NotebookEdit" | "NotebookEditCell" => &["agena.notebook.edit"],
                "Bash" | "PowerShell" if denying => &[
                    "agena.shell.exec",
                    "agena.shell.spawn",
                    "agena.shell.watch",
                    "agena.shell.open",
                    "agena.shell.write",
                ],
                "Bash" => &["agena.shell.exec", "agena.shell.spawn"],
                "BashOutput" => &["agena.shell.read", "agena.shell.logs"],
                "KillShell" => &["agena.shell.stop"],
                "WebFetch" => &["agena.web.fetch"],
                "WebSearch" => &["agena.web.search"],
                "TaskCreate" => &["agena.task.create"],
                "TaskGet" => &["agena.task.get"],
                "TaskList" => &["agena.task.list"],
                "TaskUpdate" => &["agena.task.update"],
                "TodoWrite" => &["agena.task.todo_write"],
                name if name.starts_with("agena.")
                    && name.parse::<agena_plugin_sdk::ToolKey>().is_ok() =>
                {
                    resolved.insert(name.to_owned());
                    continue;
                }
                _ => {
                    let error = format!(
                        "{}: {field} entry {name:?} has no Agena mapping",
                        document.source
                    );
                    if denying {
                        return Err(error);
                    }
                    self.diagnostics.push(error);
                    continue;
                }
            };
            resolved.extend(tools.iter().map(|tool| (*tool).to_owned()));
        }
        Ok(Some(resolved))
    }
}

fn command_id(name: &str) -> String {
    name.trim().replace(':', ".").to_ascii_lowercase()
}

fn parse_document(name: String, source: String, text: &str) -> Result<Document, String> {
    if text.len() > MAX_DOCUMENT_BYTES as usize {
        return Err(format!(
            "Markdown component {source} exceeds its size limit"
        ));
    }
    let (metadata, body) = parse_frontmatter(text, &source)?;
    if body.is_empty() {
        return Err(format!("Markdown component {source} has no instructions"));
    }
    Ok(Document {
        name,
        source,
        metadata,
        body: body.to_owned(),
    })
}

fn parse_frontmatter<'a>(
    text: &'a str,
    source: &str,
) -> Result<(serde_yaml::Value, &'a str), String> {
    let text = text.trim_start_matches('\u{feff}').trim();
    let mut lines = text.split_inclusive('\n');
    let (metadata, body) = if lines.next().is_some_and(|line| line.trim_end() == "---") {
        let mut offset = text.find('\n').map(|index| index + 1).unwrap_or(text.len());
        let start = offset;
        loop {
            let line = lines
                .next()
                .ok_or_else(|| format!("unclosed frontmatter in {source}"))?;
            if line.trim_end() == "---" {
                let metadata: serde_yaml::Value = serde_yaml::from_str(&text[start..offset])
                    .map_err(|_| format!("invalid YAML frontmatter in {source}"))?;
                break (metadata, text[offset + line.len()..].trim());
            }
            offset += line.len();
        }
    } else {
        (serde_yaml::Value::Null, text)
    };
    Ok((metadata, body))
}

pub(super) fn plugin_command_definition(
    key: &PluginKey,
    name: &str,
    description: Option<String>,
    usage: Option<String>,
    handler: &str,
) -> CommandDefinition {
    let mut root = SettingsNode::root_object("Arguments", "");
    if let SettingsNodeKind::Object { fields } = &mut root.kind {
        fields.push(SettingsNode {
            id: "args".into(),
            path: "/args".into(),
            title: "Arguments".into(),
            description: String::new(),
            required: false,
            default: None,
            constraints: Default::default(),
            sensitive: false,
            secret: false,
            kind: SettingsNodeKind::Text,
        });
    }
    CommandDefinition {
        id: name.into(),
        title: name.into(),
        group: key.to_string(),
        category: None,
        slash: Some(format!("/{key}.{name}")),
        aliases: Vec::new(),
        applies_to: ApplicableSessions::only(SessionKind::Project),
        docs: CommandDocs {
            summary: description,
            usage,
            ..Default::default()
        },
        input: SettingsContract::new(root),
        target: CommandTarget::Method {
            handler: handler.into(),
        },
    }
}

impl ImportedCommand {
    pub(super) fn workflow(workflow: &super::workflow::Workflow) -> Self {
        Self {
            name: format!("workflow.{}", workflow.meta.name),
            source: workflow.source.clone(),
            description: Some(workflow.meta.description.clone()),
            usage: Some("[--resume[=<run ID>]] [JSON or text arguments]".into()),
            body: String::new(),
            claude: None,
            execution: CommandExecution::Workflow {
                name: workflow.meta.name.clone(),
            },
            scope: HookScope::Package,
        }
    }

    pub(super) fn deepseek(
        source: String,
        definition: super::deepseek_commands::Definition,
        command: super::deepseek_commands::Command,
    ) -> Self {
        Self {
            name: command_id(&definition.name),
            source,
            description: Some(definition.description),
            usage: definition.hint,
            body: String::new(),
            claude: None,
            scope: HookScope::Package,
            execution: CommandExecution::DeepSeek(command),
        }
    }

    pub(super) fn definition(&self, key: &PluginKey) -> CommandDefinition {
        plugin_command_definition(
            key,
            &self.name,
            self.description.clone(),
            self.usage.clone(),
            match self.execution {
                CommandExecution::Prompt => "markdown",
                CommandExecution::DeepSeek(ref command) if command.uses_mcp_manager() => {
                    super::CORDIS_MCP_COMMAND_HANDLER
                }
                CommandExecution::DeepSeek(_) => super::CORDIS_COMMAND_HANDLER,
                CommandExecution::Fork { .. } | CommandExecution::Workflow { .. } => {
                    DELEGATED_COMMAND_HANDLER
                }
            },
        )
    }

    pub(super) fn source(&self) -> &str {
        &self.source
    }

    pub(super) fn shell_injection_count(&self) -> usize {
        if self.claude.is_some() {
            shell::injections(&self.body).len()
        } else {
            0
        }
    }

    pub(super) fn prepare_prompt(
        &self,
        root: &Path,
        package_name: &str,
        plugin_id: &str,
        user_config: &BTreeMap<String, String>,
        input: &CommandInvokeInput,
    ) -> agena_plugin_sdk::Result<PreparedSkillPrompt> {
        let args = input
            .input
            .get("args")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if args.len() > MAX_COMMAND_ARGUMENTS {
            return Err(PluginError::invalid_params(
                "command arguments exceed 16 KiB",
            ));
        }
        let mut template = PromptTemplate {
            input,
            root,
            plugin_id,
            scope: &self.scope,
            claude: self.claude.is_some(),
            named: self
                .claude
                .as_ref()
                .map(|options| options.arguments.as_slice())
                .unwrap_or_default(),
            args,
            positional: shlex::split(args)
                .unwrap_or_else(|| args.split_whitespace().map(ToOwned::to_owned).collect()),
            substituted_arguments: false,
        };
        let injections = if self.claude.is_some() {
            shell::injections(&self.body)
        } else {
            Vec::new()
        };
        let mut parts = Vec::with_capacity(injections.len() + 1);
        let mut commands = Vec::with_capacity(injections.len());
        let mut end = 0;
        for injection in injections {
            parts.push(template.expand(&self.body[end..injection.span.start], Some(user_config))?);
            commands.push(template.expand(injection.command, None)?);
            end = injection.span.end;
        }
        let mut tail = template.expand(&self.body[end..], Some(user_config))?;
        if !args.is_empty() && !template.substituted_arguments {
            tail.push_str("\n\nARGUMENTS: ");
            tail.push_str(args);
        }
        if !input.prompt_context.is_empty() {
            tail.push_str("\n\n");
            tail.push_str(&input.prompt_context);
        }
        parts.push(tail);
        if parts.iter().map(String::len).sum::<usize>() > MAX_RENDERED_PROMPT {
            return Err(PluginError::invalid_params(
                "rendered command prompt is too long",
            ));
        }
        let shell = if commands.is_empty() {
            None
        } else {
            let shell = match self
                .claude
                .as_ref()
                .and_then(|options| options.shell.as_deref())
            {
                None => None,
                Some("bash") => Some(ProcessShell::Bash),
                Some("powershell") => Some(ProcessShell::Powershell),
                Some(_) => {
                    return Err(PluginError::invalid_params(
                        "Claude shell frontmatter must be bash or powershell",
                    ));
                }
            };
            let (forked, explicit_agent) = match &self.execution {
                CommandExecution::Fork { agent } => (true, agent.is_some()),
                _ => (false, false),
            };
            Some(ImportedSkillShellRequest {
                command_name: format!("{package_name}:{}", self.name),
                shell,
                commands,
                forked,
                explicit_agent,
                skill_dir: match &self.scope {
                    HookScope::Skill(directory) => Some(directory.clone()),
                    _ => None,
                },
            })
        };
        let fork = match (&self.execution, &self.scope) {
            (CommandExecution::Fork { agent }, HookScope::Skill(directory)) => Some(SkillFork {
                agent: agent.clone(),
                directory: directory.clone(),
            }),
            _ => None,
        };
        Ok(PreparedSkillPrompt { parts, shell, fork })
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct SkillFork {
    pub(super) agent: Option<String>,
    pub(crate) directory: PathBuf,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(crate) struct ForkedSkillInvocation {
    pub scope: SkillFork,
    pub prompt: String,
}

pub struct PreparedSkillPrompt {
    parts: Vec<String>,
    shell: Option<ImportedSkillShellRequest>,
    fork: Option<SkillFork>,
}

impl PreparedSkillPrompt {
    pub fn take_shell(&mut self) -> Option<ImportedSkillShellRequest> {
        self.shell.take()
    }

    pub fn take_fork(&mut self) -> Option<SkillFork> {
        self.fork.take()
    }

    pub fn finish(self, result: ImportedSkillShellResult) -> agena_plugin_sdk::Result<String> {
        if result.outputs.len() + 1 != self.parts.len() {
            return Err(PluginError::internal(
                "skill shell returned the wrong number of results",
            ));
        }
        let mut remaining = MAX_RENDERED_PROMPT - self.parts.iter().map(String::len).sum::<usize>();
        let mut outputs = result.outputs.into_iter();
        let mut prompt = String::new();
        for part in self.parts {
            prompt.push_str(&part);
            if let Some(output) = outputs.next() {
                let end = output.floor_char_boundary(output.len().min(remaining));
                prompt.push_str(&output[..end]);
                remaining -= end;
            }
        }
        Ok(prompt)
    }
}

struct PromptTemplate<'a> {
    input: &'a CommandInvokeInput,
    root: &'a Path,
    plugin_id: &'a str,
    scope: &'a HookScope,
    claude: bool,
    named: &'a [String],
    args: &'a str,
    positional: Vec<String>,
    substituted_arguments: bool,
}

impl PromptTemplate<'_> {
    fn expand(&mut self, source: &str, user_config: Option<&BTreeMap<String, String>>) -> agena_plugin_sdk::Result<String> {
        static PLACEHOLDER: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"\$\{(?P<env>CLAUDE_PLUGIN_ROOT|PLUGIN_ROOT|CLAUDE_PLUGIN_DATA|CLAUDE_SESSION_ID|CLAUDE_PROJECT_DIR|CLAUDE_SKILL_DIR)\}|\$\{user_config\.(?P<option>[^}]+)\}|\$ARGUMENTS\[(?P<index>[0-9]+)\]|\$(?P<short>[0-9]+)\b|(?P<all>\$ARGUMENTS\b)|\$(?P<named>[a-zA-Z_][a-zA-Z_0-9]*)").unwrap()
        });
        let mut text = String::new();
        let mut end = 0;
        for matched in PLACEHOLDER.captures_iter(source) {
            let token = matched.get(0).expect("complete placeholder");
            text.push_str(&source[end..token.start()]);
            if let Some(env) = matched.name("env") {
                match env.as_str() {
                    "CLAUDE_PLUGIN_ROOT" | "PLUGIN_ROOT" => {
                        text.push_str(&self.root.to_string_lossy())
                    }
                    "CLAUDE_SESSION_ID" => {
                        if let Some(id) = self.input.session_id {
                            text.push_str(&id.to_string());
                        }
                    }
                    "CLAUDE_PROJECT_DIR" => text.push_str(
                        self.input
                            .workspace_root
                            .as_deref()
                            .unwrap_or(token.as_str()),
                    ),
                    "CLAUDE_PLUGIN_DATA" => {
                        if let Some(workspace) = self.input.workspace_root.as_deref() {
                            text.push_str(
                                &super::agent_plugin_data_path(
                                    Path::new(workspace),
                                    self.plugin_id,
                                )
                                .to_string_lossy(),
                            );
                        } else {
                            text.push_str(token.as_str());
                        }
                    }
                    "CLAUDE_SKILL_DIR" => {
                        if let HookScope::Skill(directory) = self.scope {
                            text.push_str(&directory.to_string_lossy());
                        } else {
                            text.push_str(token.as_str());
                        }
                    }
                    _ => unreachable!(),
                }
            } else if let Some(option) = matched.name("option") {
                let value = user_config.filter(|_| self.claude).and_then(|values| values.get(option.as_str()));
                text.push_str(value.map(String::as_str).unwrap_or(token.as_str()));
            } else if matched.name("all").is_some() {
                self.substituted_arguments = true;
                text.push_str(self.args);
            } else if let Some(name) = matched.name("named") {
                if let Some(index) = self.named.iter().position(|arg| arg == name.as_str()) {
                    self.substituted_arguments = true;
                    text.push_str(
                        self.positional
                            .get(index)
                            .map(String::as_str)
                            .unwrap_or_default(),
                    );
                } else {
                    text.push_str(token.as_str());
                }
            } else {
                let value = matched
                    .name("index")
                    .or_else(|| matched.name("short"))
                    .and_then(|index| index.as_str().parse::<usize>().ok())
                    .and_then(|index| self.positional.get(index).map(String::as_str));
                if let Some(value) = value {
                    self.substituted_arguments = true;
                    text.push_str(value);
                } else if self.claude {
                    text.push_str(token.as_str());
                } else {
                    self.substituted_arguments = true;
                }
            }
            end = token.end();
            if text.len() > MAX_RENDERED_PROMPT {
                return Err(PluginError::invalid_params(
                    "rendered command prompt is too long",
                ));
            }
        }
        text.push_str(&source[end..]);
        if text.len() > MAX_RENDERED_PROMPT {
            return Err(PluginError::invalid_params(
                "rendered command prompt is too long",
            ));
        }
        Ok(text)
    }
}

pub(super) fn expand_user_config(text: &str, values: &BTreeMap<String, String>) -> Result<String, String> {
    static OPTION: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"\$\{user_config\.([^}]+)\}").expect("valid Claude option pattern")
    });
    let too_long = || String::from("expanded Claude prompt exceeds the byte limit");
    let mut rendered = String::new();
    let mut end = 0;
    for captures in OPTION.captures_iter(text) {
        let token = captures.get(0).expect("complete option placeholder");
        let value = values.get(&captures[1]).map(String::as_str).unwrap_or_else(|| token.as_str());
        let prefix = &text[end..token.start()];
        if rendered.len() + prefix.len() + value.len() > MAX_RENDERED_PROMPT {
            return Err(too_long());
        }
        rendered.push_str(prefix);
        rendered.push_str(value);
        end = token.end();
    }
    if rendered.len() + text.len() - end > MAX_RENDERED_PROMPT {
        return Err(too_long());
    }
    rendered.push_str(&text[end..]);
    Ok(rendered)
}
