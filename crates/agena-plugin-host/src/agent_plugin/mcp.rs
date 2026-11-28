//! Translate the package formats into Agena's MCP transport inputs.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::Duration,
};

use serde_json::{Map, Value};
use url::{Host, Url};

use super::{
    AgentPluginFormat, Manifest, manifest_path,
    mcp_bundle::{self, Bundle},
    package_path, read_package_object,
};

const MCP_SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";
const MAX_MCP_BYTES: u64 = 4 * 1024 * 1024;
const CLIENT_HEADERS: &[&str] = &[
    "accept",
    "connection",
    "content-encoding",
    "content-length",
    "content-type",
    "host",
    "last-event-id",
    "mcp-protocol-version",
    "mcp-session-id",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "user-agent",
];

#[derive(Clone)]
pub struct AgentPluginMcpServer {
    pub name: String,
    pub settings: AgentPluginMcpSettings,
    format: AgentPluginFormat,
    config: AgentPluginMcpConfig,
    options: Arc<BTreeMap<String, String>>,
    bundle: Option<Bundle>,
    credential_templates: bool,
    channel: Option<super::claude_channels::Channel>,
}

#[derive(Clone)]
pub struct DeepSeekManagedMcpServer {
    pub name: String,
    pub enabled: bool,
    /// Whether the entry came from a Cordis module's registration.
    pub registered: bool,
    /// Missing if the declaration failed validation during package discovery.
    pub server: Option<AgentPluginMcpServer>,
}

#[derive(Default)]
pub(super) struct ServerCatalog {
    pub active: Vec<AgentPluginMcpServer>,
    pub managed: Vec<DeepSeekManagedMcpServer>,
}

#[derive(Clone, Default)]
pub struct AgentPluginMcpSettings {
    pub startup_timeout: Option<Duration>,
    pub tool_timeout: Option<Duration>,
    pub close_timeout: Option<Duration>,
    pub deepseek_reconnect: Option<DeepSeekReconnectPolicy>,
    pub enabled_tools: Option<BTreeSet<String>>,
    pub disabled_tools: BTreeSet<String>,
    /// Description limit when the importing harness defers tool registration.
    pub on_demand_description_limit: Option<usize>,
}

#[derive(Clone, Copy)]
pub struct DeepSeekReconnectPolicy {
    pub enabled: bool,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub max_attempts: u64,
}

impl DeepSeekReconnectPolicy {
    fn parse(value: Option<&Value>) -> Result<Self, String> {
        let empty = Value::Object(Map::new());
        let config = value.unwrap_or(&empty);
        let object = config.as_object().ok_or("reconnect must be an object")?;
        for field in object.keys() {
            if !matches!(field.as_str(), "enabled" | "initialDelayMs" | "maxDelayMs" | "maxAttempts") {
                return Err(format!("reconnect.{field} is not a reconnect option"));
            }
        }
        let delay = |name: &str, default| {
            config.get(name).map_or(Ok(Duration::from_millis(default)), |value| {
                value.as_f64().filter(|value| value.is_finite() && *value > 0.0 && *value <= f64::from(i32::MAX))
                    .map(|value| Duration::from_secs_f64(value / 1_000.0).max(Duration::from_millis(1)))
                    .ok_or_else(|| format!("reconnect.{name} must be a positive timer duration in milliseconds"))
            })
        };
        let initial_delay = delay("initialDelayMs", 500)?;
        let max_delay = delay("maxDelayMs", 30_000)?;
        if initial_delay > max_delay {
            return Err("reconnect.initialDelayMs must be less than or equal to maxDelayMs".into());
        }
        let enabled = config.get("enabled").map_or(Ok(true), |value| {
            value.as_bool().ok_or("reconnect.enabled must be a boolean")
        })?;
        let max_attempts = config.get("maxAttempts").map_or(Ok(10), |value| {
            value.as_u64().filter(|value| *value > 0)
                .ok_or("reconnect.maxAttempts must be a positive integer")
        })?;
        Ok(Self { enabled, initial_delay, max_delay, max_attempts })
    }
}

#[derive(Clone)]
enum AgentPluginMcpConfig {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<String>,
        env_vars: Vec<String>,
    },
    MemVaultSpawn {
        binary: String,
        database: String,
        embedding_provider: String,
    },
    Http {
        url: String,
        transport: AgentPluginMcpTransport,
        headers: BTreeMap<String, String>,
        env_headers: BTreeMap<String, String>,
        bearer_env: Option<String>,
        headers_helper: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPluginMcpTransport {
    Streamable,
    Sse,
    WebSocket,
}

pub enum ResolvedAgentPluginMcp {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: PathBuf,
        bundle_root: Option<PathBuf>,
        writable_directories: Vec<PathBuf>,
        credential_values: Vec<String>,
    },
    Http {
        url: String,
        transport: AgentPluginMcpTransport,
        headers: BTreeMap<String, String>,
        bearer_env: Option<String>,
        headers_helper: Option<ResolvedMcpHeaderHelper>,
        credential_values: Vec<String>,
    },
}

pub struct ResolvedMcpHeaderHelper {
    pub command: String,
    pub cwd: PathBuf,
    pub server_name: String,
    pub display_url: String,
}

