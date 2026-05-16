//! Handler for the `monitor` tool — translates `MonitorToolInput`
//! into calls on the registry attached to the executor.

use std::collections::HashMap;

use crate::message::{
    MonitorEvent, MonitorStatus, MonitorStream, MonitorSummary, MonitorToolInput, ToolPayloadOutput,
};

use super::monitor::{
    MonitorError, MonitorRead, MonitorStart, MonitorStopOutcome, ReadParams, StartParams,
};
use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution};

pub(crate) fn execute(
    executor: &ToolExecutor,
    input: &MonitorToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let registry = executor
        .monitor_registry()
        .ok_or_else(|| ToolError::InvalidInput("monitor registry is not enabled".to_string()))?;

    match input {
        MonitorToolInput::Start {
            command,
            description,
            workdir,
            filesystem_effects,
            timeout_ms,
            persistent,
            include_pattern,
            max_buffered_lines,
            capture_stderr,
        } => {
            if filesystem_effects.is_empty()
                && let Some(reason) = super::bash::mutating_command_reason(command)
            {
                return Err(ToolError::InvalidInput(format!(
                    "monitor filesystem_effects must declare at least one path because the command appears to modify files: {reason}"
                )));
            }
            let cwd = workdir
                .as_deref()
                .map(|w| executor.resolve_target_path(w))
                .unwrap_or_else(|| executor.workspace_root().to_path_buf());
            executor.ensure_read_permission(&cwd)?;
            executor.ensure_filesystem_effects_permission(filesystem_effects, &cwd)?;

            let env = inherited_environment();
            let started = registry
                .start(StartParams {
                    command: command.clone(),
                    description: description.clone(),
                    workdir: cwd,
                    timeout_ms: *timeout_ms,
                    persistent: *persistent,
                    include_pattern: include_pattern.clone(),
                    max_buffered_lines: *max_buffered_lines,
                    capture_stderr: *capture_stderr,
                    env,
                })
                .map_err(into_tool_error)?;
            Ok(render_start(started))
        }
        MonitorToolInput::List {} => Ok(render_list(registry.list())),
        MonitorToolInput::Read {
            monitor_id,
            since_seq,
            limit,
            wait_ms,
        } => {
            let read = registry
                .read(ReadParams {
                    monitor_id: monitor_id.clone(),
                    since_seq: *since_seq,
                    limit: *limit,
                    wait_ms: *wait_ms,
                })
                .map_err(into_tool_error)?;
            Ok(render_read(read))
        }
        MonitorToolInput::Stop { monitor_id } => {
            let stopped = registry
                .stop(monitor_id.as_str())
                .map_err(into_tool_error)?;
            Ok(render_stop(stopped))
        }
    }
}

fn into_tool_error(err: MonitorError) -> ToolError {
    match err {
        MonitorError::NotFound(_) | MonitorError::Invalid(_) => {
            ToolError::InvalidInput(err.to_string())
        }
        MonitorError::InvalidPattern(e) => ToolError::InvalidRegexPattern(e),
        MonitorError::RuntimeMissing | MonitorError::Spawn(_) => {
            ToolError::InvalidInput(err.to_string())
        }
    }
}

fn inherited_environment() -> HashMap<String, String> {
    std::env::vars().collect()
}

pub(crate) fn render_start(started: MonitorStart) -> ToolPayloadExecution {
    let summary = started.summary;
    let title = format!(
        "Monitor start {}",
        if summary.description.is_empty() {
            summary.command.as_str()
        } else {
            summary.description.as_str()
        }
    );
    let body = format!(
        "Started monitor {} (status={}, persistent={}).\n\
        Use action='read' with this id to pull events; action='stop' to terminate.",
        summary.monitor_id, summary.status, summary.persistent,
    );
    let mut view = ToolExecutionView::simple(title, body);
    insert_summary_metadata(&mut view, &summary);

    let output = ToolPayloadOutput::Monitor {
        action: "start".to_string(),
        monitor_id: Some(summary.monitor_id.clone()),
        status: Some(summary.status),
        events: Vec::new(),
        monitors: vec![summary.clone()],
        last_seq: summary.last_seq,
        has_more: summary.last_seq > 0,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
    };
    ToolPayloadExecution::new(output, view)
}

