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
use crate::entry::{ToolExecutor, ToolPayloadOutput, ToolRuntimeContext, orchestrator};
use crate::message::{
    ApplyPatchToolInput, BashToolInput, GlobToolInput, GrepToolInput, MonitorToolInput,
    NotebookEditToolInput, PowerShellToolInput, ReadToolInput, WebFetchToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, NetworkRequest, Plugin, PluginManifest,
    PluginToolDecl, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput,
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
    let entry_key = routed_entry_name(tool_name).map(|entry_name| InProcessContextKey {
        session_id,
        call_id,
        tool_name: entry_name.to_string(),
    });
    IN_PROCESS_TOOL_CTX_BY_CALL
        .lock()
        .ok()
        .and_then(|contexts| {
            contexts
                .get(&key)
                .cloned()
                .or_else(|| entry_key.as_ref().and_then(|key| contexts.get(key).cloned()))
        })
        .ok_or_else(|| PluginError::new("static plugin invoked without executor context"))
}

fn routed_entry_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "read" | "glob" | "grep" | "apply_patch" | "notebook_edit" => Some("fs"),
        "bash" | "powershell" | "monitor" => Some("shell"),
        "web_fetch" | "web_search" => Some("web"),
        "cron_list" | "cron_create" | "cron_delete" | "schedule_wakeup" => Some("schedule"),
        "lsp_servers" | "lsp_definition" | "lsp_references" | "lsp_hover" | "lsp_diagnostics" => {
            Some("lsp")
        }
        _ => None,
    }
}

pub(crate) struct InProcessToolPlugin {
    plugin_name: &'static str,
    description: &'static str,
    entries: Vec<PluginToolDecl>,
    resolver: Option<ToolInputResolver>,
}

impl InProcessToolPlugin {
    #[allow(dead_code)]
    pub fn new(
        plugin_name: &'static str,
        description: &'static str,
        entries: Vec<PluginToolDecl>,
    ) -> Self {
        Self {
            plugin_name,
            description,
            entries,
            resolver: None,
        }
    }

    pub fn new_with_resolver(
        plugin_name: &'static str,
        description: &'static str,
        entries: Vec<PluginToolDecl>,
        resolver: ToolInputResolver,
    ) -> Self {
        Self {
            plugin_name,
            description,
            entries,
            resolver: Some(resolver),
        }
    }

    fn resolve_tool_input(
        &self,
        tool_name: &str,
        input: JsonValue,
    ) -> SdkResult<(String, JsonValue)> {
        match self.resolver {
            Some(resolve) => resolve(tool_name, input),
            None => Ok((tool_name.to_string(), input)),
        }
    }
}

pub(crate) type ToolInputResolver = fn(&str, JsonValue) -> SdkResult<(String, JsonValue)>;

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
        let (tool_name, tool_input) = self.resolve_tool_input(&input.tool_name, input.input)?;
        invoke_tool(&tool_name, tool_input, input.session_id, input.call_id)
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<crate::plugin::sdk::PathRequest>> {
        let (tool_name, tool_input) = self.resolve_tool_input(tool, input.clone())?;
        permission_paths_for(&tool_name, &tool_input)
    }

    async fn permission_networks(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<NetworkRequest>> {
        let (tool_name, tool_input) = self.resolve_tool_input(tool, input.clone())?;
        permission_networks_for(&tool_name, &tool_input)
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
            let payload: ReadToolInput = serde_json::from_value(input.clone())?;
            Ok(vec![crate::plugin::sdk::PathRequest::read(
                payload.file_path,
            )])
        }
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
        "notebook_edit" => {
            let payload: NotebookEditToolInput = serde_json::from_value(input.clone())?;
            Ok(vec![crate::plugin::sdk::PathRequest::write(
                payload.notebook_path,
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
        "web_fetch" => {
            let payload: WebFetchToolInput = serde_json::from_value(input.clone())?;
            Ok(vec![NetworkRequest::connect(payload.url)])
        }
        "web_search" => Ok(vec![NetworkRequest::connect(
            "https://html.duckduckgo.com/html/",
        )]),
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
        ToolPayloadOutput::ApplyPatch { .. } => {
            metadata.insert("agena.effect".to_string(), "file_changes".to_string());
        }
        ToolPayloadOutput::ToolSearch { .. } => {
            metadata.insert("agena.effect".to_string(), "load_tools".to_string());
        }
        ToolPayloadOutput::TodoWrite { .. } => {
            metadata.insert("agena.effect".to_string(), "todo_list".to_string());
        }
        ToolPayloadOutput::EnterWorktree { .. } => {
            metadata.insert("agena.effect".to_string(), "enter_worktree".to_string());
        }
        ToolPayloadOutput::ExitWorktree { .. } => {
            metadata.insert("agena.effect".to_string(), "exit_worktree".to_string());
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
