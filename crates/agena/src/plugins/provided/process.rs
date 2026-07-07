//! `agena.process` plugin: run shell commands and manage background processes.

use crate::message::{ProcessShell, ProcessToolInput, ShellCommandInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    NetworkRequest, PathRequest, Plugin, PluginManifest, Result as SdkResult, ToolInvokeContext,
    ToolInvokeOutput, ToolTag, async_trait,
};
use crate::plugins::provided::router;
use agena_macros::{StaticToolSurface, ToolInputShape, ToolSuite};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const PROCESS_PLUGIN_ID: &str = "agena.process";

pub(crate) struct ProcessPlugin;

pub(crate) fn new_plugin() -> ProcessPlugin {
    ProcessPlugin
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessRunToolArgs {
    #[serde(default)]
    shell: ProcessShell,
    #[serde(flatten)]
    #[tool(flatten_shape)]
    command: ShellCommandInput,
    #[serde(default)]
    background: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("process_id"), non_empty("process_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessLogsToolArgs {
    process_id: String,
    #[serde(default)]
    since_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default)]
    wait_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("process_id"), non_empty("process_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessStopToolArgs {
    process_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "run",
    summary = "Run one shell process.",
    help = "Set `background = true` to keep the process attached to the session and receive a `process_id` for later inspection.",
    handler_receiver = ProcessPlugin,
    handle_with_context = ProcessPlugin::invoke_run,
    handle_field = args,
    permission_paths_handle = ProcessPlugin::permission_run,
    permission_networks_handle = ProcessPlugin::permission_networks_run,
    handle_by_value = true,
    display = detailed,
    tags(ToolTag::Mutating, ToolTag::Shell),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessRunToolInput {
    #[serde(flatten)]
    #[tool(flatten_shape)]
    args: ProcessRunToolArgs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "list",
    summary = "List active background processes.",
    handler_receiver = ProcessPlugin,
    handle_with_context = ProcessPlugin::invoke_list,
    permission_paths_handle = ProcessPlugin::permission_list,
    permission_networks_handle = ProcessPlugin::permission_networks_list,
    display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Shell),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessListToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "logs",
    summary = "Read background process logs.",
    handler_receiver = ProcessPlugin,
    handle_with_context = ProcessPlugin::invoke_logs,
    handle_field = args,
    permission_paths_handle = ProcessPlugin::permission_logs,
    permission_networks_handle = ProcessPlugin::permission_networks_logs,
    handle_by_value = true,
    display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Shell),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessLogsToolInput {
    #[serde(flatten)]
    #[tool(flatten_shape)]
    args: ProcessLogsToolArgs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "stop",
    summary = "Stop one background process.",
    handler_receiver = ProcessPlugin,
    handle_with_context = ProcessPlugin::invoke_stop,
    handle_field = args,
    permission_paths_handle = ProcessPlugin::permission_stop,
    permission_networks_handle = ProcessPlugin::permission_networks_stop,
    handle_by_value = true,
    display = detailed,
    tags(ToolTag::Mutating, ToolTag::Shell),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessStopToolInput {
    #[serde(flatten)]
    #[tool(flatten_shape)]
    args: ProcessStopToolArgs,
}

#[allow(dead_code)]
#[derive(Debug, ToolSuite)]
#[tool_suite(handler_receiver = ProcessPlugin)]
pub(crate) enum ProcessToolSuite {
    Run(ProcessRunToolInput),
    List(ProcessListToolInput),
    Logs(ProcessLogsToolInput),
    Stop(ProcessStopToolInput),
}

#[async_trait]
impl Plugin for ProcessPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("agena", "process", env!("CARGO_PKG_VERSION"));
        manifest.summary = Some("Command execution and background process tools.".to_string());
        manifest.set_display(crate::plugin::sdk::ToolDisplayPreset::BriefDetailed);
        manifest.tools.extend(ProcessToolSuite::tool_definitions());
        manifest
    }

    async fn tool_invoke(
        &self,
        input: crate::plugin::sdk::ToolInvokeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let crate::plugin::sdk::ToolInvokeInput {
            tool_name,
            session_id,
            call_id,
            workspace_root,
            input,
        } = input;
        let context = ToolInvokeContext {
            tool_name: tool_name.as_str(),
            session_id,
            call_id,
            workspace_root: workspace_root.as_str(),
        };
        let parsed = ProcessToolSuite::parse_tool(tool_name.as_str(), input)?;
        parsed
            .dispatch_tool_invoke_with_context(self, &context)
            .await
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        let parsed = ProcessToolSuite::parse_tool(tool, input.clone())?;
        parsed.dispatch_permission_paths(self).await
    }

    async fn permission_networks(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<NetworkRequest>> {
        let parsed = ProcessToolSuite::parse_tool(tool, input.clone())?;
        parsed.dispatch_permission_networks(self).await
    }
}

impl ProcessPlugin {
    async fn invoke_run(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessRunToolArgs,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "process",
            json_input(ProcessToolInput::Run {
                shell: args.shell,
                command: args.command,
                background: args.background,
            })?,
            context.session_id,
            context.call_id,
        )
    }

    async fn invoke_list(
        &self,
        context: &ToolInvokeContext<'_>,
        _input: &ProcessListToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "process",
            json_input(ProcessToolInput::List {})?,
            context.session_id,
            context.call_id,
        )
    }

    async fn invoke_logs(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessLogsToolArgs,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "process",
            json_input(ProcessToolInput::Logs {
                process_id: args.process_id,
                since_seq: args.since_seq,
                limit: args.limit,
                wait_ms: args.wait_ms,
            })?,
            context.session_id,
            context.call_id,
        )
    }

    async fn invoke_stop(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessStopToolArgs,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "process",
            json_input(ProcessToolInput::Stop {
                process_id: args.process_id,
            })?,
            context.session_id,
            context.call_id,
        )
    }

    async fn permission_run(&self, args: ProcessRunToolArgs) -> SdkResult<Vec<PathRequest>> {
        router::permission_paths_for(
            "process",
            &json_input(ProcessToolInput::Run {
                shell: args.shell,
                command: args.command,
                background: args.background,
            })?,
        )
    }

    async fn permission_list(&self, _input: &ProcessListToolInput) -> SdkResult<Vec<PathRequest>> {
        router::permission_paths_for("process", &json_input(ProcessToolInput::List {})?)
    }

    async fn permission_logs(&self, args: ProcessLogsToolArgs) -> SdkResult<Vec<PathRequest>> {
        router::permission_paths_for(
            "process",
            &json_input(ProcessToolInput::Logs {
                process_id: args.process_id,
                since_seq: args.since_seq,
                limit: args.limit,
                wait_ms: args.wait_ms,
            })?,
        )
    }

    async fn permission_stop(&self, args: ProcessStopToolArgs) -> SdkResult<Vec<PathRequest>> {
        router::permission_paths_for(
            "process",
            &json_input(ProcessToolInput::Stop {
                process_id: args.process_id,
            })?,
        )
    }

    async fn permission_networks_run(
        &self,
        args: ProcessRunToolArgs,
    ) -> SdkResult<Vec<NetworkRequest>> {
        router::permission_networks_for(
            "process",
            &json_input(ProcessToolInput::Run {
                shell: args.shell,
                command: args.command,
                background: args.background,
            })?,
        )
    }

    async fn permission_networks_list(
        &self,
        _input: &ProcessListToolInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        router::permission_networks_for("process", &json_input(ProcessToolInput::List {})?)
    }

    async fn permission_networks_logs(
        &self,
        args: ProcessLogsToolArgs,
    ) -> SdkResult<Vec<NetworkRequest>> {
        router::permission_networks_for(
            "process",
            &json_input(ProcessToolInput::Logs {
                process_id: args.process_id,
                since_seq: args.since_seq,
                limit: args.limit,
                wait_ms: args.wait_ms,
            })?,
        )
    }

    async fn permission_networks_stop(
        &self,
        args: ProcessStopToolArgs,
    ) -> SdkResult<Vec<NetworkRequest>> {
        router::permission_networks_for(
            "process",
            &json_input(ProcessToolInput::Stop {
                process_id: args.process_id,
            })?,
        )
    }
}

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params(err.to_string()))
}
