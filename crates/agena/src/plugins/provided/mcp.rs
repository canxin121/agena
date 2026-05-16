//! In-process plugin that exposes configured MCP server capabilities as plugin
//! tools.

use std::collections::BTreeMap;
use std::sync::Arc;

use agena_mcp_client::McpConnectionManager;
use agena_mcp_client::protocol::{
    CallToolResult, ContentBlock, GetPromptResult, ListPromptsResult, ListResourcesResult,
    ReadResourceResult, ResourceContents, ToolDescriptor,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::message::{AttachmentItem, OperationBlock};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, NetworkAccessSpec, Plugin, PluginManifest,
    PluginToolDecl, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput, ToolTag,
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
        let target = target_from_entry_name(input.tool_name.as_str()).ok_or_else(|| {
            PluginError::invalid_params(format!("invalid MCP plugin tool '{}'", input.tool_name))
        })?;
        match target {
            McpEntryTarget::Tool { server, tool } => {
                let arguments = input_arguments(input.input);
                let result = self
                    .manager
                    .call_tool(server, tool, arguments)
                    .await
                    .map_err(|err| {
                        PluginError::new(format!("mcp:{server}:tool:{tool} call failed: {err}"))
                    })?;
                invoke_tool_output(server, tool, result)
            }
            McpEntryTarget::ListResources { server } => {
                let result = self.manager.list_resources(server).await.map_err(|err| {
                    PluginError::new(format!("mcp:{server}:resources:list failed: {err}"))
                })?;
                list_resources_output(server, result)
            }
            McpEntryTarget::ReadResource { server } => {
                let input: ReadResourceInput = serde_json::from_value(input.input)?;
                let result = self
                    .manager
                    .read_resource(server, input.uri.as_str())
                    .await
                    .map_err(|err| {
                        PluginError::new(format!(
                            "mcp:{server}:resources:read '{}' failed: {err}",
                            input.uri
                        ))
                    })?;
                read_resource_output(server, input.uri.as_str(), result)
            }
            McpEntryTarget::ListPrompts { server } => {
                let result = self.manager.list_prompts(server).await.map_err(|err| {
                    PluginError::new(format!("mcp:{server}:prompts:list failed: {err}"))
                })?;
                list_prompts_output(server, result)
            }
            McpEntryTarget::GetPrompt { server } => {
                let input: GetPromptInput = serde_json::from_value(input.input)?;
                let result = self
                    .manager
                    .get_prompt(server, input.name.as_str(), input.arguments)
                    .await
                    .map_err(|err| {
                        PluginError::new(format!(
                            "mcp:{server}:prompts:get '{}' failed: {err}",
                            input.name
                        ))
                    })?;
                get_prompt_output(server, input.name.as_str(), result)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum McpEntryTarget<'a> {
    Tool { server: &'a str, tool: &'a str },
    ListResources { server: &'a str },
    ReadResource { server: &'a str },
    ListPrompts { server: &'a str },
    GetPrompt { server: &'a str },
}

pub(super) fn target_from_entry_name(name: &str) -> Option<McpEntryTarget<'_>> {
    let rest = name.strip_prefix("mcp:")?;
    let mut parts = rest.splitn(3, ':');
    let server = parts.next()?;
    let kind = parts.next()?;
    let remainder = parts.next()?;
    if server.is_empty() || remainder.is_empty() {
        return None;
    }
    match kind {
        "tool" => Some(McpEntryTarget::Tool {
            server,
            tool: remainder,
        }),
        "resources" => match remainder {
            "list" => Some(McpEntryTarget::ListResources { server }),
            "read" => Some(McpEntryTarget::ReadResource { server }),
            _ => None,
        },
        "prompts" => match remainder {
            "list" => Some(McpEntryTarget::ListPrompts { server }),
            "get" => Some(McpEntryTarget::GetPrompt { server }),
            _ => None,
        },
        _ => None,
    }
}

fn manifest_from_snapshot(
    servers: Vec<String>,
    tools: Vec<(String, ToolDescriptor)>,
    network_access: &BTreeMap<String, NetworkAccessSpec>,
) -> PluginManifest {
    let mut entries = tools
        .into_iter()
        .map(|(server, tool)| tool_entry_decl(server, tool, network_access))
        .collect::<Vec<_>>();
    for server in servers {
        entries.extend(resource_and_prompt_entry_decls(server, network_access));
    }
    PluginManifest::builder("agena-mcp", env!("CARGO_PKG_VERSION"))
        .description("Agena MCP bridge exposed as plugin tools.")
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

fn tool_entry_decl(
    server: String,
    tool: ToolDescriptor,
    network_access: &BTreeMap<String, NetworkAccessSpec>,
) -> PluginToolDecl {
    let name = format!("mcp:{server}:tool:{}", tool.name);
    let schema = tool
        .input_schema
        .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));
    let entry = PluginToolDecl::new(name, schema)
        .description(tool.description.unwrap_or_default())
        .tags([ToolTag::Mutating, ToolTag::Mcp])
        .concurrency_safe(false);
    apply_server_network_access(entry, &server, network_access)
}

fn resource_and_prompt_entry_decls(
    server: String,
    network_access: &BTreeMap<String, NetworkAccessSpec>,
) -> Vec<PluginToolDecl> {
    let entries = vec![
        PluginToolDecl::new(
            format!("mcp:{server}:resources:list"),
            empty_object_schema(),
        )
        .description(format!("List MCP resources exposed by server '{server}'."))
        .tags([ToolTag::ReadOnly, ToolTag::Mcp])
        .concurrency_safe(true)
        .deferred_load(),
        PluginToolDecl::new(
            format!("mcp:{server}:resources:read"),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "uri": { "type": "string" }
                },
                "required": ["uri"]
            }),
        )
        .description(format!(
            "Read one MCP resource from server '{server}' by URI."
        ))
        .tags([ToolTag::ReadOnly, ToolTag::Mcp])
        .concurrency_safe(true)
        .deferred_load(),
        PluginToolDecl::new(format!("mcp:{server}:prompts:list"), empty_object_schema())
            .description(format!("List MCP prompts exposed by server '{server}'."))
            .tags([ToolTag::ReadOnly, ToolTag::Mcp])
            .concurrency_safe(true)
            .deferred_load(),
        PluginToolDecl::new(
            format!("mcp:{server}:prompts:get"),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "arguments": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["name"]
            }),
        )
        .description(format!(
            "Get one MCP prompt from server '{server}' by name."
        ))
        .tags([ToolTag::ReadOnly, ToolTag::Mcp])
        .concurrency_safe(true)
        .deferred_load(),
    ];
    entries
        .into_iter()
        .map(|entry| apply_server_network_access(entry, &server, network_access))
        .collect()
}

