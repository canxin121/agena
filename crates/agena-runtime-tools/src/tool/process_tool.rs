//! Internal process-management handler for the `shell` tool.

use crate::message::ShellToolInput;
use agena_domain::{ProcessEvent, ProcessShell, ProcessStatus, ProcessStream, ProcessSummary};

use super::shell_tools::{
    inherited_environment, resolve_workdir, validate_declared_filesystem_effects,
};
use super::{
    PreparedShellCommand, ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution,
    ToolPayloadOutput, ToolRuntimeContext, bash, powershell,
};
use crate::{
    MonitorError, MonitorRead, MonitorReadParams as ReadParams, MonitorService, MonitorStart,
    MonitorStartParams as StartParams, MonitorStopOutcome,
};
use agena_tool::shell::powershell_command_for_windows;

pub(crate) fn execute(
    executor: &ToolExecutor,
    input: &ShellToolInput,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    match input {
        ShellToolInput::Run {
            shell,
            command,
            background,
        } => {
            if *background {
                execute_background_run(executor, *shell, command, context)
            } else {
                execute_foreground_run(executor, *shell, command, context)
            }
        }
        ShellToolInput::List {} => {
            let registry = process_registry(executor)?;
            Ok(render_list(registry.list()))
        }
        ShellToolInput::Logs {
            process_id,
            since_seq,
            limit,
            wait_ms,
        } => {
            let registry = process_registry(executor)?;
            let read = registry
                .read(ReadParams {
                    monitor_id: process_id.clone(),
                    since_seq: *since_seq,
                    limit: *limit,
                    wait_ms: *wait_ms,
                })
                .map_err(into_tool_error)?;
            Ok(render_logs(read))
        }
        ShellToolInput::Stop { process_id } => {
            let registry = process_registry(executor)?;
            let stopped = registry
                .stop(process_id.as_str())
                .map_err(into_tool_error)?;
            Ok(render_stop(stopped))
        }
    }
}

fn execute_foreground_run(
    executor: &ToolExecutor,
    shell: ProcessShell,
    command: &crate::message::ShellCommandInput,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    let execution = match shell {
        ProcessShell::Bash => bash::execute(executor, command, context)?,
        ProcessShell::Powershell => powershell::execute(executor, command, context)?,
    };
    let super::ToolPayloadExecution {
        output,
        mut view,
        apply_patch: _,
    } = execution;

    let (status, description, exit_code, output_text) = match output {
        ToolPayloadOutput::Shell {
            status,
            description,
            exit_code,
            output,
            ..
        } => (
            status.unwrap_or(ProcessStatus::Exited),
            description,
            exit_code,
            output.unwrap_or_else(|| view.output_text.clone()),
        ),
        other => {
            return Err(ToolError::InvalidInput(format!(
                "shell.run expected process output, got {other:?}"
            )));
        }
    };
    view.title = if command.description.trim().is_empty() {
        format!("Process run {}", command.command)
    } else {
        format!("Process run {}", command.description)
    };
    view.metadata.insert("shell".to_string(), shell.to_string());
    view.metadata
        .insert("background".to_string(), "false".to_string());
    view.metadata
        .insert("status".to_string(), status.to_string());

    let output = ToolPayloadOutput::Shell {
        action: "run".to_string(),
        shell: Some(shell),
        background: false,
        process_id: None,
        status: Some(status),
        output: Some(output_text),
        description,
        events: Vec::new(),
        processes: Vec::new(),
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        exit_code,
    };
    Ok(ToolPayloadExecution::new(output, view))
}

fn execute_background_run(
    executor: &ToolExecutor,
    shell: ProcessShell,
    command: &crate::message::ShellCommandInput,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    let registry = process_registry(executor)?;
    validate_declared_filesystem_effects(
        "shell.run",
        command.command.as_str(),
        &command.filesystem_effects,
    )?;

    let cwd = resolve_workdir(executor, command.workdir.as_deref())?;
    executor.ensure_filesystem_effects_permission(&command.filesystem_effects, &cwd)?;
    executor.ensure_network_effects_permission(&command.network_effects)?;

    let mut env = inherited_environment();
    env.extend(executor.shell_env_overrides(&cwd, context.session_id, context.call_id)?);

    let prepared = match shell {
        ProcessShell::Bash => match context.prepared_shell_command {
            Some(prepared) => Some(prepared),
            None => match (context.session_id, context.call_id) {
                (Some(session_id), Some(call_id)) => {
                    executor.prepare_shell_command(command, session_id, call_id)?
                }
                _ => None,
            },
        },
        ProcessShell::Powershell => None,
    };
    let (final_command, final_cwd) = finalize_background_command(shell, command, cwd, prepared)?;
    let started = registry
        .start(StartParams {
            command: final_command,
            description: command.description.clone(),
            workdir: final_cwd,
            timeout_ms: command.timeout_ms,
            persistent: true,
            include_pattern: None,
            max_buffered_lines: None,
            capture_stderr: true,
            env,
        })
        .map_err(into_tool_error)?;
    Ok(render_run(started, shell, command))
}

