//! `agena.monitor`: WebSocket event subscriptions. Command-output listeners
//! belong to agena.shell.watch and share Shell process management.

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
#[input(
    trim("description"),
    minimum("timeout_ms", 1),
    maximum("timeout_ms", 3600000)
)]
#[serde(deny_unknown_fields)]
pub(crate) struct MonitorStartInput {
    #[input(nested_shape)]
    ws: MonitorWsInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
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
    summary = "WebSocket event subscriptions with background notifications. Use shell.watch for local command-output monitoring.",
)]
impl MonitorPlugin {
    #[tool(
        tags(execute, network, mutate),
        summary = "Subscribe to WebSocket text events and notify the AI in bounded batches.",
        help = "Start a background WebSocket subscription using ws.url and optional protocols. This tool does not execute local commands; use shell.watch for command-output events. The endpoint is checked as a network effect. Return monitor_id and continue working; text events arrive as bounded system_notification batches at most once per second, followed by a final notification when the subscription ends. Do not poll or sleep to wait. timeout_ms defaults to 300000 and is enforced across connection establishment and the feed lifetime (maximum 3600000). The subscription also ends on disconnect, explicit stop, cancellation or session end. Use monitor.stop for cleanup."
    )]
    async fn invoke_start(
        &self,
        context: &ToolInvokeContext<'_>,
        args: MonitorStartInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "monitor",
            json_input(MonitorToolInput::Start {
                ws: args.ws,
                timeout_ms: args.timeout_ms,
                description: args.description,
            })?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(tags(mutate, network), summary = "Stop one WebSocket subscription.")]
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

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params_error(&err))
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
        assert!(!schema.contains("\"command\""));
        assert!(schema.contains("ws"));
        assert!(schema.contains("timeout_ms"));
    }
}
