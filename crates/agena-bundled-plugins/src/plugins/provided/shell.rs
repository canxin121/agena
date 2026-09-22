//! `agena.shell` plugin: run shell commands and manage background processes.

use crate::part::{
    ShellCommandInput, ShellMonitorInput, ShellSignal, ShellToolInput, ShellWriteInput,
};
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("process_id"),
    non_empty("process_id"),
    minimum("rows", 1),
    maximum("rows", 200),
    minimum("cols", 1),
    maximum("cols", 400)
)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessResizeInput {
    process_id: String,
    rows: u16,
    cols: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("process_id"), non_empty("process_id"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessSignalInput {
    process_id: String,
    signal: ShellSignal,
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
        help = "Run a shell command. Declare `reads`, `writes` and outbound `network` targets; use empty arrays when none. Set `tty=true` for an interactive CLI, REPL or full-screen terminal. This retains a PTY across tool calls and returns a process_id, incremental output, last_seq, and a current terminal screen. `yield_time_ms` (default 1000, maximum 30000) only controls this call's initial output wait: it never terminates the process. `timeout_ms`, when supplied, is the terminal's overall lifetime limit. Continue with shell.write; read without input with shell.write(chars=\"\") or shell.logs; use shell.resize for dimensions, shell.signal for interrupt/terminate/kill, and shell.stop for cleanup. Never assume a quiet prompt means completion. tty is incompatible with monitor. Without tty, normal foreground behavior is unchanged. `run_in_background=true` or `monitor` starts a non-interactive managed command; completion is notified by system_notification, so do not poll merely to wait for those jobs.",
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

    #[tool(tags(mutate, execute), summary = "Write to an interactive terminal and read its response.",
        help = "Continue a process started with shell.run(tty=true). chars is exact terminal input: never trim or automatically append a newline. Send \\r for Enter, \\u0003 for Ctrl-C, \\u0004 for Ctrl-D, \\t for Tab, or terminal escape sequences for arrow/function keys. Use chars=\"\" to read without sending input. Omit since_seq to read previously unread output; use an explicit last_seq to replay/page output. wait_ms defaults to 250 and is capped at 30000; a wait timeout does not kill the CLI. Input is an execution operation: declare every affected reads/writes path (relative to the Agena workspace) and network target, including effects of commands entered inside a shell/REPL. Requires the same owning session and workspace as the launch. A partial-write error requests terminal termination; do not resend the full input blindly. Process exit, not absence of output, indicates completion.",
        mutating, shell, network(connects = write_network_targets(input)?))]
    async fn invoke_write(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellWriteInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "shell",
            json_input(ShellToolInput::Write { input: args })?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate),
        summary = "Resize an interactive terminal.",
        mutating,
        shell
    )]
    async fn invoke_resize(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessResizeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "shell",
            json_input(ShellToolInput::Resize {
                process_id: args.process_id,
                rows: args.rows,
                cols: args.cols,
            })?,
            context.session_id,
            context.call_id,
        )
    }

    #[tool(
        tags(mutate, execute),
        summary = "Interrupt or terminate an interactive terminal.",
        help = "interrupt targets the current Unix foreground process group without closing the shell (ConPTY uses terminal Ctrl-C). terminate requests graceful session cleanup and then kills remaining jobs; kill skips the grace period. This is distinct from typing a control byte into a raw-mode program. The same owning session/workspace is required. shell.stop is equivalent to terminate.",
        mutating,
        shell
    )]
    async fn invoke_signal(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessSignalInput,
    ) -> SdkResult<ToolInvokeOutput> {
        router::invoke_tool(
            "shell",
            json_input(ShellToolInput::Signal {
                process_id: args.process_id,
                signal: args.signal,
            })?,
            context.session_id,
            context.call_id,
        )
    }
}

fn write_network_targets(args: &ShellWriteInput) -> SdkResult<Vec<String>> {
    router::permission_network_targets_for(
        "shell",
        &json_input(ShellToolInput::Write {
            input: args.clone(),
        })?,
    )
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
        assert_eq!(
            tool_names,
            ["run", "list", "logs", "stop", "write", "resize", "signal"]
        );
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