fn finalize_background_command(
    shell: ProcessShell,
    command: &crate::message::ShellCommandInput,
    cwd: std::path::PathBuf,
    prepared: Option<PreparedShellCommand>,
) -> Result<(String, std::path::PathBuf), ToolError> {
    match shell {
        ProcessShell::Bash => Ok(prepared
            .map(|prepared| (prepared.command, prepared.cwd))
            .unwrap_or_else(|| (command.command.clone(), cwd))),
        ProcessShell::Powershell => {
            if !cfg!(windows) {
                return Err(ToolError::InvalidInput(
                    "powershell tool is only available on Windows".to_string(),
                ));
            }
            Ok((
                powershell_command_for_windows(&command.command).join(" "),
                cwd,
            ))
        }
    }
}

fn process_registry(executor: &ToolExecutor) -> Result<&dyn MonitorService, ToolError> {
    executor
        .monitor_registry()
        .map(|registry| registry.as_ref())
        .ok_or_else(|| {
            ToolError::InvalidInput(
                "background process registry is not enabled in this runtime".to_string(),
            )
        })
}

fn into_tool_error(err: MonitorError) -> ToolError {
    match err {
        MonitorError::NotFound(_) | MonitorError::Invalid(_) => {
            ToolError::InvalidInput(err.to_string())
        }
        MonitorError::InvalidPattern(e) => ToolError::InvalidRegexPattern(e),
        MonitorError::RuntimeMissing => ToolError::InvalidInput(err.to_string()),
    }
}

fn render_run(
    started: MonitorStart,
    shell: ProcessShell,
    command: &crate::message::ShellCommandInput,
) -> ToolPayloadExecution {
    let summary = started.summary;
    let title = if command.description.trim().is_empty() {
        format!("Process run {}", command.command)
    } else {
        format!("Process run {}", command.description)
    };
    let body = format!(
        "Started background process {} (status={}). Use action='logs' to read output and action='stop' to terminate it.",
        summary.process_id, summary.status
    );
    let mut view = ToolExecutionView::simple(title, body);
    insert_summary_metadata(&mut view, &summary);
    view.metadata.insert("shell".to_string(), shell.to_string());

    let output = ToolPayloadOutput::Shell {
        action: "run".to_string(),
        shell: Some(shell),
        background: true,
        process_id: Some(summary.process_id.clone()),
        status: Some(summary.status),
        output: None,
        description: None,
        events: Vec::new(),
        processes: vec![summary.clone()],
        last_seq: summary.last_seq,
        has_more: summary.last_seq > 0,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
    };
    ToolPayloadExecution::new(output, view)
}

fn render_list(processes: Vec<ProcessSummary>) -> ToolPayloadExecution {
    let body = if processes.is_empty() {
        "No background processes registered in this session.".to_string()
    } else {
        let mut lines = vec![format!("{} background process(es):", processes.len())];
        for summary in &processes {
            lines.push(format!(
                "- {id} [{status}] buffered={buf} last_seq={seq} dropped={dropped}{exit} :: {command}",
                id = summary.process_id,
                status = summary.status,
                buf = summary.buffered_lines,
                seq = summary.last_seq,
                dropped = summary.dropped_lines,
                exit = summary
                    .exit_code
                    .map(|c| format!(" exit={c}"))
                    .unwrap_or_default(),
                command = summary.command,
            ));
        }
        lines.join("\n")
    };

    let mut view = ToolExecutionView::simple("Process list", body);
    view.metadata
        .insert("count".to_string(), processes.len().to_string());

    let output = ToolPayloadOutput::Shell {
        action: "list".to_string(),
        shell: None,
        background: true,
        process_id: None,
        status: None,
        output: None,
        description: None,
        events: Vec::new(),
        processes,
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        exit_code: None,
    };
    ToolPayloadExecution::new(output, view)
}

