//! Discover local agent packages and adapt their components to an Agena plugin.

mod bridge;
mod claude_channels;
pub(crate) mod claude_mods;
mod compatibility;
mod deepseek;
mod deepseek_commands;
pub use deepseek_commands::{CordisFollowupBlock, decode_content_blocks};
mod deepseek_js;
mod hooks;
mod installation;
mod lsp;
mod markdown;
mod mcp;
mod mcp_bundle;
mod memvault;
pub(crate) mod tasks;
#[cfg(test)]
mod tests;
mod theme;
mod workflow;

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use agena_plugin_sdk::{
    HostClient, PluginError, PluginKey, PluginManifest, PluginTerminalThemePalette,
};
use serde_json::{Map, Value};

pub use compatibility::{
    AgentPluginCompatibility, CordisDependency, CordisService, CordisServiceStatus,
};
pub(crate) use hooks::ToolProjection;
pub use hooks::file_watch::FileWatches;
pub use hooks::{project_claude_mod_tool_input, project_transcript_tool};
pub use installation::{
    native_installation_path, native_installation_watch_paths, resolve_agent_plugin_path,
};
pub use lsp::{AgentPluginLspServer, ResolvedAgentPluginLsp};
pub(crate) use markdown::ForkedSkillInvocation;
pub use markdown::SkillFork;
pub use markdown::{AgentPluginRole, ClaudeAgentMemoryScope, ClaudeAgentOptions};
pub use mcp::{
    AgentPluginContext, AgentPluginMcpServer, AgentPluginMcpSettings, AgentPluginMcpTransport,
    DeepSeekManagedMcpServer, DeepSeekReconnectPolicy, ResolvedAgentPluginMcp,
};

pub const AGENT_PLUGIN_MANIFEST_SCHEMA: &str =
    "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";
const SCHEMA_PREFIX: &str = "https://agent-plugins.org/schemas/";
const CODEX_MANIFEST: &str = ".codex-plugin/plugin.json";
const CLAUDE_MANIFEST: &str = ".claude-plugin/plugin.json";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_COMPONENT_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPluginFormat {
    AgentPluginsV1,
    Codex,
    ClaudeCode,
    DeepSeekHarness,
}

impl AgentPluginFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentPluginsV1 => "Agent Plugins v1",
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
            Self::DeepSeekHarness => "DeepSeek Harness",
        }
    }
}

#[derive(Clone)]
pub struct AgentPluginDescriptor {
    pub root: PathBuf,
    pub format: AgentPluginFormat,
    pub name: String,
    pub version: Option<String>,
    pub default_enabled: bool,
    pub description: Option<String>,
    pub author: Option<String>,
    pub display_name: Option<String>,
    pub skill_dirs: Vec<PathBuf>,
    pub bin_dir: Option<PathBuf>,
    pub watch_paths: Vec<PathBuf>,
    pub mcp_servers: Vec<AgentPluginMcpServer>,
    pub deepseek_managed_mcp_servers: Vec<DeepSeekManagedMcpServer>,
    deepseek_programs: Vec<Arc<deepseek_commands::Program>>,
    cordis_dependencies: Vec<CordisDependency>,
    cordis_skill_count: usize,
    pub lsp_servers: Vec<AgentPluginLspServer>,
    themes: Vec<PluginTerminalThemePalette>,
    hooks: hooks::HookSet,
    mods: Option<claude_mods::Bundle>,
    mod_runtime: Option<Arc<claude_mods::Mods>>,
    monitors: Vec<hooks::monitor::Monitor>,
    workflows: BTreeMap<String, workflow::Workflow>,
    commands: Vec<markdown::ImportedCommand>,
    pub roles: Vec<AgentPluginRole>,
    claude_option_defaults: BTreeMap<String, String>,
    claude_prompt_options: BTreeMap<String, String>,
    memvault: Vec<memvault::MemVault>,
    pub diagnostics: Vec<String>,
}