pub struct AgentPluginContext<'a> {
    pub root: &'a Path,
    pub data: &'a Path,
    pub workspace: &'a Path,
    pub environment: &'a [String],
    pub user_config_env: &'a BTreeMap<String, String>,
}

impl AgentPluginMcpServer {
    pub fn is_stdio(&self) -> bool {
        matches!(
            self.config,
            AgentPluginMcpConfig::Stdio { .. } | AgentPluginMcpConfig::MemVaultSpawn { .. }
        )
    }

    pub fn claude_channel_definition(
        &self,
        plugin_id: &str,
        environment: &BTreeMap<String, String>,
        registered: bool,
    ) -> Option<agena_domain::claude_channels::ClaudeChannelDefinition> {
        if self.format != AgentPluginFormat::ClaudeCode || !self.is_stdio() {
            return None;
        }
        let (display_name, user_options) = self.channel.as_ref().map_or_else(
            || (self.name.clone(), Vec::new()),
            |channel| (channel.display_name().to_owned(), channel.options(environment)),
        );
        Some(agena_domain::claude_channels::ClaudeChannelDefinition {
            server: format!("plugin:{plugin_id}:{}", self.name),
            display_name,
            plugin_id: plugin_id.to_owned(),
            declared: self.channel.is_some(),
            registered,
            user_options,
        })
    }

    pub fn resolve(
        &self,
        context: &AgentPluginContext<'_>,
    ) -> Result<ResolvedAgentPluginMcp, String> {
        let mut credentials = BTreeMap::new();
        let mut channel_options = None;
        let mut channel_environment = None;
        if let Some(channel) = &self.channel {
            let mut options = self.options.as_ref().clone();
            let mut environment = context.user_config_env.clone();
            channel.resolve(&mut environment, &mut options, &mut credentials)?;
            channel_options = Some(options);
            channel_environment = Some(environment);
        }
        let bundle_root = self
            .bundle
            .as_ref()
            .map(|bundle| bundle.materialize(context.root))
            .transpose()?;
        let context = &AgentPluginContext {
            root: bundle_root.as_deref().unwrap_or(context.root),
            data: context.data,
            workspace: context.workspace,
            environment: context.environment,
            user_config_env: channel_environment.as_ref().unwrap_or(context.user_config_env),
        };
        let mut expand = |value: &str| {
            if self.credential_templates {
                expand_deepseek_credentials(value, |name| {
                    if let Some(value) = credentials.get(name) {
                        return Ok::<String, String>(String::clone(value));
                    }
                    let value = allowed_var(name, context.environment)?;
                    credentials.insert(name.to_owned(), value.clone());
                    Ok(value)
                })
            } else {
                expand_template(
                    value,
                    context,
                    self.format,
                    channel_options.as_ref().unwrap_or(self.options.as_ref()),
                )
            }
        };
        match &self.config {
            AgentPluginMcpConfig::Stdio {
                command,
                args,
                env,
                cwd,
                env_vars,
            } => {
                let command = if command.starts_with("./") {
                    executable_path(manifest_path(context.root, command)?)?
                } else if self.format == AgentPluginFormat::ClaudeCode
                    && let Some(relative) = path_reference(command, "CLAUDE_PLUGIN_ROOT")
                {
                    executable_path(package_path(context.root, Path::new(relative))?)?
                } else if self.format == AgentPluginFormat::ClaudeCode
                    && let Some(relative) = path_reference(command, "CLAUDE_PLUGIN_DATA")
                        .or_else(|| path_reference(command, "CLAUDE_CONFIG_DIR"))
                {
                    executable_path(package_path(context.data, Path::new(relative))?)?
                } else {
                    let command = if self.format == AgentPluginFormat::ClaudeCode {
                        expand(command)?
                    } else {
                        command.clone()
                    };
                    if command.trim().is_empty()
                        || command.chars().any(char::is_whitespace)
                        || command.contains(['/', '\\', '\0', ':'])
                    {
                        return Err(
                            "MCP command must be a bare executable or a path inside the plugin"
                                .into(),
                        );
                    }
                    command
                };
                let args = args
                    .iter()
                    .map(|arg| expand(arg))
                    .collect::<Result<_, _>>()?;
                let mut exported = BTreeMap::new();
                for (name, value) in env {
                    if env_vars.iter().any(|key| {
                        same_environment_name(key, name) && value == &format!("${{{key}}}")
                    }) {
                        continue;
                    }
                    exported.insert(name.clone(), expand(value)?);
                }
                for name in env_vars {
                    exported.insert(name.clone(), allowed_var(name, context.environment)?);
                }
                for (name, path) in [("PLUGIN_ROOT", context.root), ("PLUGIN_DATA", context.data)] {
                    exported.insert(name.to_owned(), path_string(path)?);
                }
                if self.format == AgentPluginFormat::ClaudeCode {
                    for (name, path) in [
                        ("CLAUDE_PLUGIN_ROOT", context.root),
                        ("CLAUDE_PLUGIN_DATA", context.data),
                        ("CLAUDE_PROJECT_DIR", context.workspace),
                    ] {
                        exported.insert(name.to_owned(), path_string(path)?);
                    }
                    if !exported.keys().any(|name| same_environment_name(name, "CLAUDE_CONFIG_DIR")) {
                        exported.insert("CLAUDE_CONFIG_DIR".into(), path_string(context.data)?);
                    }
                }
                let cwd = resolve_cwd(cwd.as_deref(), context, self.format)?;
                Ok(ResolvedAgentPluginMcp::Stdio {
                    command,
                    args,
                    env: exported,
                    cwd,
                    bundle_root,
                    writable_directories: Vec::new(),
                    credential_values: credentials.into_values().collect(),
                })
            }
            AgentPluginMcpConfig::MemVaultSpawn {
                binary,
                database,
                embedding_provider,
            } => {
                let (command, database) = super::memvault::spawn_paths(context, binary, database)?;
                let database_parent = database
                    .parent()
                    .expect("resolved file path has a parent")
                    .to_owned();
                Ok(ResolvedAgentPluginMcp::Stdio {
                    command,
                    args: vec![
                        "--transport".into(),
                        "stdio".into(),
                        "--db".into(),
                        path_string(&database)?,
                    ],
                    env: BTreeMap::from([(
                        "MEMVAULT_EMBEDDING_PROVIDER".into(),
                        embedding_provider.clone(),
                    )]),
                    cwd: context.workspace.to_owned(),
                    bundle_root: None,
                    writable_directories: vec![database_parent],
                    credential_values: Vec::new(),
                })
            }
            AgentPluginMcpConfig::Http {
                url,
                transport,
                headers,
                env_headers,
                bearer_env,
                headers_helper,
            } => {
                let claude = self.format == AgentPluginFormat::ClaudeCode;
                let headers_helper = headers_helper.as_ref().map(|command| {
                    static TEMPLATE: LazyLock<regex::Regex> = LazyLock::new(|| {
                        regex::Regex::new(r"\$\{[^}]+\}").expect("valid template regex")
                    });
                    ResolvedMcpHeaderHelper {
                        command: command.clone(),
                        cwd: context.root.to_path_buf(),
                        server_name: self.name.clone(),
                        display_url: TEMPLATE.replace_all(url, "REDACTED").into_owned(),
                    }
                });
                let configured_url = url;
                let resolved = if claude || self.credential_templates {
                    expand(configured_url)?
                } else {
                    configured_url.clone()
                };
                let url = remote_url(&resolved, self.format, *transport)?;
                if self.credential_templates
                    && url.origin() != deepseek_credential_origin(configured_url, *transport)?
                {
                    return Err(
                        "DeepSeek MCP credential expansion changed the endpoint origin".into(),
                    );
                }
                let mut output = BTreeMap::new();
                for (name, value) in headers {
                    let value = if claude || self.credential_templates {
                        expand(value)?
                    } else {
                        value.clone()
                    };
                    output.insert(name.clone(), value);
                }
                for (name, env_name) in env_headers {
                    output.insert(name.clone(), allowed_var(env_name, context.environment)?);
                }
                if let Some(name) = bearer_env
                    && !context
                        .environment
                        .iter()
                        .any(|allowed| same_environment_name(allowed, name))
                {
                    return Err(format!(
                        "MCP bearer environment variable {name:?} has not been enabled for this package"
                    ));
                }
                Ok(ResolvedAgentPluginMcp::Http {
                    url: url.to_string(),
                    transport: *transport,
                    headers: output,
                    bearer_env: bearer_env.clone(),
                    headers_helper,
                    credential_values: credentials.into_values().collect(),
                })
            }
        }
    }

}

