//! `agena.process` plugin: run shell commands and manage background processes.

use crate::message::{ProcessShell, ProcessToolInput, ShellCommandInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    NetworkRequest, PathRequest, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput,
};
use crate::plugins::provided::router;
use agena_macros::ToolInputShape;
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

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "process",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Command execution and background process tools.",
    display = brief_detailed
)]
impl ProcessPlugin {
    #[tool(
        name = "run",
        summary = "Run one shell process.",
        help = "Set `background = true` to keep the process attached to the session and receive a `process_id` for later inspection.",
        mutating,
        shell,
        display = detailed,
        trim(
            "command",
            "description",
            "workdir",
            "filesystem_effects[].path",
            "network_effects[].target"
        ),
        non_empty("command"),
        permission(paths = permission_run, networks = permission_networks_run)
    )]
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

    #[tool(
        name = "list",
        summary = "List active background processes.",
        read_only,
        shell,
        display = detailed,
        permission(paths = permission_list, networks = permission_networks_list),
        concurrency_safe
    )]
    async fn invoke_list(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "process",
            json_input(ProcessToolInput::List {})?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        name = "logs",
        summary = "Read background process logs.",
        read_only,
        shell,
        display = detailed,
        trim("process_id"),
        non_empty("process_id"),
        permission(paths = permission_logs, networks = permission_networks_logs),
        concurrency_safe
    )]
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

    #[tool(
        name = "stop",
        summary = "Stop one background process.",
        mutating,
        shell,
        display = detailed,
        trim("process_id"),
        non_empty("process_id"),
        permission(paths = permission_stop, networks = permission_networks_stop)
    )]
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

    async fn permission_list(&self) -> SdkResult<Vec<PathRequest>> {
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

    async fn permission_networks_list(&self) -> SdkResult<Vec<NetworkRequest>> {
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
