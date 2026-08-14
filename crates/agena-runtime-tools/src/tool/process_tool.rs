//! Internal process-management handler for the `shell` tool.

static PROCESS_BLOCKING_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);

use crate::part::ShellToolInput;
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
    _context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    match input {
        ShellToolInput::Run { .. } => Err(ToolError::plugin(
            "shell.run must execute through the async process path".to_string(),
        )),
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

pub(crate) async fn execute_async(
    executor: &ToolExecutor,
    input: &ShellToolInput,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    match input {
        ShellToolInput::Run {
            shell: ProcessShell::Bash,
            command,
            run_in_background: false,
            monitor: None,
        } => execute_foreground_bash_async(executor, command, context).await,
        ShellToolInput::Run {
            shell: ProcessShell::Powershell,
            command,
            run_in_background: false,
            monitor: None,
        } => {
            let execution = powershell::execute_async(executor, command, context).await?;
            normalize_foreground_execution(execution, ProcessShell::Powershell, command)
        }
        ShellToolInput::Run {
            shell,
            command,
            run_in_background,
            monitor,
        } if *run_in_background || monitor.is_some() => {
            execute_background_run_async(executor, *shell, command, monitor.as_ref(), context).await
        }
        _ => {
            let executor = executor.clone();
            let input = input.clone();
            let worker_permit = PROCESS_BLOCKING_WORKERS
                .acquire()
                .await
                .map_err(|_| ToolError::plugin("process worker pool is unavailable".to_string()))?;
            tokio::task::spawn_blocking(move || {
                let _worker_permit = worker_permit;
                execute(&executor, &input, context)
            })
            .await
            .map_err(|error| {
                ToolError::plugin(format!(
                    "process tool worker failed before completion: {error}"
                ))
            })?
        }
    }
}

async fn execute_foreground_bash_async(
    executor: &ToolExecutor,
    command: &crate::part::ShellCommandInput,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    let execution = bash::execute_async(executor, command, context).await?;
    normalize_foreground_execution(execution, ProcessShell::Bash, command)
}

fn normalize_foreground_execution(
    execution: ToolPayloadExecution,
    shell: ProcessShell,
    command: &crate::part::ShellCommandInput,
) -> Result<ToolPayloadExecution, ToolError> {
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
            return Err(ToolError::invalid_input(format!(
                "shell.run expected process output, got {other:?}"
            )));
        }
    };
    view.set_title(process_run_title(command));
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

async fn execute_background_run_async(
    executor: &ToolExecutor,
    shell: ProcessShell,
    command: &crate::part::ShellCommandInput,
    monitor: Option<&crate::part::ShellMonitorInput>,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    let effects = command.filesystem_effects();
    validate_declared_filesystem_effects("shell.run", command.command.as_str(), &effects)?;
    let cwd = resolve_workdir(executor, command.workdir.as_deref())?;
    let ToolRuntimeContext {
        session_id,
        call_id,
        prepared_shell_command,
    } = context;
    let reserved_process_id = session_id
        .zip(call_id)
        .map(|(session_id, call_id)| crate::managed_process_id(session_id, call_id));
    let prepared = match shell {
        ProcessShell::Bash => match prepared_shell_command {
            Some(prepared) => Some(prepared),
            None => match (session_id, call_id) {
                (Some(session_id), Some(call_id)) => {
                    bash::prepare_command_async(executor, command, session_id, call_id).await?
                }
                _ => None,
            },
        },
        ProcessShell::Powershell => None,
    };
    let prepared_env = prepared.as_ref().map(|prepared| prepared.env.clone());
    let (final_command, final_cwd) = finalize_background_command(shell, command, cwd, prepared)?;
    let env = match prepared_env {
        Some(env) => env,
        None => {
            let mut env = inherited_environment();
            env.extend(
                executor
                    .shell_env_overrides_async(&final_cwd, session_id, call_id)
                    .await?,
            );
            env
        }
    };

    let worker_executor = executor.clone();
    let worker_command = command.clone();
    let worker_monitor = monitor.cloned();
    let worker_permit = PROCESS_BLOCKING_WORKERS
        .acquire()
        .await
        .map_err(|_| ToolError::plugin("process worker pool is unavailable".to_string()))?;
    tokio::task::spawn_blocking(move || {
        let _worker_permit = worker_permit;
        execute_background_run_prepared(
            &worker_executor,
            shell,
            &worker_command,
            worker_monitor.as_ref(),
            final_command,
            final_cwd,
            env,
            reserved_process_id,
        )
    })
    .await
    .map_err(|error| {
        ToolError::plugin(format!(
            "background process worker failed before completion: {error}"
        ))
    })?
}

