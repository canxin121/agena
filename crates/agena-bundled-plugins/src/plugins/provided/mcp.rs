//! In-process plugin that exposes configured MCP server capabilities as plugin
//! tools.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{
    MCP_PLUGIN_ID,
    config::{McpConfig, McpRuntimeConfig},
};
use agena_macros::ToolInput;
use agena_mcp_client::McpConnectionManager;
use agena_mcp_client::protocol::{
    CallToolResult, ContentBlock, GetPromptResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ReadResourceResult, ResourceContents,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::message::{AttachmentItem, OperationBlock};
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{
    InitOutcome, NetworkAccessSpec, Result as SdkResult, ToolDefinitionInput, ToolDefinitionPatch,
    ToolInvokeOutput,
};

pub(crate) struct McpPlugin {
    manager: Arc<McpConnectionManager>,
}

fn mcp_config_schema() -> Value {
    agena_runtime_tools::tool::definition::json_schema_for_default_with_metadata(
        default_mcp_config(),
        &[
            (
                "",
                "MCP Plugin Config",
                "Runtime settings and named server definitions for the agena.mcp bridge.",
            ),
            (
                "/properties/runtime",
                "Runtime",
                "Bridge-level settings that apply to all MCP servers.",
            ),
            (
                "/properties/runtime/properties/token_store",
                "Token Store",
                "Controls whether the default MCP token store is available for bearer-from-store authentication.",
            ),
            (
                "/properties/runtime/properties/token_store/properties/enabled",
                "Enabled",
                "Enables MCP credential lookup for bearer-from-store authentication.",
            ),
            (
                "/properties/runtime/properties/token_store/properties/backend",
                "Credential Backend",
                "Uses the operating-system keyring by default. Select file only for explicit legacy compatibility.",
            ),
            (
                "/properties/runtime/properties/token_store/properties/file_fallback",
                "Legacy File Fallback",
                "When keyring is selected, optionally read the legacy chmod-600 token file after keyring lookup misses or is unavailable; this never writes credentials into configuration.",
            ),
            (
                "/properties/servers",
                "Servers",
                "Named MCP server definitions keyed by server identifier.",
            ),
            (
                "/properties/servers/additionalProperties",
                "Server",
                "A single MCP server transport definition.",
            ),
            (
                "/properties/servers/additionalProperties/oneOf/0",
                "Stdio Server",
                "Launches an MCP server as a child process and communicates over stdio.",
            ),
            (
                "/properties/servers/additionalProperties/oneOf/0/properties/process",
                "Process",
                "Command, arguments, environment, and working directory for the stdio MCP server.",
            ),
            (
                "/properties/servers/additionalProperties/oneOf/0/properties/process/properties/command",
                "Command",
                "Executable used to launch the stdio MCP server.",
            ),
            (
                "/properties/servers/additionalProperties/oneOf/0/properties/process/properties/args",
                "Arguments",
                "Command-line arguments passed to the stdio MCP server.",
            ),
            (
                "/properties/servers/additionalProperties/oneOf/0/properties/process/properties/env",
                "Environment",
                "Environment variables injected into the stdio MCP server process.",
            ),
            (
                "/properties/servers/additionalProperties/oneOf/0/properties/process/properties/cwd",
                "Working Directory",
                "Working directory used when starting the stdio MCP server process.",
            ),
            (
                "/properties/servers/additionalProperties/oneOf/1",
                "HTTP Server",
                "Connects to a streamable HTTP MCP server.",
            ),
            (
                "/properties/servers/additionalProperties/oneOf/1/properties/endpoint",
                "Endpoint",
                "HTTP endpoint and headers used to connect to the MCP server.",
            ),
            (
                "/properties/servers/additionalProperties/oneOf/1/properties/endpoint/properties/url",
                "URL",
                "Base URL for the HTTP MCP server.",
            ),
            (
                "/properties/servers/additionalProperties/oneOf/1/properties/endpoint/properties/headers",
                "Headers",
                "Static HTTP headers attached to each request to this MCP server.",
            ),
            (
                "/properties/servers/additionalProperties/oneOf/1/properties/auth",
                "Authentication",
                "Optional authentication strategy used for the HTTP MCP server.",
            ),
        ],
    )
}

fn default_mcp_config() -> McpConfig {
    McpConfig {
        runtime: McpRuntimeConfig::default(),
        servers: BTreeMap::new(),
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "mcp",
    version = env!("CARGO_PKG_VERSION"),
    summary = "MCP discovery and bridge tools.",
    config_schema = mcp_config_schema(),
    display = brief
)]
impl McpPlugin {
    pub(crate) fn new(manager: Arc<McpConnectionManager>) -> Self {
        Self { manager }
    }

    #[hook(init)]
    async fn init(
        &self,
        _ctx: agena_plugin_host::sdk::InitContext,
        _host: Arc<dyn agena_plugin_host::sdk::HostClient>,
    ) -> SdkResult<agena_plugin_host::sdk::InitOutcome> {
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        name = "resources.list",
        summary = "List MCP resources from one server.",
        read_only,
        mcp,
        ui_display = brief,
        network(connects = self.server_network_targets(input.server.as_str()).await?),
        concurrency_safe
    )]
    async fn invoke_resources_list(&self, input: &McpServerInput) -> SdkResult<ToolInvokeOutput> {
        let result = self
            .manager
            .list_resources(&input.server, input.cursor.clone())
            .await
            .map_err(|err| {
                PluginError::new(format!("mcp:{}:resources:list failed: {err}", input.server))
            })?;
        list_resources_output(&input.server, result)
    }

    #[tool(
        name = "resources.templates.list",
        summary = "List MCP resource templates from one server.",
        read_only,
        mcp,
        ui_display = brief,
        network(connects = self.server_network_targets(input.server.as_str()).await?),
        concurrency_safe
    )]
    async fn invoke_resource_templates_list(
        &self,
        input: &McpServerInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let result = self
            .manager
            .list_resource_templates(&input.server, input.cursor.clone())
            .await
            .map_err(|err| {
                PluginError::new(format!(
                    "mcp:{}:resources:templates:list failed: {err}",
                    input.server
                ))
            })?;
        list_resource_templates_output(&input.server, result)
    }

    #[tool(
        name = "resources.read",
        summary = "Read one MCP resource by URI.",
        read_only,
        mcp,
        ui_display = brief,
        network(connects = self.server_network_targets(input.server.as_str()).await?),
        concurrency_safe
    )]
    async fn invoke_resources_read(
        &self,
        input: &ReadResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let result = self
            .manager
            .read_resource(&input.server, input.uri.as_str())
            .await
            .map_err(|err| {
                PluginError::new(format!(
                    "mcp:{}:resources:read '{}' failed: {err}",
                    input.server, input.uri
                ))
            })?;
        read_resource_output(&input.server, input.uri.as_str(), result)
    }

    #[tool(
        name = "prompts.list",
        summary = "List MCP prompt templates from one server.",
        read_only,
        mcp,
        ui_display = brief,
        network(connects = self.server_network_targets(input.server.as_str()).await?),
        concurrency_safe
    )]
    async fn invoke_prompts_list(&self, input: &McpServerInput) -> SdkResult<ToolInvokeOutput> {
        let result = self
            .manager
            .list_prompts(&input.server, input.cursor.clone())
            .await
            .map_err(|err| {
                PluginError::new(format!("mcp:{}:prompts:list failed: {err}", input.server))
            })?;
        list_prompts_output(&input.server, result)
    }

    #[tool(
        name = "prompts.get",
        summary = "Fetch one MCP prompt template.",
        read_only,
        mcp,
        ui_display = brief,
        network(connects = self.server_network_targets(input.server.as_str()).await?),
        concurrency_safe
    )]
    async fn invoke_prompts_get(&self, input: &GetPromptInput) -> SdkResult<ToolInvokeOutput> {
        let result = self
            .manager
            .get_prompt(&input.server, input.name.as_str(), input.arguments.clone())
            .await
            .map_err(|err| {
                PluginError::new(format!(
                    "mcp:{}:prompts:get '{}' failed: {err}",
                    input.server, input.name
                ))
            })?;
        get_prompt_output(&input.server, input.name.as_str(), result)
    }

    #[tool(
        name = "tools.call",
        summary = "Call one discovered MCP tool.",
        mutating,
        mcp,
        ui_display = brief,
        network(connects = self.server_network_targets(input.server.as_str()).await?)
    )]
    async fn invoke_tools_call(&self, input: &CallToolInput) -> SdkResult<ToolInvokeOutput> {
        let result = self
            .manager
            .call_tool(
                &input.server,
                input.name.as_str(),
                empty_object_to_none(input.arguments.clone()),
            )
            .await
            .map_err(|err| {
                PluginError::new(format!(
                    "mcp:{}:tool:{} call failed: {err}",
                    input.server, input.name
                ))
            })?;
        invoke_tool_output(&input.server, input.name.as_str(), result)
    }

    #[tool(
        name = "tools.search",
        summary = "Search the current MCP tool index without expanding all schemas.",
        read_only,
        mcp,
        discovery,
        ui_display = brief,
        concurrency_safe
    )]
    async fn invoke_tools_search(&self, input: &McpToolSearchInput) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.trim().to_ascii_lowercase();
        let mut tools = self
            .manager
            .all_tools()
            .await
            .into_iter()
            .filter_map(|(server, tool)| {
                let haystack = format!(
                    "{} {} {}",
                    server,
                    tool.name,
                    tool.description.as_deref().unwrap_or_default()
                )
                .to_ascii_lowercase();
                if !query.is_empty() && !haystack.contains(query.as_str()) {
                    return None;
                }
                let score = usize::from(tool.name.eq_ignore_ascii_case(query.as_str())) * 100
                    + usize::from(tool.name.to_ascii_lowercase().contains(query.as_str())) * 10
                    + usize::from(haystack.contains(query.as_str()));
                Some((score, server, tool))
            })
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.name.cmp(&right.2.name))
        });
        tools.truncate(input.limit as usize);
        let results = tools
            .into_iter()
            .map(|(_, server, tool)| {
                serde_json::json!({
                    "server": server,
                    "name": tool.name,
                    "title": tool.title,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                    "output_schema": tool.output_schema,
                    "annotations": tool.annotations,
                    "risk_hint": mcp_annotation_risk(tool.annotations.as_ref()),
                })
            })
            .collect::<Vec<_>>();
        let fingerprint = mcp_tool_index_fingerprint(&results);
        Ok(ToolInvokeOutput::from_parts(
            "MCP tool search",
            if results.is_empty() {
                format!("No MCP tools matched {:?}.", input.query)
            } else {
                results
                    .iter()
                    .map(|result| {
                        format!(
                            "- {}/{}: {}",
                            result["server"].as_str().unwrap_or_default(),
                            result["name"].as_str().unwrap_or_default(),
                            result["description"].as_str().unwrap_or_default(),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            Some(serde_json::json!({
                "query": input.query,
                "results": results,
                "fingerprint": fingerprint,
            })),
            std::collections::BTreeMap::from([
                ("result_count".to_string(), results.len().to_string()),
                ("fingerprint".to_string(), fingerprint),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        name = "servers.status",
        summary = "Inspect configured MCP connection health and discovered tool counts.",
        read_only,
        mcp,
        ui_display = brief,
        concurrency_safe
    )]
    async fn invoke_servers_status(&self) -> SdkResult<ToolInvokeOutput> {
        let statuses = self.manager.statuses().await;
        let text = if statuses.is_empty() {
            "No MCP servers configured.".to_string()
        } else {
            statuses
                .iter()
                .map(|status| {
                    format!(
                        "- {}: {} ({} tools){}{}",
                        status.name,
                        if status.connected {
                            "connected"
                        } else {
                            "disconnected"
                        },
                        status.tool_count,
                        status
                            .last_error
                            .as_deref()
                            .map(|error| format!(" — {error}"))
                            .or_else(|| {
                                status
                                    .last_refresh_error
                                    .as_deref()
                                    .map(|error| format!(" — {error}"))
                            })
                            .unwrap_or_default(),
                        status_summary_suffix(status)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(ToolInvokeOutput::from_parts(
            "MCP server status",
            text,
            Some(
                serde_json::json!({ "servers": statuses.iter().map(|status| serde_json::json!({
                "name": status.name,
                "connected": status.connected,
                "tool_count": status.tool_count,
                "network_target": status.network_target,
                "last_error": status.last_error,
                "tool_generation": status.tool_generation,
                "resource_generation": status.resource_generation,
                "prompt_generation": status.prompt_generation,
                "last_refresh_error": status.last_refresh_error,
                "reconnect_supervisor_running": status.reconnect_supervisor_running,
                "auth_mode": status.auth_mode.as_str(),
                "oauth_health": status.oauth_health.as_ref().map(|health| serde_json::json!({
                    "credential_state": health.credential_state.as_str(),
                    "expiry_state": health.expiry_state.map(|state| state.as_str()),
                    "refresh_available": health.refresh_available,
                    "recommendation": oauth_health_recommendation(health),
                })),
                "credential_migration": status.credential_migration.map(|migration| serde_json::json!({
                    "state": migration.as_str(),
                    "recommendation": migration.recommendation(),
                })),
            })).collect::<Vec<_>>() }),
            ),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        name = "servers.reconnect",
        summary = "Reconnect one configured MCP server and refresh its tool cache.",
        mutating,
        mcp,
        ui_display = brief,
        network(connects = self.server_network_targets(input.server.as_str()).await?)
    )]
    async fn invoke_servers_reconnect(
        &self,
        input: &McpServerNameInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.manager
            .reconnect(input.server.as_str())
            .await
            .map_err(|error| {
                PluginError::new(format!("mcp:{} reconnect failed: {error}", input.server))
            })?;
        let status = self
            .manager
            .statuses()
            .await
            .into_iter()
            .find(|status| status.name == input.server);
        Ok(ToolInvokeOutput::from_parts(
            format!("MCP reconnect {}", input.server),
            format!("Reconnected MCP server '{}'.", input.server),
            Some(serde_json::json!({
                "server": input.server,
                "connected": status.as_ref().is_some_and(|status| status.connected),
                "tool_count": status.as_ref().map_or(0, |status| status.tool_count),
            })),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    async fn server_network_targets(&self, server: &str) -> SdkResult<Vec<String>> {
        let network_access = network_access_by_server(&self.manager).await;
        Ok(network_targets_for_server(&network_access, server))
    }

    #[hook(tool.definition)]
    async fn tool_definition_patch(
        &self,
        input: ToolDefinitionInput,
    ) -> SdkResult<Option<ToolDefinitionPatch>> {
        if input.plugin_key().to_string() != MCP_PLUGIN_ID {
            return Ok(None);
        }
        let servers = self.manager.server_names().await;
        let statuses = self.manager.statuses().await;
        let tools = self.manager.all_tools().await;
        let server_count = servers.len();
        let tool_count = tools.len();
        let mut lines = vec![
            "Use `resources.list` to list resources, `resources.templates.list` to list URI templates, `resources.read` to read a resource URI, `prompts.list` to list prompt templates, `prompts.get` to fetch a prompt template, and `tools.call` with `server`, `name`, and optional `arguments` to call a discovered MCP tool. Pass a returned cursor back to the corresponding list tool to continue pagination.".to_string(),
            format!("Currently configured servers: {server_count}."),
        ];
        if servers.is_empty() {
            lines.push("- none".to_string());
        } else {
            for server in &servers {
                lines.push(format!("- {server}"));
            }
        }
        lines.push(format!("Currently discovered MCP tools: {tool_count}."));
        lines.push(
            "Use `tools.search` to retrieve matching MCP tool schemas from the fingerprinted dynamic index; the full catalog is intentionally not embedded in this help."
                .to_string(),
        );
        let instructed = statuses
            .iter()
            .filter_map(|status| {
                status.instructions.as_deref().map(|instructions| {
                    let bounded = instructions.chars().take(2_000).collect::<String>();
                    format!("Server instructions for `{}`:\n{}", status.name, bounded)
                })
            })
            .collect::<Vec<_>>();
        if !instructed.is_empty() {
            lines.push("MCP servers supplied these initialization instructions:".to_string());
            lines.extend(instructed);
        }

        let summary = match input.tool_name() {
            "resources.list" => Some(format!(
                "List MCP resources from one server. {server_count} server(s) currently available."
            )),
            "tools.call" => Some(format!(
                "Call one discovered MCP tool. {tool_count} MCP tool(s) currently available."
            )),
            _ => None,
        };

        Ok(Some(ToolDefinitionPatch {
            summary,
            help: Some(lines.join("\n")),
            description_mode: None,
            input_schema: None,
        }))
    }
}

/// Render only redacted operational state. Do not add credential values or
/// keyring errors here: this string is visible to the model and terminal UI.
fn status_summary_suffix(status: &agena_mcp_client::McpServerStatus) -> String {
    let mut parts = vec![format!("; auth={}", status.auth_mode.as_str())];
    if let Some(health) = status.oauth_health.as_ref() {
        parts.push(format!(
            "; oauth={}{}",
            health.credential_state.as_str(),
            health
                .expiry_state
                .map(|state| format!("/{}", state.as_str()))
                .unwrap_or_default()
        ));
    }
    if let Some(migration) = status.credential_migration {
        parts.push(format!("; migration={}", migration.as_str()));
    }
    parts.concat()
}

fn oauth_health_recommendation(health: &agena_mcp_client::OAuthCredentialHealth) -> &'static str {
    use agena_mcp_client::{OAuthCredentialState, OAuthExpiryState};

    match (health.credential_state, health.expiry_state) {
        (OAuthCredentialState::Missing, _) => "run_mcp_login",
        (OAuthCredentialState::Unreadable, _) => "clear_or_reauthenticate",
        (_, Some(OAuthExpiryState::Expired)) => "reconnect_or_reauthenticate",
        (_, Some(OAuthExpiryState::Expiring)) if health.refresh_available == Some(false) => {
            "reauthenticate_before_expiry"
        }
        (_, Some(OAuthExpiryState::Unknown)) => "reconnect_to_verify",
        _ => "none",
    }
}

/// Normalize advisory MCP tool annotations into a stable, non-authoritative
/// risk projection. These hints are never used to relax the bridge's static
/// `mutating` permission tag: third-party metadata may only inform UI,
/// auditing, and future additional approval checks. In particular, an absent
/// or malformed hint remains `medium`, not `low`.
fn mcp_annotation_risk(annotations: Option<&Value>) -> Value {
    let read_only = annotations
        .and_then(|value| value.get("readOnlyHint"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let destructive = annotations
        .and_then(|value| value.get("destructiveHint"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let idempotent = annotations
        .and_then(|value| value.get("idempotentHint"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let open_world = annotations
        .and_then(|value| value.get("openWorldHint"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let level = if destructive || open_world {
        "high"
    } else if read_only {
        "low"
    } else {
        "medium"
    };
    serde_json::json!({
        "level": level,
        "read_only": read_only,
        "destructive": destructive,
        "idempotent": idempotent,
        "open_world": open_world,
        "advisory_only": true,
    })
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, ToolInput, PartialEq, Eq)]
#[input(trim("server"), non_empty("server"))]
#[serde(deny_unknown_fields)]
struct McpServerInput {
    server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, ToolInput, PartialEq, Eq)]
#[input(trim("server"), non_empty("server"))]
#[serde(deny_unknown_fields)]
struct McpServerNameInput {
    server: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, ToolInput, PartialEq, Eq)]
#[input(trim("query"), minimum("limit", 1), maximum("limit", 100))]
#[serde(deny_unknown_fields)]
struct McpToolSearchInput {
    #[serde(default)]
    query: String,
    #[serde(default = "default_mcp_search_limit")]
    limit: u32,
}

const fn default_mcp_search_limit() -> u32 {
    20
}

fn mcp_tool_index_fingerprint(results: &[serde_json::Value]) -> String {
    let encoded = serde_json::to_vec(results).unwrap_or_default();
    hex::encode(Sha256::digest(encoded))
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, ToolInput, PartialEq)]
#[input(trim("server", "name"), non_empty("server", "name"))]
#[serde(deny_unknown_fields)]
struct CallToolInput {
    server: String,
    name: String,
    #[serde(default)]
    arguments: Option<Value>,
}

async fn network_access_by_server(
    manager: &McpConnectionManager,
) -> BTreeMap<String, NetworkAccessSpec> {
    manager
        .server_network_targets()
        .await
        .into_iter()
        .map(|(server, target)| (server, NetworkAccessSpec { target }))
        .collect()
}

fn network_targets_for_server(
    network_access: &BTreeMap<String, NetworkAccessSpec>,
    server: &str,
) -> Vec<String> {
    network_access
        .get(server)
        .map(|spec| vec![spec.target.clone()])
        .unwrap_or_default()
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, ToolInput, PartialEq, Eq)]
#[input(trim("server", "uri"), non_empty("server", "uri"))]
#[serde(deny_unknown_fields)]
pub(super) struct ReadResourceInput {
    server: String,
    uri: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInput, Clone, PartialEq, Eq)]
#[input(trim("server", "name"), non_empty("server", "name"))]
#[serde(deny_unknown_fields)]
pub(super) struct GetPromptInput {
    server: String,
    name: String,
    #[serde(default)]
    arguments: Option<BTreeMap<String, String>>,
}

fn empty_object_to_none(input: Option<Value>) -> Option<Value> {
    match input {
        Some(Value::Null) | None => None,
        Some(Value::Object(map)) if map.is_empty() => None,
        other => other,
    }
}

fn invoke_tool_output(
    server: &str,
    tool: &str,
    result: CallToolResult,
) -> SdkResult<ToolInvokeOutput> {
    if result.is_error {
        let combined = result
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Err(PluginError::new(format!(
            "mcp:{server}:tool:{tool} returned isError=true: {combined}"
        )));
    }

    let mut blocks: Vec<OperationBlock> = result
        .content
        .iter()
        .filter_map(content_block_to_result_block)
        .collect();
    if let Some(structured) = result.structured_content.clone() {
        blocks.push(OperationBlock::Json { value: structured });
    }
    let output_text = blocks
        .iter()
        .filter_map(|block| match block {
            OperationBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let attachments = blocks
        .iter()
        .filter_map(OperationBlock::to_attachment_item)
        .collect::<Vec<AttachmentItem>>();
    let payload = serde_json::json!({
        "server": server,
        "tool": tool,
        "content_blocks": blocks,
        "structured_content": result.structured_content,
        "mcp_meta": result.meta,
    });

    Ok(ToolInvokeOutput {
        title: format!("MCP {server}/{tool}"),
        output_text: if output_text.is_empty() {
            format!(
                "(mcp:{server}:tool:{tool} returned {} content block(s))",
                result.content.len()
            )
        } else {
            output_text
        },
        payload: Some(payload),
        metadata: Default::default(),
        attachments,
    })
}

fn list_resources_output(server: &str, result: ListResourcesResult) -> SdkResult<ToolInvokeOutput> {
    let output_text = if result.resources.is_empty() {
        format!("MCP server '{server}' returned no resources.")
    } else {
        result
            .resources
            .iter()
            .map(|resource| {
                let name = resource.name.as_deref().unwrap_or(resource.uri.as_str());
                let mime = resource.mime_type.as_deref().unwrap_or("unknown mime");
                let description = resource.description.as_deref().unwrap_or_default();
                if description.is_empty() {
                    format!("- {name} ({}) [{mime}]", resource.uri)
                } else {
                    format!("- {name} ({}) [{mime}]: {description}", resource.uri)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let payload = serde_json::json!({
        "server": server,
        "resources": result.resources,
        "next_cursor": result.next_cursor,
    });
    Ok(ToolInvokeOutput {
        title: format!("MCP {server}/resources"),
        output_text,
        payload: Some(payload),
        metadata: Default::default(),
        attachments: Vec::new(),
    })
}

fn list_resource_templates_output(
    server: &str,
    result: ListResourceTemplatesResult,
) -> SdkResult<ToolInvokeOutput> {
    let output_text = if result.resource_templates.is_empty() {
        format!("MCP server '{server}' returned no resource templates.")
    } else {
        result
            .resource_templates
            .iter()
            .map(|template| {
                let mime = template.mime_type.as_deref().unwrap_or("unknown mime");
                let description = template.description.as_deref().unwrap_or_default();
                if description.is_empty() {
                    format!("- {} ({}) [{mime}]", template.name, template.uri_template)
                } else {
                    format!(
                        "- {} ({}) [{mime}]: {description}",
                        template.name, template.uri_template
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    Ok(ToolInvokeOutput {
        title: format!("MCP {server}/resource templates"),
        output_text,
        payload: Some(serde_json::json!({
            "server": server,
            "resource_templates": result.resource_templates,
            "next_cursor": result.next_cursor,
        })),
        metadata: Default::default(),
        attachments: Vec::new(),
    })
}

fn read_resource_output(
    server: &str,
    uri: &str,
    result: ReadResourceResult,
) -> SdkResult<ToolInvokeOutput> {
    let blocks = result
        .contents
        .iter()
        .map(resource_contents_to_result_block)
        .collect::<Vec<_>>();
    let attachments = blocks
        .iter()
        .filter_map(OperationBlock::to_attachment_item)
        .collect::<Vec<AttachmentItem>>();
    let output_text = result
        .contents
        .iter()
        .map(|content| {
            let mime = content.mime_type.as_deref().unwrap_or("unknown mime");
            if let Some(text) = content.text.as_deref() {
                format!("{} [{mime}]\n{text}", content.uri)
            } else {
                format!("{} [{mime}] (base64 blob)", content.uri)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let payload = serde_json::json!({
        "server": server,
        "uri": uri,
        "contents": result.contents,
    });
    Ok(ToolInvokeOutput {
        title: format!("MCP {server}/resource"),
        output_text: if output_text.is_empty() {
            format!("MCP server '{server}' returned no content for resource '{uri}'.")
        } else {
            output_text
        },
        payload: Some(payload),
        metadata: Default::default(),
        attachments,
    })
}

fn list_prompts_output(server: &str, result: ListPromptsResult) -> SdkResult<ToolInvokeOutput> {
    let output_text = if result.prompts.is_empty() {
        format!("MCP server '{server}' returned no prompts.")
    } else {
        result
            .prompts
            .iter()
            .map(|prompt| {
                let description = prompt.description.as_deref().unwrap_or_default();
                let args = prompt
                    .arguments
                    .iter()
                    .map(|arg| {
                        if arg.required {
                            format!("{}*", arg.name)
                        } else {
                            arg.name.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                match (description.is_empty(), args.is_empty()) {
                    (true, true) => format!("- {}", prompt.name),
                    (false, true) => format!("- {}: {description}", prompt.name),
                    (true, false) => format!("- {} ({args})", prompt.name),
                    (false, false) => format!("- {} ({args}): {description}", prompt.name),
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let payload = serde_json::json!({
        "server": server,
        "prompts": result.prompts,
        "next_cursor": result.next_cursor,
    });
    Ok(ToolInvokeOutput {
        title: format!("MCP {server}/prompts"),
        output_text,
        payload: Some(payload),
        metadata: Default::default(),
        attachments: Vec::new(),
    })
}

fn get_prompt_output(
    server: &str,
    name: &str,
    result: GetPromptResult,
) -> SdkResult<ToolInvokeOutput> {
    let output_text = result
        .messages
        .iter()
        .map(|message| {
            format!(
                "{}: {}",
                message.role,
                content_block_summary(&message.content)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let payload = serde_json::json!({
        "server": server,
        "prompt": name,
        "description": result.description,
        "messages": result.messages,
    });
    Ok(ToolInvokeOutput {
        title: format!("MCP {server}/prompt {name}"),
        output_text: if output_text.is_empty() {
            format!("MCP server '{server}' returned no messages for prompt '{name}'.")
        } else {
            output_text
        },
        payload: Some(payload),
        metadata: Default::default(),
        attachments: Vec::new(),
    })
}

fn content_block_to_result_block(block: &ContentBlock) -> Option<OperationBlock> {
    match block {
        ContentBlock::Text { text, .. } => Some(OperationBlock::Text { text: text.clone() }),
        ContentBlock::Image {
            data, mime_type, ..
        } => Some(OperationBlock::Image {
            mime: mime_type.clone(),
            url: format!("data:{};base64,{}", mime_type, data),
        }),
        ContentBlock::Audio {
            data, mime_type, ..
        } => Some(OperationBlock::Audio {
            mime: mime_type.clone(),
            url: format!("data:{};base64,{}", mime_type, data),
        }),
        ContentBlock::Resource { resource, .. } => {
            Some(resource_contents_to_result_block(resource))
        }
        ContentBlock::ResourceLink { resource } => Some(OperationBlock::ResourceLink {
            uri: resource.uri.clone(),
            title: resource.title.clone().or_else(|| resource.name.clone()),
            mime_type: resource.mime_type.clone(),
        }),
        ContentBlock::Unknown { raw } => Some(OperationBlock::Json { value: raw.clone() }),
    }
}

fn resource_contents_to_result_block(resource: &ResourceContents) -> OperationBlock {
    OperationBlock::EmbeddedResource {
        uri: resource.uri.clone(),
        mime: resource.mime_type.clone().unwrap_or_default(),
        text: resource.text.clone(),
        base64: resource.blob.clone(),
    }
}

fn content_block_summary(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text { text, .. } => text.clone(),
        ContentBlock::Image { mime_type, .. } => format!("[image: {mime_type}]"),
        ContentBlock::Audio { mime_type, .. } => format!("[audio: {mime_type}]"),
        ContentBlock::Resource { resource, .. } => format!("[resource: {}]", resource.uri),
        ContentBlock::ResourceLink { resource } => format!("[resource link: {}]", resource.uri),
        ContentBlock::Unknown { raw } => format!("[unknown MCP content: {raw}]"),
    }
}
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agena_plugin_host::sdk::Plugin;

    use super::{McpConnectionManager, McpPlugin, mcp_tool_index_fingerprint};

    #[test]
    fn manifest_exposes_search_health_and_reconnect() {
        let manifest = McpPlugin::new(Arc::new(McpConnectionManager::new("test", "1"))).manifest();
        let names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        for name in ["tools.search", "servers.status", "servers.reconnect"] {
            assert!(names.contains(&name), "missing {name}");
        }
    }

    #[test]
    fn tool_index_fingerprint_is_stable_and_content_sensitive() {
        let first = vec![serde_json::json!({"server":"a","name":"read"})];
        let second = vec![serde_json::json!({"server":"a","name":"write"})];
        assert_eq!(
            mcp_tool_index_fingerprint(&first),
            mcp_tool_index_fingerprint(&first)
        );
        assert_ne!(
            mcp_tool_index_fingerprint(&first),
            mcp_tool_index_fingerprint(&second)
        );
    }
}