fn apply_server_network_access(
    entry: PluginToolDecl,
    server: &str,
    network_access: &BTreeMap<String, NetworkAccessSpec>,
) -> PluginToolDecl {
    match network_access.get(server) {
        Some(spec) => entry.network_access(spec.clone()).tag(ToolTag::Network),
        None => entry,
    }
}

fn empty_object_schema() -> Value {
    serde_json::json!({"type": "object", "properties": {}})
}

#[derive(Debug, Deserialize)]
struct ReadResourceInput {
    uri: String,
}

#[derive(Debug, Deserialize)]
struct GetPromptInput {
    name: String,
    #[serde(default)]
    arguments: Option<BTreeMap<String, String>>,
}

fn input_arguments(input: Value) -> Option<Value> {
    match input {
        Value::Null => None,
        Value::Object(map) if map.is_empty() => None,
        other => Some(other),
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use agena_mcp_client::protocol::{
        PromptArgument, PromptDescriptor, PromptMessage, ResourceDescriptor,
    };

    use super::*;

    #[test]
    fn mcp_entry_names_are_namespaced() {
        assert_eq!(
            target_from_entry_name("mcp:docs:tool:search"),
            Some(McpEntryTarget::Tool {
                server: "docs",
                tool: "search"
            })
        );
        assert_eq!(
            target_from_entry_name("mcp:docs:tool:search:advanced"),
            Some(McpEntryTarget::Tool {
                server: "docs",
                tool: "search:advanced"
            })
        );
        assert_eq!(
            target_from_entry_name("mcp:docs:resources:list"),
            Some(McpEntryTarget::ListResources { server: "docs" })
        );
        assert_eq!(
            target_from_entry_name("mcp:docs:resources:read"),
            Some(McpEntryTarget::ReadResource { server: "docs" })
        );
        assert_eq!(
            target_from_entry_name("mcp:docs:prompts:list"),
            Some(McpEntryTarget::ListPrompts { server: "docs" })
        );
        assert_eq!(
            target_from_entry_name("mcp:docs:prompts:get"),
            Some(McpEntryTarget::GetPrompt { server: "docs" })
        );
        assert_eq!(target_from_entry_name("mcp:docs:search"), None);
    }

    #[test]
    fn mcp_manifest_includes_resource_and_prompt_entries_per_server() {
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
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<BTreeSet<_>>();

        assert!(names.contains("mcp:docs:tool:search"));
        assert!(names.contains("mcp:docs:resources:list"));
        assert!(names.contains("mcp:docs:resources:read"));
        assert!(names.contains("mcp:docs:prompts:list"));
        assert!(names.contains("mcp:docs:prompts:get"));
        assert!(!names.contains("mcp:docs:search"));
    }

    #[test]
    fn mcp_manifest_marks_http_server_entries_with_network_access() {
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

        let remote_entries = manifest
            .entries
            .iter()
            .filter(|entry| entry.name.starts_with("mcp:remote:"))
            .collect::<Vec<_>>();
        assert_eq!(remote_entries.len(), 5);
        for entry in remote_entries {
            assert_eq!(entry.network_access, vec![network_access["remote"].clone()]);
            assert!(entry.tags.iter().any(|tag| tag == &ToolTag::Network));
        }

        let local_entries = manifest
            .entries
            .iter()
            .filter(|entry| entry.name.starts_with("mcp:local:"))
            .collect::<Vec<_>>();
        assert_eq!(local_entries.len(), 5);
        for entry in local_entries {
            assert!(entry.network_access.is_empty());
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