pub(crate) fn render_list(monitors: Vec<MonitorSummary>) -> ToolPayloadExecution {
    let body = if monitors.is_empty() {
        "No monitors registered in this session.".to_string()
    } else {
        let mut lines = vec![format!("{} monitor(s):", monitors.len())];
        for summary in &monitors {
            lines.push(format!(
                "- {id} [{status}] persistent={persistent} buffered={buf} last_seq={seq} dropped={dropped}{exit} :: {command}",
                id = summary.monitor_id,
                status = summary.status,
                persistent = summary.persistent,
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

    let mut view = ToolExecutionView::simple("Monitor list", body);
    view.metadata
        .insert("count".to_string(), monitors.len().to_string());

    let output = ToolPayloadOutput::Monitor {
        action: "list".to_string(),
        monitor_id: None,
        status: None,
        events: Vec::new(),
        monitors,
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        exit_code: None,
    };
    ToolPayloadExecution::new(output, view)
}

pub(crate) fn render_read(read: MonitorRead) -> ToolPayloadExecution {
    let body = format_events(&read);
    let title = format!("Monitor read {}", read.monitor_id);
    let mut view = ToolExecutionView::simple(title, body);
    view.metadata
        .insert("monitor_id".to_string(), read.monitor_id.clone());
    view.metadata
        .insert("status".to_string(), read.status.to_string());
    view.metadata
        .insert("event_count".to_string(), read.events.len().to_string());
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

    let output = ToolPayloadOutput::Monitor {
        action: "read".to_string(),
        monitor_id: Some(read.monitor_id),
        status: Some(read.status),
        events: read.events,
        monitors: Vec::new(),
        last_seq: read.last_seq,
        has_more: read.has_more,
        dropped_lines: read.dropped_lines,
        exit_code: read.exit_code,
    };
    ToolPayloadExecution::new(output, view)
}

pub(crate) fn render_stop(stop: MonitorStopOutcome) -> ToolPayloadExecution {
    let summary = stop.summary;
    let title = format!("Monitor stop {}", summary.monitor_id);
    let body = format!(
        "Stopped monitor {} (status={}{}).",
        summary.monitor_id,
        summary.status,
        summary
            .exit_code
            .map(|c| format!(", exit={c}"))
            .unwrap_or_default(),
    );
    let mut view = ToolExecutionView::simple(title, body);
    insert_summary_metadata(&mut view, &summary);

    let output = ToolPayloadOutput::Monitor {
        action: "stop".to_string(),
        monitor_id: Some(summary.monitor_id.clone()),
        status: Some(summary.status),
        events: Vec::new(),
        monitors: vec![summary.clone()],
        last_seq: summary.last_seq,
        has_more: false,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
    };
    ToolPayloadExecution::new(output, view)
}

fn format_events(read: &MonitorRead) -> String {
    if read.events.is_empty() {
        return format!(
            "No new events since seq {} (status={}, has_more={}).",
            read.last_seq, read.status, read.has_more
        );
    }
    let mut lines = Vec::with_capacity(read.events.len() + 2);
    lines.push(format!(
        "{} event(s), seq window ..{}, status={}, has_more={}, dropped={}{}:",
        read.events.len(),
        read.last_seq,
        read.status,
        read.has_more,
        read.dropped_lines,
        read.exit_code
            .map(|c| format!(", exit={c}"))
            .unwrap_or_default(),
    ));
    for event in &read.events {
        lines.push(format_event_line(event));
    }
    if read.has_more {
        lines.push("(more events available — call read again with the returned last_seq)".into());
    }
    lines.join("\n")
}

fn format_event_line(event: &MonitorEvent) -> String {
    let stream = match event.stream {
        MonitorStream::Stdout => "out",
        MonitorStream::Stderr => "err",
    };
    format!("#{:>5} {} {}", event.seq, stream, event.line)
}

fn insert_summary_metadata(view: &mut ToolExecutionView, summary: &MonitorSummary) {
    view.metadata
        .insert("monitor_id".into(), summary.monitor_id.clone());
    view.metadata
        .insert("status".into(), summary.status.to_string());
    view.metadata
        .insert("persistent".into(), summary.persistent.to_string());
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
    let _ = MonitorStatus::Running; // ensure the import stays referenced
}
