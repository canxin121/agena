//! Shell-tool adapter for persistent PTYs. Authorization remains in the tool
//! executor; live-process ownership is additionally checked here for every action.

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};
use crate::part::{ShellOpenInput, ShellSignal, ShellToolInput};
use crate::{TerminalOwner, TerminalRead, TerminalStartParams};
use agena_domain::{FilesystemEffects, ProcessShell};
use std::{collections::HashMap, path::PathBuf};
use tokio_util::sync::CancellationToken;

/// Cancelling or dropping this future also cancels the still-running blocking
/// operation. A successful return disarms cancellation so a yielded PTY lives
/// across later turns rather than inheriting the launch turn's cancellation.
pub(super) async fn blocking<T: Send + 'static>(
    executor: &ToolExecutor,
    operation: impl FnOnce(CancellationToken) -> Result<T, ToolError> + Send + 'static,
) -> Result<T, ToolError> {
    executor.ensure_not_cancelled()?;
    let cancel = CancellationToken::new();
    let guard = cancel.clone().drop_guard();
    let worker = tokio::task::spawn_blocking(move || operation(cancel));
    let cancelled = async {
        match executor.cancellation_token() {
            Some(token) => token.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    let result = tokio::select! {
        biased;
        _ = cancelled => Err(ToolError::Cancelled),
        result = worker => result.map_err(|error| ToolError::plugin(
            format!("terminal worker failed: {error}")
        ))?,
    };
    executor.ensure_not_cancelled()?;
    if result.is_ok() {
        guard.disarm();
    }
    result
}

pub(super) fn owner(
    executor: &ToolExecutor,
    session_id: Option<i64>,
) -> Result<TerminalOwner, ToolError> {
    let workspace = std::fs::canonicalize(executor.workspace_root()).map_err(|error| {
        ToolError::invalid_input(format!("resolve terminal workspace: {error}"))
    })?;
    Ok(TerminalOwner {
        workspace,
        session_id,
    })
}

pub(super) async fn owner_async(
    executor: &ToolExecutor,
    session_id: Option<i64>,
) -> Result<TerminalOwner, ToolError> {
    let workspace = tokio::fs::canonicalize(executor.workspace_root())
        .await
        .map_err(|error| {
            ToolError::invalid_input(format!("resolve terminal workspace: {error}"))
        })?;
    Ok(TerminalOwner {
        workspace,
        session_id,
    })
}

pub(super) fn registry(executor: &ToolExecutor) -> Result<&crate::TerminalRegistry, ToolError> {
    executor
        .monitor_registry()
        .and_then(|registry| registry.terminals())
        .ok_or_else(|| {
            ToolError::invalid_input("interactive terminals are unavailable in this runtime")
        })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_prepared(
    executor: &ToolExecutor,
    shell: ProcessShell,
    input: &ShellOpenInput,
    command: String,
    workdir: PathBuf,
    env: HashMap<String, String>,
    launch: Option<&agena_tool::shell::ShellLaunchSpec>,
    process_id: Option<String>,
    owner: TerminalOwner,
    output: Option<agena_storage::content::ContentWriter>,
    cancel: CancellationToken,
) -> Result<ToolPayloadExecution, ToolError> {
    let mut env = super::shell::sanitize_env(&env);
    if env
        .get("TERM")
        .is_none_or(|value| value.is_empty() || value == "dumb")
    {
        env.insert("TERM".into(), "xterm-256color".into());
    }
    // Dimensions come from the PTY ioctl; inherited shell overrides become
    // stale after resize and would make tools ignore the kernel's size.
    env.remove("COLUMNS");
    env.remove("LINES");
    let argv = crate::shell_sandbox::protect(
        executor,
        match launch {
            Some(spec) => spec.argv(command.as_str()),
            None if shell == ProcessShell::Powershell => {
                let mut argv = agena_tool::shell::powershell_command_for_windows(&command);
                if let Some(index) = argv.iter().position(|arg| arg == "-NonInteractive") {
                    argv.remove(index);
                }
                argv
            }
            None => agena_tool::shell::shell_command_for_platform(&command),
        },
        &input.command,
        &mut env,
    )?;
    let read = registry(executor)?
        .start_cancellable_with_output(
            TerminalStartParams {
                output,
                process_id,
                owner,
                command: argv,
                display_command: command,
                description: input.command.description.clone(),
                workdir,
                env,
                rows: input.rows,
                cols: input.cols,
                timeout_ms: input.command.timeout_ms,
            },
            input.yield_time_ms,
            input.command.max_output_bytes,
            input.include_screen,
            cancel,
        )
        .map_err(error)?;
    let mut execution = render("open", read, input.include_screen);
    if let Some(spec) = launch {
        for (key, value) in spec.metadata() {
            execution.view.metadata.insert(key, value);
        }
    }
    Ok(execution)
}

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &ShellToolInput,
    session_id: Option<i64>,
    cancel: &CancellationToken,
) -> Result<ToolPayloadExecution, ToolError> {
    let owner = owner(executor, session_id)?;
    let terminals = registry(executor)?;
    let (action, read, include_screen) = match input {
        ShellToolInput::Read { input } => (
            "read",
            terminals.read_with_output(
                &input.process_id,
                &owner,
                crate::ProcessReadOptions {
                    since_seq: input.since_seq,
                    event_offset: input.event_offset,
                    wait_ms: input.wait_ms,
                    limit: input.limit,
                    max_output_bytes: input.max_output_bytes,
                },
                input.include_screen,
                cancel,
            ),
            input.include_screen,
        ),
        ShellToolInput::Write { input } => {
            if input.chars.is_empty() {
                return Err(ToolError::invalid_input(
                    "shell.write requires nonempty chars; use shell.read without input",
                ));
            }
            super::shell_tools::validate_declared_filesystem_effects(
                "shell.write",
                &input.chars,
                &FilesystemEffects {
                    read: input.reads.clone(),
                    write: input.writes.clone(),
                },
            )?;
            (
                "write",
                terminals.write_with_output(
                    &input.process_id,
                    &owner,
                    &input.chars,
                    crate::ProcessReadOptions {
                        since_seq: input.since_seq,
                        event_offset: input.event_offset,
                        wait_ms: input.wait_ms,
                        limit: None,
                        max_output_bytes: input.max_output_bytes,
                    },
                    input.include_screen,
                    cancel,
                ),
                input.include_screen,
            )
        }
        ShellToolInput::Resize {
            process_id,
            rows,
            cols,
        } => (
            "resize",
            terminals.resize_cancellable(process_id, &owner, *rows, *cols, cancel),
            true,
        ),
        ShellToolInput::Signal { process_id, signal } => (
            "signal",
            terminals.signal_cancellable(process_id, &owner, *signal, cancel),
            false,
        ),
        ShellToolInput::Stop { process_id } => (
            "stop",
            terminals.signal_cancellable(process_id, &owner, ShellSignal::Terminate, cancel),
            false,
        ),
        ShellToolInput::Logs {
            process_id,
            since_seq,
            event_offset,
            max_output_bytes,
            limit,
            wait_ms,
        } => (
            "logs",
            terminals.read_with_output(
                process_id,
                &owner,
                crate::ProcessReadOptions {
                    since_seq: Some(*since_seq),
                    event_offset: *event_offset,
                    wait_ms: (*wait_ms).min(30_000),
                    limit: *limit,
                    max_output_bytes: *max_output_bytes,
                },
                false,
                cancel,
            ),
            false,
        ),
        _ => return Err(ToolError::invalid_input("not a terminal interaction")),
    };
    Ok(render(action, read.map_err(error)?, include_screen))
}

pub(super) fn render(
    action: &str,
    read: TerminalRead,
    include_screen: bool,
) -> ToolPayloadExecution {
    let summary = read.summary;
    let mut body = format!(
        "Terminal {}: {}, last_seq={}, next_event_offset={}, has_more={}, dropped_bytes={}.",
        summary.process_id,
        summary.status,
        read.last_seq,
        read.next_event_offset,
        read.has_more,
        read.dropped_bytes
    );
    if let Some(code) = summary.exit_code {
        body.push_str(&format!(" Exit code: {code}."));
    }
    if let Some(reason) = &summary.completion_reason {
        body.push_str(&format!(" Completion: {reason}."));
    }
    if !read.output.is_empty() {
        body.push_str("\nIncremental output (control sequences escaped):\n");
        body.push_str(&display_output(&read.output));
    }
    if include_screen {
        body.push_str(&format!(
            "\nScreen {}x{}, cursor=({}, {}), alternate_screen={}:\n{}",
            read.screen.cols,
            read.screen.rows,
            read.screen.cursor_row,
            read.screen.cursor_col,
            read.screen.alternate_screen,
            read.screen.text
        ));
        if read.screen.truncated {
            body.push_str("\n[screen text truncated]");
        }
    }
    if let Some(resource) = &summary.output_resource {
        body.push_str(&format!("\nOutput resource: {}", resource.resource_id));
    }
    if read.next_event_offset != 0 {
        body.push_str("\n[partial event: use last_seq as since_seq and next_event_offset as event_offset to continue, or omit both to consume unread output]");
    }
    if summary.status == agena_domain::ProcessStatus::Running {
        body.push_str("\nProcess is still running. Use shell.read for output and shell.write for exact input (\\r = Enter). Use shell.stop for cleanup.");
    }
    let mut view = ToolExecutionView::simple(
        format!("Terminal {action}"),
        summary.status.to_string(),
        body,
    );
    view.metadata
        .insert("process_id".into(), summary.process_id.clone());
    view.metadata
        .insert("status".into(), summary.status.to_string());
    view.metadata.insert("tty".into(), "true".into());
    view.metadata
        .insert("last_seq".into(), read.last_seq.to_string());
    view.metadata
        .insert("has_more".into(), read.has_more.to_string());
    let output = ToolPayloadOutput::Shell {
        output_resource: summary.output_resource.clone(),
        action: action.into(),
        terminal: include_screen.then_some(read.screen),
        dropped_bytes: read.dropped_bytes,
        shell: None,
        background: false,
        ready: false,
        process_id: Some(summary.process_id.clone()),
        status: Some(summary.status),
        output: Some(read.output),
        description: Some(summary.description.clone()),
        // Raw incremental text is authoritative; repeating it in events and
        // process summaries multiplies model/network payloads.
        events: Vec::new(),
        processes: Vec::new(),
        last_seq: read.last_seq,
        next_event_offset: read.next_event_offset,
        has_more: read.has_more,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
        completion_reason: summary.completion_reason.clone(),
    };
    ToolPayloadExecution::new(output, view)
}

pub(super) fn display_output(output: &str) -> String {
    output
        .chars()
        .flat_map(|c| {
            if c.is_control() && !matches!(c, '\n' | '\t') {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}
fn error(error: crate::MonitorError) -> ToolError {
    ToolError::invalid_input_error(&error)
}
