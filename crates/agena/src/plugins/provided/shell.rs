//! `agena.shell` plugin: hierarchical exec and monitor commands.

use crate::message::{MonitorToolInput, ShellCommandInput};
use crate::plugin::sdk::{HostCapability, ToolTag};
use crate::plugins::provided::router::InProcessToolPlugin;
use agena_macros::{StaticToolSurface, ToolInputShape, ToolSuite};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const SHELL_PLUGIN_ID: &str = "agena.shell";

pub(crate) fn new_plugin() -> InProcessToolPlugin {
    InProcessToolPlugin::new_with_tool_suite::<ShellToolSuite>(
        SHELL_PLUGIN_ID,
        "Shell command tools backed by the in-process executor bridge.",
    )
    .detailed()
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "exec.bash",
    description = "Run one Bash shell command and declare filesystem_effects or network_effects for every accessed target.",
    summary = "Run one Bash shell command.",
    help = "Run one Bash command. Declare filesystem_effects and network_effects for all accessed paths or outbound targets; pass empty arrays when there are none beyond entering workdir.",
    display = detailed,
    tags(ToolTag::Mutating, ToolTag::Shell),
    concurrency_safe = false,
    streaming = "streaming"
)]
struct ShellExecBashToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: ShellCommandInput,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "exec.powershell",
    description = "Run one PowerShell command and declare filesystem_effects or network_effects for every accessed target.",
    summary = "Run one PowerShell command.",
    help = "Run one PowerShell command. Declare filesystem_effects and network_effects for all accessed paths or outbound targets; pass empty arrays when there are none beyond entering workdir.",
    display = detailed,
    tags(ToolTag::Mutating, ToolTag::Shell),
    concurrency_safe = false,
    streaming = "streaming"
)]
struct ShellExecPowershellToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: ShellCommandInput,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[tool_input(trim("include_pattern"), non_empty_if_present("include_pattern"))]
struct ShellMonitorStartInput {
    /// Nested shell command to run under the monitor.
    #[serde(flatten)]
    #[tool(flatten_shape)]
    command: ShellCommandInput,
    /// If true, keep the monitor alive until explicitly stopped or the session ends.
    #[serde(default)]
    persistent: bool,
    /// Optional regex filter that keeps only matching lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    include_pattern: Option<String>,
    /// Maximum buffered lines to retain before evicting older output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_buffered_lines: Option<u32>,
    /// Capture stderr lines as well as stdout. Defaults to true.
    #[serde(default = "default_capture_stderr")]
    capture_stderr: bool,
}

/// No additional input is required for listing monitors.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema, ToolInputShape)]
struct ShellMonitorListInput {}

#[derive(Debug, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[tool_input(trim("monitor_id"), non_empty("monitor_id"))]
struct ShellMonitorReadInput {
    /// Monitor id to inspect.
    monitor_id: String,
    /// Return only events with `seq > since_seq`. Defaults to 0.
    #[serde(default)]
    since_seq: u64,
    /// Maximum number of events to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    /// Wait this many milliseconds for new events when none are buffered.
    #[serde(default)]
    wait_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[tool_input(trim("monitor_id"), non_empty("monitor_id"))]
struct ShellMonitorStopInput {
    /// Monitor id to stop.
    monitor_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "monitor.start",
    description = "Start one long-running monitored shell command.",
    summary = "Start one monitored shell command.",
    help = "Launch a monitored shell command and return its monitor id. Declare filesystem_effects and network_effects for all accessed targets inside the nested shell command input.",
    display = detailed,
    tags(ToolTag::Mutating, ToolTag::Shell),
    host_capabilities(HostCapability::MonitorRegistry),
    concurrency_safe = false
)]
struct ShellMonitorStartToolInput {
    #[serde(flatten)]
    #[tool(flatten_shape)]
    args: ShellMonitorStartInput,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "monitor.list",
    description = "List active or recently finished monitored shell commands.",
    summary = "List monitored shell commands.",
    help = "Return the active or recently finished monitor summaries for this session.",
    display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Shell),
    host_capabilities(HostCapability::MonitorRegistry),
    concurrency_safe = true
)]
struct ShellMonitorListToolInput {
    #[serde(flatten)]
    #[tool(flatten_shape)]
    args: ShellMonitorListInput,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "monitor.read",
    description = "Read buffered output from one monitored shell command.",
    summary = "Read buffered monitor output.",
    help = "Tail buffered output from one monitor by monitor_id, optionally resuming from a sequence number and waiting for new lines.",
    display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Shell),
    host_capabilities(HostCapability::MonitorRegistry),
    concurrency_safe = true
)]
struct ShellMonitorReadToolInput {
    #[serde(flatten)]
    #[tool(flatten_shape)]
    args: ShellMonitorReadInput,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "monitor.stop",
    description = "Stop one monitored shell command by monitor_id.",
    summary = "Stop one monitored shell command.",
    help = "Terminate one running monitor by monitor_id.",
    display = detailed,
    tags(ToolTag::Mutating, ToolTag::Shell),
    host_capabilities(HostCapability::MonitorRegistry),
    concurrency_safe = false
)]
struct ShellMonitorStopToolInput {
    #[serde(flatten)]
    #[tool(flatten_shape)]
    args: ShellMonitorStopInput,
}

