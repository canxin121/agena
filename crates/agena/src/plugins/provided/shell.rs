//! `agena.shell` plugin: exec, monitor.

use crate::message::{MonitorToolInput, ShellCommandInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::{HostCapability, ToolTag};
use crate::plugins::provided::router::InProcessToolPlugin;
use agena_macros::StaticToolSurface;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value as JsonValue;

pub(crate) const SHELL_PLUGIN_ID: &str = "agena.shell";

pub(crate) fn new_plugin() -> InProcessToolPlugin {
    InProcessToolPlugin::new_with_resolver(
        SHELL_PLUGIN_ID,
        "Shell command tools backed by the in-process executor bridge.",
        vec![ShellToolInput::tool_decl()],
        ShellToolInput::resolve_tool,
    )
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "shell",
    description = "Shell command dispatcher. Set action to exec, monitor_start, monitor_list, monitor_read, or monitor_stop. Exec payloads must declare `shell = bash|powershell` plus filesystem_effects and network_effects for every path or network target the command may access.",
    summary = "Run shell, PowerShell, or monitor commands.",
    help = "Use action `exec` for one-shot shell commands, with `shell = bash|powershell`. Use `monitor_start`, `monitor_list`, `monitor_read`, and `monitor_stop` for long-running processes. Shell execution payloads must declare `filesystem_effects` and `network_effects` for any paths or outbound targets the command may access; pass empty arrays when there is no filesystem or network effect beyond entering `workdir`.",
    tags(ToolTag::Mutating, ToolTag::Shell),
    host_capabilities(HostCapability::MonitorRegistry),
    concurrency_safe = false,
    streaming = "streaming"
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ShellToolInput {
    #[tool(map = shell_exec_tool_args)]
    Exec {
        #[serde(flatten)]
        args: ShellExecInput,
    },
    #[tool(map = shell_monitor_start_tool_args)]
    MonitorStart {
        #[serde(flatten)]
        args: ShellMonitorStartInput,
    },
    #[tool(map = shell_monitor_list_tool_args)]
    MonitorList {
        #[serde(flatten)]
        args: ShellMonitorListInput,
    },
    #[tool(map = shell_monitor_read_tool_args)]
    MonitorRead {
        #[serde(flatten)]
        args: ShellMonitorReadInput,
    },
    #[tool(map = shell_monitor_stop_tool_args)]
    MonitorStop {
        #[serde(flatten)]
        args: ShellMonitorStopInput,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ShellExecKind {
    Bash,
    #[serde(rename = "powershell")]
    PowerShell,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ShellExecInput {
    shell: ShellExecKind,
    #[serde(flatten)]
    command: ShellCommandInput,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ShellMonitorStartInput {
    #[serde(flatten)]
    command: ShellCommandInput,
    #[serde(default)]
    persistent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    include_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_buffered_lines: Option<u32>,
    #[serde(default = "default_capture_stderr")]
    capture_stderr: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ShellMonitorListInput {}

#[derive(Debug, Deserialize, JsonSchema)]
struct ShellMonitorReadInput {
    monitor_id: String,
    #[serde(default)]
    since_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default)]
    wait_ms: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ShellMonitorStopInput {
    monitor_id: String,
}

fn default_capture_stderr() -> bool {
    true
}

fn tool_args<T: serde::Serialize>(
    tool: &str,
    args: T,
) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    Ok((
        tool.to_string(),
        serde_json::to_value(args).map_err(|err| PluginError::invalid_params(err.to_string()))?,
    ))
}

fn shell_exec_tool_args(input: ShellExecInput) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    match input.shell {
        ShellExecKind::Bash => tool_args("bash", input.command),
        ShellExecKind::PowerShell => tool_args("powershell", input.command),
    }
}

fn shell_monitor_start_tool_args(
    args: ShellMonitorStartInput,
) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    tool_args(
        "monitor",
        MonitorToolInput::Start {
            command: args.command,
            persistent: args.persistent,
            include_pattern: args.include_pattern,
            max_buffered_lines: args.max_buffered_lines,
            capture_stderr: args.capture_stderr,
        },
    )
}

fn shell_monitor_list_tool_args(
    _args: ShellMonitorListInput,
) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    tool_args("monitor", MonitorToolInput::List {})
}

fn shell_monitor_read_tool_args(
    args: ShellMonitorReadInput,
) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    tool_args(
        "monitor",
        MonitorToolInput::Read {
            monitor_id: args.monitor_id,
            since_seq: args.since_seq,
            limit: args.limit,
            wait_ms: args.wait_ms,
        },
    )
}

fn shell_monitor_stop_tool_args(
    args: ShellMonitorStopInput,
) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    tool_args(
        "monitor",
        MonitorToolInput::Stop {
            monitor_id: args.monitor_id,
        },
    )
}
