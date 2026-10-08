//! Shared Shell launch and lifecycle adapter for foreground commands, background jobs and interactive terminals.

static SHELL_READ_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);
static SHELL_LAUNCH_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);

async fn worker_permit(
    executor: &ToolExecutor,
    workers: &'static tokio::sync::Semaphore,
) -> Result<tokio::sync::SemaphorePermit<'static>, ToolError> {
    executor.ensure_not_cancelled()?;
    let acquire = workers.acquire();
    let cancelled = async {
        match executor.cancellation_token() {
            Some(token) => token.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        biased;
        _ = cancelled => Err(ToolError::Cancelled),
        result = acquire => result.map_err(|_| ToolError::plugin("shell worker pool is unavailable".to_owned())),
    }
}

pub(super) async fn launch_worker_permit(
    executor: &ToolExecutor,
) -> Result<tokio::sync::SemaphorePermit<'static>, ToolError> {
    worker_permit(executor, &SHELL_LAUNCH_WORKERS).await
}

use crate::part::{ShellToolInput, ShellWatchInput, ShellWatchPolicy};
use agena_domain::{ProcessEvent, ProcessShell, ProcessStatus, ProcessStream, ProcessSummary};

use super::shell_tools::{
    inherited_environment, prepare_posix_launch, resolve_workdir, shell_settings,
    validate_declared_filesystem_effects,
};
use super::{
    PreparedShellCommand, ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution,
    ToolPayloadOutput, ToolRuntimeContext, bash, powershell,
};
use crate::{
    MonitorError, MonitorRead, MonitorService, MonitorStart, MonitorStartParams as StartParams,
    MonitorStopOutcome,
};
use agena_tool::shell::powershell_command_for_windows;

enum ManagedLaunch {
    Spawn,
    Watch(ShellWatchPolicy),
    Open(crate::part::ShellOpenInput),
}

impl ManagedLaunch {
    fn tool_name(&self) -> &'static str {
        match self {
            Self::Spawn => "agena.shell.spawn",
            Self::Watch(_) => "agena.shell.watch",
            Self::Open(_) => "agena.shell.open",
        }
    }
}

