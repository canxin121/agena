//! `agena.shell` plugin: bash, powershell, monitor.

use crate::message::{BashToolInput, MonitorToolInput, PowerShellToolInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::{HostCapability, PluginToolDecl, ToolStreamingMode, ToolTag};
use crate::plugins::provided::router::InProcessToolPlugin;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value as JsonValue;

pub(crate) const SHELL_PLUGIN_ID: &str = "agena.shell";

pub(crate) fn new_plugin() -> InProcessToolPlugin {
    InProcessToolPlugin::new_with_resolver(
        "agena-shell",
        "Shell command tools backed by the in-process executor bridge.",
        entries(),
        resolve_entry,
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum ShellToolInput {
    Bash(BashToolInput),
    #[serde(rename = "powershell")]
    PowerShell(PowerShellToolInput),
    Monitor(MonitorToolInput),
}

fn entries() -> Vec<PluginToolDecl> {
    vec![
        PluginToolDecl::new(
            "shell",
            crate::entry::definition::json_schema_for::<ShellToolInput>(),
        )
        .description(
            "Shell command dispatcher. Set command to bash, powershell, or monitor; pass that command's payload in args. Shell execution args must declare filesystem_effects for paths the command may read or write.",
        )
        .tags([ToolTag::Mutating, ToolTag::Shell])
        .concurrency_safe(false)
        .deferred_load()
        .streaming(ToolStreamingMode::Streaming)
        .host_capability(HostCapability::MonitorRegistry),
    ]
}

fn resolve_entry(entry: &str, input: JsonValue) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    if entry != "shell" {
        return Err(PluginError::invalid_params(format!(
            "unknown shell entry '{entry}'"
        )));
    }
    match serde_json::from_value::<ShellToolInput>(input)? {
        ShellToolInput::Bash(args) => tool_args("bash", args),
        ShellToolInput::PowerShell(args) => tool_args("powershell", args),
        ShellToolInput::Monitor(args) => tool_args("monitor", args),
    }
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