impl AgentPluginDescriptor {
    pub(crate) async fn provided_skills(
        &self,
        cwd: &Path,
        deadline: std::time::Instant,
        session: Option<i64>,
        host: &dyn HostClient,
    ) -> Vec<deepseek_commands::CordisSkill> {
        let mut skills = BTreeMap::<String, deepseek_commands::CordisSkill>::new();
        for program in &self.deepseek_programs {
            if std::time::Instant::now() >= deadline {
                break;
            }
            let result = tokio::time::timeout_at(
                deadline.into(),
                program.provided_skills(cwd, deadline, session, host),
            )
            .await;
            match result {
                Ok(Ok(observed)) => {
                    for message in observed.diagnostics {
                        tracing::debug!(package = %self.name, %message, "Cordis skill provider lookup");
                    }
                    for skill in observed.skills {
                        let room = skills.len() < 256;
                        match skills.entry(skill.name.clone()) {
                            std::collections::btree_map::Entry::Vacant(entry) if room => {
                                entry.insert(skill);
                            }
                            std::collections::btree_map::Entry::Occupied(mut entry) if skill.rank < entry.get().rank => {
                                entry.insert(skill);
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Err(error)) => tracing::debug!(package = %self.name, %error, "Cordis skill provider unavailable"),
                Err(_) => break,
            }
        }
        skills.into_values().collect()
    }

    pub fn has_claude_stop_hooks(&self) -> bool {
        self.hooks.has_claude_stop_hooks()
            || self.mods.as_ref().is_some_and(|mods| {
                mods.matches("classic.Stop")
                    || mods.matches("classic.SubagentStop")
                    || mods.matches("classic.TeammateIdle")
            })
    }

    pub(crate) fn watch_filenames(&self) -> impl Iterator<Item = &PathBuf> {
        self.hooks.watch_filenames()
    }

    pub(crate) fn mod_runtime(&self) -> Option<&claude_mods::Mods> {
        self.mod_runtime.as_deref()
    }

    pub(crate) fn agent_hook_timeout(&self, method: &str) -> Option<std::time::Duration> {
        self.hooks.agent_timeout(method)
    }

    pub(crate) fn worktree_timeout(&self, create: bool) -> std::time::Duration {
        self.hooks.worktree_timeout(create)
    }

    pub(crate) fn claude_lifecycle_timeout(&self, display: bool) -> std::time::Duration {
        self.hooks.claude_lifecycle_timeout(display)
    }

    pub(crate) fn directory_added_timeout(&self) -> std::time::Duration {
        let timeout = self.hooks.directory_added_timeout();
        if self
            .mods
            .as_ref()
            .is_some_and(|mods| mods.matches("classic.DirectoryAdded"))
        {
            return timeout.saturating_add(std::time::Duration::from_secs(600));
        }
        timeout
    }

    pub(crate) fn model_switch_timeout(&self, before: bool) -> std::time::Duration {
        self.hooks.model_switch_timeout(before)
    }

    pub(crate) fn model_attempt_timeout(&self, before: bool) -> std::time::Duration {
        self.hooks.model_attempt_timeout(before)
    }

    pub(crate) fn has_prompt_command(&self, id: &str) -> bool {
        self.commands.iter().any(|command| {
            command.name == id
                && matches!(
                    command.execution,
                    markdown::CommandExecution::Prompt | markdown::CommandExecution::Fork { .. }
                )
        })
    }

    pub(crate) fn has_inline_prompt_command(&self, id: &str) -> bool {
        self.commands.iter().any(|command| {
            command.name == id && matches!(command.execution, markdown::CommandExecution::Prompt)
        })
    }

    pub(crate) fn command_accepts_attachments(&self, id: &str) -> bool {
        self.commands.iter().any(|command| {
            command.name == id
                && matches!(&command.execution, markdown::CommandExecution::DeepSeek(cordis)
                    if cordis.accepts_attachments())
        })
    }

    pub(crate) fn command_shell_injection_count(&self, id: &str) -> usize {
        self.commands
            .iter()
            .find(|command| command.name == id)
            .map_or(0, markdown::ImportedCommand::shell_injection_count)
    }

    /// Render only an exact skill contributed by this package generation.
    pub fn prepare_claude_skill_prompt(
        &self,
        plugin: &PluginKey,
        path: &Path,
        document: &str,
        input: &agena_plugin_sdk::CommandInvokeInput,
    ) -> agena_plugin_sdk::Result<Option<markdown::PreparedSkillPrompt>> {
        if self.format != AgentPluginFormat::ClaudeCode
            || !self.skill_dirs.iter().any(|dir| dir == path)
        {
            return Ok(None);
        }
        markdown::prepare_model_skill(self, plugin, path, document, input)
    }

    pub fn expand_claude_prompt_options(&self, text: &str) -> Result<String, String> {
        markdown::expand_user_config(text, &self.claude_prompt_options)
    }
}

struct Manifest {
    name: String,
    version: Option<String>,
    description: Option<String>,
    author: Option<String>,
    display_name: Option<String>,
    document: Map<String, Value>,
    codex_overlay: Option<Map<String, Value>>,
}

impl Manifest {
    fn parse(
        root: &Path,
        format: AgentPluginFormat,
        document: Map<String, Value>,
        diagnostics: &mut Vec<String>,
    ) -> Result<Self, String> {
        let name = required_name(&document)?.to_owned();
        let portable = format == AgentPluginFormat::AgentPluginsV1;
        if portable {
            validate_portable_manifest(&document, &name, diagnostics)?;
        }
        let codex_overlay = if portable {
            match read_package_object(root, Path::new(CODEX_MANIFEST), MAX_MANIFEST_BYTES) {
                Ok(overlay) => overlay,
                Err(error) => {
                    diagnostics.push(error);
                    None
                }
            }
        } else {
            None
        };
        let codex_extension = document
            .get("extensions")
            .and_then(|extensions| extensions.get("com.openai"))
            .and_then(Value::as_object)
            .or(codex_overlay.as_ref())
            .filter(|_| portable);
        let interface = match format {
            AgentPluginFormat::AgentPluginsV1 => codex_extension,
            AgentPluginFormat::Codex => Some(&document),
            _ => None,
        }
        .and_then(|manifest| manifest.get("interface"));
        let interface_text = |field| interface.and_then(|info| text_field(info.get(field)));
        let author = match document.get("author") {
            Some(Value::Object(author)) => text_field(author.get("name")),
            Some(value) => text_field(Some(value)),
            None => None,
        };
        Ok(Self {
            name,
            version: text_field(document.get("version")),
            description: interface_text("shortDescription")
                .or_else(|| text_field(document.get("description"))),
            author: interface_text("developerName").or(author),
            display_name: interface_text("displayName")
                .or_else(|| text_field(document.get("displayName"))),
            document,
            codex_overlay,
        })
    }
}

/// Manifest errors reject the package. Invalid components are reported
/// individually so the rest of the package can still be activated.
pub fn load_agent_plugin(path: &Path) -> Result<AgentPluginDescriptor, String> {
    load_agent_plugin_with_environment(path, &[])
}

pub fn load_agent_plugin_with_environment(
    path: &Path,
    environment: &[String],
) -> Result<AgentPluginDescriptor, String> {
    load_agent_plugin_for_ecosystem(path, environment, None)
}

pub fn load_agent_plugin_for_ecosystem(
    path: &Path,
    environment: &[String],
    ecosystem: Option<agena_plugin_sdk::configuration::AgentPluginEcosystem>,
) -> Result<AgentPluginDescriptor, String> {
    load_agent_plugin_config(path, environment, &BTreeMap::new(), true, ecosystem)
}

pub fn agent_plugin_identity(
    path: &Path,
    ecosystem: Option<agena_plugin_sdk::configuration::AgentPluginEcosystem>,
) -> Result<(String, Option<String>), String> {
    let (format, document) = discover_manifest(path, ecosystem)?;
    let manifest = Manifest::parse(path, format, document, &mut Vec::new())?;
    Ok((manifest.name, manifest.version))
}

pub(crate) fn load_agent_plugin_config(
    path: &Path,
    environment: &[String],
    user_config_env: &BTreeMap<String, String>,
    load_hook_modules: bool,
    ecosystem: Option<agena_plugin_sdk::configuration::AgentPluginEcosystem>,
) -> Result<AgentPluginDescriptor, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("cannot inspect the plugin directory: {error}"))?;
    if !metadata.is_dir() {
        return Err("the plugin path must resolve to a directory".into());
    }
    let root = path
        .canonicalize()
        .map_err(|error| format!("cannot resolve the plugin directory: {error}"))?;
    let (format, document) = discover_manifest(&root, ecosystem)?;
    let mut diagnostics = Vec::new();
    let manifest = Manifest::parse(&root, format, document, &mut diagnostics)?;
    let mut watch_paths = manifest_watch_paths(&root)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let deepseek_rows = if format == AgentPluginFormat::DeepSeekHarness {
        deepseek::diagnose_engine_requirements(&manifest.document, &mut diagnostics);
        deepseek::literal_rows(
            &root,
            &manifest.document,
            environment,
            &mut watch_paths,
            &mut diagnostics,
        )
    } else {
        Vec::new()
    };
    let deepseek_skill_roots = deepseek::skill_roots(&deepseek_rows, &mut diagnostics);
    let skill_dirs = discover_skill_dirs(
        &root,
        format,
        &manifest.document,
        deepseek_skill_roots,
        &mut watch_paths,
        &mut diagnostics,
    );
    let deepseek_contributions = deepseek_commands::discover(
        &root,
        &manifest.document,
        &deepseek_rows,
        &mut watch_paths,
        &mut diagnostics,
    );
    let mcp::ServerCatalog {
        active: mut mcp_servers,
        managed: deepseek_managed_mcp_servers,
    } = mcp::discover_servers(
        &root,
        format,
        &manifest,
        &deepseek_rows,
        &deepseek_contributions.servers,
        &mut watch_paths,
        &mut diagnostics,
    );
    let mut memvault = memvault::discover(&deepseek_rows, &mut diagnostics);
    memvault.retain(|entry| match mcp::memvault_server(entry) {
        Ok(server)
            if mcp_servers
                .iter()
                .all(|registered| registered.name != server.name)
                && deepseek_managed_mcp_servers
                    .iter()
                    .all(|managed| managed.name != server.name) =>
        {
            mcp_servers.push(server);
            true
        }
        Ok(_) => {
            diagnostics.push(format!(
                "DeepSeek Harness MemVault MCP server {:?} is already declared",
                entry.server
            ));
            false
        }
        Err(error) => {
            diagnostics.push(format!("DeepSeek Harness MemVault: {error}"));
            false
        }
    });
    let lsp_servers = match format {
        AgentPluginFormat::ClaudeCode => {
            lsp::discover(&root, &manifest, &mut watch_paths, &mut diagnostics)
        }
        AgentPluginFormat::DeepSeekHarness => {
            lsp::discover_deepseek(&deepseek_rows, &mut diagnostics)
        }
        _ => Vec::new(),
    };
    let (mut hooks, module_paths) = hooks::discover(
        &root,
        format,
        &manifest,
        &deepseek_rows,
        &mut watch_paths,
        &mut diagnostics,
    );
    let mods = if load_hook_modules && !module_paths.is_empty() {
        let options = claude_mods::options(
            manifest.document.get("userConfig"),
            user_config_env,
            &mut diagnostics,
        );
        let bundle = claude_mods::Bundle::discover(
            &root,
            &module_paths,
            options,
            &mut watch_paths,
            &mut diagnostics,
        );
        if let Some(bundle) = &bundle {
            hooks::diagnose_mod_events(bundle, &mut diagnostics);
        }
        bundle
    } else {
        None
    };
    let mut commands = Vec::new();
    let monitors = if format == AgentPluginFormat::ClaudeCode {
        commands =
            markdown::discover_claude_skills(&root, &skill_dirs, &mut hooks, &mut diagnostics);
        hooks::monitor::discover(
            &root,
            &manifest.document,
            &mut watch_paths,
            &mut diagnostics,
        )
    } else {
        Vec::new()
    };
    let workflows = if format == AgentPluginFormat::ClaudeCode {
        workflow::discover(
            &root,
            manifest.document.get("workflows"),
            &mut watch_paths,
            &mut diagnostics,
        )
    } else {
        BTreeMap::new()
    };
    let mut components = markdown::discover(
        &root,
        format,
        &manifest,
        &mut watch_paths,
        &mut diagnostics,
        &mut hooks,
    );
    commands.extend(components.commands);
    commands.extend(deepseek_contributions.commands);
    let bin_dir = if format == AgentPluginFormat::ClaudeCode {
        match package_path(&root, Path::new("bin")) {
            Ok(path) => {
                watch_paths.insert(path.clone());
                path.is_dir().then_some(path)
            }
            Err(error) => {
                diagnostics.push(error);
                None
            }
        }
    } else {
        None
    };
    describe_unmapped_components(&root, format, &manifest, &mut diagnostics);
    let mut claude_option_defaults = BTreeMap::new();
    let mut claude_prompt_options = BTreeMap::new();
    let themes = if format == AgentPluginFormat::ClaudeCode {
        let config = manifest.document.get("userConfig");
        mcp::add_user_defaults(config, &mut claude_option_defaults);
        claude_prompt_options.clone_from(&claude_option_defaults);
        for (key, option) in config.and_then(Value::as_object).into_iter().flatten() {
            if option["sensitive"] == true {
                claude_prompt_options.insert(key.clone(), "[redacted]".into());
            } else if let Some(variable) = user_config_env.get(key) {
                claude_prompt_options.remove(key);
                if let Ok(value) = mcp::read_var(variable) {
                    claude_prompt_options.insert(key.clone(), value);
                }
            }
        }
        components.roles.retain_mut(|role| {
            match markdown::expand_user_config(&role.instructions, &claude_prompt_options) {
                Ok(instructions) => {
                    role.instructions = instructions;
                    true
                }
                Err(error) => {
                    diagnostics.push(format!("{}: {error}", role.name));
                    false
                }
            }
        });
        theme::discover(&root, &manifest, &mut watch_paths, &mut diagnostics)
    } else {
        Vec::new()
    };
    Ok(AgentPluginDescriptor {
        root,
        format,
        name: manifest.name,
        version: manifest.version,
        default_enabled: format != AgentPluginFormat::ClaudeCode
            || manifest
                .document
                .get("defaultEnabled")
                .and_then(Value::as_bool)
                != Some(false),
        description: manifest.description,
        author: manifest.author,
        display_name: manifest.display_name,
        skill_dirs,
        bin_dir,
        watch_paths: watch_paths.into_iter().collect(),
        mcp_servers,
        deepseek_managed_mcp_servers,
        deepseek_programs: deepseek_contributions.programs,
        cordis_dependencies: deepseek_contributions.dependencies,
        cordis_skill_count: deepseek_contributions.skill_count,
        lsp_servers,
        themes,
        hooks,
        mods,
        mod_runtime: None,
        monitors,
        workflows,
        commands,
        roles: components.roles,
        claude_option_defaults,
        claude_prompt_options,
        memvault,
        diagnostics,
    })
}

fn describe_unmapped_components(
    root: &Path,
    format: AgentPluginFormat,
    manifest: &Manifest,
    diagnostics: &mut Vec<String>,
) {
    let exists = |relative| package_path(root, Path::new(relative)).is_ok_and(|path| path.exists());
    if matches!(
        format,
        AgentPluginFormat::AgentPluginsV1 | AgentPluginFormat::Codex
    ) {
        let openai = match format {
            AgentPluginFormat::AgentPluginsV1 => manifest
                .document
                .get("extensions")
                .and_then(|extensions| extensions.get("com.openai"))
                .and_then(Value::as_object)
                .or(manifest.codex_overlay.as_ref()),
            _ => Some(&manifest.document),
        };
        if openai.is_some_and(|extension| extension.contains_key("apps")) || exists(".app.json") {
            diagnostics.push(
                "Codex app connectors have no Agena account adapter; MCP servers import separately"
                    .into(),
            );
        }
    }
    if format == AgentPluginFormat::ClaudeCode {
        for (field, path) in [("evals", "evals"), ("settings", "settings.json")] {
            if manifest.document.contains_key(field)
                || manifest
                    .document
                    .get("experimental")
                    .is_some_and(|value| value.get(field).is_some())
                || exists(path)
            {
                diagnostics.push(format!("Claude Code {field} has no Agena adapter"));
            }
        }
    }
}

pub fn manifest_watch_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = vec![root.to_owned()];
    for relative in [
        "plugin.json",
        CODEX_MANIFEST,
        CLAUDE_MANIFEST,
        "package.json",
    ] {
        if let Ok(path) = package_path(root, Path::new(relative)) {
            paths.push(path);
        }
    }
    paths
}