pub(crate) fn execute(
    executor: &ToolExecutor,
    input: &ShellToolInput,
    context: ToolRuntimeContext,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<ToolPayloadExecution, ToolError> {
    match input {
        ShellToolInput::Exec { .. }
        | ShellToolInput::Spawn { .. }
        | ShellToolInput::Open { .. } => Err(ToolError::plugin(
            "shell launches must execute through the async shell path".to_string(),
        )),
        ShellToolInput::List {} => {
            let registry = process_registry(executor)?;
            let owner = super::terminal_tool::owner(executor, context.session_id)?;
            Ok(render_list(
                registry
                    .list()
                    .into_iter()
                    .filter(|summary| registry.is_owned(&summary.process_id, &owner))
                    .collect(),
            ))
        }
        ShellToolInput::Watch { input } => {
            let ShellWatchInput::Process {
                process_id,
                policy,
                since_seq,
            } = input.as_ref()
            else {
                return Err(ToolError::plugin(
                    "watched launches require the async launch path",
                ));
            };
            let registry = process_registry(executor)?;
            let owner = super::terminal_tool::owner(executor, context.session_id)?;
            if !registry.is_owned(process_id, &owner) {
                return Err(into_tool_error(MonitorError::NotFound(process_id.clone())));
            }
            let summary = registry
                .configure_watch(process_id, policy.clone(), *since_seq)
                .map_err(into_tool_error)?;
            Ok(render_watch_update(summary, policy.is_some()))
        }
        ShellToolInput::Logs {
            process_id,
            since_seq,
            event_offset,
            max_output_bytes,
            limit,
            wait_ms,
        } => read_output(
            executor,
            input,
            context.session_id,
            process_id,
            crate::ProcessReadOptions {
                since_seq: Some(*since_seq),
                event_offset: *event_offset,
                max_output_bytes: *max_output_bytes,
                limit: *limit,
                wait_ms: *wait_ms,
            },
            false,
            cancel,
        ),
        ShellToolInput::Read { input: read } => read_output(
            executor,
            input,
            context.session_id,
            &read.process_id,
            crate::ProcessReadOptions {
                since_seq: read.since_seq,
                event_offset: read.event_offset,
                max_output_bytes: read.max_output_bytes,
                limit: read.limit,
                wait_ms: read.wait_ms,
            },
            read.include_screen,
            cancel,
        ),
        ShellToolInput::Stop { process_id } => {
            let registry = process_registry(executor)?;
            let owner = super::terminal_tool::owner(executor, context.session_id)?;
            if !registry.is_owned(process_id, &owner) {
                return Err(into_tool_error(MonitorError::NotFound(process_id.clone())));
            }
            if registry
                .terminals()
                .is_some_and(|terminals| terminals.contains(process_id))
            {
                return super::terminal_tool::execute(executor, input, context.session_id, cancel);
            }
            let stopped = registry
                .stop(process_id.as_str())
                .map_err(into_tool_error)?;
            Ok(render_stop(stopped))
        }
        ShellToolInput::Write { .. }
        | ShellToolInput::Resize { .. }
        | ShellToolInput::Signal { .. } => {
            super::terminal_tool::execute(executor, input, context.session_id, cancel)
        }
    }
}

pub(crate) async fn execute_async(
    executor: &ToolExecutor,
    input: &ShellToolInput,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    match input {
        ShellToolInput::Exec {
            shell: ProcessShell::Bash,
            command,
        } => execute_foreground_bash_async(executor, command, context).await,
        ShellToolInput::Exec {
            shell: ProcessShell::Powershell,
            command,
        } => {
            let execution = powershell::execute_async(executor, command, context).await?;
            normalize_foreground_execution(execution, ProcessShell::Powershell, command)
        }
        ShellToolInput::Spawn { shell, command } => {
            execute_managed_launch_async(executor, *shell, command, ManagedLaunch::Spawn, context)
                .await
        }
        ShellToolInput::Watch { input }
            if matches!(input.as_ref(), ShellWatchInput::Launch { .. }) =>
        {
            let ShellWatchInput::Launch {
                shell,
                command,
                policy,
            } = input.as_ref()
            else {
                unreachable!()
            };
            execute_managed_launch_async(
                executor,
                *shell,
                command,
                ManagedLaunch::Watch(policy.clone()),
                context,
            )
            .await
        }
        ShellToolInput::Open { input } => {
            execute_managed_launch_async(
                executor,
                input.shell,
                &input.command,
                ManagedLaunch::Open(input.as_ref().clone()),
                context,
            )
            .await
        }
        _ => {
            let worker_executor = executor.clone();
            let input = input.clone();
            // Long reads must never occupy every slot needed to stop a process.
            // At most sixteen ordinary workers block; out-of-band controls use
            // the runtime's remaining blocking capacity and never wait on them.
            let urgent = matches!(
                &input,
                ShellToolInput::Stop { .. } | ShellToolInput::Signal { .. }
            ) || matches!(&input, ShellToolInput::Watch { input } if matches!(input.as_ref(), ShellWatchInput::Process { policy: None, .. }));
            let worker_permit = if urgent {
                None
            } else {
                Some(
                    worker_permit(
                        executor,
                        if matches!(
                            input,
                            ShellToolInput::Read { .. } | ShellToolInput::Logs { .. }
                        ) {
                            &SHELL_READ_WORKERS
                        } else {
                            &SHELL_LAUNCH_WORKERS
                        },
                    )
                    .await?,
                )
            };
            super::terminal_tool::blocking(executor, move |cancel| {
                let _worker_permit = worker_permit;
                execute(&worker_executor, &input, context, &cancel)
            })
            .await
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

    let (status, description, exit_code, output_text, completion_reason, output_resource) =
        match output {
            ToolPayloadOutput::Shell {
                status,
                description,
                exit_code,
                output,
                completion_reason,
                output_resource,
                ..
            } => (
                status.unwrap_or(ProcessStatus::Exited),
                description,
                exit_code,
                output.unwrap_or_else(|| view.output_text.clone()),
                completion_reason,
                output_resource,
            ),
            other => {
                return Err(ToolError::invalid_input(format!(
                    "shell.exec expected command output, got {other:?}"
                )));
            }
        };
    view.set_title(command_title("Execute", command));
    view.metadata.insert("shell".to_string(), shell.to_string());
    view.metadata
        .insert("background".to_string(), "false".to_string());
    view.metadata
        .insert("status".to_string(), status.to_string());

    let output = ToolPayloadOutput::Shell {
        output_resource,
        terminal: None,
        dropped_bytes: 0,
        action: "exec".to_string(),
        shell: Some(shell),
        background: false,
        ready: false,
        process_id: None,
        status: Some(status),
        output: Some(output_text),
        description,
        events: Vec::new(),
        processes: Vec::new(),
        last_seq: 0,
        next_event_offset: 0,
        has_more: false,
        dropped_lines: 0,
        exit_code,
        completion_reason,
    };
    Ok(ToolPayloadExecution::new(output, view))
}

async fn execute_managed_launch_async(
    executor: &ToolExecutor,
    shell: ProcessShell,
    command: &crate::part::ShellCommandInput,
    mode: ManagedLaunch,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    let effects = command.filesystem_effects();
    let tool_name = mode.tool_name();
    validate_declared_filesystem_effects(tool_name, command.command.as_str(), &effects)?;
    let cwd = resolve_workdir(executor, command.workdir.as_deref())?;
    let ToolRuntimeContext {
        session_id,
        call_id,
        prepared_shell_command,
        launch_provenance: _,
        output,
    } = context;
    let reserved_process_id = session_id
        .zip(call_id)
        .map(|(session_id, call_id)| crate::managed_process_id(session_id, call_id));
    let prepared = match shell {
        ProcessShell::Bash => match prepared_shell_command {
            Some(prepared) => Some(prepared),
            None => match (session_id, call_id) {
                (Some(session_id), Some(call_id)) => {
                    bash::prepare_command_async(executor, command, session_id, call_id, tool_name)
                        .await?
                }
                _ => None,
            },
        },
        ProcessShell::Powershell => None,
    };
    let prepared_env = prepared.as_ref().map(|prepared| prepared.env.clone());
    let prepared_launch = prepared
        .as_ref()
        .and_then(|prepared| prepared.launch.clone());
    let (final_command, final_cwd) = finalize_background_command(shell, command, cwd, prepared)?;
    let (env, launch) = match prepared_env {
        Some(env) => (env, prepared_launch),
        None => {
            let mut env = inherited_environment();
            let launch = match shell {
                ProcessShell::Bash => Some(prepare_posix_launch(shell_settings(), &mut env).await?),
                ProcessShell::Powershell => None,
            };
            env.extend(
                executor
                    .shell_env_overrides_async(&final_cwd, session_id, call_id)
                    .await?,
            );
            (env, launch)
        }
    };

    let worker_executor = executor.clone();
    let worker_command = command.clone();
    let worker_permit = launch_worker_permit(executor).await?;
    super::terminal_tool::blocking(executor, move |cancel| {
        let _worker_permit = worker_permit;
        let process_owner = super::terminal_tool::owner(&worker_executor, session_id)?;
        if let ManagedLaunch::Open(terminal_input) = mode {
            return super::terminal_tool::start_prepared(
                &worker_executor,
                shell,
                &terminal_input,
                final_command,
                final_cwd,
                env,
                launch.as_ref(),
                reserved_process_id,
                process_owner,
                output,
                cancel,
            );
        }
        execute_spawn_prepared(
            &worker_executor,
            shell,
            &worker_command,
            match &mode {
                ManagedLaunch::Watch(policy) => Some(policy),
                _ => None,
            },
            PreparedShellCommand {
                command: final_command,
                cwd: final_cwd,
                env,
                launch,
            },
            SpawnDestination {
                reserved_process_id,
                owner: process_owner,
                output,
            },
            &cancel,
        )
    })
    .await
}

struct SpawnDestination {
    reserved_process_id: Option<String>,
    owner: crate::TerminalOwner,
    output: Option<agena_storage::content::ContentWriter>,
}

fn execute_spawn_prepared(
    executor: &ToolExecutor,
    shell: ProcessShell,
    command: &crate::part::ShellCommandInput,
    watch: Option<&ShellWatchPolicy>,
    prepared: PreparedShellCommand,
    destination: SpawnDestination,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<ToolPayloadExecution, ToolError> {
    let SpawnDestination {
        reserved_process_id,
        owner,
        output,
    } = destination;
    let PreparedShellCommand {
        command: final_command,
        cwd: final_cwd,
        mut env,
        launch,
    } = prepared;
    let argv = crate::shell_sandbox::protect(
        executor,
        match launch.as_ref() {
            Some(spec) => spec.argv(final_command.as_str()),
            None if shell == ProcessShell::Powershell => {
                powershell_command_for_windows(&final_command)
            }
            None => agena_tool::shell::shell_command_for_platform(&final_command),
        },
        command,
        &mut env,
    )?;
    let registry = process_registry(executor)?;
    let started = registry
        .start_confirmed(
            StartParams {
                output,
                owner: Some(owner),
                argv: Some(argv),
                process_id: reserved_process_id,
                command: final_command,
                ws: None,
                description: command.description.clone(),
                workdir: final_cwd,
                timeout_ms: command.timeout_ms,
                persistent: command.timeout_ms.is_none(),
                monitored: watch.is_some(),
                watch: watch.cloned(),
                include_pattern: None,
                success_pattern: None,
                failure_pattern: None,
                quiet_period_ms: None,
                max_buffered_lines: None,
                capture_stderr: true,
                env,
            },
            cancel,
        )
        .map_err(into_tool_error)?;
    let mut execution = render_spawn(started, shell, command, watch.is_some());
    if let Some(spec) = launch.as_ref() {
        for (key, value) in spec.metadata() {
            execution.view.metadata.insert(key, value);
        }
    }
    Ok(execution)
}

fn read_output(
    executor: &ToolExecutor,
    input: &ShellToolInput,
    session_id: Option<i64>,
    process_id: &str,
    options: crate::ProcessReadOptions,
    include_screen: bool,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<ToolPayloadExecution, ToolError> {
    let registry = process_registry(executor)?;
    let owner = super::terminal_tool::owner(executor, session_id)?;
    if !registry.is_owned(process_id, &owner) {
        return Err(into_tool_error(MonitorError::NotFound(process_id.into())));
    }
    if registry
        .terminals()
        .is_some_and(|terminals| terminals.contains(process_id))
    {
        return super::terminal_tool::execute(executor, input, session_id, cancel);
    }
    if include_screen {
        return Err(ToolError::invalid_input(
            "include_screen is available only for shell.open terminals",
        ));
    }
    let read = registry
        .read_output_cancellable(process_id, options, cancel)
        .map_err(into_tool_error)?;
    Ok(render_logs(read, input.action()))
}

fn render_watch_update(summary: ProcessSummary, enabled: bool) -> ToolPayloadExecution {
    let mut view = ToolExecutionView::simple(
        "Configure process watch",
        if enabled {
            "Watch updated"
        } else {
            "Watch removed"
        },
        format!(
            "{}: process_id={}, status={}, ready={}. The existing process continues and its original completion notification remains active. Read output with shell.read; stop the process with shell.stop.",
            if enabled {
                "Replaced output watch"
            } else {
                "Removed output watch"
            },
            summary.process_id,
            summary.status,
            summary.ready
        ),
    );
    insert_summary_metadata(&mut view, &summary);
    let output = ToolPayloadOutput::Shell {
        action: if enabled {
            "watch_update"
        } else {
            "watch_remove"
        }
        .into(),
        output_resource: summary.output_resource.clone(),
        terminal: None,
        dropped_bytes: 0,
        shell: None,
        background: true,
        ready: summary.ready,
        process_id: Some(summary.process_id),
        status: Some(summary.status),
        output: None,
        description: None,
        events: Vec::new(),
        processes: Vec::new(),
        last_seq: 0,
        next_event_offset: 0,
        has_more: summary.last_seq > 0,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
        completion_reason: summary.completion_reason,
    };
    ToolPayloadExecution::new(output, view)
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
            Ok((command.command.clone(), cwd))
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
            ToolError::invalid_input_error(&err)
        }
        MonitorError::InvalidPattern(e) => ToolError::InvalidRegexPattern(e),
        MonitorError::RuntimeMissing => ToolError::invalid_input_error(&err),
    }
}

fn command_title(action: &str, command: &crate::part::ShellCommandInput) -> String {
    let subject = if command.description.trim().is_empty() {
        command.command.trim()
    } else {
        command.description.trim()
    };
    format!("{action} command · {subject}")
}

fn render_spawn(
    started: MonitorStart,
    shell: ProcessShell,
    command: &crate::part::ShellCommandInput,
    watched: bool,
) -> ToolPayloadExecution {
    let summary = started.summary;
    let action = if watched { "watch" } else { "spawn" };
    let title = command_title(if watched { "Watch" } else { "Spawn" }, command);
    let notifications = if watched {
        "Readiness notifies once without ending the process; include_pattern notifies once by default. No ordinary output is notified without an include_pattern. Completion is delivered once."
    } else {
        "You will receive one system_notification when it ends."
    };
    let body = format!(
        "Started background shell job {} (status={}). Continue other work now. {notifications} Do not poll merely to wait. Use shell.read for incremental diagnostics and shell.stop for cleanup.",
        summary.process_id, summary.status
    );
    let mut view = ToolExecutionView::simple(
        title,
        format!(
            "{} · {}",
            if watched { "Watching" } else { "Background" },
            summary.status
        ),
        body,
    );
    insert_summary_metadata(&mut view, &summary);
    view.metadata.insert("shell".to_string(), shell.to_string());
    view.metadata
        .insert("monitored".to_string(), watched.to_string());

    let output = ToolPayloadOutput::Shell {
        terminal: None,
        dropped_bytes: 0,
        action: action.to_string(),
        output_resource: summary.output_resource.clone(),
        shell: Some(shell),
        background: true,
        ready: summary.ready,
        process_id: Some(summary.process_id.clone()),
        status: Some(summary.status),
        output: None,
        description: None,
        events: Vec::new(),
        processes: Vec::new(),
        // A launch receipt has consumed no output. Returning the registry's
        // current cursor would make the first shell.logs call skip startup logs.
        last_seq: 0,
        next_event_offset: 0,
        has_more: summary.last_seq > 0,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
        completion_reason: summary.completion_reason.clone(),
    };
    ToolPayloadExecution::new(output, view)
}

fn render_list(processes: Vec<ProcessSummary>) -> ToolPayloadExecution {
    let body = if processes.is_empty() {
        "No shell jobs or interactive terminals registered in this session.".to_string()
    } else {
        let mut lines = vec![format!("{} shell job(s)/terminal(s):", processes.len())];
        for summary in &processes {
            lines.push(format!(
                "- {id} [{status}] kind={kind} buffered={buf} last_seq={seq} dropped={dropped}{exit}{reason} :: {command}",
                id = summary.process_id,
                status = summary.status,
                kind = if summary.tty { "terminal" } else if summary.websocket { "websocket" } else if summary.monitored { "watch" } else { "background" },
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
        terminal: None,
        dropped_bytes: 0,
        action: "list".to_string(),
        output_resource: None,
        shell: None,
        background: true,
        ready: false,
        process_id: None,
        status: None,
        output: None,
        description: None,
        events: Vec::new(),
        processes,
        last_seq: 0,
        next_event_offset: 0,
        has_more: false,
        dropped_lines: 0,
        exit_code: None,
        completion_reason: None,
    };
    ToolPayloadExecution::new(output, view)
}

fn render_logs(read: MonitorRead, action: &str) -> ToolPayloadExecution {
    // Slicing happens before consumption in the process registry. A partial
    // event carries its byte cursor, never silently advancing past unseen text.
    let process_id = read.monitor_id.clone();
    let status = read.status;
    let events = read.events;
    let mut body = format_process_events(
        process_id.as_str(),
        status,
        events.as_slice(),
        read.last_seq,
        read.has_more,
        read.dropped_lines,
        read.exit_code,
        read.completion_reason.as_deref(),
    );
    if let Some(resource) = &read.output_resource {
        body.push_str(&format!("\nOutput resource: {}", resource.resource_id));
    }
    if read.next_event_offset != 0 {
        body.push_str(&format!("\n[partial event: continue with since_seq={} and event_offset={} (next_event_offset); no remaining text was discarded]", read.last_seq, read.next_event_offset));
    }
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
        terminal: None,
        dropped_bytes: 0,
        action: action.to_string(),
        output_resource: read.output_resource.clone(),
        shell: None,
        background: true,
        ready: read.ready,
        process_id: Some(process_id),
        status: Some(status),
        output: None,
        description: None,
        events,
        processes: Vec::new(),
        last_seq: read.last_seq,
        next_event_offset: read.next_event_offset,
        has_more: read.has_more,
        dropped_lines: read.dropped_lines,
        exit_code: read.exit_code,
        completion_reason: read.completion_reason,
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
        terminal: None,
        dropped_bytes: 0,
        action: "stop".to_string(),
        output_resource: summary.output_resource.clone(),
        shell: None,
        background: true,
        ready: summary.ready,
        process_id: Some(summary.process_id.clone()),
        status: Some(summary.status),
        output: None,
        description: None,
        events: Vec::new(),
        processes: vec![summary.clone()],
        last_seq: summary.last_seq,
        next_event_offset: 0,
        has_more: false,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
        completion_reason: summary.completion_reason.clone(),
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