fn expand_deepseek_credentials(
    value: &str,
    mut resolve: impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut result = String::new();
    let mut rest = value;
    while let Some((prefix, token)) = rest.split_once("${") {
        let (name, tail) = token
            .split_once('}')
            .ok_or("unterminated DeepSeek MCP credential template")?;
        let name = name
            .strip_prefix("credential:")
            .filter(|name| valid_env_name(name))
            .ok_or("expected a DeepSeek MCP credential template ${credential:NAME}")?;
        result.push_str(prefix);
        result.push_str(&resolve(name)?);
        rest = tail;
    }
    result.push_str(rest);
    Ok(result)
}

fn deepseek_credential_origin(
    url: &str,
    transport: AgentPluginMcpTransport,
) -> Result<url::Origin, String> {
    let (scheme, endpoint) = url.split_once("://").ok_or("invalid DeepSeek MCP URL")?;
    let authority = endpoint.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty()
        || authority.contains(['\\', '\t', '\r', '\n'])
        || scheme.contains("${")
        || authority.contains("${")
    {
        return Err("DeepSeek MCP credential templates require a static endpoint origin".into());
    }
    let probe = expand_deepseek_credentials(url, |_| Ok("credential".to_owned()))?;
    Ok(remote_url(&probe, AgentPluginFormat::DeepSeekHarness, transport)?.origin())
}

