//! Shared scaffolding for static plugins backed by agena's in-process
//! executor implementations. These tools use the same plugin registry and
//! permission path as any other plugin tool; this module only supplies the
//! executor context needed by their Rust implementations.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::Value as JsonValue;

use crate::message::{
    ApplyPatchToolInput, GlobToolInput, GrepToolInput, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput, NetworkEffect,
    ProcessToolInput, ReadToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::{NetworkRequest, Result as SdkResult, ToolInput, ToolInvokeOutput};
use crate::tool::result::ToolPayloadExecution;
use crate::tool::{ToolExecutor, ToolPayloadOutput, ToolRuntimeContext, orchestrator};

thread_local! {
    static IN_PROCESS_TOOL_CTX: RefCell<Option<ToolExecutor>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InProcessContextKey {
    session_id: i64,
    call_id: i64,
    tool_name: String,
}

static IN_PROCESS_TOOL_CTX_BY_CALL: LazyLock<Mutex<HashMap<InProcessContextKey, ToolExecutor>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) struct ExecutorContextGuard {
    keys: Vec<InProcessContextKey>,
    previous: Option<ToolExecutor>,
}

impl Drop for ExecutorContextGuard {
    fn drop(&mut self) {
        IN_PROCESS_TOOL_CTX.with(|cell| {
            *cell.borrow_mut() = self.previous.take();
        });
        if let Ok(mut contexts) = IN_PROCESS_TOOL_CTX_BY_CALL.lock() {
            for key in &self.keys {
                contexts.remove(key);
            }
        }
    }
}

pub(crate) fn install_executor_context(
    executor: &ToolExecutor,
    session_id: i64,
    call_id: i64,
    tool_name: String,
) -> ExecutorContextGuard {
    let mut keys = vec![InProcessContextKey {
        session_id,
        call_id,
        tool_name,
    }];
    for alias in routed_internal_tool_names(keys[0].tool_name.as_str()) {
        keys.push(InProcessContextKey {
            session_id,
            call_id,
            tool_name: alias.to_string(),
        });
    }
    if let Ok(mut contexts) = IN_PROCESS_TOOL_CTX_BY_CALL.lock() {
        for key in &keys {
            contexts.insert(key.clone(), executor.clone());
        }
    }
    let previous = IN_PROCESS_TOOL_CTX.with(|cell| cell.replace(Some(executor.clone())));
    ExecutorContextGuard { keys, previous }
}

fn current_executor(
    session_id: i64,
    call_id: i64,
    tool_name: &str,
) -> Result<ToolExecutor, PluginError> {
    if let Some(executor) = IN_PROCESS_TOOL_CTX.with(|cell| cell.borrow().clone()) {
        return Ok(executor);
    }
    let key = InProcessContextKey {
        session_id,
        call_id,
        tool_name: tool_name.to_string(),
    };
    let tool_key = routed_tool_name(tool_name).map(|tool_name| InProcessContextKey {
        session_id,
        call_id,
        tool_name: tool_name.to_string(),
    });
    IN_PROCESS_TOOL_CTX_BY_CALL
        .lock()
        .ok()
        .and_then(|contexts| {
            contexts
                .get(&key)
                .cloned()
                .or_else(|| tool_key.as_ref().and_then(|key| contexts.get(key).cloned()))
        })
        .ok_or_else(|| PluginError::new("static plugin invoked without executor context"))
}

fn routed_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "task" => Some("run"),
        "tool_search" => Some("search"),
        "ask_user" => Some("request_input"),
        "enter_snapshot" => Some("enter"),
        "exit_snapshot" => Some("exit"),
        "cron_list" => Some("list"),
        "cron_create" => Some("create"),
        "cron_delete" => Some("delete"),
        "schedule_wakeup" => Some("wakeup"),
        "lsp_definition" => Some("definition"),
        "lsp_references" => Some("references"),
        "lsp_hover" => Some("hover"),
        "lsp_diagnostics" => Some("diagnostics"),
        _ => None,
    }
}

fn routed_internal_tool_names(tool_name: &str) -> &'static [&'static str] {
    match tool_name {
        "run" => &["process", "task"],
        "list" => &["process", "cron_list"],
        "logs" | "stop" => &["process"],
        "search" => &["tool_search"],
        "request_input" => &["ask_user"],
        "enter" => &["enter_snapshot"],
        "exit" => &["exit_snapshot"],
        "create" => &["cron_create"],
        "delete" => &["cron_delete"],
        "wakeup" => &["schedule_wakeup"],
        "definition" => &["lsp_definition"],
        "references" => &["lsp_references"],
        "hover" => &["lsp_hover"],
        "diagnostics" => &["lsp_diagnostics"],
        _ => &[],
    }
}

pub(crate) fn invoke_tool(
    tool_name: &str,
    input: JsonValue,
    session_id: i64,
    call_id: i64,
) -> SdkResult<ToolInvokeOutput> {
    let executor = current_executor(session_id, call_id, tool_name)?;
    let context = ToolRuntimeContext {
        session_id: (session_id >= 0).then_some(session_id),
        call_id: (call_id >= 0).then_some(call_id),
        session_context: None,
        prepared_shell_command: None,
    };
    let execution = orchestrator::execute_tool(&executor, tool_name, input, context)
        .map_err(|err| PluginError::new(format!("{tool_name}: {err}")))?;
    Ok(tool_execution_to_invoke_output(execution))
}

