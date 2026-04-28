//! In-process plugins that wrap each of agena's built-in tools.
//!
//! All eleven built-ins (bash, read, view_file, apply_patch, glob, grep, task,
//! tool_search, todo_write, ask_user, monitor) are registered with the
//! [`PluginHost`] via [`PluginHostBuilder::register_static`]. Their `tool_invoke`
//! delegates to the corresponding `super::<tool>::execute` function via a
//! task-local [`BuiltinPluginContext`] that carries a `ToolExecutor` clone so
//! the body can access workspace, sandbox, subtask manager, monitor registry,
//! and permission state.
//!
//! Wire types (`BuiltinToolInput` / `BuiltinToolOutput`) are unchanged so
//! persisted history and provider tool-call serialization are not affected.
//! What changes is the dispatch path: `execute_invocation_detailed::Builtin`
//! now goes through the plugin host instead of `super::orchestrator`.

use std::cell::RefCell;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginManifest, Result as SdkResult,
    ToolBehavior as SdkToolBehavior, ToolDecl, ToolInvokeInput, ToolInvokeOutput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::HostClient;

use crate::message::{
    ApplyPatchToolInput, AskUserToolInput, BashToolInput, BuiltinToolInput, BuiltinToolOutput,
    GlobToolInput, GrepToolInput, MonitorToolInput, ReadToolInput, TaskToolInput,
    TodoWriteToolInput, ToolSearchToolInput, ViewFileToolInput,
};
use crate::tool::result::BuiltinExecution;
use crate::tool::{BuiltinExecutionContext, ToolExecutor, orchestrator};

thread_local! {
    /// Task-local executor used by the in-process built-in plugins. Set by
    /// [`with_executor`] for the duration of a `tool_invoke` call.
    static BUILTIN_CTX: RefCell<Option<ToolExecutor>> = const { RefCell::new(None) };
}

/// Run `f` with the given executor visible to in-process built-in plugin
/// dispatch. Used by `ToolExecutor::execute_invocation_detailed` when it
/// hands control to the plugin host.
pub(crate) fn with_executor<R>(executor: &ToolExecutor, f: impl FnOnce() -> R) -> R {
    BUILTIN_CTX.with(|cell| {
        let prev = cell.replace(Some(executor.clone()));
        let out = f();
        *cell.borrow_mut() = prev;
        out
    })
}

fn current_executor() -> Result<ToolExecutor, PluginError> {
    BUILTIN_CTX.with(|cell| {
        cell.borrow()
            .clone()
            .ok_or_else(|| PluginError::new("built-in plugin invoked without executor context"))
    })
}

/// Static plugin id used for every built-in tool. Keep stable: hot-reload
/// reuses transports keyed by id.
pub(crate) const BUILTIN_PLUGIN_ID: &str = "agena.builtin";

/// One-stop in-process plugin that exposes every built-in tool. We use a
/// single plugin (rather than 11) to keep the manifest small and avoid
/// duplicating registration boilerplate.
pub(crate) struct BuiltinPlugin;