pub(super) fn expand_template(
    value: &str,
    context: &AgentPluginContext<'_>,
    format: AgentPluginFormat,
    options: &BTreeMap<String, String>,
) -> Result<String, String> {
    let mut result = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let tail = &rest[start + 2..];
        let Some(end) = tail.find('}') else {
            break;
        };
        result.push_str(&rest[..start]);
        let name = &tail[..end];
        match name {
            "PLUGIN_ROOT" => result.push_str(&path_string(context.root)?),
            "PLUGIN_DATA" => result.push_str(&path_string(context.data)?),
            "CLAUDE_PLUGIN_ROOT" if format == AgentPluginFormat::ClaudeCode => {
                result.push_str(&path_string(context.root)?);
            }
            "CLAUDE_PLUGIN_DATA" if format == AgentPluginFormat::ClaudeCode => {
                result.push_str(&path_string(context.data)?);
            }
            "CLAUDE_CONFIG_DIR" if format == AgentPluginFormat::ClaudeCode => {
                result.push_str(&path_string(context.data)?);
            }
            "CLAUDE_PROJECT_DIR" if format == AgentPluginFormat::ClaudeCode => {
                result.push_str(&path_string(context.workspace)?);
            }
            _ if format == AgentPluginFormat::ClaudeCode => {
                if let Some(key) = name.strip_prefix("user_config.") {
                    let value = if let Some(variable) = context.user_config_env.get(key) {
                        read_var(variable)?
                    } else {
                        options.get(key).cloned().ok_or_else(|| {
                            format!("Claude user_config option {key:?} needs a value")
                        })?
                    };
                    result.push_str(&value);
                } else {
                    let (variable, default) = name
                        .split_once(":-")
                        .map_or((name, None), |(variable, default)| {
                            (variable, Some(default))
                        });
                    match allowed_var(variable, context.environment) {
                        Ok(value) if !value.is_empty() || default.is_none() => {
                            result.push_str(&value)
                        }
                        _ if default.is_some() => result.push_str(default.unwrap_or_default()),
                        Err(error) => return Err(error),
                        Ok(_) => {}
                    }
                }
            }
            _ => result.push_str(&rest[start..start + 3 + end]),
        }
        rest = &tail[end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

pub(super) fn discover_servers(
    root: &Path,
    format: AgentPluginFormat,
    manifest: &Manifest,
    deepseek_rows: &[(String, serde_yaml::Value)],
    deepseek_registered: &[(String, Vec<(String, serde_yaml::Value)>)],
    watch: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<String>,
) -> ServerCatalog {
    let mut sources = Sources {
        root,
        watch,
        diagnostics,
        allow_bundles: format == AgentPluginFormat::ClaudeCode,
        bundles: BTreeMap::new(),
    };
    let mut entries = BTreeMap::new();
    let mut overlay = BTreeMap::new();
    match format {
        AgentPluginFormat::AgentPluginsV1 => {
            if let Some(mut map) = sources.read("./mcp.json") {
                if map.remove("$schema").as_ref().and_then(Value::as_str) == Some(MCP_SCHEMA)
                    && map.len() == 1
                    && let Some(Value::Object(servers)) = map.remove("mcpServers")
                {
                    entries.extend(servers);
                } else {
                    sources.diagnostics.push(
                        "mcp.json must declare the Agent Plugins v1 MCP schema and mcpServers"
                            .into(),
                    );
                }
            }
            if let Some(manifest) = &manifest.codex_overlay {
                sources.add_codex(manifest, &mut overlay);
            }
        }
        AgentPluginFormat::Codex => sources.add_codex(&manifest.document, &mut entries),
        AgentPluginFormat::ClaudeCode | AgentPluginFormat::DeepSeekHarness => {
            if let Some(map) = sources
                .read("./.mcp.json")
                .or_else(|| sources.read("./mcp.json"))
            {
                sources.add_map(map, &mut entries);
            }
            if let Some(value) = manifest.document.get("mcpServers") {
                let values = value
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_else(|| std::slice::from_ref(value));
                for item in values {
                    sources.add(item, &mut entries);
                }
            }
        }
    }

    let mut managed = Vec::new();
    let mut managed_positions = BTreeMap::new();
    if format == AgentPluginFormat::DeepSeekHarness {
        let deepseek = super::deepseek::literal_mcp_servers(
            deepseek_rows,
            deepseek_registered,
            sources.diagnostics,
        );
        for entry in deepseek.managed {
            // A generic MCP manifest cannot replace a manager-owned declaration.
            entries.remove(&entry.name);
            managed_positions.insert(entry.name.clone(), managed.len());
            managed.push(entry);
        }
        for (name, config) in deepseek.configs {
            entries.entry(name).or_insert(config);
        }
    }

    let mut options = BTreeMap::new();
    let mut channels = if format == AgentPluginFormat::ClaudeCode {
        add_user_defaults(manifest.document.get("userConfig"), &mut options);
        super::claude_channels::discover(manifest.document.get("channels"), sources.diagnostics)
    } else {
        BTreeMap::new()
    };
    let options = Arc::new(options);
    let active = entries
        .into_iter()
        .filter(|(_, value)| {
            format == AgentPluginFormat::AgentPluginsV1
                || (value.get("enabled") != Some(&Value::Bool(false))
                    && value.get("disabled") != Some(&Value::Bool(true)))
        })
        .filter_map(|(name, value)| {
            let codex_settings = overlay.get(&name);
            let parsed = parse_server(&name, &value, format).and_then(|config| {
                let settings = match codex_settings {
                    Some(value) => parse_settings(AgentPluginFormat::Codex, value),
                    None => parse_settings(format, &value),
                }?;
                Ok((config, settings))
            });
            let (mut config, settings) = match parsed {
                Ok(parsed) => parsed,
                Err(error) => {
                    sources
                        .diagnostics
                        .push(format!("MCP server {name:?}: {error}"));
                    return None;
                }
            };
            if let AgentPluginMcpConfig::Stdio { env_vars, .. } = &mut config
                && let Some(value) = codex_settings.filter(|value| value.get("command").is_some())
            {
                match local_env_variables(value) {
                    Ok(vars) => *env_vars = vars,
                    Err(error) => sources
                        .diagnostics
                        .push(format!("Codex MCP overlay for {name:?}: {error}")),
                }
            }
            let bundle = sources.bundles.remove(&name);
            let server_options = if let Some(bundle) = &bundle {
                let mut defaults = options.as_ref().clone();
                defaults.extend(bundle.options.clone());
                Arc::new(defaults)
            } else {
                Arc::clone(&options)
            };
            let channel = match channels.remove(&name) {
                Some(_) if !matches!(&config, AgentPluginMcpConfig::Stdio { .. }) => {
                    sources.diagnostics.push(format!("Claude channel {name:?} requires a stdio MCP server"));
                    None
                }
                channel => channel,
            };
            let server = AgentPluginMcpServer {
                name,
                settings,
                format,
                config,
                options: server_options,
                bundle,
                credential_templates: format == AgentPluginFormat::DeepSeekHarness
                    && value.get("expandTemplates") == Some(&Value::Bool(true)),
                channel,
            };
            if let Some(&index) = managed_positions.get(&server.name) {
                let entry = &mut managed[index];
                entry.server = Some(server.clone());
                if !entry.enabled {
                    return None;
                }
            }
            Some(server)
        })
        .collect();
    for (name, _) in channels {
        sources.diagnostics.push(format!(
            "Claude channel {name:?} must name an enabled, valid MCP server in the plugin",
        ));
    }
    ServerCatalog { active, managed }
}

pub(super) fn memvault_server(
    definition: &super::memvault::MemVault,
) -> Result<AgentPluginMcpServer, String> {
    let format = AgentPluginFormat::DeepSeekHarness;
    let config = match &definition.backend {
        super::memvault::Backend::Attach(url) => parse_server(
            &definition.server,
            &serde_json::json!({"type":"streamable-http", "url":url}),
            format,
        )?,
        super::memvault::Backend::Spawn {
            binary,
            database,
            embedding_provider,
        } => AgentPluginMcpConfig::MemVaultSpawn {
            binary: binary.clone(),
            database: database.clone(),
            embedding_provider: embedding_provider.clone(),
        },
    };
    Ok(AgentPluginMcpServer {
        name: definition.server.clone(),
        settings: AgentPluginMcpSettings::default(),
        format,
        config,
        options: Arc::default(),
        bundle: None,
        credential_templates: false,
        channel: None,
    })
}

fn parse_settings(
    format: AgentPluginFormat,
    value: &Value,
) -> Result<AgentPluginMcpSettings, String> {
    let mut settings = AgentPluginMcpSettings::default();
    match format {
        AgentPluginFormat::Codex => {
            settings.startup_timeout = match timeout_field(value, "startup_timeout_sec", 1.0)? {
                Some(timeout) => Some(timeout),
                None => timeout_field(value, "startup_timeout_ms", 1_000.0)?,
            };
            settings.tool_timeout = timeout_field(value, "tool_timeout_sec", 1.0)?;
            let tools = |field: &str| -> Result<Option<BTreeSet<String>>, String> {
                value
                    .get(field)
                    .filter(|value| !value.is_null())
                    .map(|value| {
                        serde_json::from_value(value.clone())
                            .map_err(|_| format!("{field} must be a list of tool names"))
                    })
                    .transpose()
            };
            settings.enabled_tools = tools("enabled_tools")?;
            settings.disabled_tools = tools("disabled_tools")?.unwrap_or_default();
        }
        AgentPluginFormat::ClaudeCode => {
            // Claude ignores values that are not whole milliseconds of at
            // least one second.
            settings.tool_timeout = value["timeout"]
                .as_f64()
                .filter(|ms| *ms >= 1_000.0 && ms.fract() == 0.0)
                .and_then(|ms| Duration::try_from_secs_f64(ms / 1_000.0).ok());
        }
        AgentPluginFormat::DeepSeekHarness => {
            settings.startup_timeout = deepseek_timer(value, "startupTimeoutMs")?;
            settings.tool_timeout = timeout_field(value, "toolCallTimeoutMs", 1_000.0)?;
            settings.close_timeout = deepseek_timer(value, "closeTimeoutMs")?;
            settings.deepseek_reconnect = Some(DeepSeekReconnectPolicy::parse(value.get("reconnect"))?);
            let description_limit = value
                .get("catalogDescriptionMaxLength")
                .map(|value| {
                    value
                        .as_u64()
                        .and_then(|limit| usize::try_from(limit).ok())
                        .filter(|limit| *limit >= 3)
                        .ok_or("catalogDescriptionMaxLength must be an integer of at least 3")
                })
                .transpose()?
                .unwrap_or(160);
            match value.get("toolLoading").map(Value::as_str) {
                None | Some(Some("eager")) => {}
                Some(Some("on-demand")) => {
                    settings.on_demand_description_limit = Some(description_limit);
                }
                Some(_) => return Err("toolLoading must be eager or on-demand".into()),
            }
        }
        AgentPluginFormat::AgentPluginsV1 => {}
    }
    Ok(settings)
}

fn deepseek_timer(config: &Value, field: &str) -> Result<Option<Duration>, String> {
    // The native manager uses JavaScript timers for startup and cleanup.
    let timeout = timeout_field(config, field, 1_000.0)?;
    if timeout.is_some_and(|duration| {
        !(Duration::from_millis(1)..=Duration::from_millis(i32::MAX as u64)).contains(&duration)
    }) {
        return Err(format!(
            "{field} is outside the supported DeepSeek timer range"
        ));
    }
    Ok(timeout)
}

fn timeout_field(
    config: &Value,
    field: &str,
    units_per_second: f64,
) -> Result<Option<Duration>, String> {
    config
        .get(field)
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_f64()
                .and_then(|value| Duration::try_from_secs_f64(value / units_per_second).ok())
                .ok_or_else(|| format!("{field} must specify a valid duration"))
        })
        .transpose()
}