pub fn validate_agent_plugin_environment(
    environment: &[String],
    user_config_env: &BTreeMap<String, String>,
) -> Result<(), String> {
    mcp::validate_environment_options(environment, user_config_env)
}

pub(crate) const DELEGATED_COMMAND_HANDLER: &str = "agent_task";
pub(crate) const CORDIS_MCP_COMMAND_HANDLER: &str = "cordis_mcp_command";
pub(crate) const CORDIS_COMMAND_HANDLER: &str = "cordis_command";
pub(crate) const FORKED_SKILL_COMMAND: &str = "$agena.native/claude-skill-fork";

/// Publish discovered components through the native Agena plugin contracts.
pub(crate) struct AgentPluginAdapter {
    manifest: PluginManifest,
    bridge: bridge::AgentBridge,
    hooks: Option<hooks::HookRunner>,
    root: PathBuf,
    package_name: String,
    claude_prompt_options: BTreeMap<String, String>,
    commands: BTreeMap<String, markdown::ImportedCommand>,
    workflows: BTreeMap<String, workflow::Workflow>,
    memvault: Vec<memvault::MemVaultBridge>,
    deepseek_programs: Vec<Arc<deepseek_commands::Program>>,
    deepseek_tool_programs: BTreeMap<String, Arc<deepseek_commands::Program>>,
    start_deepseek_observers: bool,
    host: std::sync::OnceLock<Arc<dyn HostClient>>,
}

