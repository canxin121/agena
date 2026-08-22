//! Runtime boundary for tools invoked outside a session lifecycle.
//!
//! MCP server calls have no Agena session ID, so they cannot use the
//! session-authorized tool port. Concrete executor selection (the active
//! session executor or a bootstrap fallback) remains inside the runtime
//! composition adapter.

use std::path::PathBuf;

use agena_domain::{RawOutput, ToolInvocation, ViewBlock};
use agena_runtime_tools::tool::human_view::BuiltinHumanRenderer;
use agena_tool::ToolHumanRenderer;
use async_trait::async_trait;

/// Ephemeral human projection of a raw tool result. This value is produced on
/// demand and must never be written back to a session part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeToolHumanPresentation {
    pub title: String,
    pub summary: String,
    pub blocks: Vec<ViewBlock>,
}

/// Ephemeral model and human projections of the one durable raw result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeToolResultProjection {
    pub model: String,
    pub human: RuntimeToolHumanPresentation,
}

#[derive(Debug, Clone)]
/// Descriptor of a runtime tool.
pub struct RuntimeToolDescriptor {
    pub name: String,
    pub summary: Option<String>,
    pub before_help: Option<String>,
    pub after_help: Option<String>,
    pub input_schema: serde_json::Value,
    pub output_schema: Option<serde_json::Value>,
    /// Whether this tool is marked as interactive by either its permission
    /// contract or its discovery metadata. Non-interactive hosts, such as MCP
    /// clients, must not expose it.
    pub interactive: bool,
    /// True only when the authority-bearing permission contract qualifies the
    /// tool for Agena's read-only fast path (no shell, writes, mutation, or
    /// network access).
    pub read_only: bool,
    /// The tool may mutate state, write paths, or execute arbitrary shell
    /// commands. This is exposed to MCP clients as a safety hint and is also
    /// used by the MCP server's default-deny exposure policy.
    pub destructive: bool,
    /// The tool can reach network targets outside the local workspace.
    pub open_world: bool,
    /// Long-running autonomous task tools are not suitable for the stateless
    /// ChatGPT connector surface.
    pub task: bool,
    /// Stable full plugin id (for example, `agena.fs`).
    pub plugin_id: Option<String>,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("runtime tool execution failed: {message}")]
/// Error executing a runtime tool.
pub struct RuntimeToolExecutionError {
    message: String,
}

impl RuntimeToolExecutionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn from_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::new(agena_failure::diagnostic::format_error_chain(error))
    }
}

#[async_trait]
/// Service executing tools in the runtime.
pub trait RuntimeToolExecutionService: Send + Sync {
    async fn available_runtime_tools(&self) -> Vec<RuntimeToolDescriptor>;

    /// Provider-facing Tool API function declarations available to model
    /// executions. These are provider-contract values, not concrete registry
    /// bindings, so diagnostics and integration fixtures can validate the
    /// advertised model surface without traversing a concrete tool executor.
    async fn available_tool_api_definitions(&self) -> Vec<agena_provider::ToolApiDefinition>;

    async fn execute_runtime_tool(
        &self,
        invocation: &ToolInvocation,
        call_id: i64,
    ) -> Result<crate::SessionToolExecutionOutcome, RuntimeToolExecutionError>;

    /// Render one invocation/result pair at the read boundary. Concrete
    /// runtimes delegate to the owning plugin/tool first. The default uses
    /// the same built-in renderer as the concrete executor, so lightweight
    /// runtime adapters do not regress to an opaque JSON card.
    async fn render_tool_result(
        &self,
        invocation: &ToolInvocation,
        output: &RawOutput,
    ) -> RuntimeToolResultProjection {
        let model = match output.payload.as_ref() {
            Some(payload) => match serde_json::to_string(payload) {
                Ok(payload) => payload,
                Err(error) => {
                    tracing::error!(
                        tool = %invocation.name,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "serialize the default runtime tool result payload for model projection",
                            &error,
                        ),
                        "default runtime tool result projection fell back to its text representation"
                    );
                    if output.text.is_empty() {
                        "[tool result payload could not be serialized]".to_owned()
                    } else {
                        output.text.clone()
                    }
                }
            },
            None => output.text.clone(),
        };
        let mut renderer = BuiltinHumanRenderer::new(invocation.name.as_str());
        if let Some(command) = invocation
            .input
            .get("command")
            .and_then(|value| value.as_text())
            .filter(|command| !command.trim().is_empty())
        {
            renderer = renderer.with_command(command);
        }
        if let Some(cwd) = invocation
            .input
            .get("workdir")
            .or_else(|| invocation.input.get("cwd"))
            .and_then(|value| value.as_text())
            .filter(|cwd| !cwd.trim().is_empty())
        {
            renderer = renderer.with_cwd(cwd);
        }
        let context = agena_tool::RenderContext {
            workspace_root: PathBuf::new(),
            command: None,
        };
        let blocks = renderer
            .render_human(&context, output)
            .unwrap_or_else(|_| Vec::new());
        RuntimeToolResultProjection {
            human: RuntimeToolHumanPresentation {
                title: agena_tool::completed_tool_title(invocation, output),
                summary: BuiltinHumanRenderer::human_summary_for_tool(
                    invocation.name.as_str(),
                    output,
                ),
                blocks,
            },
            model,
        }
    }
}