#[allow(dead_code)]
#[derive(Debug, ToolSuite)]
enum ShellToolSuite {
    #[tool(route = "bash", field = args, shape = ShellCommandInput)]
    ExecBash(ShellExecBashToolInput),
    #[tool(route = "powershell", field = args, shape = ShellCommandInput)]
    ExecPowershell(ShellExecPowershellToolInput),
    #[tool(route = "monitor", route_action = "start", field = args, shape = MonitorToolInput)]
    MonitorStart(ShellMonitorStartToolInput),
    #[tool(route = "monitor", route_action = "list", field = args, shape = MonitorToolInput)]
    MonitorList(ShellMonitorListToolInput),
    #[tool(route = "monitor", route_action = "read", field = args, shape = MonitorToolInput)]
    MonitorRead(ShellMonitorReadToolInput),
    #[tool(route = "monitor", route_action = "stop", field = args, shape = MonitorToolInput)]
    MonitorStop(ShellMonitorStopToolInput),
}

fn default_capture_stderr() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_suite_route_attributes_map_exec_and_monitor_variants() {
        let (bash_tool, bash_input) = ShellToolSuite::resolve_tool(
            "exec.bash",
            json!({
                "command": "echo ok",
                "workdir": "  crates/agena  ",
                "filesystem_effects": [],
                "network_effects": []
            }),
        )
        .expect("exec.bash should route to bash");
        assert_eq!(bash_tool, "bash");
        assert_eq!(
            bash_input,
            json!({
                "command": "echo ok",
                "description": "",
                "workdir": "crates/agena",
                "filesystem_effects": [],
                "network_effects": []
            })
        );

        let (monitor_list_tool, monitor_list_input) =
            ShellToolSuite::resolve_tool("monitor.list", json!({}))
                .expect("monitor.list should route to monitor");
        assert_eq!(monitor_list_tool, "monitor");
        assert_eq!(monitor_list_input, json!({ "action": "list" }));

        let (monitor_tool, monitor_input) = ShellToolSuite::resolve_tool(
            "monitor.read",
            json!({
                "monitor_id": "job-1",
                "since_seq": 3,
                "limit": 10,
                "wait_ms": 25
            }),
        )
        .expect("monitor.read should route to monitor");
        assert_eq!(monitor_tool, "monitor");
        assert_eq!(
            monitor_input,
            json!({
                "action": "read",
                "monitor_id": "job-1",
                "since_seq": 3,
                "limit": 10,
                "wait_ms": 25
            })
        );
    }

    #[test]
    fn monitor_inputs_reuse_nested_shape_rules_at_parse_time() {
        let start = ShellMonitorStartInput::parse_input(json!({
            "command": "  echo ok  ",
            "workdir": "  crates/agena  ",
            "filesystem_effects": [],
            "network_effects": [],
            "include_pattern": "  warn  ",
            "persistent": true,
            "capture_stderr": false
        }))
        .expect("monitor.start input should parse");
        assert_eq!(start.command.command, "echo ok");
        assert_eq!(start.command.workdir.as_deref(), Some("crates/agena"));
        assert_eq!(start.include_pattern.as_deref(), Some("warn"));
        assert!(start.persistent);
        assert!(!start.capture_stderr);

        let read = ShellMonitorReadInput::parse_input(json!({
            "monitor_id": "  job-1  ",
            "since_seq": 3,
            "limit": 10,
            "wait_ms": 25
        }))
        .expect("monitor.read input should parse");
        assert_eq!(read.monitor_id, "job-1");
        assert_eq!(read.since_seq, 3);
        assert_eq!(read.limit, Some(10));
        assert_eq!(read.wait_ms, 25);

        let stop = ShellMonitorStopInput::parse_input(json!({
            "monitor_id": "  job-1  "
        }))
        .expect("monitor.stop input should parse");
        assert_eq!(stop.monitor_id, "job-1");

        ShellMonitorListInput::parse_input(json!({})).expect("monitor.list input should parse");
    }

    #[test]
    fn monitor_schema_usage_renders_inner_flattened_docs() {
        let tool_decl = ShellMonitorStartToolInput::tool_decl();
        let usage = crate::tool::definition::schema_usage_text(&tool_decl.input_schema)
            .expect("usage text");
        assert!(usage.contains("Filesystem paths the command may read or write."));
        assert!(usage.contains("Outbound network targets the command may connect to."));
        assert!(usage.contains("Optional regex filter that keeps only matching lines."));
        assert!(usage.contains("Capture stderr lines as well as stdout. Defaults to true."));
    }
}