struct Sources<'a> {
    root: &'a Path,
    watch: &'a mut BTreeSet<PathBuf>,
    diagnostics: &'a mut Vec<String>,
    allow_bundles: bool,
    bundles: BTreeMap<String, Bundle>,
}

impl Sources<'_> {
    fn read(&mut self, raw: &str) -> Option<Map<String, Value>> {
        let result = (|| {
            let path = manifest_path(self.root, raw)?;
            self.watch.insert(path.clone());
            read_package_object(
                self.root,
                path.strip_prefix(self.root)
                    .expect("component under package root"),
                MAX_MCP_BYTES,
            )
        })();
        match result {
            Ok(map) => map,
            Err(error) => {
                self.diagnostics.push(error);
                None
            }
        }
    }

    fn add_map(&mut self, mut map: Map<String, Value>, entries: &mut BTreeMap<String, Value>) {
        map.remove("$schema");
        let servers = match map.remove("mcpServers") {
            Some(Value::Object(servers)) => servers,
            None => map,
            Some(_) => {
                self.diagnostics.push("mcpServers must be an object".into());
                return;
            }
        };
        for name in servers.keys() {
            self.bundles.remove(name);
        }
        entries.extend(servers);
    }

    fn add(&mut self, source: &Value, entries: &mut BTreeMap<String, Value>) {
        match source {
            Value::Object(map) => self.add_map(map.clone(), entries),
            Value::String(raw) if self.allow_bundles && mcp_bundle::is_bundle_path(raw) => {
                let result = (|| {
                    let path = manifest_path(self.root, raw).map_err(|_| {
                        "MCP bundle URLs must be downloaded by the Agena marketplace installer"
                            .to_owned()
                    })?;
                    self.watch.insert(path.clone());
                    let relative = path
                        .strip_prefix(self.root)
                        .expect("MCP bundle inside plugin root");
                    Bundle::read(self.root, relative)
                })();
                match result {
                    Ok((name, config, bundle)) => {
                        self.bundles.insert(name.clone(), bundle);
                        entries.insert(name, config);
                    }
                    Err(error) => self.diagnostics.push(format!("{raw}: {error}")),
                }
            }
            Value::String(raw) if raw.starts_with("./") && raw.ends_with(".json") => {
                if let Some(map) = self.read(raw) {
                    self.add_map(map, entries);
                } else {
                    self.diagnostics
                        .push(format!("could not load declared MCP file {raw:?}"));
                }
            }
            _ => self.diagnostics.push(
                "MCP declaration needs a local JSON file or inline map for direct import".into(),
            ),
        }
    }

    fn add_codex(&mut self, manifest: &Map<String, Value>, entries: &mut BTreeMap<String, Value>) {
        if let Some(value @ Value::Object(_)) = manifest.get("mcpServers") {
            self.add(value, entries);
            return;
        }
        if let Some(map) = self.read("./.mcp.json") {
            self.add_map(map, entries);
        }
        if let Some(value) = manifest.get("mcpServers") {
            if value.as_str() != Some("./.mcp.json") {
                self.add(value, entries);
            }
        } else if entries.is_empty()
            && let Some(map) = self.read("./mcp.json")
        {
            self.add_map(map, entries);
        }
    }
}

