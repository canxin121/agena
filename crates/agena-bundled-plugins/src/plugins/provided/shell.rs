//! `agena.shell` plugin: run shell commands and manage background processes.

use crate::part::{ShellCommandInput, ShellMonitorInput, ShellToolInput};
use crate::plugins::provided::router;
use agena_domain::ProcessShell;
use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const SHELL_PLUGIN_ID: &str = "agena.shell";

pub(crate) struct ShellPlugin;

pub(crate) fn new_plugin() -> ShellPlugin {
    ShellPlugin
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShellRunInput {
    #[serde(default)]
    shell: ProcessShell,
    #[serde(flatten)]
    #[input(flatten_shape)]
    command: ShellCommandInput,
    #[serde(default)]
    run_in_background: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    monitor: Option<ShellMonitorInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("process_id"), non_empty("process_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessLogsInput {
    process_id: String,
    #[serde(default)]
    since_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default)]
    wait_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("process_id"), non_empty("process_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessStopInput {
    process_id: String,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "shell",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Shell command execution and background process tools.",
)]
impl ShellPlugin {
    #[tool(
        tags(execute),
        summary = "Run one shell process.",
        help = "Run one shell process. Always pass the required `reads` and `writes` path arrays declaring every file or directory the command reads or modifies - empty arrays `[]` when the command touches only its executables (never list the executables). Pass the `network` array of outbound targets (host names, `host:port`, or URLs) the command may connect to - empty array `[]` when none. Set `run_in_background = true` to keep the process attached to the session; you will be notified when it completes — do not poll. Add `monitor` for success/failure regex or literal conditions, quiet-period completion, bounded capture, and timeout. Both modes return one `process_id` used by shell.list/logs/stop. Background launches return immediately; you will be notified with a `system_notification` when the process settles — do not poll shell.list/logs waiting for it.",
        mutating,
        shell,
        network(connects = run_network_targets(input)?)
    )]
    async fn invoke_run(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellRunInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "shell",
            json_input(ShellToolInput::Run {
                shell: args.shell,
                command: Box::new(args.command),
                run_in_background: args.run_in_background || args.monitor.is_some(),
                monitor: args.monitor,
            })?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(query, discovery),
        summary = "List active background processes.",
        read_only,
        shell,
        concurrency_safe
    )]
    async fn invoke_list(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "shell",
            json_input(ShellToolInput::List {})?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(query),
        summary = "Read background process logs.",
        read_only,
        shell,
        concurrency_safe
    )]
    async fn invoke_logs(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessLogsInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "shell",
            json_input(ShellToolInput::Logs {
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
        tags(mutate, execute),
        summary = "Stop one background process.",
        mutating,
        shell
    )]
    async fn invoke_stop(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessStopInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "shell",
            json_input(ShellToolInput::Stop {
                process_id: args.process_id,
            })?,
            context.session_id,
            context.call_id,
        )
    }
}

fn run_network_targets(args: &ShellRunInput) -> SdkResult<Vec<String>> {
    router::permission_network_targets_for(
        "shell",
        &json_input(ShellToolInput::Run {
            shell: args.shell,
            command: Box::new(args.command.clone()),
            run_in_background: args.run_in_background,
            monitor: args.monitor.clone(),
        })?,
    )
}

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params_error(&err))
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::Plugin;

    use super::ShellPlugin;

    #[test]
    fn manifest_exposes_shell_tools_under_the_shell_plugin() {
        let manifest = ShellPlugin.manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "shell");
        assert_eq!(tool_names, ["run", "list", "logs", "stop"]);
        let run = manifest
            .tools
            .iter()
            .find(|tool| tool.name == "run")
            .expect("shell.run manifest");
        let schema = serde_json::to_string(&run.input_schema()).expect("serialize schema");
        assert!(schema.contains("success_pattern"));
        assert!(schema.contains("failure_pattern"));
        assert!(schema.contains("quiet_period_ms"));

        let examples =
            agena_runtime_tools::tool::definition::schema_example_texts(&run.input_schema());
        let example: serde_json::Value =
            serde_json::from_str(examples.first().expect("shell.run generated example"))
                .expect("shell.run example must be JSON");
        assert!(example.get("reads").is_some());
        assert!(example.get("writes").is_some());
        assert!(example.get("network").is_some());
        assert!(
            example
                .pointer("/reads")
                .and_then(serde_json::Value::as_array)
                .is_some(),
            "reads declares read paths"
        );
        assert!(
            example
                .pointer("/writes")
                .and_then(serde_json::Value::as_array)
                .is_some(),
            "writes declares write paths"
        );
        assert!(
            example.pointer("/reads/0").is_some(),
            "generated example includes a read path"
        );
        assert_eq!(
            example.pointer("/network/0"),
            Some(&serde_json::json!("<target>"))
        );
    }
}
