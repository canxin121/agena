//! First-party `agena.shell` plugin: bash, powershell, monitor.

use crate::message::{BashToolInput, MonitorToolInput, PowerShellToolInput};
use crate::plugin::sdk::manifest::{InputPathSpec, PathKind};
use crate::plugin::sdk::{
    EntryBehavior as SdkEntryBehavior, EntryStreamingMode, HostCapability, PlanModePolicy,
    PluginEntryDecl,
};
use crate::plugins::bundled::router::FirstPartyRouterPlugin;

pub(crate) const SHELL_PLUGIN_ID: &str = "agena.shell";

pub(crate) fn new_plugin() -> FirstPartyRouterPlugin {
    FirstPartyRouterPlugin::new(
        "agena-shell",
        "Shell tools (bash, powershell, monitor) routed through the shared first-party executor bridge.",
        entries(),
    )
}

fn entries() -> Vec<PluginEntryDecl> {
    vec![
        PluginEntryDecl::new(
            "bash",
            crate::entry::definition::json_schema_for::<BashToolInput>(),
        )
        .description("Execute a shell command. The input must declare filesystem_effects for paths the command may read or write.")
        .behavior(SdkEntryBehavior::Mutating)
        .input_path(optional_path("$.workdir", PathKind::Read))
        .search_terms(["shell", "terminal", "command", "script"])
        .deferred_load()
        .plan_mode_policy(PlanModePolicy::ConditionalShellReadOnly),
        PluginEntryDecl::new(
            "powershell",
            crate::entry::definition::json_schema_for::<PowerShellToolInput>(),
        )
        .description("Execute a Windows PowerShell command. The input must declare filesystem_effects for paths the command may read or write.")
        .behavior(SdkEntryBehavior::Mutating)
        .input_path(optional_path("$.workdir", PathKind::Read))
        .search_terms(["windows", "powershell", "pwsh", "command"])
        .deferred_load()
        .plan_mode_policy(PlanModePolicy::ConditionalShellReadOnly),
        PluginEntryDecl::new(
            "monitor",
            crate::entry::definition::json_schema_for::<MonitorToolInput>(),
        )
        .description(
            "Run a long-lived shell command in the background and stream its stdout/stderr as numbered events. start inputs must declare filesystem_effects for paths the command may read or write. Actions: start (spawn), list (enumerate), read (pull events with optional blocking wait), stop (kill).",
        )
        .behavior(SdkEntryBehavior::Mutating)
        .input_path(optional_path("$.workdir", PathKind::Read))
        .search_terms([
            "monitor",
            "background process",
            "long running",
            "watch logs",
            "tail",
            "follow",
            "stream output",
        ])
        .deferred_load()
        .plan_mode_policy(PlanModePolicy::Allowed)
        .streaming(EntryStreamingMode::Streaming)
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