fn parse_server(
    name: &str,
    value: &Value,
    format: AgentPluginFormat,
) -> Result<AgentPluginMcpConfig, String> {
    let portable = format == AgentPluginFormat::AgentPluginsV1;
    if name.trim() != name || name.is_empty() || name.chars().any(char::is_control) {
        return Err("invalid server name".into());
    }
    let object = value.as_object().ok_or("server must be an object")?;
    let transport = match object.get("type") {
        Some(Value::String(transport)) => transport.as_str(),
        None if !portable && object.contains_key("command") => "stdio",
        None if !portable && object.contains_key("url") => "http",
        _ => return Err("server requires a transport type".into()),
    };
    let deepseek_templates = if format == AgentPluginFormat::DeepSeekHarness {
        match object.get("expandTemplates") {
            Some(value) => value
                .as_bool()
                .ok_or("DeepSeek MCP expandTemplates must be a boolean")?,
            None => false,
        }
    } else {
        false
    };

    if portable {
        let fields: &[&str] = match transport {
            "stdio" => &["type", "command", "args", "env", "cwd"],
            "streamable-http" | "sse" => &["type", "url", "headers"],
            _ => return Err("unknown Agent Plugins MCP transport".into()),
        };
        if object
            .iter()
            .any(|(name, value)| !fields.contains(&name.as_str()) || value.is_null())
        {
            return Err("unexpected field or null value for the declared transport".into());
        }
    }
    if transport == "stdio" {
        if object.contains_key("url") {
            return Err("stdio server declares a remote URL".into());
        }
        let command = string_field(object, "command")?.ok_or("missing MCP command")?;
        let args: Vec<String> = decode_field(object, "args")?;
        let env: BTreeMap<String, String> = decode_field(object, "env")?;
        let cwd = string_field(object, "cwd")?;
        if deepseek_templates {
            if command.contains("${") || cwd.as_ref().is_some_and(|cwd| cwd.contains("${")) {
                return Err("DeepSeek MCP executable and working directory must be static".into());
            }
            for field in args.iter().chain(env.values()) {
                expand_deepseek_credentials(field, |_| Ok(String::new()))?;
            }
        }
        for name in env.keys() {
            if [
                "PLUGIN_ROOT",
                "PLUGIN_DATA",
                "CLAUDE_PLUGIN_ROOT",
                "CLAUDE_PLUGIN_DATA",
                "CLAUDE_PROJECT_DIR",
            ]
            .iter()
            .any(|reserved| same_environment_name(reserved, name))
            {
                return Err("server env attempts to replace a reserved plugin variable".into());
            }
        }
        let env_vars = match format {
            AgentPluginFormat::Codex => local_env_variables(value)?,
            _ => Vec::new(),
        };
        return Ok(AgentPluginMcpConfig::Stdio {
            command,
            args,
            env,
            cwd,
            env_vars,
        });
    }
    let transport = match transport {
        "http" | "streamable-http" => AgentPluginMcpTransport::Streamable,
        "sse" => AgentPluginMcpTransport::Sse,
        "ws" | "websocket" if format == AgentPluginFormat::ClaudeCode => {
            AgentPluginMcpTransport::WebSocket
        }
        _ => {
            return Err(format!(
                "transport {transport:?} has no Agena transport adapter"
            ));
        }
    };
    if object.contains_key("command") {
        return Err("HTTP server declares a stdio command".into());
    }
    let headers_helper = string_field(object, "headersHelper")?;
    if let Some(helper) = &headers_helper
        && (format != AgentPluginFormat::ClaudeCode
            || helper.len() > 64 * 1024
            || helper.trim().is_empty()
            || helper.contains('\0')
            || helper.contains("${user_config."))
    {
        return Err(
            "invalid headersHelper; use a Claude shell command without user_config expansion"
                .into(),
        );
    }
    let mut headers: BTreeMap<String, String> = decode_field(object, "headers")?;
    let (env_headers, bearer_env) = if format == AgentPluginFormat::Codex {
        headers.extend(decode_field::<BTreeMap<String, String>>(
            object,
            "http_headers",
        )?);
        (
            decode_field(object, "env_http_headers")?,
            string_field(object, "bearer_token_env_var")?,
        )
    } else {
        (BTreeMap::new(), None)
    };
    let mut names = BTreeSet::new();
    for name in headers.keys().chain(env_headers.keys()) {
        reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "invalid MCP HTTP header name")?;
        let name = name.to_ascii_lowercase();
        if !names.insert(name) {
            return Err("HTTP header names must be unique regardless of case".into());
        }
    }
    if headers
        .values()
        .any(|value| reqwest::header::HeaderValue::from_str(value).is_err())
    {
        return Err("invalid MCP HTTP header value".into());
    }
    headers.retain(|name, _| {
        let lower = name.to_ascii_lowercase();
        !CLIENT_HEADERS.contains(&lower.as_str())
            && !lower.starts_with("sec-websocket-")
            && (!portable
                || !matches!(
                    lower.as_str(),
                    "authorization" | "proxy-authorization" | "cookie"
                ))
    });
    let env_headers = env_headers
        .into_iter()
        .filter(|(name, _)| !CLIENT_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
        .collect();
    let url = string_field(object, "url")?.ok_or("missing MCP URL")?;
    if deepseek_templates {
        deepseek_credential_origin(&url, transport)?;
        for header in headers.values() {
            expand_deepseek_credentials(header, |_| Ok(String::new()))?;
        }
    } else if format != AgentPluginFormat::ClaudeCode || !url.contains("${") {
        remote_url(&url, format, transport)?;
    }
    Ok(AgentPluginMcpConfig::Http {
        url,
        transport,
        headers,
        env_headers,
        bearer_env,
        headers_helper,
    })
}

