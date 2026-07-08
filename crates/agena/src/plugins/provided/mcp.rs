//! In-process plugin that exposes configured MCP server capabilities as plugin
//! tools.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use agena_macros::ToolInput;
use agena_mcp_client::protocol::{
    CallToolResult, ContentBlock, GetPromptResult, ListPromptsResult, ListResourcesResult,
    ReadResourceResult, ResourceContents,
};
use agena_mcp_client::{FileTokenStore, McpConnectionManager, ServerSpec, TokenStore};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::message::{AttachmentItem, OperationBlock};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    InitOutcome, NetworkAccessSpec, NetworkRequest, PluginManifest, Result as SdkResult,
    ToolDefinitionInput, ToolDefinitionPatch, ToolInvokeOutput,
};

pub(crate) const MCP_PLUGIN_ID: &str = "agena.mcp";

pub(crate) fn static_manifest() -> PluginManifest {
    let manager = Arc::new(McpConnectionManager::new(
        crate::provider::CODEX_MCP_CLIENT_NAME,
        crate::provider::CODEX_PACKAGE_VERSION,
    ));
    crate::plugin::sdk::Plugin::manifest(&McpPlugin::new(manager))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct McpConfig {
    pub runtime: McpRuntimeConfig,
    /// Map of `<server_name> -> <transport spec>`.
    pub servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct McpRuntimeConfig {
    pub token_store: McpTokenStoreConfig,
}

impl Default for McpRuntimeConfig {
    fn default() -> Self {
        Self {
            token_store: McpTokenStoreConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct McpTokenStoreConfig {
    pub enabled: bool,
}

impl Default for McpTokenStoreConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum McpServerConfig {
    /// Spawn a child process and exchange newline-delimited JSON over
    /// its stdin/stdout (the typical MCP server style).
    Stdio { process: McpStdioProcessConfig },
    /// Connect to a streamable HTTP MCP server.
    Http {
        endpoint: McpHttpEndpointConfig,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<McpHttpAuthConfig>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct McpStdioProcessConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct McpHttpEndpointConfig {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

impl Default for McpHttpEndpointConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            headers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum McpHttpAuthConfig {
    /// Static `Authorization: Bearer <token>`.
    Bearer { token: String },
    /// Read the bearer token from the named env var at connect time.
    BearerFromEnv { env: String },
    /// Resolve via the runtime's MCP token store.
    BearerFromStore,
    /// Free-form header map.
    Custom { headers: BTreeMap<String, String> },
}

pub(crate) fn config_from_plugins(
    plugins: &crate::plugin::PluginsConfig,
) -> Result<McpConfig, String> {
    let Some(configured_plugin) = plugins.list.get(MCP_PLUGIN_ID) else {
        return Ok(McpConfig::default());
    };
    if configured_plugin.disabled()
        || !matches!(
            configured_plugin.package,
            crate::plugin::PluginPackage::Static { .. }
        )
    {
        return Ok(McpConfig::default());
    }
    if configured_plugin.config().is_null() {
        return Ok(McpConfig::default());
    }
    serde_json::from_value(configured_plugin.config().clone())
        .map_err(|err| format!("plugins.list.\"{MCP_PLUGIN_ID}\".config: {err}"))
}

pub(crate) fn static_bridge_enabled(plugins: &crate::plugin::PluginsConfig) -> bool {
    plugins
        .list
        .get(MCP_PLUGIN_ID)
        .is_some_and(|configured_plugin| {
            !configured_plugin.disabled()
                && matches!(
                    configured_plugin.package,
                    crate::plugin::PluginPackage::Static { .. }
                )
        })
}

pub(crate) async fn build_manager(config: &McpConfig) -> Arc<McpConnectionManager> {
    let mut manager = McpConnectionManager::new(
        crate::provider::CODEX_MCP_CLIENT_NAME,
        crate::provider::CODEX_PACKAGE_VERSION,
    );

    if config.runtime.token_store.enabled {
        match FileTokenStore::open_default() {
            Ok(store) => {
                manager.set_token_store(Arc::new(store) as Arc<dyn TokenStore>);
            }
            Err(err) => {
                tracing::warn!(
                    target: "agena::mcp",
                    "failed to open default token store: {err}"
                );
            }
        }
    }

    let manager = Arc::new(manager);
    for (name, server_config) in &config.servers {
        let manager = manager.clone();
        let name = name.clone();
        let spec = match server_config {
            McpServerConfig::Stdio { process } => ServerSpec::Stdio {
                command: process.command.clone(),
                args: process.args.clone(),
                env: process
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                cwd: process.cwd.clone(),
            },
            McpServerConfig::Http { endpoint, auth } => {
                let Some(parsed) = parse_mcp_server_url(name.as_str(), endpoint.url.as_str())
                else {
                    continue;
                };
                let auth = map_mcp_auth(auth.as_ref());
                ServerSpec::Http {
                    url: parsed,
                    headers: endpoint
                        .headers
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    auth,
                }
            }
        };
        if let Err(e) = manager.add_server(&name, spec).await {
            tracing::warn!(
                target: "agena::mcp",
                "failed to connect MCP server '{name}': {e}"
            );
        } else {
            tracing::info!(target: "agena::mcp", "connected MCP server '{name}'");
        }
    }
    manager
}

fn parse_mcp_server_url(name: &str, url: &str) -> Option<url::Url> {
    match url::Url::parse(url) {
        Ok(parsed) => Some(parsed),
        Err(err) => {
            tracing::warn!(
                target: "agena::mcp",
                "skipping mcp server '{name}': invalid url '{url}': {err}"
            );
            None
        }
    }
}

fn map_mcp_auth(auth: Option<&McpHttpAuthConfig>) -> Option<agena_mcp_client::HttpAuth> {
    auth.map(|cfg| match cfg {
        McpHttpAuthConfig::Bearer { token } => agena_mcp_client::HttpAuth::Bearer(token.clone()),
        McpHttpAuthConfig::BearerFromEnv { env } => {
            agena_mcp_client::HttpAuth::BearerFromEnv(env.clone())
        }
        McpHttpAuthConfig::BearerFromStore => agena_mcp_client::HttpAuth::BearerFromStore,
        McpHttpAuthConfig::Custom { headers } => agena_mcp_client::HttpAuth::Custom(
            headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
    })
}

pub(crate) struct McpPlugin {
    manager: Arc<McpConnectionManager>,
}

fn mcp_config_schema() -> Value {
    crate::tool::definition::json_schema_for_default_with_metadata(
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
                "Opens the default token store so MCP HTTP servers can resolve tokens at connect time.",
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

#[crate::plugin::sdk::agena_plugin(
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

    #[hook]
    async fn init(
        &self,
        _ctx: crate::plugin::sdk::InitContext,
        _host: Arc<dyn crate::plugin::sdk::HostClient>,
    ) -> SdkResult<crate::plugin::sdk::InitOutcome> {
        Ok(InitOutcome::ack(crate::plugin::sdk::Plugin::manifest(self)))
    }

    #[tool(
        name = "resources.list",
        summary = "List MCP resources from one server.",
        read_only,
        mcp,
        ui_display = brief,
        permission(networks = permission_networks_resources_list),
        concurrency_safe
    )]
    async fn invoke_resources_list(&self, input: &McpServerInput) -> SdkResult<ToolInvokeOutput> {
        let result = self
            .manager
            .list_resources(&input.server)
            .await
            .map_err(|err| {
                PluginError::new(format!("mcp:{}:resources:list failed: {err}", input.server))
            })?;
        list_resources_output(&input.server, result)
    }

    #[tool(
        name = "resources.read",
        summary = "Read one MCP resource by URI.",
        read_only,
        mcp,
        ui_display = brief,
        permission(networks = permission_networks_resources_read),
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
        permission(networks = permission_networks_prompts_list),
        concurrency_safe
    )]
    async fn invoke_prompts_list(&self, input: &McpServerInput) -> SdkResult<ToolInvokeOutput> {
        let result = self
            .manager
            .list_prompts(&input.server)
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
        permission(networks = permission_networks_prompts_get),
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
        permission(networks = permission_networks_tools_call)
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

    async fn permission_networks_for_server(&self, server: &str) -> SdkResult<Vec<NetworkRequest>> {
        let network_access = network_access_by_server(&self.manager).await;
        Ok(network_requests_for_server(&network_access, server))
    }

    async fn permission_networks_resources_list(
        &self,
        input: &McpServerInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        self.permission_networks_for_server(&input.server).await
    }

    async fn permission_networks_resources_read(
        &self,
        input: &ReadResourceInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        self.permission_networks_for_server(&input.server).await
    }

    async fn permission_networks_prompts_list(
        &self,
        input: &McpServerInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        self.permission_networks_for_server(&input.server).await
    }

    async fn permission_networks_prompts_get(
        &self,
        input: &GetPromptInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        self.permission_networks_for_server(&input.server).await
    }

    async fn permission_networks_tools_call(
        &self,
        input: &CallToolInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        self.permission_networks_for_server(&input.server).await
    }

    #[hook(tool_definition)]
    async fn tool_definition_patch(
        &self,
        input: ToolDefinitionInput,
    ) -> SdkResult<Option<ToolDefinitionPatch>> {
        if input.plugin_key().to_string() != MCP_PLUGIN_ID {
            return Ok(None);
        }
        let servers = self.manager.server_names().await;
        let tools = self.manager.all_tools().await;
        let server_count = servers.len();
        let tool_count = tools.len();
        let mut lines = vec![
            "Use `resources.list` to list resources, `resources.read` to read a resource URI, `prompts.list` to list prompt templates, `prompts.get` to fetch a prompt template, and `tools.call` with `server`, `name`, and optional `arguments` to call a discovered MCP tool.".to_string(),
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
        if tools.is_empty() {
            lines.push("- none".to_string());
        } else {
            for (server, tool) in tools.iter().take(24) {
                let description = tool.description.as_deref().unwrap_or("").trim();
                if description.is_empty() {
                    lines.push(format!("- {server}/{}", tool.name));
                } else {
                    lines.push(format!("- {server}/{}: {description}", tool.name));
                }
            }
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

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, ToolInput, PartialEq, Eq)]
#[input(trim("server"), non_empty("server"))]
#[serde(deny_unknown_fields)]
struct McpServerInput {
    server: String,
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

fn network_requests_for_server(
    network_access: &BTreeMap<String, NetworkAccessSpec>,
    server: &str,
) -> Vec<NetworkRequest> {
    network_access
        .get(server)
        .map(|spec| vec![NetworkRequest::connect(spec.target.clone())])
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
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Err(PluginError::new(format!(
            "mcp:{server}:tool:{tool} returned isError=true: {combined}"
        )));
    }

    let blocks: Vec<OperationBlock> = result
        .content
        .iter()
        .filter_map(content_block_to_result_block)
        .collect();
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
        ContentBlock::Text { text } => Some(OperationBlock::Text { text: text.clone() }),
        ContentBlock::Image { data, mime_type } => Some(OperationBlock::Image {
            mime: mime_type.clone(),
            url: format!("data:{};base64,{}", mime_type, data),
        }),
        ContentBlock::Resource { resource } => Some(resource_contents_to_result_block(resource)),
        ContentBlock::Other => None,
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
        ContentBlock::Text { text } => text.clone(),
        ContentBlock::Image { mime_type, .. } => format!("[image: {mime_type}]"),
        ContentBlock::Resource { resource } => format!("[resource: {}]", resource.uri),
        ContentBlock::Other => "[unsupported content block]".to_string(),
    }
}

pub(crate) fn block_on<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        match handle.runtime_flavor() {
            tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(|| handle.block_on(fut))
            }
            _ => block_on_fallback_runtime(fut),
        }
    } else {
        block_on_fallback_runtime(fut)
    }
}

static FALLBACK_RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("agena-mcp-fallback")
        .thread_stack_size(32 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("mcp plugin fallback runtime")
});

fn block_on_fallback_runtime<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(move || FALLBACK_RUNTIME.handle().block_on(fut))
            .join()
            .expect("mcp plugin fallback runtime thread panicked")
    })
}
