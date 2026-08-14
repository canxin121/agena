//! Builtin tool execution contracts and executors.

pub(crate) mod apply_patch;
pub mod ask_user;
pub(crate) mod bash;
pub(crate) mod builtin_tools;
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
pub(crate) mod powershell;
pub(crate) mod process_tool;
pub(crate) mod read;
pub mod render;
pub mod result;
pub mod router;
pub(crate) mod shell;
pub(crate) mod shell_tools;
pub(crate) mod snapshot;
pub(crate) mod task;
pub mod tool_registry;
pub(crate) mod tool_search;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use crate::authorization::ExecutionPrincipal;
use crate::part::AskUserToolInput;
use agena_domain::AccessKind;
use agena_domain::NetworkTarget;
use agena_domain::PermissionDecision;
use agena_domain::StructuredObject;
use agena_domain::ToolInvocation;
use agena_domain::ToolOutput;
use agena_plugin_host::{
    PluginHost, ToolAfterInput as PluginToolAfterInput, ToolBeforeInput as PluginToolBeforeInput,
    ToolDefinitionInput as PluginToolDefinitionInput, ToolFailureInput as PluginToolFailureInput,
    ToolInvokeInput as PluginToolInvokeInput,
    ToolPermissionNetworksInput as PluginToolPermissionNetworksInput,
    ToolPermissionPathsInput as PluginToolPermissionPathsInput,
    registry::RegisteredTool,
    sdk::{
        InputNetworkSpec as SdkInputNetworkSpec, InputPathSpec as SdkInputPathSpec,
        NetworkAccessSpec as SdkNetworkAccessSpec, PathAccessSpec as SdkPathAccessSpec,
        PathKind as SdkPathKind, ShellEnvInput as PluginShellEnvInput,
        ToolResultPolicy as SdkToolResultPolicy, ToolStreamingMode as SdkToolStreamingMode,
    },
};
use agena_tool::{
    PreparedShellCommand, PreparedToolInvocation, ShellError, ShellOutput, ShellRequest,
    ToolPermissionCheck,
};

// Model-facing tool results must be small enough that a sequence of noisy
// commands cannot consume the whole context window. The complete result is
// durably stored under `.agena/tool-results`; this preview keeps the beginning
// and end, which normally contain setup and final diagnostics.
const TOOL_MODEL_OUTPUT_MAX_LINES: usize = 400;
const TOOL_MODEL_OUTPUT_MAX_BYTES: usize = 16 * 1024;
const TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES: usize = 12 * 1024;
const TOOL_MODEL_STRUCTURED_MAX_DEPTH: usize = 6;
const TOOL_MODEL_STRUCTURED_MAX_FIELDS: usize = 32;
const TOOL_MODEL_STRUCTURED_MAX_ITEMS: usize = 32;
const TOOL_MODEL_STRUCTURED_STRING_MAX_BYTES: usize = 768;
use self::output_helpers::*;
pub use self::tool_registry::*;

pub use crate::{MonitorError, MonitorRead, MonitorReadParams, MonitorService, MonitorStartParams};
pub use builtin_tools::BuiltinToolSet;
pub use payload::{ToolPayloadInput, ToolPayloadOutput};
pub use render::{
    DetailSource, MarkdownWriter, RenderContext, ToolResultRender, render_tool_payload_markdown,
    render_tool_payload_markdown_with_name,
};
pub use result::{ToolExecutionView, ToolInvocationExecution, ToolPayloadExecution};
pub use snapshot::registry_for_executor as snapshot_registry_for_executor;
pub use tool_registry::{
    ExecutionPermissionInspector, ExecutionTool, ToolApiBinding, ToolError, ToolExecutor,
};

#[cfg(test)]
mod tests;
