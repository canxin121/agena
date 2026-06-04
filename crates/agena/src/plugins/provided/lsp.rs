//! `agena.lsp` plugin: read-only observability of the configured
//! LSP servers plus model-visible navigation commands.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use agena_macros::StaticToolSurface;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::message::{
    LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostLspListServersResponse};
use crate::plugin::sdk::{
    HookSubscription, HostCapability, InitContext, InitOutcome, PathRequest, Plugin,
    PluginManifest, PluginToolDecl, Result as SdkResult, ToolDescriptionMode, ToolInvokeInput,
    ToolInvokeOutput, ToolTag, UiTextDisplayMode,
};
use crate::plugins::provided::router;

pub(crate) const LSP_PLUGIN_ID: &str = "agena.lsp";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LspConfig {
    pub defaults: LspServerDefaultsConfig,
    /// Map of `<server_name> -> <server spec>`. Each server config is spawned on
    /// demand by [`agena_lsp::LspRegistry`] when an LSP-using tool first
    /// touches a matching file.
    pub servers: BTreeMap<String, LspServerConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LspServerDefaultsConfig {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Marker filenames whose presence identifies the project root.
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default)]
    pub initialization_options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LspServerConfig {
    pub process: LspServerProcessConfig,
    pub routing: LspServerRoutingConfig,
    pub session: LspServerSessionConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LspServerProcessConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LspServerRoutingConfig {
    /// File extensions (without the leading `.`) routed to this server.
    /// Empty matches everything.
    #[serde(default)]
    pub file_extensions: Vec<String>,
    /// Marker filenames whose presence identifies the project root.
    #[serde(default)]
    pub root_markers: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LspServerSessionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initialization_options: Option<serde_json::Value>,
}

impl LspServerConfig {
    pub(crate) fn runtime_spec(
        &self,
        name: String,
        defaults: &LspServerDefaultsConfig,
    ) -> agena_lsp::LspServerSpec {
        let mut env = defaults.env.clone();
        env.extend(self.process.env.clone());
        let root_markers = if self.routing.root_markers.is_empty() {
            defaults.root_markers.clone()
        } else {
            self.routing.root_markers.clone()
        };
        let initialization_options = self
            .session
            .initialization_options
            .clone()
            .or_else(|| defaults.initialization_options.clone());
        agena_lsp::LspServerSpec {
            name,
            command: self.process.command.clone(),
            args: self.process.args.clone(),
            env,
            file_extensions: self.routing.file_extensions.clone(),
            root_markers,
            initialization_options,
        }
    }
}

fn lsp_config_schema() -> serde_json::Value {
    let mut schema = crate::tool::definition::json_schema_for_with_default(LspConfig::default());
    for (pointer, title, description) in [
        (
            "",
            "LSP Plugin Config",
            "Shared defaults and per-server settings for the agena.lsp plugin.",
        ),
        (
            "/properties/defaults",
            "Defaults",
            "Shared runtime settings applied to language servers unless a server overrides them.",
        ),
        (
            "/properties/defaults/properties/env",
            "Environment",
            "Environment variables injected into every configured language server process by default.",
        ),
        (
            "/properties/defaults/properties/root_markers",
            "Root Markers",
            "Marker files used to discover project roots when a server does not define its own routing markers.",
        ),
        (
            "/properties/defaults/properties/initialization_options",
            "Initialization Options",
            "Default JSON initialization options sent during server startup.",
        ),
        (
            "/properties/servers",
            "Servers",
            "Named language server definitions keyed by server identifier.",
        ),
        (
            "/properties/servers/additionalProperties",
            "Server",
            "A single named language server definition.",
        ),
        (
            "/properties/servers/additionalProperties/properties/process",
            "Process",
            "Executable command, arguments, and environment for this language server.",
        ),
        (
            "/properties/servers/additionalProperties/properties/routing",
            "Routing",
            "File matching and root detection rules for this server.",
        ),
        (
            "/properties/servers/additionalProperties/properties/session",
            "Session",
            "Per-server LSP session settings such as initialization options.",
        ),
        (
            "/properties/servers/additionalProperties/properties/process/properties/command",
            "Command",
            "Executable used to start the language server.",
        ),
        (
            "/properties/servers/additionalProperties/properties/process/properties/args",
            "Arguments",
            "Command-line arguments passed to the language server process.",
        ),
        (
            "/properties/servers/additionalProperties/properties/process/properties/env",
            "Environment",
            "Environment variables merged on top of the shared LSP defaults for this server.",
        ),
        (
            "/properties/servers/additionalProperties/properties/routing/properties/file_extensions",
            "File Extensions",
            "File extensions routed to this server. Leave empty to match all files.",
        ),
        (
            "/properties/servers/additionalProperties/properties/routing/properties/root_markers",
            "Root Markers",
            "Project-root markers used for this server. Leave empty to inherit the shared defaults.",
        ),
        (
            "/properties/servers/additionalProperties/properties/session/properties/initialization_options",
            "Initialization Options",
            "Server-specific JSON initialization options. Leave unset to inherit the shared defaults.",
        ),
    ] {
        crate::tool::definition::set_schema_metadata(
            &mut schema,
            pointer,
            Some(title),
            Some(description),
        );
    }
    schema
}

pub(crate) fn config_from_plugins(
    plugins: &crate::plugin::PluginsConfig,
) -> Result<LspConfig, String> {
    let Some(configured_plugin) = plugins.list.get(LSP_PLUGIN_ID) else {
        return Ok(LspConfig::default());
    };
    if configured_plugin.disabled() || configured_plugin.config().is_null() {
        return Ok(LspConfig::default());
    }
    serde_json::from_value(configured_plugin.config().clone())
        .map_err(|err| format!("plugins.list.\"{LSP_PLUGIN_ID}\".config: {err}"))
}

pub(crate) struct LspPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

impl LspPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
        }
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("lsp plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("lsp plugin invoked before init"))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "lsp",
    description = "LSP command dispatcher. Set action to servers, definition, references, hover, or diagnostics.",
    summary = "Query configured language servers.",
    help = "Use action `servers` to list configured LSP servers, `definition` and `references` for symbol navigation, `hover` for hover text, and `diagnostics` for diagnostics.",
    description_mode = "brief",
    ui_display_mode = "summary",
    tags(ToolTag::ReadOnly, ToolTag::FilesystemRead, ToolTag::Lsp),
    host_capabilities(HostCapability::LspRegistry),
    concurrency_safe = true
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum LspToolInput {
    #[tool(exec = "lsp_servers")]
    Servers,
    #[tool(exec = "lsp_definition")]
    Definition {
        #[serde(flatten)]
        args: LspDefinitionToolInput,
    },
    #[tool(exec = "lsp_references")]
    References {
        #[serde(flatten)]
        args: LspReferencesToolInput,
    },
    #[tool(exec = "lsp_hover")]
    Hover {
        #[serde(flatten)]
        args: LspHoverToolInput,
    },
    #[tool(exec = "lsp_diagnostics")]
    Diagnostics {
        #[serde(flatten)]
        args: LspDiagnosticsToolInput,
    },
}

#[derive(Debug, Serialize)]
struct LspServersOutput {
    servers: Vec<LspServerSummary>,
}

#[derive(Debug, Serialize)]
struct LspServerSummary {
    name: String,
    command: String,
    args: Vec<String>,
    file_extensions: Vec<String>,
}

#[async_trait]
impl Plugin for LspPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder(LSP_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description("LSP read-only observability and navigation tools.")
            .tool_description_mode(ToolDescriptionMode::Brief)
            .ui_display_mode(UiTextDisplayMode::Summary)
            .hooks(HookSubscription::TOOL_INVOKE)
            .config_schema(lsp_config_schema())
            .tool(lsp_decl())
            .build()
    }

    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("lsp plugin host lock poisoned"))? = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        if input.tool_name != "lsp" {
            return Err(PluginError::invalid_params(format!(
                "unknown lsp plugin tool '{}'",
                input.tool_name
            )));
        }
        match parse_lsp_input(input.input)? {
            LspToolInput::Servers => {
                let HostLspListServersResponse { servers } =
                    self.host()?.lsp_list_servers().await?;
                let summary = LspServersOutput {
                    servers: servers
                        .into_iter()
                        .map(|server| LspServerSummary {
                            name: server.name,
                            command: server.command,
                            args: server.args,
                            file_extensions: server.file_extensions,
                        })
                        .collect(),
                };
                let body = serde_json::to_string_pretty(&summary)
                    .map_err(|err| PluginError::new(err.to_string()))?;
                let title = format!("lsp_servers: {} configured", summary.servers.len());
                Ok(ToolInvokeOutput {
                    title,
                    output_text: body,
                    payload: serde_json::to_value(&summary).ok(),
                    metadata: Default::default(),
                    attachments: Vec::new(),
                })
            }
            LspToolInput::Definition { args } => {
                let _ = self.host()?;
                router::invoke_tool(
                    "lsp_definition",
                    serde_json::to_value(args)
                        .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                    input.session_id,
                    input.call_id,
                )
            }
            LspToolInput::References { args } => {
                let _ = self.host()?;
                router::invoke_tool(
                    "lsp_references",
                    serde_json::to_value(args)
                        .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                    input.session_id,
                    input.call_id,
                )
            }
            LspToolInput::Hover { args } => {
                let _ = self.host()?;
                router::invoke_tool(
                    "lsp_hover",
                    serde_json::to_value(args)
                        .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                    input.session_id,
                    input.call_id,
                )
            }
            LspToolInput::Diagnostics { args } => {
                let _ = self.host()?;
                router::invoke_tool(
                    "lsp_diagnostics",
                    serde_json::to_value(args)
                        .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                    input.session_id,
                    input.call_id,
                )
            }
        }
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        if tool != "lsp" {
            return Ok(Vec::new());
        }
        let (tool_name, tool_input) = lsp_route(input.clone())?;
        router::permission_paths_for(tool_name.as_str(), &tool_input)
    }
}

