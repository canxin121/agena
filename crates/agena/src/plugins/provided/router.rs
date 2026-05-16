//! Shared scaffolding for static plugins backed by agena's in-process
//! executor implementations. These tools use the same plugin registry and
//! permission path as any other plugin tool; this module only supplies the
//! executor context needed by their Rust implementations.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use crate::entry::result::ToolPayloadExecution;
use crate::entry::{ToolExecutor, ToolRuntimeContext, orchestrator};
use crate::message::{
    ApplyPatchToolInput, BashToolInput, GlobToolInput, GrepToolInput, MonitorToolInput,
    PowerShellToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginManifest, PluginToolDecl,
    Result as SdkResult, ToolInvokeInput, ToolInvokeOutput,
};

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
    key: InProcessContextKey,
    previous: Option<ToolExecutor>,
}

impl Drop for ExecutorContextGuard {
    fn drop(&mut self) {
        IN_PROCESS_TOOL_CTX.with(|cell| {
            *cell.borrow_mut() = self.previous.take();
        });
        if let Ok(mut contexts) = IN_PROCESS_TOOL_CTX_BY_CALL.lock() {
            contexts.remove(&self.key);
        }
    }
}

pub(crate) fn install_executor_context(
    executor: &ToolExecutor,
    session_id: i64,
    call_id: i64,
    tool_name: String,
) -> ExecutorContextGuard {
    let key = InProcessContextKey {
        session_id,
        call_id,
        tool_name,
    };
    if let Ok(mut contexts) = IN_PROCESS_TOOL_CTX_BY_CALL.lock() {
        contexts.insert(key.clone(), executor.clone());
    }
    let previous = IN_PROCESS_TOOL_CTX.with(|cell| cell.replace(Some(executor.clone())));
    ExecutorContextGuard { key, previous }
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
    IN_PROCESS_TOOL_CTX_BY_CALL
        .lock()
        .ok()
        .and_then(|contexts| contexts.get(&key).cloned())
        .ok_or_else(|| PluginError::new("static plugin invoked without executor context"))
}

#[cfg(test)]
pub(crate) fn current_executor_for_test(
    session_id: i64,
    call_id: i64,
    tool_name: &str,
) -> Option<ToolExecutor> {
    current_executor_lookup(session_id, call_id, tool_name)
}

#[cfg(test)]
pub(crate) fn current_executor_lookup(
    session_id: i64,
    call_id: i64,
    tool_name: &str,
) -> Option<ToolExecutor> {
    if let Some(executor) = IN_PROCESS_TOOL_CTX.with(|cell| cell.borrow().clone()) {
        return Some(executor);
    }
    let key = InProcessContextKey {
        session_id,
        call_id,
        tool_name: tool_name.to_string(),
    };
    IN_PROCESS_TOOL_CTX_BY_CALL
        .lock()
        .ok()
        .and_then(|contexts| contexts.get(&key).cloned())
}

pub(crate) struct InProcessToolPlugin {
    plugin_name: &'static str,
    description: &'static str,
    entries: Vec<PluginToolDecl>,
}

impl InProcessToolPlugin {
    pub fn new(
        plugin_name: &'static str,
        description: &'static str,
        entries: Vec<PluginToolDecl>,
    ) -> Self {
        Self {
            plugin_name,
            description,
            entries,
        }
    }
}

