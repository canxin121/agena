//! `agena.shell` plugin: exec and monitor subcommands.

use crate::message::{MonitorToolInput, ShellCommandInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    HostCapability, PluginToolDecl, Result as SdkResult, ToolDescriptionMode, ToolStreamingMode,
    ToolTag, UiTextDisplayMode,
};
use crate::plugins::provided::router::InProcessToolPlugin;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value as JsonValue;

pub(crate) const SHELL_PLUGIN_ID: &str = "agena.shell";

pub(crate) fn new_plugin() -> InProcessToolPlugin {
    InProcessToolPlugin::new_with_resolver(
        SHELL_PLUGIN_ID,
        "Shell command tools backed by the in-process executor bridge.",
        shell_tool_decls(),
        resolve_shell_tool_input,
    )
    .tool_description_mode(ToolDescriptionMode::Detailed)
    .ui_display_mode(UiTextDisplayMode::Detailed)
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

fn shell_tool_decls() -> Vec<PluginToolDecl> {
    vec![
        PluginToolDecl::new(
            "exec.bash",
            crate::tool::definition::json_schema_for::<ShellCommandInput>(),
        )
        .description(
            "Run one Bash command. Declare filesystem_effects and network_effects for every accessed path or outbound target.",
        )
        .summary("Run one Bash command.")
        .help(
            "Runs one Bash command through the in-process executor. Declare filesystem_effects and network_effects for all accessed paths or outbound targets; pass empty arrays when there are none beyond entering workdir.",
        )
        .ui_display_mode(UiTextDisplayMode::Detailed)
        .tags([ToolTag::Mutating, ToolTag::Shell])
        .concurrency_safe(false)
        .streaming(ToolStreamingMode::Streaming),
        PluginToolDecl::new(
            "exec.powershell",
            crate::tool::definition::json_schema_for::<ShellCommandInput>(),
        )
        .description(
            "Run one PowerShell command. Declare filesystem_effects and network_effects for every accessed path or outbound target.",
        )
        .summary("Run one PowerShell command.")
        .help(
            "Runs one PowerShell command through the in-process executor. Declare filesystem_effects and network_effects for all accessed paths or outbound targets; pass empty arrays when there are none beyond entering workdir.",
        )
        .ui_display_mode(UiTextDisplayMode::Detailed)
        .tags([ToolTag::Mutating, ToolTag::Shell])
        .concurrency_safe(false)
        .streaming(ToolStreamingMode::Streaming),
        PluginToolDecl::new(
            "monitor.start",
            crate::tool::definition::json_schema_for::<ShellMonitorStartInput>(),
        )
        .description("Start a long-running monitored shell command.")
        .summary("Start one monitored shell command.")
        .help(
            "Starts a monitored shell command with the declared filesystem_effects and network_effects. Use monitor.list, monitor.read, and monitor.stop to inspect or end it.",
        )
        .ui_display_mode(UiTextDisplayMode::Detailed)
        .tags([ToolTag::Mutating, ToolTag::Shell])
        .host_capability(HostCapability::MonitorRegistry)
        .concurrency_safe(false),
        PluginToolDecl::new(
            "monitor.list",
            crate::tool::definition::json_schema_for::<ShellMonitorListInput>(),
        )
        .description("List active shell monitors.")
        .summary("List active shell monitors.")
        .help("Lists the active shell monitors tracked by the runtime.")
        .ui_display_mode(UiTextDisplayMode::Detailed)
        .tags([ToolTag::ReadOnly, ToolTag::Shell])
        .host_capability(HostCapability::MonitorRegistry)
        .concurrency_safe(true),
        PluginToolDecl::new(
            "monitor.read",
            crate::tool::definition::json_schema_for::<ShellMonitorReadInput>(),
        )
        .description("Read buffered output from one shell monitor.")
        .summary("Read one shell monitor buffer.")
        .help(
            "Reads buffered output from one shell monitor by monitor_id. Use since_seq and wait_ms to tail incrementally.",
        )
        .ui_display_mode(UiTextDisplayMode::Detailed)
        .tags([ToolTag::ReadOnly, ToolTag::Shell])
        .host_capability(HostCapability::MonitorRegistry)
        .concurrency_safe(true),
        PluginToolDecl::new(
            "monitor.stop",
            crate::tool::definition::json_schema_for::<ShellMonitorStopInput>(),
        )
        .description("Stop one shell monitor by id.")
        .summary("Stop one shell monitor.")
        .help("Stops one shell monitor by monitor_id and returns its final handle state.")
        .ui_display_mode(UiTextDisplayMode::Detailed)
        .tags([ToolTag::Mutating, ToolTag::Shell])
        .host_capability(HostCapability::MonitorRegistry)
        .concurrency_safe(false),
    ]
}

fn parse_input<T>(input: JsonValue) -> SdkResult<T>
where
    T: for<'de> serde::Deserialize<'de>,
{
    serde_json::from_value(input).map_err(|err| PluginError::invalid_params(err.to_string()))
}

fn tool_args<T: serde::Serialize>(tool: &str, args: T) -> SdkResult<(String, JsonValue)> {
    Ok((
        tool.to_string(),
        serde_json::to_value(args).map_err(|err| PluginError::invalid_params(err.to_string()))?,
    ))
}

fn resolve_shell_tool_input(tool_name: &str, input: JsonValue) -> SdkResult<(String, JsonValue)> {
    match tool_name {
        "exec.bash" => tool_args("bash", parse_input::<ShellCommandInput>(input)?),
        "exec.powershell" => tool_args("powershell", parse_input::<ShellCommandInput>(input)?),
        "monitor.start" => {
            let args: ShellMonitorStartInput = parse_input(input)?;
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
        "monitor.list" => tool_args("monitor", MonitorToolInput::List {}),
        "monitor.read" => {
            let args: ShellMonitorReadInput = parse_input(input)?;
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
        "monitor.stop" => {
            let args: ShellMonitorStopInput = parse_input(input)?;
            tool_args(
                "monitor",
                MonitorToolInput::Stop {
                    monitor_id: args.monitor_id,
                },
            )
        }
        other => Err(PluginError::invalid_params(format!(
            "unknown shell tool '{other}'"
        ))),
    }
}
