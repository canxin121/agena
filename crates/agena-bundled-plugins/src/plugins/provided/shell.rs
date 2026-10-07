//! `agena.shell`: foreground, background, watched and interactive execution.

use crate::part::{
    ShellLaunchInput, ShellOpenInput, ShellReadInput, ShellSignal, ShellToolInput, ShellWatchInput,
    ShellWriteInput,
};
use crate::plugins::provided::router;
use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput, ToolTag};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const SHELL_PLUGIN_ID: &str = "agena.shell";

pub(crate) struct ShellPlugin;

pub(crate) fn new_plugin() -> ShellPlugin {
    ShellPlugin
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("process_id"),
    non_empty("process_id"),
    minimum("limit", 1),
    maximum("limit", 2000),
    maximum("wait_ms", 30000),
    minimum("max_output_bytes", 1024),
    maximum("max_output_bytes", 16384)
)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessLogsInput {
    process_id: String,
    /// Return events after this cursor; continue with the returned last_seq.
    #[serde(default)]
    since_seq: u64,
    #[serde(default)]
    event_offset: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_output_bytes: Option<u32>,
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
    summary = "Shell commands: exec waits, spawn runs in background, watch monitors output events, open provides an interactive terminal/PTY; shared logs and lifecycle controls manage them.",
)]
impl ShellPlugin {
    #[tool(
        tags(execute, shell, mutate),
        summary = "Execute a non-interactive shell command and wait for its final output and exit result.",
        help = "Run a command to completion. Declare actual reads/writes paths and network targets; use empty arrays when none. timeout_ms limits lifetime (default 120000); timeout/cancellation cleans up the owned process tree. max_output_bytes sets the output budget (1024–16384, default 16384), accounting for escaping and reserving lifecycle metadata; previews keep beginning/end and at most 200 lines. Capture is independent of preview: output_archive.path is the stable startup prefix, and output_archive.segments lists all retained files with original captured byte ranges. Search each needed file with fs.grep or read selected lines/byte ranges with fs.read. Segment-local byte offsets start at 0; gaps are unavailable output. Retention is 16 MiB per process (4 MiB startup plus up to 12 MiB recent output), 256 MiB per workspace. Check pending/complete/truncated/error. Use shell.spawn for background work, shell.watch for readiness/selected notifications, and shell.open for interactive input."
    )]
    async fn invoke_exec(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellLaunchInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Exec {
                shell: args.shell,
                command: Box::new(args.command),
            },
            context,
        )
    }

    #[tool(
        tags(execute, shell, ToolTag::Custom("background".to_owned()), mutate),
        summary = "Spawn a non-interactive shell command in the background; return immediately so the AI can continue, then notify on completion.",
        help = "Launch with closed stdin; return process_id only after actual startup and registration. Continue useful work immediately. Completion, failure, timeout or stop is delivered once as system_notification; do not poll merely to wait. Use shell.read for bounded diagnostic output, shell.list for state and shell.stop for cleanup. Omitted timeout_ms permits running until exit/stop; a supplied timeout is enforced. Declare reads/writes/network effects. Attach or replace a watch on this same process with shell.watch(process_id, policy); omit/null policy to remove it. A process-targeted watch never spawns a second command or a second background operation. Use shell.open when later interactive input is required."
    )]
    async fn invoke_spawn(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellLaunchInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Spawn {
                shell: args.shell,
                command: Box::new(args.command),
            },
            context,
        )
    }

    #[tool(
        tags(execute, shell, ToolTag::Custom("background".to_owned()), ToolTag::Custom("monitor".to_owned()), ToolTag::Custom("watch".to_owned()), mutate),
        summary = "Start a command with readiness/selected output notifications, or attach, replace or remove the watch on an existing background shell process.",
        help = "Provide either command with shell/effects (new launch) or process_id (existing noninteractive process); never both. Launch binds the watch before the process can output. A process target replaces its single policy and keeps the original launch/completion operation. Omit or pass null policy on a process target to remove the watch without stopping the process; an empty policy disables ordinary notifications. Attach observes future output by default; since_seq optionally scans retained raw logs once. Identical policy updates are idempotent. ready_pattern notifies once and leaves the service running. include_pattern is opt-in; omission sends no ordinary output notifications. notifications defaults to once; on_change deduplicates unchanged matches and coalesces changed matches to the latest, using notification_interval_ms (default 30000, min 1000). Readiness/completion bypass that throttle. success_pattern/failure_pattern stop the process tree; failure wins. pattern_kind applies to all patterns (regex default, literal available); preserve whitespace. quiet_period_ms stops successfully after no raw stdout/stderr activity. timeout_ms belongs only to the launch and remains enforced after watch changes/removal. Logs/archives always retain excluded output. Read with shell.read and stop with shell.stop; do not poll merely to wait. PTYs and WebSocket subscriptions cannot receive this watch."
    )]
    async fn invoke_watch(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellWatchInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Watch {
                input: Box::new(args),
            },
            context,
        )
    }

    #[tool(
        tags(execute, shell, ToolTag::Custom("terminal".to_owned()), interactive, ToolTag::Custom("tty".to_owned()), ToolTag::Custom("pty".to_owned()), mutate),
        summary = "Open a persistent interactive shell terminal/PTY for a CLI, REPL or full-screen program; continue with shell.read and shell.write.",
        help = "Return process_id, output, last_seq, next_event_offset and state after confirmed startup. yield_time_ms (default 1000, max 30000) limits only the initial wait, never lifetime; timeout_ms is an optional lifetime deadline. Interactive exit does not send background completion notifications or wake the AI. max_output_bytes (1024–16384, default 16384) controls preview, not capture. With include_screen the available content budget is split between raw output and a bounded screen. Silence/prompt/yield is not exit. Continue with shell.read or exact-input shell.write; shell.signal interrupts/terminates/kills, shell.resize changes dimensions and shell.stop cleans up. Declare initial effects and subsequent write effects."
    )]
    async fn invoke_open(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellOpenInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Open {
                input: Box::new(args),
            },
            context,
        )
    }

    #[tool(
        tags(query, discovery, shell, read_only),
        summary = "List this session's background shell jobs, watched commands and interactive terminals, including their type and state."
    )]
    async fn invoke_list(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        invoke(ShellToolInput::List {}, context)
    }

    #[tool(
        tags(query, shell, ToolTag::Custom("background".to_owned()), read_only),
        summary = "Read bounded output with an explicit replay cursor; shell.read is the unified, automatically consuming entry point.",
        help = "Compatibility replay entry for shell process output. since_seq defaults to 0; continue with last_seq AND next_event_offset as event_offset whenever nonzero. last_seq refers to fully consumed events; partially returned event text is resumable without loss. max_output_bytes (1024–16384, default 16384) includes escaping and event structure, with room reserved for lifecycle/cursor metadata. limit caps event count; wait_ms is a diagnostic wait (max 30000). has_more describes buffered output, not archive completeness. output_archive.path is the startup prefix; segments identify retained recent files and original byte ranges. Use fs.grep/fs.read selected ranges in those files; file-local byte offsets start at 0. Use shell.read without since_seq for new unread output. Background jobs notify when they finish; do not poll to wait."
    )]
    async fn invoke_logs(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessLogsInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Logs {
                process_id: args.process_id,
                since_seq: args.since_seq,
                event_offset: args.event_offset,
                max_output_bytes: args.max_output_bytes,
                limit: args.limit,
                wait_ms: args.wait_ms,
            },
            context,
        )
    }

    #[tool(
        tags(query, shell, ToolTag::Custom("terminal".to_owned()), interactive, ToolTag::Custom("tty".to_owned()), ToolTag::Custom("pty".to_owned()), read_only),
        summary = "Read bounded incremental output and state from any owned background shell job or interactive terminal, without input.",
        help = "Works with process_id from shell.spawn, shell.watch or shell.open. Omit since_seq and event_offset to consume unread output; explicit since_seq replays without changing the automatic cursor. A partial event returns last_seq for fully consumed events and next_event_offset for the next event; pass both as since_seq/event_offset to resume. A nonzero event_offset requires since_seq and must preserve UTF-8 boundaries. Reads/writes on one process serialize automatic consumption; cancellation leaves it alive. wait_ms defaults to 250 (max 30000) and is a bounded output wait, not completion. max_output_bytes is 1024–16384 (default 16384), counting escaping/event structure and reserving metadata. include_screen is for PTYs only; enabling it splits the content budget with a bounded screen. ready is independent of terminal status. has_more describes the rolling buffer. For evicted text inspect output_archive.segments and use fs.grep/fs.read on only needed files/ranges; original byte ranges may have gaps and file-local offsets start at 0. Background completion is notified once, so do not poll merely to wait."
    )]
    async fn invoke_read(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellReadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(ShellToolInput::Read { input: args }, context)
    }

    #[tool(
        tags(mutate, execute, shell, ToolTag::Custom("terminal".to_owned()), interactive, ToolTag::Custom("tty".to_owned()), ToolTag::Custom("pty".to_owned())),
        summary = "Send exact nonempty input to an interactive shell.open terminal and collect a bounded, resumable response.",
        help = "chars is exact terminal input: never trim or append a newline. Send \\r for Enter, \\u0003 for Ctrl-C, \\u0004 for Ctrl-D, \\t for Tab, or terminal escape sequences. Use shell.read without input. Omit since_seq/event_offset for unread output; explicit cursors replay without changing automatic consumption. Continue a partial result with last_seq and next_event_offset as since_seq/event_offset. wait_ms defaults to 250 (max 30000), independent of lifetime. max_output_bytes is 1024–16384 (default 16384); include_screen splits that content budget. Cursor/effect validation occurs before sending input. Declare reads/writes/network effects of the entered operation. Requires the owning session/workspace. A partial-write error requests termination; never blindly resend the full input. Use shell.signal for out-of-band interruption and shell.stop for cleanup."
    )]
    async fn invoke_write(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ShellWriteInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(ShellToolInput::Write { input: args }, context)
    }

    #[tool(
        tags(mutate, execute, shell),
        summary = "Stop an owned background shell job, watched command or interactive terminal and clean up its process tree."
    )]
    async fn invoke_stop(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessStopInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Stop {
                process_id: args.process_id,
            },
            context,
        )
    }

    #[tool(tags(mutate, shell, ToolTag::Custom("terminal".to_owned()), ToolTag::Custom("tty".to_owned()), ToolTag::Custom("pty".to_owned())), summary = "Resize an interactive shell.open terminal in character rows and columns.")]
    async fn invoke_resize(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessResizeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Resize {
                process_id: args.process_id,
                rows: args.rows,
                cols: args.cols,
            },
            context,
        )
    }

    #[tool(
        tags(mutate, execute, shell, ToolTag::Custom("terminal".to_owned()), interactive, ToolTag::Custom("tty".to_owned()), ToolTag::Custom("pty".to_owned())),
        summary = "Interrupt, gracefully terminate or kill an owned interactive shell terminal.",
        help = "interrupt targets the Unix foreground process group without closing the shell; ConPTY uses terminal Ctrl-C. terminate requests graceful cleanup then kills remaining jobs; kill skips the grace period. Out-of-band interruption also works for raw-mode programs. Requires the owning session/workspace. shell.stop is equivalent to terminate."
    )]
    async fn invoke_signal(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ProcessSignalInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke(
            ShellToolInput::Signal {
                process_id: args.process_id,
                signal: args.signal,
            },
            context,
        )
    }
}

fn invoke(input: ShellToolInput, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
    router::invoke_tool(
        "shell",
        json_input(input)?,
        context.session_id,
        context.call_id,
    )
}

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params_error(&err))
}

#[cfg(test)]
mod tests {
    use super::ShellPlugin;
    use agena_plugin_host::sdk::Plugin;

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
            [
                "exec", "spawn", "watch", "open", "list", "logs", "read", "write", "stop",
                "resize", "signal"
            ]
        );
        for name in ["exec", "spawn", "watch", "open"] {
            let tool = manifest
                .tools
                .iter()
                .find(|tool| tool.name == name)
                .expect("shell launch manifest");
            let schema = serde_json::to_string(&tool.input_schema()).expect("serialize schema");
            assert!(!schema.contains("run_in_background"));
            assert!(!schema.contains("\"tty\""));
            let examples =
                agena_runtime_tools::tool::definition::schema_example_texts(&tool.input_schema());
            let example: serde_json::Value =
                serde_json::from_str(examples.first().expect("shell generated example"))
                    .expect("shell example must be JSON");
            assert!(example.get("reads").is_some());
            assert!(example.get("writes").is_some());
            assert!(example.get("network").is_some());
        }
    }
}
