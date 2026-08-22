//! Internal handler for the `monitor` tool: continuous-stream background
//! listeners (a shell command or a WebSocket endpoint) whose every event is
//! projected as a `system_notification` part (everything-is-a-part, §7.3).

use std::collections::HashMap;

use agena_domain::ProcessSummary;

use crate::part::MonitorToolInput;
use crate::{
    MonitorError, MonitorService, MonitorStart, MonitorStartParams as StartParams,
    MonitorStopOutcome, MonitorWsParams,
};

use super::shell_tools::resolve_workdir;
use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

pub(crate) async fn execute_async(
    executor: &ToolExecutor,
    input: &MonitorToolInput,
    context: super::ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    let registry = monitor_registry(executor)?;
    match input {
        MonitorToolInput::Start {
            command,
            ws,
            timeout_ms,
            persistent,
            description,
        } => {
            let started = registry
                .start(StartParams {
                    process_id: context.session_id.zip(context.call_id).map(
                        |(session_id, call_id)| crate::managed_process_id(session_id, call_id),
                    ),
                    command: command.clone().unwrap_or_default(),
                    ws: ws.as_ref().map(|ws| MonitorWsParams {
                        url: ws.url.clone(),
                        protocols: ws.protocols.clone(),
                    }),
                    description: description.clone(),
                    workdir: resolve_workdir(executor, None)?,
                    timeout_ms: *timeout_ms,
                    persistent: *persistent,
                    // Monitor-tool launches are always background; they project
                    // per-event `system_notification` parts (unlike a monitored
                    // shell, whose events stay in the streaming buffer).
                    monitored: true,
                    include_pattern: None,
                    success_pattern: None,
                    failure_pattern: None,
                    quiet_period_ms: None,
                    max_buffered_lines: None,
                    capture_stderr: true,
                    env: std::env::vars().collect::<HashMap<_, _>>(),
                })
                .map_err(into_tool_error)?;
            Ok(render_start(started))
        }
        MonitorToolInput::Stop { monitor_id } => {
            let stopped = registry.stop(monitor_id).map_err(into_tool_error)?;
            Ok(render_stop(stopped))
        }
    }
}

fn monitor_registry(executor: &ToolExecutor) -> Result<&dyn MonitorService, ToolError> {
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

fn render_start(started: MonitorStart) -> ToolPayloadExecution {
    let summary = started.summary;
    let body = format!(
        "Monitor started ({}) — you will be notified with a `system_notification` on each event. Keep working; do not poll or sleep.",
        summary.process_id
    );
    let mut view = ToolExecutionView::simple(
        "Start monitor",
        format!("Monitor · {}", summary.status),
        body,
    );
    insert_summary_metadata(&mut view, &summary);

    let output = ToolPayloadOutput::Monitor {
        action: "start".to_string(),
        monitor_id: Some(summary.process_id.clone()),
        status: Some(summary.status),
        output: None,
        processes: vec![summary],
        last_seq: 0,
        exit_code: None,
        completion_reason: None,
    };
    ToolPayloadExecution::new(output, view)
}

fn render_stop(stopped: MonitorStopOutcome) -> ToolPayloadExecution {
    let summary = stopped.summary;
    let body = format!(
        "Stopped monitor {} (status={}{}{}).",
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
    let mut view = ToolExecutionView::simple("Stop monitor", stop_summary, body);
    insert_summary_metadata(&mut view, &summary);

    let output = ToolPayloadOutput::Monitor {
        action: "stop".to_string(),
        monitor_id: Some(summary.process_id.clone()),
        status: Some(summary.status),
        output: None,
        processes: vec![summary],
        last_seq: 0,
        exit_code: None,
        completion_reason: None,
    };
    ToolPayloadExecution::new(output, view)
}

fn insert_summary_metadata(view: &mut ToolExecutionView, summary: &ProcessSummary) {
    view.metadata
        .insert("monitor_id".into(), summary.process_id.clone());
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
    if let Some(code) = summary.exit_code {
        view.metadata.insert("exit_code".into(), code.to_string());
    }
    if let Some(reason) = summary.completion_reason.as_ref() {
        view.metadata
            .insert("completion_reason".into(), reason.clone());
    }
}
