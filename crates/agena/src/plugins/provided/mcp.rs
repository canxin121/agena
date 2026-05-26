//! In-process plugin that exposes configured MCP server capabilities as plugin
//! tools.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use agena_macros::StaticToolSurface;
use agena_mcp_client::protocol::{
    CallToolResult, ContentBlock, GetPromptResult, ListPromptsResult, ListResourcesResult,
    ReadResourceResult, ResourceContents, ToolDescriptor,
};
use agena_mcp_client::{FileTokenStore, McpConnectionManager, ServerSpec, TokenStore};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::message::{AttachmentItem, OperationBlock};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, NetworkAccessSpec, NetworkRequest, Plugin,
    PluginManifest, PluginToolDecl, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput,
    ToolTag,
};

pub(crate) const MCP_PLUGIN_ID: &str = "agena.mcp";

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
    let mut schema = crate::tool::definition::json_schema_for_with_default(McpConfig::default());
    for (pointer, title, description) in [
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

impl McpPlugin {
    pub(crate) fn new(manager: Arc<McpConnectionManager>) -> Self {
        Self { manager }
    }

    fn manifest_from_snapshot(
        &self,
        servers: Vec<String>,
        tools: Vec<(String, ToolDescriptor)>,
        network_access: BTreeMap<String, NetworkAccessSpec>,
    ) -> PluginManifest {
        manifest_from_snapshot(servers, tools, &network_access)
    }
}

#[async_trait]
impl Plugin for McpPlugin {
    fn manifest(&self) -> PluginManifest {
        let manager = Arc::clone(&self.manager);
        let (servers, tools, network_access) = block_on(async move {
            let servers = manager.server_names().await;
            let tools = manager.all_tools().await;
            let network_access = network_access_by_server(&manager).await;
            (servers, tools, network_access)
        });
        self.manifest_from_snapshot(servers, tools, network_access)
    }

    async fn init(
        &self,
        _ctx: InitContext,
        _host: Arc<dyn crate::plugin::sdk::HostClient>,
    ) -> SdkResult<InitOutcome> {
        let servers = self.manager.server_names().await;
        let tools = self.manager.all_tools().await;
        let network_access = network_access_by_server(&self.manager).await;
        Ok(InitOutcome::ack(self.manifest_from_snapshot(
            servers,
            tools,
            network_access,
        )))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let target = target_from_invocation(input.tool_name.as_str(), input.input)?;
        match target {
            McpToolTarget::Tool {
                server,
                tool,
                arguments,
            } => {
                let result = self
                    .manager
                    .call_tool(&server, &tool, arguments)
                    .await
                    .map_err(|err| {
                        PluginError::new(format!("mcp:{server}:tool:{tool} call failed: {err}"))
                    })?;
                invoke_tool_output(&server, &tool, result)
            }
            McpToolTarget::ListResources { server } => {
                let result = self.manager.list_resources(&server).await.map_err(|err| {
                    PluginError::new(format!("mcp:{server}:resources:list failed: {err}"))
                })?;
                list_resources_output(&server, result)
            }
            McpToolTarget::ReadResource { server } => {
                let result = self
                    .manager
                    .read_resource(&server.server, server.uri.as_str())
                    .await
                    .map_err(|err| {
                        PluginError::new(format!(
                            "mcp:{}:resources:read '{}' failed: {err}",
                            server.server, server.uri
                        ))
                    })?;
                read_resource_output(&server.server, server.uri.as_str(), result)
            }
            McpToolTarget::ListPrompts { server } => {
                let result = self.manager.list_prompts(&server).await.map_err(|err| {
                    PluginError::new(format!("mcp:{server}:prompts:list failed: {err}"))
                })?;
                list_prompts_output(&server, result)
            }
            McpToolTarget::GetPrompt { server } => {
                let server_name = server.server;
                let prompt_name = server.name;
                let result = self
                    .manager
                    .get_prompt(&server_name, prompt_name.as_str(), server.arguments)
                    .await
                    .map_err(|err| {
                        PluginError::new(format!(
                            "mcp:{}:prompts:get '{}' failed: {err}",
                            server_name, prompt_name
                        ))
                    })?;
                get_prompt_output(&server_name, prompt_name.as_str(), result)
            }
        }
    }

    async fn permission_networks(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<NetworkRequest>> {
        let target = target_from_invocation(tool, input.clone())?;
        let server = target.server_name();
        let network_access = network_access_by_server(&self.manager).await;
        Ok(network_access
            .get(server)
            .map(|spec| vec![NetworkRequest::connect(spec.target.clone())])
            .unwrap_or_default())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum McpToolTarget {
    Tool {
        server: String,
        tool: String,
        arguments: Option<Value>,
    },
    ListResources {
        server: String,
    },
    ReadResource {
        server: ReadResourceInput,
    },
    ListPrompts {
        server: String,
    },
    GetPrompt {
        server: GetPromptInput,
    },
}

impl McpToolTarget {
    fn server_name(&self) -> &str {
        match self {
            Self::Tool { server, .. }
            | Self::ListResources { server }
            | Self::ListPrompts { server } => server,
            Self::ReadResource { server } => server.server.as_str(),
            Self::GetPrompt { server } => server.server.as_str(),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "mcp",
    description = "MCP bridge command. Use action `list_resources`, `read_resource`, `list_prompts`, `get_prompt`, or `call` to access capabilities exposed by configured MCP servers.",
    summary = "Read MCP resources or prompt templates, or call discovered MCP tools.",
    help = "Use action `list_resources`, `read_resource`, `list_prompts`, `get_prompt`, or `call`. MCP prompts here are server-provided prompt templates/messages, not Agena chat prompts or permission prompts.",
    tags(ToolTag::ReadOnly, ToolTag::Mutating, ToolTag::Mcp),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum McpToolInput {
    #[tool(exec = "list_resources")]
    ListResources {
        #[serde(flatten)]
        args: McpServerInput,
    },
    #[tool(exec = "read_resource")]
    ReadResource {
        #[serde(flatten)]
        args: ReadResourceInput,
    },
    #[tool(exec = "list_prompts")]
    ListPrompts {
        #[serde(flatten)]
        args: McpServerInput,
    },
    #[tool(exec = "get_prompt")]
    GetPrompt {
        #[serde(flatten)]
        args: GetPromptInput,
    },
    #[tool(exec = "call")]
    Call {
        #[serde(flatten)]
        args: CallToolInput,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct McpServerInput {
    server: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
struct CallToolInput {
    server: String,
    name: String,
    #[serde(default)]
    arguments: Option<Value>,
}

pub(super) fn target_from_invocation(tool: &str, input: Value) -> SdkResult<McpToolTarget> {
    let (action, args) = McpToolInput::resolve_tool(tool, input)?;
    match action.as_str() {
        "list_resources" => {
            let args: McpServerInput = serde_json::from_value(args)?;
            Ok(McpToolTarget::ListResources {
                server: args.server,
            })
        }
        "read_resource" => {
            let args: ReadResourceInput = serde_json::from_value(args)?;
            Ok(McpToolTarget::ReadResource { server: args })
        }
        "list_prompts" => {
            let args: McpServerInput = serde_json::from_value(args)?;
            Ok(McpToolTarget::ListPrompts {
                server: args.server,
            })
        }
        "get_prompt" => {
            let args: GetPromptInput = serde_json::from_value(args)?;
            Ok(McpToolTarget::GetPrompt { server: args })
        }
        "call" => {
            let args: CallToolInput = serde_json::from_value(args)?;
            Ok(McpToolTarget::Tool {
                server: args.server,
                tool: args.name,
                arguments: empty_object_to_none(args.arguments),
            })
        }
        other => Err(PluginError::invalid_params(format!(
            "invalid MCP action '{other}'"
        ))),
    }
}

fn manifest_from_snapshot(
    servers: Vec<String>,
    tools: Vec<(String, ToolDescriptor)>,
    network_access: &BTreeMap<String, NetworkAccessSpec>,
) -> PluginManifest {
    let mut tool_decls = Vec::new();
    if !servers.is_empty() || !tools.is_empty() {
        tool_decls.push(mcp_decl(&servers, &tools, !network_access.is_empty()));
    }
    PluginManifest::builder(MCP_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
        .description("Agena MCP bridge exposed as hierarchical plugin commands.")
        .hooks(HookSubscription::TOOL_INVOKE)
        .config_schema(mcp_config_schema())
        .tools(tool_decls)
        .build()
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

fn mcp_decl(
    servers: &[String],
    tools: &[(String, ToolDescriptor)],
    has_network_servers: bool,
) -> PluginToolDecl {
    let server_count = servers.len();
    let tool_count = tools.len();
    let tool_decl = McpToolInput::tool_decl()
    .description(format!(
        "MCP command for {server_count} configured server(s) and {tool_count} discovered tool(s). Set action to list_resources, read_resource, list_prompts, get_prompt, or call. MCP prompts are server-provided prompt templates/messages, not Agena chat prompts."
    ))
    .summary(format!(
        "Read MCP resources/prompts or call discovered MCP tools across {server_count} server(s)."
    ))
    .help(mcp_help(servers, tools))
    .tags([ToolTag::ReadOnly, ToolTag::Mutating, ToolTag::Mcp]);
    maybe_network_tag(tool_decl, has_network_servers)
}

fn mcp_help(servers: &[String], tools: &[(String, ToolDescriptor)]) -> String {
    let mut lines = vec![
        "Use action `list_resources` to list resources, `read_resource` to read a resource URI, `list_prompts` to list prompts, `get_prompt` to fetch a prompt, and `call` with `server`, `name`, and optional `arguments` to call a discovered MCP tool.".to_string(),
        "Configured servers:".to_string(),
    ];
    for server in servers {
        lines.push(format!("- {server}"));
    }
    lines.push("Discovered MCP tools:".to_string());
    for (server, tool) in tools {
        let description = tool.description.as_deref().unwrap_or("").trim();
        if description.is_empty() {
            lines.push(format!("- {server}/{}", tool.name));
        } else {
            lines.push(format!("- {server}/{}: {description}", tool.name));
        }
        if let Some(schema) = tool.input_schema.as_ref()
            && let Ok(schema) = serde_json::to_string_pretty(schema)
        {
            lines.push(format!("  inputSchema: {schema}"));
        }
    }
    lines.join("\n")
}

fn maybe_network_tag(tool_decl: PluginToolDecl, has_network_servers: bool) -> PluginToolDecl {
    if has_network_servers {
        tool_decl.tag(ToolTag::Network)
    } else {
        tool_decl
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadResourceInput {
    server: String,
    uri: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, PartialEq, Eq)]
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
    if matches!(result.is_error, Some(true)) {
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
                        if arg.required.unwrap_or(false) {
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
#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use agena_mcp_client::protocol::{
        PromptArgument, PromptDescriptor, PromptMessage, ResourceDescriptor,
    };

    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_invocation_routes_hierarchical_commands() {
        assert_eq!(
            target_from_invocation(
                "mcp",
                serde_json::json!({
                    "action": "call",
                    "server": "docs",
                    "name": "search",
                    "arguments": { "q": "rust" }
                })
            )
            .expect("mcp tool target"),
            McpToolTarget::Tool {
                server: "docs".to_string(),
                tool: "search".to_string(),
                arguments: Some(serde_json::json!({ "q": "rust" })),
            }
        );
        assert_eq!(
            target_from_invocation(
                "mcp",
                serde_json::json!({
                    "action": "list_resources",
                    "server": "docs"
                })
            )
            .expect("mcp resources list target"),
            McpToolTarget::ListResources {
                server: "docs".to_string()
            }
        );
        assert_eq!(
            target_from_invocation(
                "mcp",
                serde_json::json!({
                    "action": "read_resource",
                    "server": "docs",
                    "uri": "file:///README.md"
                })
            )
            .expect("mcp resources read target"),
            McpToolTarget::ReadResource {
                server: ReadResourceInput {
                    server: "docs".to_string(),
                    uri: "file:///README.md".to_string(),
                }
            }
        );
        assert_eq!(
            target_from_invocation(
                "mcp",
                serde_json::json!({
                    "action": "list_prompts",
                    "server": "docs"
                })
            )
            .expect("mcp prompts list target"),
            McpToolTarget::ListPrompts {
                server: "docs".to_string()
            }
        );
        assert_eq!(
            target_from_invocation(
                "mcp",
                serde_json::json!({
                    "action": "get_prompt",
                    "server": "docs",
                    "name": "summarize"
                })
            )
            .expect("mcp prompts get target"),
            McpToolTarget::GetPrompt {
                server: GetPromptInput {
                    server: "docs".to_string(),
                    name: "summarize".to_string(),
                    arguments: None,
                }
            }
        );
        assert!(target_from_invocation("mcp:docs:search", serde_json::json!({})).is_err());
        assert!(
            target_from_invocation(
                "mcp",
                serde_json::json!({
                    "action": "tool",
                    "server": "docs",
                    "name": "search"
                })
            )
            .is_err()
        );
    }

    #[test]
    fn nested_mcp_config_parses() {
        let config: McpConfig = serde_json::from_value(json!({
            "runtime": {
                "token_store": {
                    "enabled": false
                }
            },
            "servers": {
                "docs": {
                    "transport": "http",
                    "endpoint": {
                        "url": "https://example.com/mcp",
                        "headers": {
                            "x-client": "agena"
                        }
                    },
                    "auth": {
                        "kind": "bearer_from_env",
                        "env": "MCP_TOKEN"
                    }
                },
                "local": {
                    "transport": "stdio",
                    "process": {
                        "command": "npx",
                        "args": ["-y", "@modelcontextprotocol/server-filesystem"],
                        "cwd": "/tmp/mcp"
                    }
                }
            }
        }))
        .expect("nested mcp config should parse");

        assert!(!config.runtime.token_store.enabled);
        match config.servers.get("docs").expect("http server") {
            McpServerConfig::Http { endpoint, auth } => {
                assert_eq!(endpoint.url, "https://example.com/mcp");
                assert_eq!(
                    endpoint.headers.get("x-client").map(String::as_str),
                    Some("agena")
                );
                assert!(matches!(
                    auth,
                    Some(McpHttpAuthConfig::BearerFromEnv { env }) if env == "MCP_TOKEN"
                ));
            }
            other => panic!("unexpected config: {other:?}"),
        }
        match config.servers.get("local").expect("stdio server") {
            McpServerConfig::Stdio { process } => {
                assert_eq!(process.command, "npx");
                assert_eq!(
                    process
                        .cwd
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string()),
                    Some("/tmp/mcp".to_string())
                );
            }
            other => panic!("unexpected config: {other:?}"),
        }
    }

    #[test]
    fn legacy_mcp_http_shape_is_rejected() {
        let err = serde_json::from_value::<McpConfig>(json!({
            "servers": {
                "docs": {
                    "transport": "http",
                    "url": "https://example.com/mcp"
                }
            }
        }))
        .expect_err("legacy mcp config should fail");

        assert!(err.to_string().contains("unknown field `url`"));
    }

    #[test]
    fn mcp_manifest_includes_hierarchical_entries() {
        let manifest = manifest_from_snapshot(
            vec!["docs".to_string()],
            vec![(
                "docs".to_string(),
                ToolDescriptor {
                    name: "search".to_string(),
                    description: Some("Search docs".to_string()),
                    input_schema: None,
                },
            )],
            &BTreeMap::new(),
        );
        let names = manifest
            .tools
            .iter()
            .map(|tool_decl| tool_decl.name.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(names, BTreeSet::from(["mcp"]));
    }

    #[test]
    fn mcp_manifest_marks_hierarchical_entries_for_network_servers() {
        let mut network_access = BTreeMap::new();
        network_access.insert(
            "remote".to_string(),
            NetworkAccessSpec {
                target: "https://mcp.example.com/".to_string(),
            },
        );
        let manifest = manifest_from_snapshot(
            vec!["local".to_string(), "remote".to_string()],
            vec![
                (
                    "local".to_string(),
                    ToolDescriptor {
                        name: "read".to_string(),
                        description: None,
                        input_schema: None,
                    },
                ),
                (
                    "remote".to_string(),
                    ToolDescriptor {
                        name: "search".to_string(),
                        description: None,
                        input_schema: None,
                    },
                ),
            ],
            &network_access,
        );

        assert_eq!(manifest.tools.len(), 1);
        for tool_decl in &manifest.tools {
            assert!(tool_decl.network_access.is_empty());
            assert!(tool_decl.tags.iter().any(|tag| tag == &ToolTag::Network));
        }
    }

    #[test]
    fn mcp_resource_outputs_are_rendered() {
        let list = list_resources_output(
            "docs",
            ListResourcesResult {
                resources: vec![ResourceDescriptor {
                    uri: "file:///README.md".to_string(),
                    name: Some("README".to_string()),
                    description: Some("Project readme".to_string()),
                    mime_type: Some("text/markdown".to_string()),
                }],
                next_cursor: None,
            },
        )
        .expect("list resources output");
        assert!(list.output_text.contains("README"));
        assert!(list.output_text.contains("Project readme"));

        let read = read_resource_output(
            "docs",
            "file:///README.md",
            ReadResourceResult {
                contents: vec![ResourceContents {
                    uri: "file:///README.md".to_string(),
                    mime_type: Some("text/markdown".to_string()),
                    text: Some("# Demo".to_string()),
                    blob: None,
                }],
            },
        )
        .expect("read resource output");
        assert!(read.output_text.contains("# Demo"));
        assert_eq!(read.attachments.len(), 1);
    }

    #[test]
    fn mcp_prompt_outputs_are_rendered() {
        let list = list_prompts_output(
            "docs",
            ListPromptsResult {
                prompts: vec![PromptDescriptor {
                    name: "summarize".to_string(),
                    description: Some("Summarize docs".to_string()),
                    arguments: vec![PromptArgument {
                        name: "topic".to_string(),
                        description: None,
                        required: Some(true),
                    }],
                }],
                next_cursor: None,
            },
        )
        .expect("list prompts output");
        assert!(list.output_text.contains("summarize"));
        assert!(list.output_text.contains("topic*"));

        let get = get_prompt_output(
            "docs",
            "summarize",
            GetPromptResult {
                description: Some("Summarize docs".to_string()),
                messages: vec![PromptMessage {
                    role: "user".to_string(),
                    content: ContentBlock::Text {
                        text: "Summarize plugin docs".to_string(),
                    },
                }],
            },
        )
        .expect("get prompt output");
        assert!(get.output_text.contains("user: Summarize plugin docs"));
    }
}
