//! Builtin tool execution contracts and executors.

pub(crate) mod apply_patch;
pub mod ask_user;
pub(crate) mod bash;
pub(crate) mod builtin_tools;
pub(crate) mod content;
pub(crate) mod cron;
pub mod definition;
mod discovery;
mod executor;
pub(crate) mod file_attachment;
pub(crate) mod glob;
pub(crate) mod grep;
pub mod human_view;
pub(crate) mod lsp;
pub(crate) mod monitor_tool;
pub(crate) mod orchestrator;
mod output_helpers;
pub mod payload;
mod post_edit;
pub(crate) mod powershell;
pub(crate) mod read;
mod read_media;
pub mod result;
pub mod router;
pub(crate) mod shell;
pub(crate) mod shell_tool;
pub(crate) mod shell_tools;
pub(crate) mod task;
mod terminal_tool;
pub mod tool_registry;
pub(crate) mod tool_search;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use crate::authorization::ExecutionPrincipal;
use crate::part::AskUserToolInput;
use agena_domain::AccessKind;
use agena_domain::FilesystemEffects;
use agena_domain::NetworkTarget;
use agena_domain::PermissionDecision;
use agena_domain::StructuredObject;
use agena_domain::ToolInvocation;
use agena_domain::ToolOutput;
use agena_plugin_host::{
    PluginHost, ToolAfterInput as PluginToolAfterInput, ToolBeforeInput as PluginToolBeforeInput,
    ToolDefinitionInput as PluginToolDefinitionInput, ToolFailureInput as PluginToolFailureInput,
    ToolInvokeInput as PluginToolInvokeInput,
    registry::RegisteredTool,
    sdk::{ShellEnvInput as PluginShellEnvInput, ToolStreamingMode as SdkToolStreamingMode},
};
use agena_tool::{
    PreparedShellCommand, PreparedToolInvocation, ShellError, ShellOutput, ShellRequest,
    ToolPermissionCheck,
};

// Model-facing tool results are bounded in one place, at the session layer
// that builds a request's runs (`agena-runtime-session`'s
// `bound_model_tool_outputs`). A tool never declares its own output budget.
use self::output_helpers::*;
pub use self::tool_registry::*;

pub use crate::{MonitorError, MonitorRead, MonitorReadParams, MonitorService, MonitorStartParams};
pub use builtin_tools::BuiltinToolSet;
pub use output_helpers::{bounded_model_output_preview, line_count, model_output_exceeds_boundary};
pub use payload::{ToolPayloadInput, ToolPayloadOutput};
pub use result::{ToolExecutionView, ToolInvocationExecution, ToolPayloadExecution};
pub use tool_registry::{
    ExecutionPermissionInspector, ExecutionTool, ToolApiBinding, ToolError, ToolExecutor,
};

#[cfg(test)]
mod tests;