impl AgentPluginAdapter {
    pub(crate) fn new(
        key: &PluginKey,
        package: &mut AgentPluginDescriptor,
        environment: &[String],
        user_config_env: &BTreeMap<String, String>,
        enable_hooks: bool,
    ) -> Result<Self, String> {
        mcp::validate_environment_options(environment, user_config_env)?;
        let bridge = bridge::AgentBridge::new(key, package);
        let mut manifest = PluginManifest::new(
            key.namespace(),
            key.name(),
            package.version.as_deref().unwrap_or("0.0.0"),
        );
        manifest.display_name = Some(
            package
                .display_name
                .as_deref()
                .unwrap_or(&package.name)
                .to_owned(),
        );
        manifest.summary = package.description.clone();
        manifest.authors.extend(package.author.iter().cloned());
        manifest.surface.terminal.themes = package
            .themes
            .iter()
            .cloned()
            .map(|mut theme| {
                theme.id = format!("{key}.{}", theme.id);
                theme
            })
            .collect();
        manifest.tools.push(bridge.tool());
        if !package.workflows.is_empty() {
            manifest.tools.push(workflow::tool(&package.workflows));
        }
        package.mod_runtime = if enable_hooks {
            package.mods.clone().map(|bundle| {
                let static_commands = package
                    .commands
                    .iter()
                    .map(|command| command.name.clone())
                    .chain(
                        package
                            .workflows
                            .keys()
                            .map(|name| format!("workflow.{name}")),
                    )
                    .collect();
                Arc::new(claude_mods::Mods::new(
                    bundle,
                    key,
                    &package.name,
                    &package.root,
                    environment,
                    static_commands,
                ))
            })
        } else {
            None
        };
        let hooks = (enable_hooks
            && (!package.hooks.is_empty()
                || !package.monitors.is_empty()
                || package.mods.is_some()))
        .then(|| {
            package.hooks.register(&mut manifest);
            if let Some(mods) = &package.mods {
                hooks::register_mod_events(mods, &mut manifest);
            }
            if package
                .commands
                .iter()
                .any(|cmd| matches!(cmd.execution, markdown::CommandExecution::Fork { .. }))
            {
                manifest
                    .hooks
                    .insert(agena_plugin_sdk::HookSubscription::SUBAGENT_START);
            }
            if !package.monitors.is_empty() {
                manifest.hooks.insert(
                    agena_plugin_sdk::HookSubscription::SESSION_START
                        | agena_plugin_sdk::HookSubscription::SESSION_END
                        | agena_plugin_sdk::HookSubscription::USER_PROMPT_SUBMIT
                        | agena_plugin_sdk::HookSubscription::TOOL_AFTER,
                );
            }
            hooks::HookRunner::new(package, key, environment, user_config_env)
        });
        let mut commands = BTreeMap::new();
        let memvault = if enable_hooks {
            package
                .memvault
                .iter()
                .map(|entry| memvault::MemVaultBridge::bind(entry, key))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for agent in &memvault {
            manifest.hooks.insert(agent.subscriptions());
        }
        let mut deepseek_tool_programs = BTreeMap::new();
        for program in &package.deepseek_programs {
            for tool in program.tools() {
                if manifest.tools.iter().any(|entry| entry.name == tool.name) {
                    package
                        .diagnostics
                        .push(format!("duplicate Cordis tool {:?}", tool.name));
                    continue;
                }
                let definition = tool.definition();
                if let Err(error) =
                    crate::registry::RegisteredTool::new(key.clone(), definition.clone())
                {
                    package
                        .diagnostics
                        .push(format!("invalid Cordis tool {:?}: {error}", tool.name));
                    continue;
                }
                manifest.tools.push(definition);
                deepseek_tool_programs.insert(tool.name.clone(), Arc::clone(program));
            }
        }
        if !package.deepseek_programs.is_empty() {
            manifest
                .hooks
                .insert(agena_plugin_sdk::HookSubscription::SESSION_END);
            if enable_hooks {
                manifest
                    .hooks
                    .insert(agena_plugin_sdk::HookSubscription::SESSION_START);
                if package
                    .deepseek_programs
                    .iter()
                    .any(|program| program.uses_system_prompt())
                {
                    manifest
                        .hooks
                        .insert(agena_plugin_sdk::HookSubscription::CHAT_SYSTEM_TRANSFORM);
                }
                if package
                    .deepseek_programs
                    .iter()
                    .any(|program| program.has_conditional_tools())
                {
                    manifest
                        .hooks
                        .insert(agena_plugin_sdk::HookSubscription::TOOL_DEFINITION);
                }
            }
        }
        let workflow_commands = package
            .workflows
            .values()
            .map(markdown::ImportedCommand::workflow);
        for command in package.commands.iter().cloned().chain(workflow_commands) {
            let definition = command.definition(key);
            if let Err(error) = definition.validate() {
                package
                    .diagnostics
                    .push(format!("{}: {error}", command.source()));
                continue;
            }
            if commands.contains_key(&definition.id) {
                package.diagnostics.push(format!(
                    "{}: duplicate command {:?}",
                    command.source(),
                    definition.id
                ));
                continue;
            }
            commands.insert(definition.id.clone(), command);
            manifest.commands.push(definition);
        }
        Ok(Self {
            manifest,
            bridge,
            hooks,
            commands,
            workflows: package.workflows.clone(),
            memvault,
            deepseek_programs: package.deepseek_programs.clone(),
            deepseek_tool_programs,
            start_deepseek_observers: enable_hooks,
            root: package.root.clone(),
            package_name: package.name.clone(),
            claude_prompt_options: package.claude_prompt_options.clone(),
            host: std::sync::OnceLock::new(),
        })
    }

    fn host(&self) -> agena_plugin_sdk::Result<&dyn HostClient> {
        self.host
            .get()
            .map(Arc::as_ref)
            .ok_or_else(|| PluginError::internal("Agent Plugin host is unavailable"))
    }
}

pub fn agent_plugin_data_path(workspace: &Path, plugin_id: &str) -> PathBuf {
    agena_storage_layout::StorageLayout::current()
        .plugin_storage_dir()
        .join("agent-plugins")
        .join(agena_storage_layout::workspace_key(workspace))
        .join(blake3::hash(plugin_id.as_bytes()).to_hex().as_str())
}

pub fn agent_plugin_mcp_server_name(plugin_id: &str, server_name: &str) -> String {
    let mut identity = blake3::Hasher::new();
    identity
        .update(plugin_id.as_bytes())
        .update(&[0])
        .update(server_name.as_bytes());
    let hash = identity.finalize().to_hex();
    let suffix = server_name.chars().take(90).collect::<String>();
    format!(
        "agent-plugin-{}-{}",
        &hash.as_str()[..24],
        suffix.trim_end()
    )
}

fn discover_manifest(
    root: &Path,
    ecosystem: Option<agena_plugin_sdk::configuration::AgentPluginEcosystem>,
) -> Result<(AgentPluginFormat, Map<String, Value>), String> {
    use agena_plugin_sdk::configuration::AgentPluginEcosystem;

    if ecosystem == Some(AgentPluginEcosystem::Claude)
        && let Some(manifest) =
            read_package_object(root, Path::new(CLAUDE_MANIFEST), MAX_MANIFEST_BYTES)?
    {
        return Ok((AgentPluginFormat::ClaudeCode, manifest));
    }
    let portable = read_package_object(root, Path::new("plugin.json"), MAX_MANIFEST_BYTES)?;
    if let Some(manifest) = &portable
        && manifest
            .get("$schema")
            .and_then(Value::as_str)
            .is_some_and(|schema| schema.starts_with(SCHEMA_PREFIX))
    {
        return Ok((AgentPluginFormat::AgentPluginsV1, manifest.clone()));
    }
    for (relative, format) in [
        (CODEX_MANIFEST, AgentPluginFormat::Codex),
        (CLAUDE_MANIFEST, AgentPluginFormat::ClaudeCode),
    ] {
        if let Some(manifest) = read_package_object(root, Path::new(relative), MAX_MANIFEST_BYTES)?
        {
            return Ok((format, manifest));
        }
    }
    if let Some(manifest) =
        read_package_object(root, Path::new("package.json"), MAX_MANIFEST_BYTES)?
        && deepseek::patch_paths(root, &manifest)?.is_some()
    {
        return Ok((AgentPluginFormat::DeepSeekHarness, manifest));
    }
    if let Some(manifest) = portable {
        if manifest.contains_key("$schema") {
            return Err("unsupported plugin.json schema".into());
        }
        // Open Claude Code also recognizes a root plugin.json.
        return Ok((AgentPluginFormat::ClaudeCode, manifest));
    }
    if [
        "SKILL.md",
        "skills",
        "commands",
        "agents",
        "hooks/hooks.json",
        ".mcp.json",
        ".lsp.json",
        "workflows",
        "output-styles",
        "themes",
        "monitors/monitors.json",
    ]
    .iter()
    .any(|path| package_path(root, Path::new(path)).is_ok_and(|path| path.exists()))
    {
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("plugin directory must have a UTF-8 name")?;
        return Ok((
            AgentPluginFormat::ClaudeCode,
            Map::from_iter([("name".to_owned(), Value::String(name.to_owned()))]),
        ));
    }
    Err("expected an Agent Plugins, Codex, Claude Code, or DeepSeek Harness package".into())
}

fn required_name(object: &Map<String, Value>) -> Result<&str, String> {
    object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| {
            !name.is_empty() && name.trim() == *name && !name.chars().any(char::is_control)
        })
        .ok_or_else(|| "plugin manifest requires a nonempty name".into())
}

