//! Shared scaffolding for static plugins backed by agena's in-process
//! executor implementations. These tools use the same plugin registry and
//! permission path as any other plugin tool; this module only supplies the
//! executor context needed by their Rust implementations.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::{LazyLock, Mutex};

use serde_json::Value as JsonValue;

use crate::message::{
    ApplyPatchToolInput, GlobToolInput, GrepToolInput, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput, MonitorToolInput,
    NetworkEffect, NotebookEditToolInput, ReadToolInput, ShellCommandInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    HookSubscription, InitOutcome, NetworkRequest, PluginManifest, PluginToolDecl,
    Result as SdkResult, ToolDisplayPreset, ToolInputShape, ToolInvokeOutput, ToolSuiteSurface,
    ToolSurface,
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

fn routed_internal_tool_names(tool_name: &str) -> &'static [&'static str] {
    match tool_name {
        "exec" => &["bash", "powershell"],
        "exec.bash" => &["bash"],
        "exec.powershell" => &["powershell"],
        "monitor" | "monitor.start" | "monitor.list" | "monitor.read" | "monitor.stop" => {
            &["monitor"]
        }
        "schedule" => &["cron_list", "cron_create", "cron_delete", "schedule_wakeup"],
        "schedule.list" => &["cron_list"],
        "schedule.create" => &["cron_create"],
        "schedule.delete" => &["cron_delete"],
        "schedule.wakeup" => &["schedule_wakeup"],
        _ => &[],
    }
}

pub(crate) struct InProcessToolPlugin {
    plugin_name: &'static str,
    description: &'static str,
    tools: Vec<PluginToolDecl>,
    resolver: Option<ToolInputResolver>,
    display: Option<ToolDisplayPreset>,
}

impl InProcessToolPlugin {
    pub fn new_with_tool_surface<T: ToolSurface>(
        plugin_name: &'static str,
        description: &'static str,
    ) -> Self {
        Self::new_with_resolver(
            plugin_name,
            description,
            vec![T::tool_decl()],
            T::resolve_tool,
        )
    }