pub(crate) fn permission_paths_for(
    tool: &str,
    input: &serde_json::Value,
) -> SdkResult<Vec<crate::plugin::sdk::PathRequest>> {
    match tool {
        "read" => {
            let payload: ReadToolInput = parse_shape_input(input)?;
            Ok(vec![crate::plugin::sdk::PathRequest::read(
                payload.file_path,
            )])
        }
        "apply_patch" => {
            let payload = ApplyPatchToolInput::parse_input(input.clone())?;
            let paths = crate::tool::apply_patch::planned_paths(&payload.patch)
                .map_err(|err| PluginError::new(err.to_string()))?;
            Ok(paths
                .into_iter()
                .map(crate::plugin::sdk::PathRequest::write)
                .collect())
        }
        "glob" => {
            let payload: GlobToolInput = parse_shape_input(input)?;
            Ok(base_path_read_request(payload.path.as_deref()))
        }
        "grep" => {
            let payload: GrepToolInput = parse_shape_input(input)?;
            Ok(base_path_read_request(payload.path.as_deref()))
        }
        "process" => {
            let payload: ProcessToolInput = parse_shape_input(input)?;
            Ok(match payload {
                ProcessToolInput::Run { command, .. } => {
                    workdir_read_request(command.workdir.as_deref())
                }
                ProcessToolInput::List {}
                | ProcessToolInput::Logs { .. }
                | ProcessToolInput::Stop { .. } => Vec::new(),
            })
        }
        "lsp_definition" => {
            let payload: LspDefinitionToolInput = parse_shape_input(input)?;
            Ok(vec![crate::plugin::sdk::PathRequest::read(
                payload.position.file_path,
            )])
        }
        "lsp_references" => {
            let payload: LspReferencesToolInput = parse_shape_input(input)?;
            Ok(vec![crate::plugin::sdk::PathRequest::read(
                payload.position.file_path,
            )])
        }
        "lsp_hover" => {
            let payload: LspHoverToolInput = parse_shape_input(input)?;
            Ok(vec![crate::plugin::sdk::PathRequest::read(
                payload.position.file_path,
            )])
        }
        "lsp_diagnostics" => {
            let payload: LspDiagnosticsToolInput = parse_shape_input(input)?;
            Ok(vec![crate::plugin::sdk::PathRequest::read(
                payload.file_path,
            )])
        }
        _ => Ok(Vec::new()),
    }
}

pub(crate) fn permission_networks_for(
    tool: &str,
    input: &serde_json::Value,
) -> SdkResult<Vec<NetworkRequest>> {
    match tool {
        "process" => {
            let payload: ProcessToolInput = parse_shape_input(input)?;
            match payload {
                ProcessToolInput::Run { command, .. } => declared_shell_network_requests(
                    "process",
                    command.command.as_str(),
                    &command.network_effects,
                ),
                ProcessToolInput::List {}
                | ProcessToolInput::Logs { .. }
                | ProcessToolInput::Stop { .. } => Ok(Vec::new()),
            }
        }
        _ => Ok(Vec::new()),
    }
}

fn parse_shape_input<T: ToolInput>(input: &serde_json::Value) -> SdkResult<T> {
    T::parse_input(input.clone())
}

fn workdir_read_request(workdir: Option<&str>) -> Vec<crate::plugin::sdk::PathRequest> {
    vec![crate::plugin::sdk::PathRequest::read(
        normalized_declared_path(workdir),
    )]
}

fn base_path_read_request(path: Option<&str>) -> Vec<crate::plugin::sdk::PathRequest> {
    vec![crate::plugin::sdk::PathRequest::read(
        normalized_declared_path(path),
    )]
}

fn normalized_declared_path(path: Option<&str>) -> String {
    path.map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or("")
        .to_string()
}

fn declared_shell_network_requests(
    tool: &str,
    command: &str,
    effects: &[NetworkEffect],
) -> SdkResult<Vec<NetworkRequest>> {
    if effects.is_empty()
        && let Some(reason) = crate::tool::shell_tools::network_command_reason(command)
    {
        return Err(PluginError::invalid_params(format!(
            "{tool} network_effects must declare at least one target because the command appears to use the network: {reason}"
        )));
    }
    Ok(effects
        .iter()
        .map(|effect| NetworkRequest::connect(effect.target.clone()))
        .collect())
}

pub(crate) fn tool_execution_to_invoke_output(execution: ToolPayloadExecution) -> ToolInvokeOutput {
    let mut metadata = execution.view.metadata;
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
    let payload = execution.output.into_tool_output().to_json_payload();
    ToolInvokeOutput {
        title: execution.view.title,
        output_text: execution.view.output_text,
        payload,
        metadata: metadata.into_iter().collect(),
        attachments: execution.view.attachments,
    }
}
