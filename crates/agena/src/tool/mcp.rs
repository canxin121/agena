//! First-party in-process plugin that exposes configured MCP server tools as
//! plugin entries.

use std::sync::Arc;

use agena_mcp_client::McpConnectionManager;
use agena_mcp_client::protocol::{CallToolResult, ContentBlock, ToolDescriptor};
use async_trait::async_trait;
use serde_json::Value;

use crate::message::{AttachmentItem, ToolResultBlock};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    EntryBehavior as SdkEntryBehavior, HookSubscription, InitContext, InitOutcome, Plugin,
    PluginEntryDecl, PluginManifest, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput,
};

pub(super) const MCP_PLUGIN_ID: &str = "agena.mcp";

pub(super) struct McpPlugin {
    manager: Arc<McpConnectionManager>,
}

impl McpPlugin {
    pub(super) fn new(manager: Arc<McpConnectionManager>) -> Self {
        Self { manager }
    }

    fn manifest_from_tools(&self, tools: Vec<(String, ToolDescriptor)>) -> PluginManifest {
        PluginManifest::builder("agena-mcp", env!("CARGO_PKG_VERSION"))
            .description("Agena MCP bridge exposed as first-party plugin entries.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .entries(
                tools
                    .into_iter()
                    .map(|(server, tool)| entry_decl(server, tool)),
            )
            .build()
    }
}

#[async_trait]
impl Plugin for McpPlugin {
    fn manifest(&self) -> PluginManifest {
        let manager = Arc::clone(&self.manager);
        self.manifest_from_tools(block_on(async move { manager.all_tools().await }))
    }

    async fn init(
        &self,
        _ctx: InitContext,
        _host: Arc<dyn crate::plugin::sdk::HostClient>,
    ) -> SdkResult<InitOutcome> {
        Ok(InitOutcome::ack(
            self.manifest_from_tools(self.manager.all_tools().await),
        ))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let (server, tool) = target_from_entry_name(input.tool_name.as_str()).ok_or_else(|| {
            PluginError::invalid_params(format!("invalid MCP plugin entry '{}'", input.tool_name))
        })?;
        let arguments = input_arguments(input.input);
        let result = self
            .manager
            .call_tool(server, tool, arguments)
            .await
            .map_err(|err| PluginError::new(format!("mcp:{server}:{tool} call failed: {err}")))?;
        invoke_output(server, tool, result)
    }
}

pub(super) fn target_from_entry_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp:")?;
    let (server, tool) = rest.split_once(':')?;
    (!server.is_empty() && !tool.is_empty()).then_some((server, tool))
}

fn entry_decl(server: String, tool: ToolDescriptor) -> PluginEntryDecl {
    let name = format!("mcp:{server}:{}", tool.name);
    let schema = tool
        .input_schema
        .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));
    PluginEntryDecl::new(name, schema)
        .description(tool.description.unwrap_or_default())
        .behavior(SdkEntryBehavior::WriteUnsandboxed)
        .search_terms(["mcp".to_string(), server, tool.name])
}

fn input_arguments(input: Value) -> Option<Value> {
    match input {
        Value::Null => None,
        Value::Object(map) if map.is_empty() => None,
        other => Some(other),
    }
}

fn invoke_output(server: &str, tool: &str, result: CallToolResult) -> SdkResult<ToolInvokeOutput> {
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
            "mcp:{server}:{tool} returned isError=true: {combined}"
        )));
    }

    let blocks: Vec<ToolResultBlock> = result
        .content
        .iter()
        .filter_map(content_block_to_result_block)
        .collect();
    let output_text = blocks
        .iter()
        .filter_map(|block| match block {
            ToolResultBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let attachments = blocks
        .iter()
        .filter_map(ToolResultBlock::to_attachment_item)
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
                "(mcp:{server}:{tool} returned {} content block(s))",
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

fn content_block_to_result_block(block: &ContentBlock) -> Option<ToolResultBlock> {
    match block {
        ContentBlock::Text { text } => Some(ToolResultBlock::Text { text: text.clone() }),
        ContentBlock::Image { data, mime_type } => Some(ToolResultBlock::Image {
            mime: mime_type.clone(),
            url: format!("data:{};base64,{}", mime_type, data),
        }),
        ContentBlock::Resource { resource } => Some(ToolResultBlock::EmbeddedResource {
            uri: resource.uri.clone(),
            mime: resource.mime_type.clone().unwrap_or_default(),
            text: resource.text.clone(),
            base64: resource.blob.clone(),
        }),
        ContentBlock::Other => None,
    }
}

pub(super) fn block_on<F>(fut: F) -> F::Output
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