fn execute_background_run_prepared(
    executor: &ToolExecutor,
    shell: ProcessShell,
    command: &crate::part::ShellCommandInput,
    monitor: Option<&crate::part::ShellMonitorInput>,
    final_command: String,
    final_cwd: std::path::PathBuf,
    env: std::collections::HashMap<String, String>,
    reserved_process_id: Option<String>,
) -> Result<ToolPayloadExecution, ToolError> {
    let registry = process_registry(executor)?;
    let pattern = |value: Option<&String>| {
        value.map(|value| match monitor.map(|monitor| monitor.pattern_kind) {
            Some(crate::part::ShellMonitorPatternKind::Literal) => regex::escape(value),
            _ => value.clone(),
        })
    };
    let started = registry
        .start(StartParams {
            process_id: reserved_process_id,
            command: final_command,
            ws: None,
            description: command.description.clone(),
            workdir: final_cwd,
            timeout_ms: monitor
                .and_then(|monitor| monitor.timeout_ms)
                .or(command.timeout_ms),
            persistent: monitor.map(|monitor| monitor.persistent).unwrap_or(true),
            monitored: monitor.is_some(),
            include_pattern: monitor.and_then(|monitor| monitor.include_pattern.clone()),
            success_pattern: pattern(monitor.and_then(|monitor| monitor.success_pattern.as_ref())),
            failure_pattern: pattern(monitor.and_then(|monitor| monitor.failure_pattern.as_ref())),
            quiet_period_ms: monitor.and_then(|monitor| monitor.quiet_period_ms),
            max_buffered_lines: monitor.and_then(|monitor| monitor.max_buffered_lines),
            capture_stderr: monitor
                .map(|monitor| monitor.capture_stderr)
                .unwrap_or(true),
            env,
        })
        .map_err(into_tool_error)?;
    Ok(render_run(started, shell, command, monitor.is_some()))
}