impl BuiltinPlugin {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for BuiltinPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-builtins", env!("CARGO_PKG_VERSION"))
            .description("Agena built-in tools delivered as in-process plugin.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .tool(decl::<BashToolInput>(
                "bash",
                "Execute a shell command inside the sandboxed workspace.",
                SdkToolBehavior::WriteSandboxed,
            ))
            .tool(decl::<ReadToolInput>(
                "read",
                "Read a UTF-8 text file or list a directory with optional pagination.",
                SdkToolBehavior::ReadOnly,
            ))
            .tool(decl::<ViewFileToolInput>(
                "view_file",
                "Load a local file and attach it back to the conversation as inline multimodal input.",
                SdkToolBehavior::ReadOnly,
            ))
            .tool(decl::<ApplyPatchToolInput>(
                "apply_patch",
                "Apply a structured patch that can add, update, move, or delete files.",
                SdkToolBehavior::WriteSandboxed,
            ))
            .tool(decl::<GlobToolInput>(
                "glob",
                "Search files by glob pattern from the workspace or a subdirectory.",
                SdkToolBehavior::ReadOnly,
            ))
            .tool(decl::<GrepToolInput>(
                "grep",
                "Search file contents by regex pattern with optional include glob.",
                SdkToolBehavior::ReadOnly,
            ))
            .tool(decl::<TaskToolInput>(
                "task",
                "Create or resume a typed subagent task session.",
                SdkToolBehavior::Task,
            ))
            .tool(decl::<ToolSearchToolInput>(
                "tool_search",
                "Search the tool catalog and optionally load deferred tools.",
                SdkToolBehavior::ReadOnly,
            ))
            .tool(decl::<TodoWriteToolInput>(
                "todo_write",
                "Replace the session todo list with a short execution plan.",
                SdkToolBehavior::ReadOnly,
            ))
            .tool(decl::<AskUserToolInput>(
                "ask_user",
                "Ask short questions and wait for answers.",
                SdkToolBehavior::ReadOnly,
            ))
            .tool(decl::<MonitorToolInput>(
                "monitor",
                "Run a long-lived shell command in the background and stream its events.",
                SdkToolBehavior::WriteSandboxed,
            ))
            .build()
    }

    async fn init(
        &self,
        _ctx: InitContext,
        _host: Arc<dyn HostClient>,
    ) -> SdkResult<InitOutcome> {
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let executor = current_executor()?;
        let builtin = parse_builtin(&input.tool_name, input.input)
            .map_err(|err| PluginError::new(format!("parse {}: {err}", input.tool_name)))?;
        let context = BuiltinExecutionContext {
            session_id: if input.session_id < 0 {
                None
            } else {
                Some(input.session_id)
            },
            call_id: if input.call_id < 0 {
                None
            } else {
                Some(input.call_id)
            },
        };
        let execution = orchestrator::execute_builtin(&executor, &builtin, context)
            .map_err(|err| PluginError::new(format!("{}: {err}", input.tool_name)))?;
        Ok(builtin_to_invoke_output(execution))
    }
}

fn decl<T: schemars::JsonSchema>(
    name: &str,
    description: &str,
    behavior: SdkToolBehavior,
) -> ToolDecl {
    ToolDecl::new(
        name,
        crate::tool::definition::json_schema_for::<T>(),
    )
    .description(description)
    .behavior(behavior)
}

fn parse_builtin(tool: &str, input: JsonValue) -> Result<BuiltinToolInput, serde_json::Error> {
    Ok(match tool {
        "bash" => BuiltinToolInput::Bash(serde_json::from_value(input)?),
        "read" => BuiltinToolInput::Read(serde_json::from_value(input)?),
        "view_file" => BuiltinToolInput::ViewFile(serde_json::from_value(input)?),
        "apply_patch" => BuiltinToolInput::ApplyPatch(serde_json::from_value(input)?),
        "glob" => BuiltinToolInput::Glob(serde_json::from_value(input)?),
        "grep" => BuiltinToolInput::Grep(serde_json::from_value(input)?),
        "task" => BuiltinToolInput::Task(serde_json::from_value(input)?),
        "tool_search" => BuiltinToolInput::ToolSearch(serde_json::from_value(input)?),
        "todo_write" => BuiltinToolInput::TodoWrite(serde_json::from_value(input)?),
        "ask_user" | "request_user_input" => {
            BuiltinToolInput::AskUser(serde_json::from_value(input)?)
        }
        "monitor" => BuiltinToolInput::Monitor(serde_json::from_value(input)?),
        other => {
            return Err(serde::de::Error::custom(format!(
                "unknown built-in tool `{other}`"
            )));
        }
    })
}

fn builtin_to_invoke_output(execution: BuiltinExecution) -> ToolInvokeOutput {
    let envelope = BuiltinResponseEnvelope {
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

/// Wire envelope used to round-trip a [`BuiltinExecution`] through the
/// plugin host's JSON payload. Lets the executor reconstruct the
/// `apply_patch` side-channel that the plugin wire types do not preserve.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct BuiltinResponseEnvelope {
    pub output: BuiltinToolOutput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_patch: Option<crate::tool::apply_patch::ApplyPatchExecution>,
}

/// Decode the payload emitted by [`builtin_to_invoke_output`] back into a
/// [`BuiltinResponseEnvelope`].
pub(crate) fn payload_to_builtin_envelope(
    payload: Option<&JsonValue>,
) -> Result<BuiltinResponseEnvelope, serde_json::Error> {
    match payload {
        Some(value) => serde_json::from_value(value.clone()),
        None => Err(serde::de::Error::custom(
            "built-in plugin response missing payload",
        )),
    }
}
