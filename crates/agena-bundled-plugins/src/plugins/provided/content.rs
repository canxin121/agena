//! Definition for bounded canonical content reads dispatched by ToolExecutor.

use crate::part::ContentReadToolInput;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};

pub(crate) struct ContentPlugin;

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena", name = "content", version = env!("CARGO_PKG_VERSION"),
    summary = "Read canonical resource-backed text and tool output with resumable byte-bounded pages."
)]
impl ContentPlugin {
    #[tool(
        tags(query, read_only),
        summary = "Read retained source text from a content resource in this session.",
        help = "Use resource_id from a Part or tool result field. The first read omits epoch/after/offset. Continue with next_position.after.epoch as epoch, next_position.after.sequence as after and next_position.offset as offset; offsets safely resume inside a single large UTF-8 record. max_bytes defaults to 4096 (4–4096 raw text bytes); slices preserve whitespace and stdout/stderr. has_more means more retained records, gap signals retention loss, and capture state is independent of process success. Non-text document/terminal records advance the position without fabricated text. This is a diagnostic read, not a wait or polling mechanism."
    )]
    async fn invoke_read(
        &self,
        context: &ToolInvokeContext<'_>,
        input: ContentReadToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let input = serde_json::to_value(input)
            .map_err(|error| agena_plugin_host::PluginError::invalid_params_error(&error))?;
        super::router::invoke_tool("content", input, context.session_id, context.call_id)
    }
}