fn finalize_background_command(
    shell: ProcessShell,
    command: &crate::part::ShellCommandInput,
    cwd: std::path::PathBuf,
    prepared: Option<PreparedShellCommand>,
) -> Result<(String, std::path::PathBuf), ToolError> {
    match shell {
        ProcessShell::Bash => Ok(prepared
            .map(|prepared| (prepared.command, prepared.cwd))
            .unwrap_or_else(|| (command.command.clone(), cwd))),
        ProcessShell::Powershell => {
            if !cfg!(windows) {
                return Err(ToolError::invalid_input(
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
            ToolError::invalid_input(
                "background process registry is not enabled in this runtime".to_string(),
            )
        })
}

fn into_tool_error(err: MonitorError) -> ToolError {
    match err {
        MonitorError::NotFound(_) | MonitorError::Invalid(_) => {
            ToolError::invalid_input(err.to_string())
        }
        MonitorError::InvalidPattern(e) => ToolError::InvalidRegexPattern(e),
        MonitorError::RuntimeMissing => ToolError::invalid_input(err.to_string()),
    }
}

fn process_run_title(command: &crate::part::ShellCommandInput) -> String {
    let subject = if command.description.trim().is_empty() {
        command.command.trim()
    } else {
        command.description.trim()
    };
    format!("Run process · {subject}")
}

fn render_run(
    started: MonitorStart,
    shell: ProcessShell,
    command: &crate::part::ShellCommandInput,
    monitored: bool,
) -> ToolPayloadExecution {
    let summary = started.summary;
    let title = process_run_title(command);
    let body = format!(
        "Started {} process {} (status={}). You will be notified with a `system_notification` when it settles — do not poll shell.list/shell.logs waiting for it. Use shell.stop to terminate it.",
        if monitored { "monitored" } else { "background" },
        summary.process_id,
        summary.status
    );
    let mut view = ToolExecutionView::simple(
        title,
        format!(
            "{} · {}",
            if monitored { "Monitored" } else { "Background" },
            summary.status
        ),
        body,
    );
    insert_summary_metadata(&mut view, &summary);
    view.metadata.insert("shell".to_string(), shell.to_string());
    view.metadata
        .insert("monitored".to_string(), monitored.to_string());

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
                "- {id} [{status}] kind={kind} buffered={buf} last_seq={seq} dropped={dropped}{exit}{reason} :: {command}",
                id = summary.process_id,
                status = summary.status,
                kind = if summary.monitored { "monitor" } else { "process" },
                buf = summary.buffered_lines,
                seq = summary.last_seq,
                dropped = summary.dropped_lines,
                exit = summary
                    .exit_code
                    .map(|c| format!(" exit={c}"))
                    .unwrap_or_default(),
                reason = summary
                    .completion_reason
                    .as_ref()
                    .map(|reason| format!(" reason={reason}"))
                    .unwrap_or_default(),
                command = summary.command,
            ));
        }
        lines.join("\n")
    };

    let mut view = ToolExecutionView::simple(
        "List processes",
        format!("{} processes", processes.len()),
        body,
    );
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
        read.completion_reason.as_deref(),
    );
    let log_summary = if read.has_more {
        format!("{} events · {} · more available", events.len(), status)
    } else {
        format!("{} events · {}", events.len(), status)
    };
    let mut view = ToolExecutionView::simple("Process logs", log_summary, body);
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
    if let Some(reason) = read.completion_reason.as_ref() {
        view.metadata
            .insert("completion_reason".to_string(), reason.clone());
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
    let title = "Stop process";
    let body = format!(
        "Stopped managed process {} (status={}{}{}).",
        summary.process_id,
        summary.status,
        summary
            .exit_code
            .map(|code| format!(", exit={code}"))
            .unwrap_or_default(),
        summary
            .completion_reason
            .as_ref()
            .map(|reason| format!(", reason={reason}"))
            .unwrap_or_default(),
    );
    let stop_summary = summary
        .exit_code
        .map(|code| format!("{} · exit {code}", summary.status))
        .unwrap_or_else(|| summary.status.to_string());
    let mut view = ToolExecutionView::simple(title, stop_summary, body);
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
    completion_reason: Option<&str>,
) -> String {
    if events.is_empty() {
        return format!(
            "No new log events for {} since seq {} (status={}, has_more={}{}).",
            process_id,
            last_seq,
            status,
            has_more,
            completion_reason
                .map(|reason| format!(", reason={reason}"))
                .unwrap_or_default()
        );
    }
    let mut lines = Vec::with_capacity(events.len() + 2);
    lines.push(format!(
        "{} log event(s), seq window ..{}, status={}, has_more={}, dropped={}{}{}:",
        events.len(),
        last_seq,
        status,
        has_more,
        dropped_lines,
        exit_code
            .map(|code| format!(", exit={code}"))
            .unwrap_or_default(),
        completion_reason
            .map(|reason| format!(", reason={reason}"))
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
        .insert("monitored".into(), summary.monitored.to_string());
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
    if let Some(reason) = summary.completion_reason.as_ref() {
        view.metadata
            .insert("completion_reason".into(), reason.clone());
    }
}
