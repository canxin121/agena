//! In-process plugin that exposes configured MCP server capabilities as plugin
//! tools.

use std::collections::BTreeMap;
use std::sync::Arc;

use agena_macros::StaticToolSurface;
use agena_mcp_client::McpConnectionManager;
use agena_mcp_client::protocol::{
    CallToolResult, ContentBlock, GetPromptResult, ListPromptsResult, ListResourcesResult,
    ReadResourceResult, ResourceContents, ToolDescriptor,
};
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

pub(crate) struct McpPlugin {
    manager: Arc<McpConnectionManager>,
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
            McpEntryTarget::Tool {
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
            McpEntryTarget::ListResources { server } => {
                let result = self.manager.list_resources(&server).await.map_err(|err| {
                    PluginError::new(format!("mcp:{server}:resources:list failed: {err}"))
                })?;
                list_resources_output(&server, result)
            }
            McpEntryTarget::ReadResource { server } => {
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
            McpEntryTarget::ListPrompts { server } => {
                let result = self.manager.list_prompts(&server).await.map_err(|err| {
                    PluginError::new(format!("mcp:{server}:prompts:list failed: {err}"))
                })?;
                list_prompts_output(&server, result)
            }
            McpEntryTarget::GetPrompt { server } => {
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
pub(super) enum McpEntryTarget {
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

impl McpEntryTarget {
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
    entry = "mcp",
    description = "MCP command surface.",
    summary = "Read MCP resources/prompts or call discovered MCP tools.",
    help = "Use action `list_resources`, `read_resource`, `list_prompts`, `get_prompt`, or `call`.",
    tags(ToolTag::ReadOnly, ToolTag::Mutating, ToolTag::Mcp),
    concurrency_safe = false,
    load = "deferred"
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum McpToolInput {
    #[serde(alias = "resources_list")]
    #[tool(exec = "list_resources")]
    ListResources {
        #[serde(flatten)]
        args: McpServerInput,
    },
    #[serde(alias = "resources_read")]
    #[tool(exec = "read_resource")]
    ReadResource {
        #[serde(flatten)]
        args: ReadResourceInput,
    },
    #[serde(alias = "prompts_list")]
    #[tool(exec = "list_prompts")]
    ListPrompts {
        #[serde(flatten)]
        args: McpServerInput,
    },
    #[serde(alias = "prompts_get")]
    #[tool(exec = "get_prompt")]
    GetPrompt {
        #[serde(flatten)]
        args: GetPromptInput,
    },
    #[serde(alias = "tool")]
    #[tool(exec = "call")]
    Call {
        #[serde(flatten)]
        args: CallToolInput,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
struct McpServerInput {
    server: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq)]
struct CallToolInput {
    server: String,
    #[serde(alias = "tool")]
    name: String,
    #[serde(default)]
    arguments: Option<Value>,
}

pub(super) fn target_from_invocation(entry: &str, input: Value) -> SdkResult<McpEntryTarget> {
    let (action, args) = McpToolInput::resolve_entry(entry, input)?;
    match action.as_str() {
        "list_resources" => {
            let args: McpServerInput = serde_json::from_value(args)?;
            Ok(McpEntryTarget::ListResources {
                server: args.server,
            })
        }
        "read_resource" => {
            let args: ReadResourceInput = serde_json::from_value(args)?;
            Ok(McpEntryTarget::ReadResource { server: args })
        }
        "list_prompts" => {
            let args: McpServerInput = serde_json::from_value(args)?;
            Ok(McpEntryTarget::ListPrompts {
                server: args.server,
            })
        }
        "get_prompt" => {
            let args: GetPromptInput = serde_json::from_value(args)?;
            Ok(McpEntryTarget::GetPrompt { server: args })
        }
        "call" => {
            let args: CallToolInput = serde_json::from_value(args)?;
            Ok(McpEntryTarget::Tool {
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
    let mut entries = Vec::new();
    if !servers.is_empty() || !tools.is_empty() {
        entries.push(mcp_decl(&servers, &tools, !network_access.is_empty()));
    }
    PluginManifest::builder("agena-mcp", env!("CARGO_PKG_VERSION"))
        .description("Agena MCP bridge exposed as hierarchical plugin commands.")
        .hooks(HookSubscription::TOOL_INVOKE)
        .tools(entries)
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
    let entry = McpToolInput::tool_decl()
    .description(format!(
        "MCP command for {server_count} configured server(s) and {tool_count} discovered tool(s). Set action to list_resources, read_resource, list_prompts, get_prompt, or call."
    ))
    .summary(format!(
        "Read MCP resources/prompts or call discovered MCP tools across {server_count} server(s)."
    ))
    .help(mcp_help(servers, tools))
    .tags([ToolTag::ReadOnly, ToolTag::Mutating, ToolTag::Mcp]);
    maybe_network_tag(entry, has_network_servers)
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

fn maybe_network_tag(entry: PluginToolDecl, has_network_servers: bool) -> PluginToolDecl {
    if has_network_servers {
        entry.tag(ToolTag::Network)
    } else {
        entry
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub(super) struct ReadResourceInput {
    server: String,
    uri: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, PartialEq, Eq)]
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
            _ => block_on_scoped_thread(fut),
        }
    } else {
        block_on_new_runtime(fut)
    }
}

fn block_on_scoped_thread<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(move || block_on_new_runtime(fut))
            .join()
            .expect("mcp plugin fallback runtime thread panicked")
    })
}

fn block_on_new_runtime<F>(fut: F) -> F::Output
where
    F: std::future::Future,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("mcp plugin fallback runtime");
    rt.block_on(fut)
}
