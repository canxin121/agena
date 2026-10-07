//! WebSocket subscriptions. Local command listeners use the shared Shell path.

use agena_domain::ProcessSummary;

use crate::part::MonitorToolInput;
use crate::{MonitorError, MonitorStart, MonitorStartParams as StartParams, MonitorWsParams};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

pub(crate) async fn execute_async(
    executor: &ToolExecutor,
    input: &MonitorToolInput,
    context: super::ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    match input {
        MonitorToolInput::Start {
            command,
            ws,
            timeout_ms,
            description,
            policy,
            workdir,
            reads,
            writes,
            network,
            ..
        } => {
            // Keep historical payloads decodable without retaining a second
            // executable local-command entry point or silently dropping effects.
            if command.is_some() || policy.is_some() {
                return Err(ToolError::invalid_input(
                    "command-output monitoring uses shell.watch; monitor.start subscribes to WebSockets",
                ));
            }
            if workdir.is_some() || !reads.is_empty() || !writes.is_empty() || !network.is_empty() {
                return Err(ToolError::invalid_input(
                    "WebSocket subscriptions do not accept shell workdir or command effects",
                ));
            }
            let ws = ws
                .as_ref()
                .ok_or_else(|| ToolError::invalid_input("monitor.start requires ws"))?;
            let params = StartParams {
                argv: None,
                owner: Some(super::terminal_tool::owner_async(executor, context.session_id).await?),
                process_id: context
                    .session_id
                    .zip(context.call_id)
                    .map(|(session, call)| crate::managed_process_id(session, call)),
                command: String::new(),
                ws: Some(MonitorWsParams {
                    url: ws.url.clone(),
                    protocols: ws.protocols.clone(),
                }),
                description: description.clone(),
                workdir: executor.workspace_root().to_path_buf(),
                timeout_ms: *timeout_ms,
                persistent: false,
                monitored: true,
                watch: None,
                include_pattern: None,
                success_pattern: None,
                failure_pattern: None,
                quiet_period_ms: None,
                max_buffered_lines: None,
                capture_stderr: true,
                env: Default::default(),
            };
            let registry = executor
                .monitor_registry()
                .cloned()
                .ok_or_else(|| ToolError::invalid_input("monitor registry unavailable"))?;
            let permit = super::shell_tool::launch_worker_permit(executor).await?;
            let started = super::terminal_tool::blocking(executor, move |_cancel| {
                let _permit = permit;
                registry.start(params).map_err(into_tool_error)
            })
            .await?;
            Ok(render_start(started))
        }
        MonitorToolInput::Stop { monitor_id } => {
            super::shell_tool::execute_async(
                executor,
                &crate::part::ShellToolInput::Stop {
                    process_id: monitor_id.clone(),
                },
                context,
            )
            .await
        }
    }
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
        "WebSocket subscription started ({}) — bounded event batches arrive as `system_notification` messages. Keep working; do not poll or sleep.",
        summary.process_id
    );
    let mut view = ToolExecutionView::simple(
        "Subscribe to WebSocket",
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
