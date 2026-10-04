//! Schema helpers shared by bundled tool definitions.
//!
//! Bundled executor-backed tools are dispatched explicitly by `ToolExecutor`.
//! Their plugin handlers remain definition-only adapters and must never depend
//! on thread-local or process-global execution context.

use serde_json::Value as JsonValue;

use crate::tool::ToolPayloadOutput;
use crate::tool::result::ToolPayloadExecution;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeOutput};

pub fn invoke_tool(
    tool_name: &str,
    input: JsonValue,
    session_id: i64,
    call_id: i64,
) -> SdkResult<ToolInvokeOutput> {
    let _ = (input, session_id, call_id);
    Err(PluginError::internal(format!(
        "bundled execution handler `{tool_name}` must be dispatched by ToolExecutor"
    )))
}

pub fn tool_execution_to_invoke_output(execution: ToolPayloadExecution) -> ToolInvokeOutput {
    let summary = execution.summary();
    let mut metadata = summary.metadata;
    match &execution.output {
        ToolPayloadOutput::ApplyPatch { .. } => {
            metadata.insert("agena.effect".to_string(), "file_changes".to_string());
        }
        ToolPayloadOutput::ToolSearch { .. } => {
            metadata.insert("agena.effect".to_string(), "load_tools".to_string());
        }
        _ => {}
    }
    let payload = summary.payload.clone();
    ToolInvokeOutput {
        title: summary.title,
        summary: summary.summary,
        output_text: summary.output_text,
        payload,
        metadata: metadata.into_iter().collect(),
        attachments: execution.view.attachments,
    }
}
