//! Shared scaffolding for static plugins backed by agena's in-process
//! executor implementations. These tools use the same plugin registry and
//! permission path as any other plugin tool; this module only supplies the
//! executor context needed by their Rust implementations.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use crate::message::{
    ApplyPatchToolInput, GlobToolInput, GrepToolInput, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput, MonitorToolInput,
    NetworkEffect, NotebookEditToolInput, ReadToolInput, ShellCommandInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, NetworkRequest, Plugin, PluginManifest,
    PluginToolDecl, Result as SdkResult, ToolDescriptionMode, ToolInvokeInput, ToolInvokeOutput,
    UiTextDisplayMode,
};
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
    if let Some(alias) = routed_internal_tool_name(keys[0].tool_name.as_str()) {
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
        "read" | "glob" | "grep" | "apply_patch" | "notebook_edit" => Some("fs"),
        "bash" => Some("exec.bash"),
        "powershell" => Some("exec.powershell"),
        "cron_list" => Some("schedule.list"),
        "cron_create" => Some("schedule.create"),
        "cron_delete" => Some("schedule.delete"),
        "schedule_wakeup" => Some("schedule.wakeup"),
        "lsp_servers" | "lsp_definition" | "lsp_references" | "lsp_hover" | "lsp_diagnostics" => {
            Some("lsp")
        }
        _ => None,
    }
}

fn routed_internal_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "exec.bash" => Some("bash"),
        "exec.powershell" => Some("powershell"),
        "monitor.start" | "monitor.list" | "monitor.read" | "monitor.stop" => Some("monitor"),
        "schedule.list" => Some("cron_list"),
        "schedule.create" => Some("cron_create"),
        "schedule.delete" => Some("cron_delete"),
        "schedule.wakeup" => Some("schedule_wakeup"),
        _ => None,
    }
}

pub(crate) struct InProcessToolPlugin {
    plugin_name: &'static str,
    description: &'static str,
    tools: Vec<PluginToolDecl>,
    resolver: Option<ToolInputResolver>,
    tool_description_mode: Option<ToolDescriptionMode>,
    ui_display_mode: Option<UiTextDisplayMode>,
}

impl InProcessToolPlugin {
    pub fn new_with_resolver(
        plugin_name: &'static str,
        description: &'static str,
        tools: Vec<PluginToolDecl>,
        resolver: ToolInputResolver,
    ) -> Self {
        Self {
            plugin_name,
            description,
            tools,
            resolver: Some(resolver),
            tool_description_mode: None,
            ui_display_mode: None,
        }
    }

    pub fn tool_description_mode(mut self, mode: ToolDescriptionMode) -> Self {
        self.tool_description_mode = Some(mode);
        self
    }

    pub fn ui_display_mode(mut self, mode: UiTextDisplayMode) -> Self {
        self.ui_display_mode = Some(mode);
        self
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
            .hooks(HookSubscription::TOOL_INVOKE)
            .config_schema(crate::tool::definition::empty_config_schema());
        if let Some(mode) = self.tool_description_mode {
            builder = builder.tool_description_mode(mode);
        }
        if let Some(mode) = self.ui_display_mode {
            builder = builder.ui_display_mode(mode);
        }
        for tool in &self.tools {
            builder = builder.tool(tool.clone());
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
            let paths = crate::tool::apply_patch::planned_paths(&payload.patch)
                .map_err(|err| PluginError::new(err.to_string()))?;
            Ok(paths
                .into_iter()
                .map(crate::plugin::sdk::PathRequest::write)
                .collect())
        }
        "bash" => {
            let payload: ShellCommandInput = serde_json::from_value(input.clone())?;
            Ok(workdir_read_request(payload.workdir.as_deref()))
        }
        "powershell" => {
            let payload: ShellCommandInput = serde_json::from_value(input.clone())?;
            Ok(workdir_read_request(payload.workdir.as_deref()))
        }
        "glob" => {
            let payload: GlobToolInput = serde_json::from_value(input.clone())?;
            Ok(base_path_read_request(payload.path.as_deref()))
        }
        "grep" => {
            let payload: GrepToolInput = serde_json::from_value(input.clone())?;
            Ok(base_path_read_request(payload.path.as_deref()))
        }
        "monitor" => {
            let payload: MonitorToolInput = serde_json::from_value(input.clone())?;
            Ok(match payload {
                MonitorToolInput::Start { command, .. } => {
                    workdir_read_request(command.workdir.as_deref())
                }
                MonitorToolInput::List {}
                | MonitorToolInput::Read { .. }
                | MonitorToolInput::Stop { .. } => Vec::new(),
            })
        }
        "notebook_edit" => {
            let payload: NotebookEditToolInput = serde_json::from_value(input.clone())?;
            Ok(vec![crate::plugin::sdk::PathRequest::write(
                payload.notebook_path,
            )])
        }
        "lsp_definition" => {
            let payload: LspDefinitionToolInput = serde_json::from_value(input.clone())?;
            Ok(vec![crate::plugin::sdk::PathRequest::read(
                payload.position.file_path,
            )])
        }
        "lsp_references" => {
            let payload: LspReferencesToolInput = serde_json::from_value(input.clone())?;
            Ok(vec![crate::plugin::sdk::PathRequest::read(
                payload.position.file_path,
            )])
        }
        "lsp_hover" => {
            let payload: LspHoverToolInput = serde_json::from_value(input.clone())?;
            Ok(vec![crate::plugin::sdk::PathRequest::read(
                payload.position.file_path,
            )])
        }
        "lsp_diagnostics" => {
            let payload: LspDiagnosticsToolInput = serde_json::from_value(input.clone())?;
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
        "bash" => {
            let payload: ShellCommandInput = serde_json::from_value(input.clone())?;
            declared_shell_network_requests(
                "bash",
                payload.command.as_str(),
                &payload.network_effects,
            )
        }
        "powershell" => {
            let payload: ShellCommandInput = serde_json::from_value(input.clone())?;
            declared_shell_network_requests(
                "powershell",
                payload.command.as_str(),
                &payload.network_effects,
            )
        }
        "monitor" => {
            let payload: MonitorToolInput = serde_json::from_value(input.clone())?;
            match payload {
                MonitorToolInput::Start { command, .. } => declared_shell_network_requests(
                    "monitor",
                    command.command.as_str(),
                    &command.network_effects,
                ),
                MonitorToolInput::List {}
                | MonitorToolInput::Read { .. }
                | MonitorToolInput::Stop { .. } => Ok(Vec::new()),
            }
        }
        _ => Ok(Vec::new()),
    }
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
