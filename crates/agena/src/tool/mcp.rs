//! Bridge between agena's tool subsystem and `agena-mcp-client`.
//!
//! Two responsibilities:
//!
//! * Convert an MCP `tools/call` result (a vector of [`ContentBlock`]) into
//!   the agena-side [`McpToolOutput`] (vector of [`ToolResultBlock`] +
//!   optional structured payload).
//! * Provide a sync wrapper around the async `call_tool` so the existing
//!   sync `ToolExecutor::execute_invocation_detailed` path can call into
//!   the MCP layer without going async itself.

use std::sync::Arc;

use agena_mcp_client::McpConnectionManager;
use agena_mcp_client::protocol::{CallToolResult, ContentBlock};
use serde_json::Value;

use crate::message::{McpToolOutput, StructuredObject, ToolOutput, ToolResultBlock};

use super::{ToolError, ToolExecutionView, ToolInvocationExecution};

/// Block-on the MCP call from a sync context, then translate the result.
pub(super) fn invoke(
    manager: &Arc<McpConnectionManager>,
    server: &str,
    tool: &str,
    input: &StructuredObject,
) -> Result<ToolInvocationExecution, ToolError> {
    let arguments: Option<Value> = if input.fields.is_empty() {
        None
    } else {
        match serde_json::to_value(input) {
            Ok(v) => Some(v),
            Err(e) => {
                return Err(ToolError::Plugin(format!(
                    "mcp:{server}:{tool} argument serialization failed: {e}"
                )));
            }
        }
    };

    let manager_for_call = manager.clone();
    let server_name = server.to_string();
    let tool_name = tool.to_string();
    let result: CallToolResult = block_on(async move {
        manager_for_call
            .call_tool(&server_name, &tool_name, arguments)
            .await
    })
    .map_err(|e| ToolError::Plugin(format!("mcp:{server}:{tool} call failed: {e}")))?;

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
        return Err(ToolError::Plugin(format!(
            "mcp:{server}:{tool} returned isError=true: {combined}"
        )));
    }

    let blocks: Vec<ToolResultBlock> = result
        .content
        .iter()
        .filter_map(content_block_to_result_block)
        .collect();

    let summary_text = blocks
        .iter()
        .filter_map(|b| match b {
            ToolResultBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let view = ToolExecutionView {
        title: format!("MCP {server}/{tool}"),
        output_text: if summary_text.is_empty() {
            format!("(mcp:{server}:{tool} returned {} content block(s))", blocks.len())
        } else {
            summary_text
        },
        metadata: Default::default(),
        attachments: blocks
            .iter()
            .filter_map(|b| b.to_attachment_item())
            .collect(),
    };

    let output = ToolOutput::Mcp {
        output: McpToolOutput {
            server: server.to_string(),
            tool: tool.to_string(),
            content_blocks: blocks,
            structured_content: None,
        },
    };

    let execution = ToolInvocationExecution::new(output, view);
    Ok(execution)
}

fn content_block_to_result_block(block: &ContentBlock) -> Option<ToolResultBlock> {
    match block {
        ContentBlock::Text { text } => Some(ToolResultBlock::Text { text: text.clone() }),
        ContentBlock::Image { data, mime_type } => Some(ToolResultBlock::Image {
            mime: mime_type.clone(),
            // The MCP wire format embeds the bytes as base64; surface them
            // as a `data:` URL so downstream attachment handling can pick
            // them up uniformly.
            url: format!("data:{};base64,{}", mime_type, data),
        }),
        ContentBlock::Resource { resource } => {
            let mime = resource.mime_type.clone().unwrap_or_default();
            Some(ToolResultBlock::EmbeddedResource {
                uri: resource.uri.clone(),
                mime,
                text: resource.text.clone(),
                base64: resource.blob.clone(),
            })
        }
        ContentBlock::Other => None,
    }
}

/// Run an async future to completion from a sync context.  Mirrors
/// `agena_plugin_host::host::PluginHost::block_on` so callers don't need
/// to know whether they're already on a tokio runtime.
pub(super) fn block_on<F: std::future::Future>(fut: F) -> F::Output
where
    F: Send,
    F::Output: Send,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(fut))
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("mcp bridge fallback runtime");
        rt.block_on(fut)
    }
}