fn render_logs(read: MonitorRead) -> ToolPayloadExecution {
    let process_id = read.monitor_id.clone();
    let status = read.status;
    let events = read.events;
    let body = format_process_events(
        process_id.as_str(),
        status,
        events.as_slice(),
        read.last_seq,
        read.has_more,
        read.dropped_lines,
        read.exit_code,
    );
    let mut view = ToolExecutionView::simple(format!("Process logs {process_id}"), body);
    view.metadata
        .insert("process_id".to_string(), process_id.clone());
    view.metadata
        .insert("status".to_string(), status.to_string());
    view.metadata
        .insert("event_count".to_string(), events.len().to_string());
    view.metadata
        .insert("last_seq".to_string(), read.last_seq.to_string());
    view.metadata
        .insert("has_more".to_string(), read.has_more.to_string());
    view.metadata
        .insert("dropped_lines".to_string(), read.dropped_lines.to_string());
    if let Some(code) = read.exit_code {
        view.metadata
            .insert("exit_code".to_string(), code.to_string());
    }

    let output = ToolPayloadOutput::Shell {
        action: "logs".to_string(),
        shell: None,
        background: true,
        process_id: Some(process_id),
        status: Some(status),
        output: None,
        description: None,
        events,
        processes: Vec::new(),
        last_seq: read.last_seq,
        has_more: read.has_more,
        dropped_lines: read.dropped_lines,
        exit_code: read.exit_code,
    };
    ToolPayloadExecution::new(output, view)
}

fn render_stop(stop: MonitorStopOutcome) -> ToolPayloadExecution {
    let summary = stop.summary;
    let title = format!("Process stop {}", summary.process_id);
    let body = format!(
        "Stopped background process {} (status={}{}).",
        summary.process_id,
        summary.status,
        summary
            .exit_code
            .map(|code| format!(", exit={code}"))
            .unwrap_or_default(),
    );
    let mut view = ToolExecutionView::simple(title, body);
    insert_summary_metadata(&mut view, &summary);

    let output = ToolPayloadOutput::Shell {
        action: "stop".to_string(),
        shell: None,
        background: true,
        process_id: Some(summary.process_id.clone()),
        status: Some(summary.status),
        output: None,
        description: None,
        events: Vec::new(),
        processes: vec![summary.clone()],
        last_seq: summary.last_seq,
        has_more: false,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
    };
    ToolPayloadExecution::new(output, view)
}

fn format_process_events(
    process_id: &str,
    status: ProcessStatus,
    events: &[ProcessEvent],
    last_seq: u64,
    has_more: bool,
    dropped_lines: u64,
    exit_code: Option<i32>,
) -> String {
    if events.is_empty() {
        return format!(
            "No new log events for {} since seq {} (status={}, has_more={}).",
            process_id, last_seq, status, has_more
        );
    }
    let mut lines = Vec::with_capacity(events.len() + 2);
    lines.push(format!(
        "{} log event(s), seq window ..{}, status={}, has_more={}, dropped={}{}:",
        events.len(),
        last_seq,
        status,
        has_more,
        dropped_lines,
        exit_code
            .map(|code| format!(", exit={code}"))
            .unwrap_or_default(),
    ));
    for event in events {
        let stream = match event.stream {
            ProcessStream::Stdout => "out",
            ProcessStream::Stderr => "err",
        };
        lines.push(format!("#{:>5} {} {}", event.seq, stream, event.line));
    }
    if has_more {
        lines.push(
            "(more log events available — call logs again with the returned last_seq)".into(),
        );
    }
    lines.join("\n")
}

fn insert_summary_metadata(view: &mut ToolExecutionView, summary: &ProcessSummary) {
    view.metadata
        .insert("process_id".into(), summary.process_id.clone());
    view.metadata
        .insert("status".into(), summary.status.to_string());
    view.metadata
        .insert("background".into(), summary.background.to_string());
    view.metadata
        .insert("started_at_ms".into(), summary.started_at_ms.to_string());
    if let Some(ended) = summary.ended_at_ms {
        view.metadata
            .insert("ended_at_ms".into(), ended.to_string());
    }
    view.metadata
        .insert("buffered_lines".into(), summary.buffered_lines.to_string());
    view.metadata
        .insert("last_seq".into(), summary.last_seq.to_string());
    view.metadata
        .insert("dropped_lines".into(), summary.dropped_lines.to_string());
    if let Some(code) = summary.exit_code {
        view.metadata.insert("exit_code".into(), code.to_string());
    }
}
