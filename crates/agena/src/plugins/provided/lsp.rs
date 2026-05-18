//! `agena.lsp` plugin: read-only observability of the configured
//! LSP servers plus model-visible navigation commands.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::message::{
    LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostLspListServersResponse};
use crate::plugin::sdk::{
    HookSubscription, HostCapability, InitContext, InitOutcome, PathRequest, Plugin,
    PluginManifest, PluginToolDecl, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput,
    ToolTag,
};
use crate::plugins::provided::router;

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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum LspToolInput {
    Servers(LspServersInput),
    Definition(LspDefinitionToolInput),
    References(LspReferencesToolInput),
    Hover(LspHoverToolInput),
    Diagnostics(LspDiagnosticsToolInput),
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
        PluginManifest::builder("agena-lsp", env!("CARGO_PKG_VERSION"))
            .description("LSP read-only observability and navigation tools.")
            .hooks(HookSubscription::TOOL_INVOKE)
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
        match serde_json::from_value::<LspToolInput>(input.input)? {
            LspToolInput::Servers(_) => {
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
            LspToolInput::Definition(args) => {
                let _ = self.host()?;
                router::invoke_tool(
                    "lsp_definition",
                    serde_json::to_value(args)
                        .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                    input.session_id,
                    input.call_id,
                )
            }
            LspToolInput::References(args) => {
                let _ = self.host()?;
                router::invoke_tool(
                    "lsp_references",
                    serde_json::to_value(args)
                        .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                    input.session_id,
                    input.call_id,
                )
            }
            LspToolInput::Hover(args) => {
                let _ = self.host()?;
                router::invoke_tool(
                    "lsp_hover",
                    serde_json::to_value(args)
                        .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                    input.session_id,
                    input.call_id,
                )
            }
            LspToolInput::Diagnostics(args) => {
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
    PluginToolDecl::new(
        "lsp",
        crate::entry::definition::json_schema_for::<LspToolInput>(),
    )
    .description(
        "LSP command dispatcher. Set command to servers, definition, references, hover, or diagnostics; pass that command's payload in args.",
    )
    .summary("Query configured language servers.")
    .help("Use command `servers` to list configured LSP servers, `definition` and `references` for symbol navigation, `hover` for hover text, and `diagnostics` for diagnostics. Pass the selected command payload under `args`.")
    .tags([ToolTag::ReadOnly, ToolTag::FilesystemRead, ToolTag::Lsp])
    .concurrency_safe(true)
    .always_load()
    .host_capability(HostCapability::LspRegistry)
}

fn lsp_route(input: serde_json::Value) -> SdkResult<(String, serde_json::Value)> {
    match serde_json::from_value::<LspToolInput>(input)? {
        LspToolInput::Servers(_) => Ok(("lsp_servers".to_string(), serde_json::json!({}))),
        LspToolInput::Definition(args) => tool_args("lsp_definition", args),
        LspToolInput::References(args) => tool_args("lsp_references", args),
        LspToolInput::Hover(args) => tool_args("lsp_hover", args),
        LspToolInput::Diagnostics(args) => tool_args("lsp_diagnostics", args),
    }
}

fn tool_args<T: serde::Serialize>(tool: &str, args: T) -> SdkResult<(String, serde_json::Value)> {
    Ok((
        tool.to_string(),
        serde_json::to_value(args).map_err(|err| PluginError::invalid_params(err.to_string()))?,
    ))
}

#[cfg(test)]
mod tests {
    use crate::plugin::sdk::host_api::{
        EventSubscription, HostLspListDiagnosticsRequest, HostLspListDiagnosticsResponse,
        HostLspListServersResponse, HostLspServer, LogLevel, ToolDescriptor,
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
                tool_name: "lsp".to_string(),
                session_id: 1,
                call_id: 1,
                workspace_root: "/tmp".to_string(),
                input: serde_json::json!({
                    "command": "servers",
                    "args": {}
                }),
            })
            .await
            .expect("lsp_servers");
        assert!(output.title.starts_with("lsp_servers"));
        assert!(output.output_text.contains("rust-analyzer"));
    }
}
