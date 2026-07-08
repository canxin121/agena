//! `agena.lsp` plugin: read-only observability of the configured
//! LSP servers plus model-visible navigation commands.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::message::{
    LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostLspListServersResponse};
use crate::plugin::sdk::{
    HostCapability, PathRequest, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput,
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
    crate::tool::definition::json_schema_for_default_with_metadata(
        default_lsp_config(),
        &[
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
        ],
    )
}

fn default_lsp_config() -> LspConfig {
    LspConfig {
        defaults: LspServerDefaultsConfig::default(),
        servers: BTreeMap::new(),
    }
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

    fn invoke_routed_tool<T: serde::Serialize>(
        &self,
        tool_name: &str,
        args: T,
        session_id: i64,
        call_id: i64,
    ) -> SdkResult<ToolInvokeOutput> {
        let _ = self.host()?;
        router::invoke_tool(
            tool_name,
            serde_json::to_value(args)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
            session_id,
            call_id,
        )
    }
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

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "lsp",
    version = env!("CARGO_PKG_VERSION"),
    summary = "LSP read-only observability and navigation tools.",
    config_schema = lsp_config_schema(),
    display = brief
)]
impl LspPlugin {
    #[hook]
    async fn init(
        &self,
        _ctx: crate::plugin::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<crate::plugin::sdk::InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("lsp plugin host lock poisoned"))? = Some(host);
        Ok(crate::plugin::sdk::InitOutcome::ack(
            crate::plugin::sdk::Plugin::manifest(self),
        ))
    }

    async fn permission_servers(&self) -> SdkResult<Vec<PathRequest>> {
        Ok(Vec::new())
    }

    async fn permission_definition(
        &self,
        input: LspDefinitionToolInput,
    ) -> SdkResult<Vec<PathRequest>> {
        Ok(vec![PathRequest::read(input.position.file_path)])
    }

    async fn permission_references(
        &self,
        input: LspReferencesToolInput,
    ) -> SdkResult<Vec<PathRequest>> {
        Ok(vec![PathRequest::read(input.position.file_path)])
    }

    async fn permission_hover(&self, input: LspHoverToolInput) -> SdkResult<Vec<PathRequest>> {
        Ok(vec![PathRequest::read(input.position.file_path)])
    }

    async fn permission_diagnostics(
        &self,
        input: LspDiagnosticsToolInput,
    ) -> SdkResult<Vec<PathRequest>> {
        Ok(vec![PathRequest::read(input.file_path)])
    }

    #[tool(
        name = "servers",
        summary = "List configured language servers.",
        read_only,
        lsp,
        capabilities(HostCapability::LspRegistry),
        display = brief,
        permission(paths = permission_servers),
        concurrency_safe
    )]
    async fn dispatch_servers(&self) -> SdkResult<ToolInvokeOutput> {
        let HostLspListServersResponse { servers } = self.host()?.lsp_list_servers().await?;
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

    #[tool(
        name = "definition",
        summary = "Resolve symbol definitions.",
        read_only,
        filesystem_read,
        lsp,
        capabilities(HostCapability::LspRegistry),
        display = brief,
        trim("file_path"),
        non_empty("file_path"),
        permission(paths = permission_definition),
        concurrency_safe
    )]
    async fn dispatch_definition(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspDefinitionToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool("lsp_definition", args, context.session_id, context.call_id)
    }

    #[tool(
        name = "references",
        summary = "Find symbol references.",
        read_only,
        filesystem_read,
        lsp,
        capabilities(HostCapability::LspRegistry),
        display = brief,
        trim("file_path"),
        non_empty("file_path"),
        permission(paths = permission_references),
        concurrency_safe
    )]
    async fn dispatch_references(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspReferencesToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool("lsp_references", args, context.session_id, context.call_id)
    }

    #[tool(
        name = "hover",
        summary = "Fetch hover text.",
        read_only,
        filesystem_read,
        lsp,
        capabilities(HostCapability::LspRegistry),
        display = brief,
        trim("file_path"),
        non_empty("file_path"),
        permission(paths = permission_hover),
        concurrency_safe
    )]
    async fn dispatch_hover(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspHoverToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool("lsp_hover", args, context.session_id, context.call_id)
    }

    #[tool(
        name = "diagnostics",
        summary = "Fetch file diagnostics.",
        read_only,
        filesystem_read,
        lsp,
        capabilities(HostCapability::LspRegistry),
        display = brief,
        trim("file_path"),
        non_empty("file_path"),
        permission(paths = permission_diagnostics),
        concurrency_safe
    )]
    async fn dispatch_diagnostics(
        &self,
        context: &ToolInvokeContext<'_>,
        args: LspDiagnosticsToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_routed_tool("lsp_diagnostics", args, context.session_id, context.call_id)
    }
}
