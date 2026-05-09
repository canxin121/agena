//! First-party `agena.lsp` plugin: read-only observability of the configured
//! LSP servers plus the model-visible LSP entries (`lsp_definition`,
//! `lsp_references`, `lsp_hover`, `lsp_diagnostics`). `lsp_servers` is
//! plugin-native and uses `host.lsp_list_servers`; the other LSP entries share
//! the in-process router bridge while remaining normal plugin entries.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::message::{
    LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostLspListServersResponse};
use crate::plugin::sdk::manifest::{InputPathSpec, PathKind};
use crate::plugin::sdk::{
    EntryBehavior as SdkEntryBehavior, HookSubscription, HostCapability, InitContext, InitOutcome,
    PlanModePolicy, Plugin, PluginEntryDecl, PluginManifest, Result as SdkResult, ToolInvokeInput,
    ToolInvokeOutput,
};
use crate::plugins::bundled::router;

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
            .description(
                "LSP read-only observability and first-party LSP entry surface exposed as a plugin.",
            )
            .hooks(HookSubscription::TOOL_INVOKE)
            .entry(lsp_servers_decl())
            .entry(lsp_definition_decl())
            .entry(lsp_references_decl())
            .entry(lsp_hover_decl())
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
            name @ ("lsp_definition" | "lsp_references" | "lsp_hover" | "lsp_diagnostics") => {
                let _ = self.host()?;
                router::invoke_first_party_tool(name, input.input, input.session_id, input.call_id)
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

pub(crate) fn lsp_definition_decl() -> PluginEntryDecl {
    PluginEntryDecl::new(
        "lsp_definition",
        crate::entry::definition::json_schema_for::<LspDefinitionToolInput>(),
    )
    .description(
        "Resolve the symbol at file_path:line:character to its definition site(s) via the configured LSP server.",
    )
    .behavior(SdkEntryBehavior::ReadOnly)
    .input_path(required_path("$.file_path", PathKind::Read))
    .search_terms(["lsp", "definition", "go to def", "jump"])
    .deferred_load()
    .plan_mode_policy(PlanModePolicy::Allowed)
    .host_capability(HostCapability::LspRegistry)
}

pub(crate) fn lsp_references_decl() -> PluginEntryDecl {
    PluginEntryDecl::new(
        "lsp_references",
        crate::entry::definition::json_schema_for::<LspReferencesToolInput>(),
    )
    .description(
        "List every reference to the symbol at file_path:line:character via the configured LSP server.",
    )
    .behavior(SdkEntryBehavior::ReadOnly)
    .input_path(required_path("$.file_path", PathKind::Read))
    .search_terms(["lsp", "references", "callers", "usages"])
    .deferred_load()
    .plan_mode_policy(PlanModePolicy::Allowed)
    .host_capability(HostCapability::LspRegistry)
}

pub(crate) fn lsp_hover_decl() -> PluginEntryDecl {
    PluginEntryDecl::new(
        "lsp_hover",
        crate::entry::definition::json_schema_for::<LspHoverToolInput>(),
    )
    .description(
        "Read the hover documentation / type signature for the symbol at file_path:line:character.",
    )
    .behavior(SdkEntryBehavior::ReadOnly)
    .input_path(required_path("$.file_path", PathKind::Read))
    .search_terms(["lsp", "hover", "type", "signature", "docs"])
    .deferred_load()
    .plan_mode_policy(PlanModePolicy::Allowed)
    .host_capability(HostCapability::LspRegistry)
}

pub(crate) fn lsp_diagnostics_decl() -> PluginEntryDecl {
    PluginEntryDecl::new(
        "lsp_diagnostics",
        crate::entry::definition::json_schema_for::<LspDiagnosticsToolInput>(),
    )
    .description(
        "Return the latest LSP-published diagnostics (errors / warnings / hints) for a file.",
    )
    .behavior(SdkEntryBehavior::ReadOnly)
    .input_path(required_path("$.file_path", PathKind::Read))
    .search_terms(["lsp", "diagnostics", "errors", "warnings", "lint"])
    .deferred_load()
    .plan_mode_policy(PlanModePolicy::Allowed)
    .host_capability(HostCapability::LspRegistry)
}

fn required_path(jsonpath: &str, kind: PathKind) -> InputPathSpec {
    InputPathSpec {
        jsonpath: jsonpath.to_string(),
        kind,
        optional: false,
    }
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
