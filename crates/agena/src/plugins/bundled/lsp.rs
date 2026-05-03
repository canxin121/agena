//! First-party `agena.lsp` plugin: read-only observability of the configured
//! LSP servers and their cached diagnostics. Exposes two model-visible entries
//! (`lsp_servers`, `lsp_diagnostics`) that route through the LspRegistry host
//! API and return JSON. Mirrors the substrate-only model used by
//! `agena.skills`.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{
    HostClient, HostLspListDiagnosticsRequest, HostLspListServersResponse,
};
use crate::plugin::sdk::{
    EntryBehavior as SdkEntryBehavior, HookSubscription, HostCapability, InitContext, InitOutcome,
    Plugin, PluginEntryDecl, PluginManifest, Result as SdkResult, ToolInvokeInput,
    ToolInvokeOutput,
};

pub(crate) const LSP_PLUGIN_ID: &str = "agena.lsp";

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

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
struct LspServersInput {}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
struct LspDiagnosticsInput {
    /// Optional URI filter — only return diagnostics for this exact uri.
    #[serde(default)]
    uri: Option<String>,
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

#[derive(Debug, Serialize)]
struct LspDiagnosticsOutput {
    entries: Vec<LspDiagnosticEntry>,
}

#[derive(Debug, Serialize)]
struct LspDiagnosticEntry {
    uri: String,
    severity: String,
    message: String,
    start_line: u32,
    start_character: u32,
    end_line: u32,
    end_character: u32,
    source: Option<String>,
    code: Option<String>,
}

#[async_trait]
impl Plugin for LspPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-lsp", env!("CARGO_PKG_VERSION"))
            .description("Read-only observability for the configured LSP fleet.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .entry(lsp_servers_decl())
            .entry(lsp_diagnostics_decl())
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
        match input.tool_name.as_str() {
            "lsp_servers" => {
                let _: LspServersInput = serde_json::from_value(input.input).unwrap_or_default();
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
            "lsp_diagnostics" => {
                let payload: LspDiagnosticsInput =
                    serde_json::from_value(input.input).unwrap_or_default();
                let response = self
                    .host()?
                    .lsp_list_diagnostics(HostLspListDiagnosticsRequest {
                        uri: payload.uri.clone(),
                    })
                    .await?;
                let summary = LspDiagnosticsOutput {
                    entries: response
                        .entries
                        .into_iter()
                        .map(|d| LspDiagnosticEntry {
                            uri: d.uri,
                            severity: d.severity,
                            message: d.message,
                            start_line: d.start_line,
                            start_character: d.start_character,
                            end_line: d.end_line,
                            end_character: d.end_character,
                            source: d.source,
                            code: d.code,
                        })
                        .collect(),
                };
                let body = serde_json::to_string_pretty(&summary)
                    .map_err(|err| PluginError::new(err.to_string()))?;
                let title = format!("lsp_diagnostics: {} entries", summary.entries.len());
                Ok(ToolInvokeOutput {
                    title,
                    output_text: body,
                    payload: serde_json::to_value(&summary).ok(),
                    metadata: Default::default(),
                    attachments: Vec::new(),
                })
            }
            other => Err(PluginError::invalid_params(format!(
                "unknown lsp plugin entry '{other}'"
            ))),
        }
    }
}

pub(crate) fn lsp_servers_decl() -> PluginEntryDecl {
    PluginEntryDecl::new(
        "lsp_servers",
        crate::entry::definition::json_schema_for::<LspServersInput>(),
    )
    .description("List the configured LSP servers known to the host (read-only).")
    .behavior(SdkEntryBehavior::ReadOnly)
    .search_terms(["lsp", "language-server", "list"])
    .host_capability(HostCapability::LspRegistry)
}

pub(crate) fn lsp_diagnostics_decl() -> PluginEntryDecl {
    PluginEntryDecl::new(
        "lsp_diagnostics",
        crate::entry::definition::json_schema_for::<LspDiagnosticsInput>(),
    )
    .description("List cached LSP diagnostics across spawned servers, optionally filtered by uri.")
    .behavior(SdkEntryBehavior::ReadOnly)
    .search_terms(["lsp", "diagnostics", "errors"])
    .host_capability(HostCapability::LspRegistry)
}

#[cfg(test)]
mod tests {
    use crate::plugin::sdk::host_api::{
        EventSubscription, HostLspListDiagnosticsResponse, HostLspListServersResponse,
        HostLspServer, LogLevel, ToolDescriptor,
    };
    use crate::plugin::sdk::{
        EventEnvelope, EventFilter, PermissionAskInput, PermissionDecision, ToolInvokeOutput,
    };

    use super::*;

    struct TestHost;

    #[async_trait]
    impl HostClient for TestHost {
        async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}

        async fn publish_event(&self, _env: EventEnvelope) -> SdkResult<()> {
            Ok(())
        }

        async fn subscribe_events(&self, _filter: EventFilter) -> SdkResult<EventSubscription> {
            Ok(EventSubscription { id: "sub".into() })
        }

        async fn ask_permission(&self, _req: PermissionAskInput) -> SdkResult<PermissionDecision> {
            Ok(PermissionDecision::Prompt)
        }

        async fn read_config(&self, _path: Option<String>) -> SdkResult<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }

        async fn invoke_tool(
            &self,
            tool: String,
            _input: serde_json::Value,
        ) -> SdkResult<ToolInvokeOutput> {
            Err(PluginError::new(format!(
                "unexpected invoke_tool for {tool}"
            )))
        }

        async fn list_tools(&self) -> SdkResult<Vec<ToolDescriptor>> {
            Ok(Vec::new())
        }

        async fn lsp_list_servers(&self) -> SdkResult<HostLspListServersResponse> {
            Ok(HostLspListServersResponse {
                servers: vec![HostLspServer {
                    name: "rust-analyzer".into(),
                    command: "rust-analyzer".into(),
                    args: vec![],
                    file_extensions: vec!["rs".into()],
                }],
            })
        }

        async fn lsp_list_diagnostics(
            &self,
            _req: HostLspListDiagnosticsRequest,
        ) -> SdkResult<HostLspListDiagnosticsResponse> {
            Ok(HostLspListDiagnosticsResponse {
                entries: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn lsp_servers_renders_summary_via_host_api() {
        let plugin = LspPlugin::new();
        plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: "/tmp".into(),
                    plugin_id: LSP_PLUGIN_ID.to_string(),
                    host_callback_url: None,
                    host_callback_token: None,
                    options: serde_json::Value::Null,
                    protocol_version: crate::plugin::sdk::rpc::PROTOCOL_VERSION,
                },
                Arc::new(TestHost),
            )
            .await
            .expect("init");

        let output = plugin
            .tool_invoke(ToolInvokeInput {
                tool_name: "lsp_servers".to_string(),
                session_id: 1,
                call_id: 1,
                workspace_root: "/tmp".to_string(),
                input: serde_json::Value::Null,
            })
            .await
            .expect("lsp_servers");
        assert!(output.title.starts_with("lsp_servers"));
        assert!(output.output_text.contains("rust-analyzer"));
    }
}
