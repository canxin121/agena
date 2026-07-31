//! `agena.lsp` plugin: read-only observability of the configured
//! LSP servers plus model-visible navigation commands.

use std::sync::{Arc, RwLock};

use crate::LspConfig;
use serde::Serialize;

use crate::message::{
    LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput,
};
use crate::plugins::provided::router;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{HostClient, HostLspListServersResponse};
use agena_plugin_host::sdk::{
    HostCapability, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput,
};

fn lsp_config_schema() -> serde_json::Value {
    agena_runtime_tools::tool::definition::json_schema_for_default_with_metadata(
        LspConfig::default(),
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
            .map_err(|_| PluginError::internal("lsp plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::internal("lsp plugin invoked before init"))
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

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "lsp",
    version = env!("CARGO_PKG_VERSION"),
    summary = "LSP read-only observability and navigation tools.",
    config_schema = lsp_config_schema(),
    display = brief
)]
impl LspPlugin {
    #[hook(init)]
    async fn init(
        &self,
        _ctx: agena_plugin_host::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<agena_plugin_host::sdk::InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::internal("lsp plugin host lock poisoned"))? = Some(host);
        Ok(agena_plugin_host::sdk::InitOutcome::ack(
            agena_plugin_host::sdk::Plugin::manifest(self),
        ))
    }

    #[tool(
        summary = "List configured language servers.",
        read_only,
        lsp,
        capabilities(HostCapability::LspRegistry),
        display = brief,
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
            .map_err(|err| PluginError::internal(err.to_string()))?;
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
        summary = "Resolve symbol definitions.",
        read_only,
        filesystem_read,
        lsp,
        capabilities(HostCapability::LspRegistry),
        display = brief,
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
        summary = "Find symbol references.",
        read_only,
        filesystem_read,
        lsp,
        capabilities(HostCapability::LspRegistry),
        display = brief,
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
        summary = "Fetch hover text.",
        read_only,
        filesystem_read,
        lsp,
        capabilities(HostCapability::LspRegistry),
        display = brief,
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
        summary = "Fetch file diagnostics.",
        read_only,
        filesystem_read,
        lsp,
        capabilities(HostCapability::LspRegistry),
        display = brief,
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