    pub fn new_with_tool_suite<T: ToolSuiteSurface>(
        plugin_name: &'static str,
        description: &'static str,
    ) -> Self {
        Self::new_with_resolver(plugin_name, description, T::tool_decls(), T::resolve_tool)
    }

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
            display: None,
        }
    }

    #[allow(dead_code)]
    pub fn compact(self) -> Self {
        self.display(ToolDisplayPreset::Compact)
    }

    #[allow(dead_code)]
    pub fn brief(self) -> Self {
        self.display(ToolDisplayPreset::Compact)
    }

    #[allow(dead_code)]
    pub fn brief_detailed(self) -> Self {
        self.display(ToolDisplayPreset::BriefDetailed)
    }

    pub fn detailed(self) -> Self {
        self.display(ToolDisplayPreset::Detailed)
    }

    pub fn display(mut self, preset: ToolDisplayPreset) -> Self {
        self.display = Some(preset);
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

#[crate::plugin::sdk::async_trait]
impl crate::plugin::sdk::Plugin for InProcessToolPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut builder = PluginManifest::builder(self.plugin_name, env!("CARGO_PKG_VERSION"))
            .description(self.description)
            .hooks(HookSubscription::TOOL_INVOKE)
            .tools(self.tools.clone());
        if let Some(display) = self.display {
            builder = builder.display(display);
        }
        builder.build()
    }

    async fn init(
        &self,
        _ctx: crate::plugin::sdk::InitContext,
        _host: Arc<dyn crate::plugin::sdk::HostClient>,
    ) -> SdkResult<InitOutcome> {
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(
        &self,
        input: crate::plugin::sdk::ToolInvokeInput,
    ) -> SdkResult<ToolInvokeOutput> {
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
    ) -> SdkResult<Vec<crate::plugin::sdk::NetworkRequest>> {
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
        "bash" => {
            let payload: ShellCommandInput = parse_shape_input(input)?;
            Ok(workdir_read_request(payload.workdir.as_deref()))
        }
        "powershell" => {
            let payload: ShellCommandInput = parse_shape_input(input)?;
            Ok(workdir_read_request(payload.workdir.as_deref()))
        }
        "glob" => {
            let payload: GlobToolInput = parse_shape_input(input)?;
            Ok(base_path_read_request(payload.path.as_deref()))
        }
        "grep" => {
            let payload: GrepToolInput = parse_shape_input(input)?;
            Ok(base_path_read_request(payload.path.as_deref()))
        }
        "monitor" => {
            let payload: MonitorToolInput = parse_shape_input(input)?;
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
            let payload: NotebookEditToolInput = parse_shape_input(input)?;
            Ok(vec![crate::plugin::sdk::PathRequest::write(
                payload.notebook_path,
            )])
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

#[cfg(test)]
pub(crate) fn permission_paths_for_surface<T: ToolSurface>(
    tool_name: &str,
    input: &serde_json::Value,
) -> SdkResult<Vec<crate::plugin::sdk::PathRequest>> {
    let (resolved_tool_name, resolved_input) = T::resolve_tool(tool_name, input.clone())?;
    permission_paths_for(&resolved_tool_name, &resolved_input)
}

pub(crate) fn permission_networks_for(
    tool: &str,
    input: &serde_json::Value,
) -> SdkResult<Vec<NetworkRequest>> {
    match tool {
        "bash" => {
            let payload: ShellCommandInput = parse_shape_input(input)?;
            declared_shell_network_requests(
                "bash",
                payload.command.as_str(),
                &payload.network_effects,
            )
        }
        "powershell" => {
            let payload: ShellCommandInput = parse_shape_input(input)?;
            declared_shell_network_requests(
                "powershell",
                payload.command.as_str(),
                &payload.network_effects,
            )
        }
        "monitor" => {
            let payload: MonitorToolInput = parse_shape_input(input)?;
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

fn parse_shape_input<T: ToolInputShape>(input: &serde_json::Value) -> SdkResult<T> {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::plugin::sdk::{Plugin, ToolDescriptionMode, UiTextDisplayMode};

    use super::*;

    fn passthrough(tool: &str, input: JsonValue) -> SdkResult<(String, JsonValue)> {
        Ok((tool.to_string(), input))
    }

    #[test]
    fn permission_helpers_use_shape_normalization_for_monitor_inputs() {
        let paths = permission_paths_for("monitor", &json!({}))
            .expect("empty monitor input should normalize to list");
        assert!(paths.is_empty());

        let paths = permission_paths_for(
            "monitor",
            &json!({
                "action": "start",
                "command": "echo ok",
                "workdir": "  crates/agena  ",
                "filesystem_effects": [],
                "network_effects": []
            }),
        )
        .expect("monitor start should parse through ToolInputShape");
        assert_eq!(
            paths,
            vec![crate::plugin::sdk::PathRequest::read("crates/agena")]
        );
    }

    #[test]
    fn tool_surface_permission_helpers_resolve_before_extracting_permissions() {
        let paths = permission_paths_for_surface::<crate::plugins::provided::lsp::LspToolInput>(
            "lsp",
            &json!({
                "action": "definition",
                "file_path": " src/main.rs ",
                "line": 4,
                "character": 8
            }),
        )
        .expect("lsp surface should resolve into routed definition input");
        assert_eq!(
            paths,
            vec![crate::plugin::sdk::PathRequest::read("src/main.rs")]
        );
    }

    #[test]
    fn in_process_plugin_display_shortcuts_set_manifest_display_modes() {
        let manifest = InProcessToolPlugin::new_with_resolver(
            "test.plugin",
            "Test plugin.",
            vec![],
            passthrough,
        )
        .compact()
        .manifest();
        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Summary));

        let manifest = InProcessToolPlugin::new_with_resolver(
            "test.plugin",
            "Test plugin.",
            vec![],
            passthrough,
        )
        .brief()
        .manifest();
        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Summary));

        let manifest = InProcessToolPlugin::new_with_resolver(
            "test.plugin",
            "Test plugin.",
            vec![],
            passthrough,
        )
        .brief_detailed()
        .manifest();
        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Detailed));

        let manifest = InProcessToolPlugin::new_with_resolver(
            "test.plugin",
            "Test plugin.",
            vec![],
            passthrough,
        )
        .detailed()
        .manifest();
        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Detailed)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Detailed));
    }
}