fn validate_portable_manifest(
    object: &Map<String, Value>,
    name: &str,
    diagnostics: &mut Vec<String>,
) -> Result<(), String> {
    if object.get("$schema").and_then(Value::as_str) != Some(AGENT_PLUGIN_MANIFEST_SCHEMA) {
        return Err("unsupported Agent Plugins manifest schema".into());
    }
    if name.len() > 64
        || !name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name.as_bytes()[name.len() - 1].is_ascii_alphanumeric()
        || name.contains("--")
        || name.contains("..")
    {
        return Err("invalid Agent Plugins name; expected 1–64 lowercase ASCII letters, digits, hyphens, or periods".into());
    }
    for field in [
        "version",
        "description",
        "homepage",
        "repository",
        "license",
    ] {
        if object.get(field).is_some_and(|value| !value.is_string()) {
            return Err(format!(
                "Agent Plugins manifest field {field} must be a string"
            ));
        }
    }
    if let Some(author) = object.get("author") {
        let author = author
            .as_object()
            .ok_or("Agent Plugins author must be an object")?;
        if author.iter().any(|(key, value)| {
            !["name", "email", "url"].contains(&key.as_str()) || !value.is_string()
        }) {
            return Err(
                "Agent Plugins author accepts only string name, email, and url fields".into(),
            );
        }
    }
    if object.get("keywords").is_some_and(|value| {
        !value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string))
    }) {
        return Err("Agent Plugins keywords must be an array of strings".into());
    }
    if object
        .get("extensions")
        .is_some_and(|value| !value.is_object())
    {
        diagnostics.push("ignored non-object Agent Plugins extensions".into());
    }
    let allowed = [
        "$schema",
        "name",
        "version",
        "description",
        "author",
        "homepage",
        "repository",
        "license",
        "keywords",
        "extensions",
    ];
    for field in object.keys().filter(|key| !allowed.contains(&key.as_str())) {
        diagnostics.push(format!(
            "ignored unknown Agent Plugins manifest field {field:?}"
        ));
    }
    Ok(())
}

