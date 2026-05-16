//! `agena.shell` plugin: bash, powershell, monitor.

use crate::message::{BashToolInput, MonitorToolInput, PowerShellToolInput};
use crate::plugin::sdk::manifest::{InputPathSpec, PathKind};
use crate::plugin::sdk::{HostCapability, PluginToolDecl, ToolStreamingMode, ToolTag};
use crate::plugins::provided::router::InProcessToolPlugin;

pub(crate) const SHELL_PLUGIN_ID: &str = "agena.shell";

pub(crate) fn new_plugin() -> InProcessToolPlugin {
    InProcessToolPlugin::new(
        "agena-shell",
        "Shell tools (bash, powershell, monitor) backed by the in-process executor bridge.",
        entries(),
    )
}

fn entries() -> Vec<PluginToolDecl> {
    vec![
        PluginToolDecl::new(
            "bash",
            crate::entry::definition::json_schema_for::<BashToolInput>(),
        )
        .description("Execute a shell command. The input must declare filesystem_effects for paths the command may read or write.")
        .tags([ToolTag::Mutating, ToolTag::Shell])
        .input_path(optional_path("$.workdir", PathKind::Read))
        .concurrency_safe(false)
        .deferred_load(),
        PluginToolDecl::new(
            "powershell",
            crate::entry::definition::json_schema_for::<PowerShellToolInput>(),
        )
        .description("Execute a Windows PowerShell command. The input must declare filesystem_effects for paths the command may read or write.")
        .tags([ToolTag::Mutating, ToolTag::Shell])
        .input_path(optional_path("$.workdir", PathKind::Read))
        .concurrency_safe(false)
        .deferred_load(),
        PluginToolDecl::new(
            "monitor",
            crate::entry::definition::json_schema_for::<MonitorToolInput>(),
        )
        .description(
            "Run a long-lived shell command in the background and stream its stdout/stderr as numbered events. start inputs must declare filesystem_effects for paths the command may read or write. Actions: start (spawn), list (enumerate), read (pull events with optional blocking wait), stop (kill).",
        )
        .tags([ToolTag::Mutating, ToolTag::Shell])
        .input_path(optional_path("$.workdir", PathKind::Read))
        .concurrency_safe(false)
        .deferred_load()
        .streaming(ToolStreamingMode::Streaming)
        .host_capability(HostCapability::MonitorRegistry),
    ]
}

fn optional_path(jsonpath: &str, kind: PathKind) -> InputPathSpec {
    InputPathSpec {
        jsonpath: jsonpath.to_string(),
        kind,
        optional: true,
    }
}