#[async_trait]
impl Plugin for InProcessToolPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut builder = PluginManifest::builder(self.plugin_name, env!("CARGO_PKG_VERSION"))
            .description(self.description)
            .hooks(HookSubscription::TOOL_INVOKE);
        for entry in &self.entries {
            builder = builder.tool(entry.clone());
        }
        builder.build()
    }

    async fn init(
        &self,
        _ctx: InitContext,
        _host: std::sync::Arc<dyn crate::plugin::sdk::host_api::HostClient>,
    ) -> SdkResult<InitOutcome> {
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        invoke_tool(
            &input.tool_name,
            input.input,
            input.session_id,
            input.call_id,
        )
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<crate::plugin::sdk::PathRequest>> {
        permission_paths_for(tool, input)
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
        "apply_patch" => {
            let payload: ApplyPatchToolInput = serde_json::from_value(input.clone())?;
            let paths = crate::entry::apply_patch::planned_paths(&payload.patch)
                .map_err(|err| PluginError::new(err.to_string()))?;
            Ok(paths
                .into_iter()
                .map(crate::plugin::sdk::PathRequest::write)
                .collect())
        }
        "bash" => {
            let payload: BashToolInput = serde_json::from_value(input.clone())?;
            Ok(default_workspace_read(payload.workdir.is_none()))
        }
        "powershell" => {
            let payload: PowerShellToolInput = serde_json::from_value(input.clone())?;
            Ok(default_workspace_read(payload.workdir.is_none()))
        }
        "glob" => {
            let payload: GlobToolInput = serde_json::from_value(input.clone())?;
            Ok(default_workspace_read(payload.path.is_none()))
        }
        "grep" => {
            let payload: GrepToolInput = serde_json::from_value(input.clone())?;
            Ok(default_workspace_read(payload.path.is_none()))
        }
        "monitor" => {
            let payload: MonitorToolInput = serde_json::from_value(input.clone())?;
            let needs_workspace = matches!(payload, MonitorToolInput::Start { workdir: None, .. });
            Ok(default_workspace_read(needs_workspace))
        }
        _ => Ok(Vec::new()),
    }
}

fn default_workspace_read(needs_workspace: bool) -> Vec<crate::plugin::sdk::PathRequest> {
    needs_workspace
        .then(|| crate::plugin::sdk::PathRequest::read(""))
        .into_iter()
        .collect()
}

pub(crate) fn tool_execution_to_invoke_output(execution: ToolPayloadExecution) -> ToolInvokeOutput {
    let mut metadata = execution.view.metadata;
    match &execution.output {
        crate::message::ToolPayloadOutput::ApplyPatch { .. } => {
            metadata.insert("agena.effect".to_string(), "file_changes".to_string());
        }
        crate::message::ToolPayloadOutput::ToolSearch { .. } => {
            metadata.insert("agena.effect".to_string(), "load_tools".to_string());
        }
        crate::message::ToolPayloadOutput::TodoWrite { .. } => {
            metadata.insert("agena.effect".to_string(), "todo_list".to_string());
        }
        crate::message::ToolPayloadOutput::EnterWorktree { .. } => {
            metadata.insert("agena.effect".to_string(), "enter_worktree".to_string());
        }
        crate::message::ToolPayloadOutput::ExitWorktree { .. } => {
            metadata.insert("agena.effect".to_string(), "exit_worktree".to_string());
        }
        _ => {}
    }
    let payload = Some(serde_json::Value::from(
        execution.output.into_custom_output().payload,
    ));
    ToolInvokeOutput {
        title: execution.view.title,
        output_text: execution.view.output_text,
        payload,
        metadata: metadata.into_iter().collect(),
        attachments: execution.view.attachments,
    }
}

#[cfg(test)]
pub(crate) fn payload_to_tool_output(
    tool_name: &str,
    payload: Option<&JsonValue>,
) -> Result<crate::message::ToolPayloadOutput, serde_json::Error> {
    let value = payload.cloned().unwrap_or(serde_json::json!({}));
    let payload = crate::message::StructuredObject::try_from(value)
        .map_err(|err| serde::de::Error::custom(err.to_string()))?;
    let output = crate::message::CustomToolOutput {
        name: tool_name.to_string(),
        payload,
    };
    crate::message::ToolPayloadOutput::from_custom(&output).ok_or_else(|| {
        serde::de::Error::custom(format!(
            "payload for tool `{tool_name}` did not match tool payload schema"
        ))
    })
}