fn discover_skill_dirs<'a>(
    root: &Path,
    format: AgentPluginFormat,
    manifest: &'a Map<String, Value>,
    extra_roots: Vec<&'a str>,
    watch_paths: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<String>,
) -> Vec<PathBuf> {
    let mut components = vec!["./skills"];
    components.extend(extra_roots);
    if matches!(
        format,
        AgentPluginFormat::ClaudeCode | AgentPluginFormat::Codex
    ) {
        let explicit = manifest.get("skills");
        if format == AgentPluginFormat::Codex && explicit.is_some() {
            components.clear();
        }
        match explicit {
            Some(Value::String(path)) => components.push(path),
            Some(Value::Array(paths)) if paths.iter().all(Value::is_string) => {
                components.extend(paths.iter().filter_map(Value::as_str));
            }
            Some(_) => diagnostics.push("skills must be a path or an array of paths".into()),
            None if format == AgentPluginFormat::ClaudeCode && !root.join("skills").exists() => {
                if root.join("SKILL.md").is_file() {
                    components.push(".");
                }
            }
            None => {}
        }
    }

    let mut directories = BTreeSet::new();
    for component in components {
        let path = match format {
            AgentPluginFormat::DeepSeekHarness if !component.is_empty() => {
                let path = Path::new(component);
                if path.is_absolute() {
                    path.strip_prefix(root)
                        .map_err(|_| {
                            format!("DeepSeek skill path {component:?} is outside its package")
                        })
                        .and_then(|relative| package_path(root, relative))
                } else {
                    package_path(root, path)
                }
            }
            _ => manifest_path(root, component),
        };
        let path = match path {
            Ok(path) => path,
            Err(error) => {
                diagnostics.push(error);
                continue;
            }
        };
        watch_paths.insert(path.clone());
        let root_document = path.join("SKILL.md");
        if format != AgentPluginFormat::AgentPluginsV1
            && component != "./skills"
            && regular_file(&root_document)
        {
            watch_paths.insert(root_document);
            directories.insert(path);
            continue;
        }
        let entries = match std::fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                diagnostics.push(format!(
                    "cannot read skills directory {component:?}: {error}"
                ));
                continue;
            }
        };
        for (index, entry) in entries.enumerate() {
            if index == MAX_COMPONENT_ENTRIES {
                diagnostics.push(format!(
                    "too many entries in skills directory {component:?}"
                ));
                break;
            }
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.file_type().is_ok_and(|file_type| file_type.is_dir()) {
                continue;
            }
            let path = entry.path();
            let document = path.join("SKILL.md");
            watch_paths.insert(document.clone());
            if regular_file(&document) {
                directories.insert(path);
            }
        }
    }
    directories.into_iter().collect()
}

