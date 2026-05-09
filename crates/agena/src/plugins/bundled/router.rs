//! Shared scaffolding for first-party plugins backed by agena's in-process
//! executor implementations.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use crate::entry::result::FirstPartyExecution;
use crate::entry::{FirstPartyExecutionContext, ToolExecutor, orchestrator};
use crate::message::{
    ApplyPatchToolInput, BashToolInput, FirstPartyToolInput, FirstPartyToolOutput, GlobToolInput,
    GrepToolInput, MonitorToolInput, PowerShellToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginEntryDecl, PluginManifest,
    Result as SdkResult, ToolInvokeInput, ToolInvokeOutput,
};

thread_local! {
    static BUILTIN_CTX: RefCell<Option<ToolExecutor>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BuiltinContextKey {
    session_id: i64,
    call_id: i64,
    tool_name: String,
}

static BUILTIN_CTX_BY_CALL: LazyLock<Mutex<HashMap<BuiltinContextKey, ToolExecutor>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn with_executor<R>(
    executor: &ToolExecutor,
    session_id: i64,
    call_id: i64,
    tool_name: String,
    f: impl FnOnce() -> R,
) -> R {
    let key = BuiltinContextKey {
        session_id,
        call_id,
        tool_name,
    };
    if let Ok(mut contexts) = BUILTIN_CTX_BY_CALL.lock() {
        contexts.insert(key.clone(), executor.clone());
    }
    let out = BUILTIN_CTX.with(|cell| {
        let prev = cell.replace(Some(executor.clone()));
        let out = f();
        *cell.borrow_mut() = prev;
        out
    });
    if let Ok(mut contexts) = BUILTIN_CTX_BY_CALL.lock() {
        contexts.remove(&key);
    }
    out
}

fn current_executor(
    session_id: i64,
    call_id: i64,
    tool_name: &str,
) -> Result<ToolExecutor, PluginError> {
    if let Some(executor) = BUILTIN_CTX.with(|cell| cell.borrow().clone()) {
        return Ok(executor);
    }
    let key = BuiltinContextKey {
        session_id,
        call_id,
        tool_name: tool_name.to_string(),
    };
    BUILTIN_CTX_BY_CALL
        .lock()
        .ok()
        .and_then(|contexts| contexts.get(&key).cloned())
        .ok_or_else(|| PluginError::new("first-party plugin invoked without executor context"))
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
    if let Some(executor) = BUILTIN_CTX.with(|cell| cell.borrow().clone()) {
        return Some(executor);
    }
    let key = BuiltinContextKey {
        session_id,
        call_id,
        tool_name: tool_name.to_string(),
    };
    BUILTIN_CTX_BY_CALL
        .lock()
        .ok()
        .and_then(|contexts| contexts.get(&key).cloned())
}

pub(crate) struct FirstPartyRouterPlugin {
    plugin_name: &'static str,
    description: &'static str,
    entries: Vec<PluginEntryDecl>,
}

impl FirstPartyRouterPlugin {
    pub fn new(
        plugin_name: &'static str,
        description: &'static str,
        entries: Vec<PluginEntryDecl>,
    ) -> Self {
        Self {
            plugin_name,
            description,
            entries,
        }
    }
}

#[async_trait]
impl Plugin for FirstPartyRouterPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut builder = PluginManifest::builder(self.plugin_name, env!("CARGO_PKG_VERSION"))
            .description(self.description)
            .hooks(HookSubscription::TOOL_INVOKE);
        for entry in &self.entries {
            builder = builder.entry(entry.clone());
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
        invoke_first_party_tool(
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

pub(crate) fn invoke_first_party_tool(
    tool_name: &str,
    input: JsonValue,
    session_id: i64,
    call_id: i64,
) -> SdkResult<ToolInvokeOutput> {
    let first_party = parse_first_party_tool(tool_name, input)
        .map_err(|err| PluginError::new(format!("parse {tool_name}: {err}")))?;
    let executor = current_executor(session_id, call_id, tool_name)?;
    let context = FirstPartyExecutionContext {
        session_id: (session_id >= 0).then_some(session_id),
        call_id: (call_id >= 0).then_some(call_id),
        session_context: None,
    };
    let execution = orchestrator::execute_first_party(&executor, &first_party, context)
        .map_err(|err| PluginError::new(format!("{tool_name}: {err}")))?;
    Ok(first_party_to_invoke_output(execution))
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

pub(crate) fn parse_first_party_tool(
    tool: &str,
    input: JsonValue,
) -> Result<FirstPartyToolInput, serde_json::Error> {
    Ok(match tool {
        "bash" => FirstPartyToolInput::Bash(serde_json::from_value(input)?),
        "read" => FirstPartyToolInput::Read(serde_json::from_value(input)?),
        "view_file" => FirstPartyToolInput::ViewFile(serde_json::from_value(input)?),
        "apply_patch" => FirstPartyToolInput::ApplyPatch(serde_json::from_value(input)?),
        "glob" => FirstPartyToolInput::Glob(serde_json::from_value(input)?),
        "grep" => FirstPartyToolInput::Grep(serde_json::from_value(input)?),
        "task" => FirstPartyToolInput::Task(serde_json::from_value(input)?),
        "tool_search" => FirstPartyToolInput::ToolSearch(serde_json::from_value(input)?),
        "todo_write" => FirstPartyToolInput::TodoWrite(serde_json::from_value(input)?),
        "ask_user" => FirstPartyToolInput::AskUser(serde_json::from_value(input)?),
        "monitor" => FirstPartyToolInput::Monitor(serde_json::from_value(input)?),
        "web_fetch" => FirstPartyToolInput::WebFetch(serde_json::from_value(input)?),
        "web_search" => FirstPartyToolInput::WebSearch(serde_json::from_value(input)?),
        "enter_plan_mode" => FirstPartyToolInput::EnterPlanMode(serde_json::from_value(input)?),
        "exit_plan_mode" => FirstPartyToolInput::ExitPlanMode(serde_json::from_value(input)?),
        "enter_worktree" => FirstPartyToolInput::EnterWorktree(serde_json::from_value(input)?),
        "exit_worktree" => FirstPartyToolInput::ExitWorktree(serde_json::from_value(input)?),
        "cron_create" => FirstPartyToolInput::CronCreate(serde_json::from_value(input)?),
        "cron_list" => FirstPartyToolInput::CronList(serde_json::from_value(input)?),
        "cron_delete" => FirstPartyToolInput::CronDelete(serde_json::from_value(input)?),
        "schedule_wakeup" => FirstPartyToolInput::ScheduleWakeup(serde_json::from_value(input)?),
        "lsp_definition" => FirstPartyToolInput::LspDefinition(serde_json::from_value(input)?),
        "lsp_references" => FirstPartyToolInput::LspReferences(serde_json::from_value(input)?),
        "lsp_hover" => FirstPartyToolInput::LspHover(serde_json::from_value(input)?),
        "lsp_diagnostics" => FirstPartyToolInput::LspDiagnostics(serde_json::from_value(input)?),
        "notebook_edit" => FirstPartyToolInput::NotebookEdit(serde_json::from_value(input)?),
        "powershell" => FirstPartyToolInput::PowerShell(serde_json::from_value(input)?),
        other => {
            return Err(serde::de::Error::custom(format!(
                "unknown first-party tool `{other}`"
            )));
        }
    })
}

pub(crate) fn first_party_to_invoke_output(execution: FirstPartyExecution) -> ToolInvokeOutput {
    let envelope = FirstPartyResponseEnvelope {
        output: execution.output,
        apply_patch: execution.apply_patch,
    };
    let payload = serde_json::to_value(&envelope).ok();
    ToolInvokeOutput {
        title: execution.view.title,
        output_text: execution.view.output_text,
        payload,
        metadata: execution.view.metadata.into_iter().collect(),
        attachments: execution.view.attachments,
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct FirstPartyResponseEnvelope {
    pub output: FirstPartyToolOutput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_patch: Option<crate::entry::apply_patch::ApplyPatchExecution>,
}

pub(crate) fn payload_to_first_party_envelope(
    payload: Option<&JsonValue>,
) -> Result<FirstPartyResponseEnvelope, serde_json::Error> {
    match payload {
        Some(value) => serde_json::from_value(value.clone()),
        None => Err(serde::de::Error::custom(
            "first-party plugin response missing payload",
        )),
    }
}