fn decode_field<T: serde::de::DeserializeOwned + Default>(
    object: &Map<String, Value>,
    field: &str,
) -> Result<T, String> {
    object
        .get(field)
        .map(|value| serde_json::from_value(value.clone()).map_err(|_| format!("invalid {field}")))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn string_field(object: &Map<String, Value>, field: &str) -> Result<Option<String>, String> {
    object
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{field} must be a string"))
        })
        .transpose()
}

fn local_env_variables(value: &Value) -> Result<Vec<String>, String> {
    let Some(list) = value.get("env_vars") else {
        return Ok(Vec::new());
    };
    let list = list.as_array().ok_or("env_vars must be an array")?;
    let mut result = Vec::new();
    for item in list {
        let name = match item {
            Value::String(name) => Some(name.as_str()),
            Value::Object(item) if item.get("source").and_then(Value::as_str) == Some("remote") => {
                continue;
            }
            Value::Object(item) => item.get("name").and_then(Value::as_str),
            _ => None,
        };
        result.push(
            name.filter(|name| valid_env_name(name))
                .ok_or("invalid env_vars entry")?
                .to_owned(),
        );
    }
    result.sort();
    result.dedup();
    Ok(result)
}

pub(super) fn add_user_defaults(value: Option<&Value>, defaults: &mut BTreeMap<String, String>) {
    let Some(options) = value.and_then(Value::as_object) else {
        return;
    };
    for (name, option) in options {
        if option.get("sensitive").and_then(Value::as_bool) == Some(true) {
            defaults.remove(name);
            continue;
        }
        let value = match option.get("default") {
            Some(Value::String(value)) => Some(value.clone()),
            Some(value @ (Value::Bool(_) | Value::Number(_))) => Some(value.to_string()),
            Some(value @ Value::Array(items))
                if option["multiple"] == true && items.iter().all(Value::is_string) =>
            {
                Some(value.to_string())
            }
            _ => None,
        };
        if let Some(value) = value {
            defaults.insert(name.clone(), value);
        }
    }
}

