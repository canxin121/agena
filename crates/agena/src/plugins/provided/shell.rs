//! `agena.shell` plugin: exec, monitor.

use crate::message::{BashToolInput, FilesystemEffect, MonitorToolInput, PowerShellToolInput};
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
        "agena-shell",
        "Shell command tools backed by the in-process executor bridge.",
        vec![ShellToolInput::tool_decl()],
        ShellToolInput::resolve_entry,
    )
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "shell",
    description = "Shell command dispatcher. Set action to exec, monitor_start, monitor_list, monitor_read, or monitor_stop. Exec payloads must declare `shell = bash|powershell` plus filesystem_effects for paths the command may read or write.",
    summary = "Run shell, PowerShell, or monitor commands.",
    help = "Use action `exec` for one-shot shell commands, with `shell = bash|powershell`. Use `monitor_start`, `monitor_list`, `monitor_read`, and `monitor_stop` for long-running processes. Shell execution payloads must declare `filesystem_effects` for any paths the command may read or write.",
    tags(ToolTag::Mutating, ToolTag::Shell),
    host_capabilities(HostCapability::MonitorRegistry),
    concurrency_safe = false,
    load = "deferred",
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
    command: String,
    #[serde(default)]
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workdir: Option<String>,
    filesystem_effects: Vec<FilesystemEffect>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ShellMonitorStartInput {
    command: String,
    #[serde(default)]
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workdir: Option<String>,
    filesystem_effects: Vec<FilesystemEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
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
        ShellExecKind::Bash => tool_args(
            "bash",
            BashToolInput {
                command: input.command,
                description: input.description,
                timeout_ms: input.timeout_ms,
                workdir: input.workdir,
                filesystem_effects: input.filesystem_effects,
            },
        ),
        ShellExecKind::PowerShell => tool_args(
            "powershell",
            PowerShellToolInput {
                command: input.command,
                description: input.description,
                timeout_ms: input.timeout_ms,
                workdir: input.workdir,
                filesystem_effects: input.filesystem_effects,
            },
        ),
    }
}

fn shell_monitor_start_tool_args(
    args: ShellMonitorStartInput,
) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    tool_args(
        "monitor",
        MonitorToolInput::Start {
            command: args.command,
            description: args.description,
            workdir: args.workdir,
            filesystem_effects: args.filesystem_effects,
            timeout_ms: args.timeout_ms,
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn resolve_exec_bash_to_underlying_bash() {
        let (tool, input) = ShellToolInput::resolve_entry(
            "shell",
            json!({
                "action": "exec",
                "shell": "bash",
                "command": "pwd",
                "description": "print cwd",
                "timeout_ms": 1000,
                "workdir": "repo",
                "filesystem_effects": [{"path": ".", "access": "read"}]
            }),
        )
        .expect("shell exec should resolve");

        assert_eq!(tool, "bash");
        assert_eq!(input["command"], "pwd");
        assert_eq!(input["workdir"], "repo");
    }

    #[test]
    fn resolve_exec_powershell_to_underlying_powershell() {
        let (tool, input) = ShellToolInput::resolve_entry(
            "shell",
            json!({
                "action": "exec",
                "shell": "powershell",
                "command": "Get-Location",
                "description": "print cwd",
                "filesystem_effects": [{"path": ".", "access": "read"}]
            }),
        )
        .expect("shell exec should resolve");

        assert_eq!(tool, "powershell");
        assert_eq!(input["command"], "Get-Location");
    }
}