pub(crate) fn lsp_decl() -> PluginToolDecl {
    LspToolInput::tool_decl()
}

fn lsp_route(input: serde_json::Value) -> SdkResult<(String, serde_json::Value)> {
    LspToolInput::resolve_tool("lsp", input)
}

fn parse_lsp_input(input: serde_json::Value) -> SdkResult<LspToolInput> {
    LspToolInput::parse_input(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nested_lsp_config_parses_and_merges_defaults() {
        let config: LspConfig = serde_json::from_value(json!({
            "defaults": {
                "env": {
                    "RUST_BACKTRACE": "1"
                },
                "root_markers": ["Cargo.toml"],
                "initialization_options": {
                    "lint": true
                }
            },
            "servers": {
                "rust-analyzer": {
                    "process": {
                        "command": "rust-analyzer",
                        "args": ["--stdio"],
                        "env": {
                            "RA_LOG": "error"
                        }
                    },
                    "routing": {
                        "file_extensions": ["rs"]
                    },
                    "session": {}
                }
            }
        }))
        .expect("nested lsp config should parse");

        let spec = config
            .servers
            .get("rust-analyzer")
            .expect("server config")
            .runtime_spec("rust-analyzer".to_string(), &config.defaults);
        assert_eq!(spec.command, "rust-analyzer");
        assert_eq!(spec.args, vec!["--stdio".to_string()]);
        assert_eq!(spec.file_extensions, vec!["rs".to_string()]);
        assert_eq!(spec.root_markers, vec!["Cargo.toml".to_string()]);
        assert_eq!(
            spec.env.get("RUST_BACKTRACE").map(String::as_str),
            Some("1")
        );
        assert_eq!(spec.env.get("RA_LOG").map(String::as_str), Some("error"));
        assert_eq!(spec.initialization_options, Some(json!({ "lint": true })));
    }

    #[test]
    fn legacy_lsp_server_shape_is_rejected() {
        let err = serde_json::from_value::<LspConfig>(json!({
            "servers": {
                "rust-analyzer": {
                    "command": "rust-analyzer",
                    "file_extensions": ["rs"]
                }
            }
        }))
        .expect_err("legacy lsp config should fail");

        assert!(err.to_string().contains("unknown field `command`"));
    }
}