fn regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
}

fn text_field(value: Option<&Value>) -> Option<String> {
    value?
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Check every existing component. Missing paths are returned for watch lists;
/// a caller about to read or execute still has to require the expected type.
fn package_path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut path = root.to_owned();
    for component in relative.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(name) if !name.to_string_lossy().contains(['\\', '\0']) => {
                path.push(name);
            }
            _ => {
                return Err(format!(
                    "invalid plugin component path {:?}",
                    relative.display()
                ));
            }
        }
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_symlink() => {
                return Err(format!(
                    "plugin component path {relative:?} contains a symlink"
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot inspect plugin component {relative:?}: {error}"
                ));
            }
        }
    }
    Ok(path)
}

fn manifest_path(root: &Path, raw: &str) -> Result<PathBuf, String> {
    if raw != "." && !raw.starts_with("./") {
        return Err("expected a local component path beginning with ./".into());
    }
    package_path(root, Path::new(raw))
}

pub(crate) fn read_package_object(
    root: &Path,
    relative: &Path,
    limit: u64,
) -> Result<Option<Map<String, Value>>, String> {
    let Some(bytes) = read_package_bytes(root, relative, limit)? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        format!(
            "invalid JSON object in {relative:?} at line {}, column {}",
            error.line(),
            error.column()
        )
    })
}

fn read_package_bytes(root: &Path, relative: &Path, limit: u64) -> Result<Option<Vec<u8>>, String> {
    let path = package_path(root, relative)?;
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot inspect {relative:?}: {error}")),
    };
    if !metadata.is_file() || metadata.len() > limit {
        return Err(format!(
            "{relative:?} must be a regular file of at most {limit} bytes"
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("cannot read {relative:?}: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{relative:?} exceeds {limit} bytes"));
    }
    Ok(Some(bytes))
}
