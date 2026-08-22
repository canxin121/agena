//! Schema and permission-hook helpers shared by bundled tool definitions.
//!
//! Bundled executor-backed tools are dispatched explicitly by `ToolExecutor`.
//! Their plugin handlers remain definition-only adapters and must never depend
//! on thread-local or process-global execution context.

use serde_json::Value as JsonValue;

use crate::part::{ApplyPatchToolInput, ShellToolInput};
use crate::tool::ToolPayloadOutput;
use crate::tool::result::ToolPayloadExecution;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInput, ToolInvokeOutput};

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

pub fn permission_paths_for(
    tool: &str,
    input: &serde_json::Value,
) -> SdkResult<Vec<agena_plugin_host::sdk::PathRequest>> {
    match tool {
        "apply_patch" => {
            let payload = ApplyPatchToolInput::parse_input(input.clone())?;
            let paths = crate::tool::apply_patch::planned_paths(&payload.patch)
                .map_err(|error| PluginError::internal_error(&error))?;
            Ok(paths
                .into_iter()
                .map(agena_plugin_host::sdk::PathRequest::write)
                .collect())
        }
        _ => Ok(Vec::new()),
    }
}

pub fn permission_network_targets_for(
    tool: &str,
    input: &serde_json::Value,
) -> SdkResult<Vec<String>> {
    match tool {
        "shell" => {
            let payload: ShellToolInput = parse_shape_input(input)?;
            match payload {
                ShellToolInput::Run { command, .. } => declared_shell_network_targets(
                    "shell",
                    command.command.as_str(),
                    &command.network,
                ),
                ShellToolInput::List {}
                | ShellToolInput::Logs { .. }
                | ShellToolInput::Stop { .. } => Ok(Vec::new()),
            }
        }
        _ => Ok(Vec::new()),
    }
}

fn parse_shape_input<T: ToolInput>(input: &serde_json::Value) -> SdkResult<T> {
    T::parse_input(input.clone())
}

fn declared_shell_network_targets(
    tool: &str,
    command: &str,
    effects: &[String],
) -> SdkResult<Vec<String>> {
    if effects.is_empty()
        && let Some(reason) = agena_tool::shell_analysis::network_command_reason(command)
    {
        return Err(PluginError::invalid_params(format!(
            "{tool} network must declare at least one target because the command appears to use the network: {reason}"
        )));
    }
    Ok(effects.to_vec())
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
        ToolPayloadOutput::EnterSnapshot { .. } => {
            metadata.insert("agena.effect".to_string(), "enter_snapshot".to_string());
        }
        ToolPayloadOutput::ExitSnapshot { .. } => {
            metadata.insert("agena.effect".to_string(), "exit_snapshot".to_string());
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
