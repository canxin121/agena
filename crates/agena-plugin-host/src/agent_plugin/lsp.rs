use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Deserialize;
use serde_json::{Map, Value};

use super::{
    AgentPluginContext, AgentPluginFormat, MAX_MANIFEST_BYTES, Manifest, deepseek, manifest_path,
    mcp, read_package_object,
};

#[derive(Clone)]
pub struct AgentPluginLspServer {
    pub name: String,
    pub command: String,
    pub language_ids: BTreeMap<String, String>,
    pub initialization_options: Option<Value>,
    pub settings: Option<Value>,
    pub startup_timeout: Option<u64>,
    pub shutdown_timeout: Option<u64>,
    pub request_timeout: Option<u64>,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    workspace_folder: Option<String>,
    options: Arc<BTreeMap<String, String>>,
    format: AgentPluginFormat,
}

pub struct ResolvedAgentPluginLsp {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub workspace: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerConfig {
    command: String,
    extension_to_language: BTreeMap<String, String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    transport: Option<String>,
    initialization_options: Option<Value>,
    #[serde(alias = "configuration")]
    settings: Option<Value>,
    workspace_folder: Option<String>,
    startup_timeout: Option<u64>,
    #[serde(alias = "shutdownTimeoutMs")]
    shutdown_timeout: Option<u64>,
    request_timeout: Option<u64>,
}

impl AgentPluginLspServer {
    pub fn resolve(
        &self,
        context: &AgentPluginContext<'_>,
    ) -> Result<ResolvedAgentPluginLsp, String> {
        let expand = |value: &str| -> Result<String, String> {
            let value = mcp::expand_template(value, context, self.format, &self.options)?;
            if value.contains('\0') {
                return Err("LSP process configuration contains NUL".into());
            }
            Ok(value)
        };
        let command = expand(&self.command)?;
        let command = if Path::new(&command).is_absolute() {
            command
        } else if command.starts_with("./") || command.starts_with(".\\") {
            mcp::path_string(&context.workspace.join(command))?
        } else if command.is_empty()
            || command.contains(['/', '\\', ':'])
            || command.chars().any(char::is_whitespace)
        {
            return Err("LSP command must name an executable or an explicit path".into());
        } else {
            command
        };
        let args = self
            .args
            .iter()
            .map(|value| expand(value))
            .collect::<Result<_, _>>()?;
        let mut env = self
            .env
            .iter()
            .map(|(key, value)| Ok::<_, String>((key.clone(), expand(value)?)))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        if self.format == AgentPluginFormat::ClaudeCode {
            for (key, path) in [
                ("CLAUDE_PLUGIN_ROOT", context.root),
                ("CLAUDE_PLUGIN_DATA", context.data),
                ("CLAUDE_PROJECT_DIR", context.workspace),
            ] {
                env.insert(key.to_owned(), mcp::path_string(path)?);
            }
        }
        let workspace = self
            .workspace_folder
            .as_deref()
            .map(expand)
            .transpose()?
            .map_or_else(
                || context.workspace.to_path_buf(),
                |path| context.workspace.join(path),
            );
        Ok(ResolvedAgentPluginLsp {
            command,
            args,
            env,
            workspace,
        })
    }
}

pub(super) fn discover(
    root: &Path,
    manifest: &Manifest,
    watch: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<String>,
) -> Vec<AgentPluginLspServer> {
    let mut entries = Map::new();
    let mut add_file = |path: &str,
                        entries: &mut Map<String, Value>,
                        diagnostics: &mut Vec<String>| {
        let map = manifest_path(root, path).and_then(|path| {
            watch.insert(path.clone());
            let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
            read_package_object(root, relative, MAX_MANIFEST_BYTES)
        });
        match map {
            Ok(Some(map)) => entries.extend(map.into_iter().filter(|(name, _)| name != "$schema")),
            Ok(None) if path == "./.lsp.json" => {}
            Ok(None) => diagnostics.push(format!("LSP configuration file {path:?} is missing")),
            Err(error) => diagnostics.push(format!("LSP configuration {path:?}: {error}")),
        }
    };
    add_file("./.lsp.json", &mut entries, diagnostics);
    if let Some(sources) = manifest.document.get("lspServers") {
        for value in sources
            .as_array()
            .map_or(std::slice::from_ref(sources), Vec::as_slice)
        {
            match value {
                Value::String(path) => add_file(path, &mut entries, diagnostics),
                Value::Object(map) => entries.extend(map.clone()),
                _ => diagnostics
                    .push("lspServers entries must be objects or local JSON paths".into()),
            }
        }
    }
    let mut options = BTreeMap::new();
    mcp::add_user_defaults(manifest.document.get("userConfig"), &mut options);
    parse_servers(
        entries,
        AgentPluginFormat::ClaudeCode,
        Arc::new(options),
        diagnostics,
    )
}

pub(super) fn discover_deepseek(
    rows: &[(String, serde_yaml::Value)],
    diagnostics: &mut Vec<String>,
) -> Vec<AgentPluginLspServer> {
    let mut entries = Map::new();
    let mut conflicts = BTreeSet::new();
    for (id, row) in rows
        .iter()
        .filter(|(_, row)| deepseek::module_name(row) == Some(deepseek::LSP_MODULE))
    {
        match serde_json::to_value(&row["config"]["servers"]) {
            Ok(Value::Object(servers)) if !servers.is_empty() => {
                for (name, server) in servers {
                    if entries.insert(name.clone(), server).is_some() {
                        conflicts.insert(name);
                    }
                }
            }
            _ => diagnostics.push(format!(
                "DeepSeek Harness LSP entry {id:?} requires config.servers"
            )),
        }
    }
    for name in conflicts {
        entries.remove(&name);
        diagnostics.push(format!(
            "DeepSeek Harness LSP server {name:?} is declared more than once"
        ));
    }
    parse_servers(
        entries,
        AgentPluginFormat::DeepSeekHarness,
        Arc::new(BTreeMap::new()),
        diagnostics,
    )
}

fn parse_servers(
    entries: Map<String, Value>,
    format: AgentPluginFormat,
    options: Arc<BTreeMap<String, String>>,
    diagnostics: &mut Vec<String>,
) -> Vec<AgentPluginLspServer> {
    entries
        .into_iter()
        .filter_map(|(name, value)| {
            let result = parse_server(&name, value, format, Arc::clone(&options));
            match result {
                Ok(server) => Some(server),
                Err(error) => {
                    diagnostics.push(format!("LSP server {name:?}: {error}"));
                    None
                }
            }
        })
        .collect()
}

fn parse_server(
    name: &str,
    value: Value,
    format: AgentPluginFormat,
    options: Arc<BTreeMap<String, String>>,
) -> Result<AgentPluginLspServer, String> {
    let config: ServerConfig =
        serde_json::from_value(value).map_err(|_| "invalid language server configuration")?;
    if name.is_empty() || name.chars().any(char::is_control) || config.command.trim().is_empty() {
        return Err("a language server needs a name and a command".into());
    }
    if config.extension_to_language.is_empty()
        || config
            .extension_to_language
            .iter()
            .any(|(extension, language)| {
                !extension.starts_with('.')
                    || extension.len() < 2
                    || extension.contains(['/', '\\'])
                    || extension.chars().any(char::is_whitespace)
                    || language.trim().is_empty()
            })
    {
        return Err("extensionToLanguage must map dotted file extensions to language IDs".into());
    }
    if config
        .transport
        .as_deref()
        .is_some_and(|transport| !matches!(transport, "stdio" | "socket"))
        || config.env.keys().any(|name| !mcp::valid_env_name(name))
        || [
            config.startup_timeout,
            config.shutdown_timeout,
            config.request_timeout,
        ]
        .contains(&Some(0))
    {
        return Err("invalid transport, environment variable name, or timeout".into());
    }
    Ok(AgentPluginLspServer {
        name: name.to_owned(),
        command: config.command,
        args: config.args,
        env: config.env,
        language_ids: config
            .extension_to_language
            .into_iter()
            .map(|(extension, language)| (extension[1..].to_ascii_lowercase(), language))
            .collect(),
        initialization_options: config.initialization_options,
        settings: config.settings,
        workspace_folder: config.workspace_folder,
        startup_timeout: config.startup_timeout,
        shutdown_timeout: config.shutdown_timeout,
        request_timeout: config.request_timeout,
        options,
        format,
    })
}
