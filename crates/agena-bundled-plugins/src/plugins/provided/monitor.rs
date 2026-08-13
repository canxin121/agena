//! `agena.monitor` plugin: continuous-stream background monitors (a shell
//! command or a WebSocket endpoint). Every event is projected as a
//! `system_notification` part (everything-is-a-part, §7.3) — no polling.

use crate::part::{MonitorToolInput, MonitorWsInput};
use crate::plugins::provided::router;
use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const MONITOR_PLUGIN_ID: &str = "agena.monitor";

pub(crate) struct MonitorPlugin;

pub(crate) fn new_plugin() -> MonitorPlugin {
    MonitorPlugin
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct MonitorStartInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ws: Option<MonitorWsInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(default)]
    persistent: bool,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("monitor_id"), non_empty("monitor_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct MonitorStopInput {
    monitor_id: String,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "monitor",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Continuous-stream background monitoring tools.",
)]
impl MonitorPlugin {
    #[tool(
        tags(execute),
        summary = "Start a continuous background monitor.",
        help = "Start a continuous background monitor. Pass exactly one of `command` (a long-running shell command, e.g. `tail -f`) or `ws` (a WebSocket endpoint; text frames become events). The monitor starts immediately and returns a `monitor_id`. You will be notified with a `system_notification` on each event — keep working, do not poll or sleep. Terminate it with `monitor.stop`, or it ends when the session does.",
        mutating,
        shell,
        network(connects = ws_url_targets(input)?)
    )]
    async fn invoke_start(
        &self,
        context: &ToolInvokeContext<'_>,
        args: MonitorStartInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "monitor",
            json_input(MonitorToolInput::Start {
                command: args.command,
                ws: args.ws,
                timeout_ms: args.timeout_ms,
                persistent: args.persistent,
                description: args.description,
            })?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate, execute),
        summary = "Stop one background monitor.",
        mutating,
        shell
    )]
    async fn invoke_stop(
        &self,
        context: &ToolInvokeContext<'_>,
        args: MonitorStopInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "monitor",
            json_input(MonitorToolInput::Stop {
                monitor_id: args.monitor_id,
            })?,
            context.session_id,
            context.call_id,
        )
    }
}

fn ws_url_targets(args: &MonitorStartInput) -> SdkResult<Vec<String>> {
    Ok(args
        .ws
        .as_ref()
        .map(|ws| vec![ws.url.clone()])
        .unwrap_or_default())
}

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params(err.to_string()))
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::Plugin;

    use super::MonitorPlugin;

    #[test]
    fn manifest_exposes_monitor_tools_under_the_monitor_plugin() {
        let manifest = MonitorPlugin.manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "monitor");
        assert_eq!(tool_names, ["start", "stop"]);
        let start = manifest
            .tools
            .iter()
            .find(|tool| tool.name == "start")
            .expect("monitor.start manifest");
        let schema = serde_json::to_string(&start.input_schema()).expect("serialize schema");
        assert!(schema.contains("command"));
        assert!(schema.contains("ws"));
        assert!(schema.contains("timeout_ms"));
    }
}
