//! `agena.lsp` plugin: read-only observability of the configured
//! LSP servers plus model-visible navigation commands.

use std::sync::{Arc, RwLock};

use crate::LspConfig;
use serde::Serialize;

use crate::part::{
    LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput,
};
use crate::plugins::provided::router;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{HostClient, HostLspListServersResponse};
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};

fn lsp_settings_metadata() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        (
            "",
            "LSP Plugin Config",
            "Shared defaults and per-server settings for the agena.lsp plugin.",
        ),
        (
            "/defaults",
            "Defaults",
            "Shared runtime settings applied to language servers unless a server overrides them.",
        ),
        (
            "/defaults/env",
            "Environment",
            "Environment variables injected into every configured language server process by default.",
        ),
        (
            "/defaults/root_markers",
            "Root Markers",
            "Marker files used to discover project roots when a server does not define its own routing markers.",
        ),
        (
            "/defaults/initialization_options",
            "Initialization Options",
            "Default JSON initialization options sent during server startup.",
        ),
        (
            "/servers",
            "Servers",
            "Named language server definitions keyed by server identifier.",
        ),
        (
            "/servers/*",
            "Server",
            "A single named language server definition.",
        ),
        (
            "/servers/*/process",
            "Process",
            "Executable command, arguments, and environment for this language server.",
        ),
        (
            "/servers/*/routing",
            "Routing",
            "File matching and root detection rules for this server.",
        ),
        (
            "/servers/*/session",
            "Session",
            "Per-server LSP session settings such as initialization options.",
        ),
        (
            "/servers/*/process/command",
            "Command",
            "Executable used to start the language server.",
        ),
        (
            "/servers/*/process/args",
            "Arguments",
            "Command-line arguments passed to the language server process.",
        ),
        (
            "/servers/*/process/env",
            "Environment",
            "Environment variables merged on top of the shared LSP defaults for this server.",
        ),
        (
            "/servers/*/routing/file_extensions",
            "File Extensions",
            "File extensions routed to this server. Leave empty to match all files.",
        ),
        (
            "/servers/*/routing/root_markers",
            "Root Markers",
            "Project-root markers used for this server. Leave empty to inherit the shared defaults.",
        ),
        (
            "/servers/*/session/initialization_options",
            "Initialization Options",
            "Server-specific JSON initialization options. Leave unset to inherit the shared defaults.",
        ),
    ]
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
    settings = LspConfig,
    settings_default = default,
    settings_metadata = lsp_settings_metadata(),
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
        tags(query, lsp, discovery),
        summary = "List configured language servers.",
        read_only,
        lsp,
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
        Ok(ToolInvokeOutput {
            title: "Language servers".to_string(),
            summary: format!("{} configured servers", summary.servers.len()),
            output_text: body,
            payload: serde_json::to_value(&summary).ok(),
            metadata: Default::default(),
            attachments: Vec::new(),
        })
    }

    #[tool(
        tags(query, lsp, filesystem),
        summary = "Resolve symbol definitions.",
        read_only,
        lsp,
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
        tags(query, lsp, filesystem),
        summary = "Find symbol references.",
        read_only,
        lsp,
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
        tags(query, lsp, filesystem),
        summary = "Fetch hover text.",
        read_only,
        lsp,
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
        tags(query, lsp, filesystem),
        summary = "Fetch file diagnostics.",
        read_only,
        lsp,
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

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::{
        MAX_JSON_ESCAPE_BYTES, MAX_JSON_ESCAPE_DEPTH, Plugin, SettingsNode, SettingsNodeKind,
    };

    use super::LspPlugin;

    fn node_at<'a>(node: &'a SettingsNode, path: &str) -> Option<&'a SettingsNode> {
        if node.path == path {
            return Some(node);
        }
        match &node.kind {
            SettingsNodeKind::Object { fields } => {
                fields.iter().find_map(|field| node_at(field, path))
            }
            SettingsNodeKind::List { item } => node_at(item, path),
            SettingsNodeKind::Record { value } => node_at(value, path),
            SettingsNodeKind::TaggedVariant { variants, .. } => variants
                .iter()
                .flat_map(|variant| variant.fields.iter())
                .find_map(|field| node_at(field, path)),
            _ => None,
        }
    }

    #[test]
    fn manifest_uses_typed_settings_and_bounded_json_initialization_options() {
        let manifest = LspPlugin::new().manifest();
        let settings = manifest.settings.expect("typed LSP settings contract");
        settings.validate().expect("valid LSP settings contract");
        assert_eq!(settings.root.title, "LSP Plugin Config");
        for path in [
            "/defaults/initialization_options",
            "/servers/*/session/initialization_options",
        ] {
            let node = node_at(&settings.root, path).expect("LSP initialization options node");
            assert!(matches!(
                node.kind,
                SettingsNodeKind::Json {
                    max_bytes: MAX_JSON_ESCAPE_BYTES,
                    max_depth: MAX_JSON_ESCAPE_DEPTH,
                }
            ));
        }
    }
}