fn resolve_cwd(
    cwd: Option<&str>,
    context: &AgentPluginContext<'_>,
    format: AgentPluginFormat,
) -> Result<PathBuf, String> {
    let Some(cwd) = cwd else {
        return Ok(context.root.to_owned());
    };
    let mut roots = vec![("PLUGIN_ROOT", context.root), ("PLUGIN_DATA", context.data)];
    if format == AgentPluginFormat::ClaudeCode {
        roots.extend([
            ("CLAUDE_PLUGIN_ROOT", context.root),
            ("CLAUDE_PLUGIN_DATA", context.data),
            ("CLAUDE_CONFIG_DIR", context.data),
        ]);
    }
    let (path, data) = if let Some((base, suffix)) = roots
        .into_iter()
        .find_map(|(var, base)| path_reference(cwd, var).map(|suffix| (base, suffix)))
    {
        (package_path(base, Path::new(suffix))?, base == context.data)
    } else {
        (manifest_path(context.root, cwd)?, false)
    };
    if data {
        agena_storage_layout::secure_directory(&path).map_err(|error| {
            format!("cannot prepare MCP working directory in plugin data: {error}")
        })?;
    }
    if !path.is_dir() {
        return Err("MCP working directory does not exist".into());
    }
    Ok(path)
}

fn path_reference<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let tail = value
        .strip_prefix("${")?
        .strip_prefix(name)?
        .strip_prefix('}')?;
    if tail.is_empty() {
        Some(tail)
    } else {
        tail.strip_prefix('/')
    }
}

pub(super) fn executable_path(path: PathBuf) -> Result<String, String> {
    if !super::regular_file(&path) {
        return Err("plugin MCP executable is missing or is not a regular file".into());
    }
    path_string(&path)
}

pub(super) fn path_string(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "plugin path must be UTF-8".into())
}

fn remote_url(
    value: &str,
    format: AgentPluginFormat,
    transport: AgentPluginMcpTransport,
) -> Result<Url, String> {
    let mut url = Url::parse(value).map_err(|_| "invalid MCP endpoint URL")?;
    if transport == AgentPluginMcpTransport::WebSocket {
        // Use equivalent HTTP origins for Agena's network permission checks.
        let scheme = match url.scheme() {
            "ws" => "http",
            "wss" => "https",
            _ => return Err("WebSocket MCP requires a ws:// or wss:// URL".into()),
        };
        url.set_scheme(scheme)
            .map_err(|()| "invalid WebSocket MCP URL")?;
    }
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("MCP endpoint must be an HTTP(S) URL without user info or fragment".into());
    }
    let loopback = match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    if url.scheme() == "http" && !loopback && format == AgentPluginFormat::AgentPluginsV1 {
        return Err("Agent Plugins non-loopback endpoints require HTTPS".into());
    }
    Ok(url)
}

fn same_environment_name(left: &str, right: &str) -> bool {
    left == right || (cfg!(windows) && left.eq_ignore_ascii_case(right))
}

pub(super) fn read_var(name: &str) -> Result<String, String> {
    if !valid_env_name(name) {
        return Err("invalid plugin environment variable name".into());
    }
    std::env::var(name).map_err(|_| format!("plugin environment variable {name:?} is unavailable"))
}

fn allowed_var(name: &str, environment: &[String]) -> Result<String, String> {
    if !environment
        .iter()
        .any(|allowed| same_environment_name(allowed, name))
    {
        return Err(format!(
            "environment variable {name:?} is not enabled for this plugin"
        ));
    }
    read_var(name)
}

pub(super) fn valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub(super) fn validate_environment_options(
    environment: &[String],
    user_config_env: &BTreeMap<String, String>,
) -> Result<(), String> {
    if environment.iter().any(|name| !valid_env_name(name))
        || user_config_env
            .iter()
            .any(|(key, name)| !valid_env_name(key) || !valid_env_name(name))
    {
        return Err("plugin environment and user_config_env must contain variable names".into());
    }
    Ok(())
}
