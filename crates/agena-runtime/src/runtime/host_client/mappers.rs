pub(super) fn host_unavailable(message: impl Into<String>) -> PluginError {
    PluginError::from_kind(
        agena_plugin_host::sdk::PluginErrorKind::HostUnavailable,
        message.into(),
    )
}

pub(super) fn tool_execution_to_invoke_output(
    execution: crate::tool::ToolInvocationExecution,
) -> ToolInvokeOutput {
    let summary = execution.summary();
    ToolInvokeOutput {
        title: summary.title,
        output_text: summary.output_text,
        payload: summary.payload,
        metadata: summary.metadata.into_iter().collect(),
        attachments: execution.view.attachments,
    }
}

pub(super) fn map_storage_error(err: PluginStorageError) -> PluginError {
    use agena_plugin_host::sdk::PluginErrorKind;
    match err {
        PluginStorageError::MissingSessionId
        | PluginStorageError::MissingWorkspaceRoot
        | PluginStorageError::EmptyNamespace
        | PluginStorageError::EmptyKey
        | PluginStorageError::Data(_) => PluginError::invalid_params(err.to_string()),
        PluginStorageError::SecretUnavailable(_) => {
            PluginError::from_kind(PluginErrorKind::HostUnavailable, err)
        }
        PluginStorageError::Io(_) | PluginStorageError::Secret(_) => {
            PluginError::internal(err.to_string())
        }
    }
}

pub(super) fn host_permission_check_response_from_resolution(
    resolution: agena_domain::PermissionResolution,
) -> HostPermissionCheckResponse {
    let outcome = host_permission_outcome_from_decision(&resolution.decision);
    let (decision, reason) = plugin_permission_decision_and_reason(resolution.decision);
    HostPermissionCheckResponse {
        decision,
        outcome,
        reason,
        explanation: resolution.explanation,
        details: None,
    }
}

pub(super) fn host_permission_check_response_from_decision(
    decision: agena_domain::PermissionDecision,
) -> HostPermissionCheckResponse {
    let outcome = host_permission_outcome_from_decision(&decision);
    let (decision, reason) = plugin_permission_decision_and_reason(decision);
    let explanation = reason
        .clone()
        .unwrap_or_else(|| "permission allowed by current policy".to_string());
    HostPermissionCheckResponse {
        decision,
        outcome,
        reason,
        explanation,
        details: None,
    }
}

fn host_permission_outcome_from_decision(
    decision: &agena_domain::PermissionDecision,
) -> agena_plugin_host::sdk::host_api::HostPermissionOutcome {
    match decision {
        agena_domain::PermissionDecision::Allow => {
            agena_plugin_host::sdk::host_api::HostPermissionOutcome::Allowed
        }
        agena_domain::PermissionDecision::Ask { .. } => {
            agena_plugin_host::sdk::host_api::HostPermissionOutcome::ApprovalRequired
        }
        agena_domain::PermissionDecision::Deny { .. } => {
            agena_plugin_host::sdk::host_api::HostPermissionOutcome::PolicyDenied
        }
    }
}

pub(super) fn plugin_permission_decision_and_reason(
    decision: agena_domain::PermissionDecision,
) -> (PluginPermissionDecision, Option<String>) {
    match decision {
        agena_domain::PermissionDecision::Allow => (PluginPermissionDecision::Allow, None),
        agena_domain::PermissionDecision::Ask { reason } => {
            (PluginPermissionDecision::Prompt, Some(reason))
        }
        agena_domain::PermissionDecision::Deny { reason } => {
            (PluginPermissionDecision::Deny, Some(reason))
        }
    }
}

pub(super) fn render_tool_descriptor(
    tool: agena_plugin_host::registry::RegisteredTool,
) -> ToolDescriptor {
    let brief_summary = tool.summary_text().map(ToString::to_string);
    let mut help_parts = Vec::new();
    if let Some(before_help) = tool.before_help_text() {
        help_parts.push(before_help.to_string());
    }
    if let Some(help) = tool.help_text() {
        help_parts.push(help.to_string());
    }
    if let Some(after_help) = tool.after_help_text() {
        help_parts.push(after_help.to_string());
    }
    let help = (!help_parts.is_empty()).then(|| help_parts.join("\n\n"));
    let summary = match tool.definition.preferred_description_mode() {
        Some(agena_plugin_host::ToolDescriptionMode::Detailed) => {
            let mut parts = brief_summary.into_iter().collect::<Vec<_>>();
            if let Some(help) = help.as_deref()
                && !parts.iter().any(|part| part.trim() == help.trim())
            {
                parts.push(help.to_owned());
            }
            (!parts.is_empty()).then(|| parts.join("\n\n"))
        }
        Some(agena_plugin_host::ToolDescriptionMode::Brief) | None => brief_summary,
    };
    let input_schema = Some(tool.input_schema());
    ToolDescriptor {
        name: crate::tool::compact_tool_call_name(tool.canonical_name().as_str()),
        summary,
        help,
        examples: tool.definition.model.examples.clone(),
        input_schema,
    }
}

pub(super) fn render_monitor_handle(summary: agena_domain::ProcessSummary) -> MonitorHandle {
    MonitorHandle {
        id: summary.process_id,
        label: (!summary.description.trim().is_empty()).then_some(summary.description),
        command: (!summary.command.trim().is_empty()).then_some(summary.command),
        status: Some(
            match summary.status {
                ProcessStatus::Running => "running",
                ProcessStatus::Exited => "exited",
                ProcessStatus::Failed => "failed",
                ProcessStatus::Stopped => "stopped",
                ProcessStatus::TimedOut => "timed_out",
            }
            .to_string(),
        ),
        persistent: summary.background,
        monitored: summary.monitored,
        started_at_ms: summary.started_at_ms,
        ended_at_ms: summary.ended_at_ms,
        buffered_lines: summary.buffered_lines,
        last_seq: summary.last_seq,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
        completion_reason: summary.completion_reason,
    }
}

pub(super) fn render_monitor_event(event: agena_domain::ProcessEvent) -> MonitorEvent {
    MonitorEvent {
        seq: event.seq,
        stream: event.stream.to_string(),
        ts_ms: event.ts_ms,
        line: event.line,
    }
}

pub(super) fn render_monitor_read(read: crate::tool::MonitorRead) -> MonitorReadResponse {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let events = read
        .events
        .into_iter()
        .map(|event| {
            match event.stream {
                ProcessStream::Stdout => stdout.push(event.line.clone()),
                ProcessStream::Stderr => stderr.push(event.line.clone()),
            }
            render_monitor_event(event)
        })
        .collect::<Vec<_>>();
    MonitorReadResponse {
        monitor_id: Some(read.monitor_id),
        events,
        monitors: Vec::new(),
        stdout: stdout.join("\n"),
        stderr: stderr.join("\n"),
        running: matches!(read.status, ProcessStatus::Running),
        status: Some(read.status.to_string()),
        last_seq: read.last_seq,
        has_more: read.has_more,
        dropped_lines: read.dropped_lines,
        exit_code: read.exit_code,
        completion_reason: read.completion_reason,
    }
}

pub(super) fn map_monitor_error(err: MonitorError) -> PluginError {
    match err {
        MonitorError::NotFound(_) | MonitorError::Invalid(_) | MonitorError::InvalidPattern(_) => {
            PluginError::invalid_params(err.to_string())
        }
        other => PluginError::internal(other.to_string()),
    }
}

pub(super) fn join_monitor_command(command: &[String]) -> Result<String, PluginError> {
    if command.is_empty() {
        return Err(PluginError::invalid_params(
            "monitor_start requires at least one command token",
        ));
    }
    Ok(command.join(" "))
}

pub(super) fn ask_user_tool_input(req: AskUserRequest) -> Result<AskUserToolInput, PluginError> {
    if !req.questions.is_empty() {
        let questions = req
            .questions
            .into_iter()
            .map(|question| UserInputQuestion {
                id: question.id,
                header: question.header,
                question: question.question,
                options: question
                    .options
                    .into_iter()
                    .map(|option| UserInputOption {
                        label: option.label,
                        description: option.description,
                        preview_markdown: option.preview_markdown,
                    })
                    .collect(),
                multiple: question.multiple,
                allow_custom: question.allow_custom,
            })
            .collect();
        let input = AskUserToolInput {
            title: req.title,
            body_markdown: req.body_markdown,
            kind: req.kind,
            submit_label: req.submit_label,
            cancel_label: req.cancel_label,
            auto_resolution_ms: req.auto_resolution_ms,
            questions,
        };
        return AskUserToolInput::parse_input(
            serde_json::to_value(input)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
        );
    }

    let options = req
        .options
        .into_iter()
        .map(|label| UserInputOption {
            label,
            description: String::new(),
            preview_markdown: String::new(),
        })
        .collect();
    let input = AskUserToolInput {
        title: req.title,
        body_markdown: req.body_markdown,
        kind: req.kind,
        submit_label: req.submit_label,
        cancel_label: req.cancel_label,
        auto_resolution_ms: req.auto_resolution_ms,
        questions: vec![UserInputQuestion {
            id: "reply".to_string(),
            header: String::new(),
            question: req.prompt,
            options,
            multiple: false,
            allow_custom: req.allow_free_text,
        }],
    };
    AskUserToolInput::parse_input(
        serde_json::to_value(input).map_err(|err| PluginError::invalid_params(err.to_string()))?,
    )
}

pub(super) fn host_session_from_session(session: &crate::session::Session) -> HostSession {
    HostSession {
        id: session.id,
        parent_id: session.parent_id,
        root_id: session.root_id,
        workspace_id: session.workspace_id,
        title: session.title.clone(),
        is_subagent: session.is_subagent(),
    }
}



pub(super) fn scheduler_job_to_sdk(job: agena_scheduler::ScheduledJob) -> HostSchedulerJob {
    let (kind, cron_expression, fire_at_ms) = match &job.kind {
        agena_scheduler::JobKind::Cron { expression, .. } => {
            ("cron".to_string(), Some(expression.clone()), None)
        }
        agena_scheduler::JobKind::Once { at } => {
            ("once".to_string(), None, Some(at.timestamp_millis()))
        }
    };
    HostSchedulerJob {
        id: job.id.to_string(),
        kind,
        prompt: job.prompt.clone(),
        cron_expression,
        fire_at_ms,
        owner_session_id: job.owner_session_id,
        next_fire_at_ms: job.next_fire_at.map(|t| t.timestamp_millis()),
        last_fired_at_ms: job.last_fired_at.map(|t| t.timestamp_millis()),
    }
}

pub(super) fn lsp_severity_string(
    severity: Option<agena_lsp::lsp_types::DiagnosticSeverity>,
) -> String {
    match severity {
        Some(agena_lsp::lsp_types::DiagnosticSeverity::ERROR) => "error".to_string(),
        Some(agena_lsp::lsp_types::DiagnosticSeverity::WARNING) => "warning".to_string(),
        Some(agena_lsp::lsp_types::DiagnosticSeverity::INFORMATION) => "information".to_string(),
        Some(agena_lsp::lsp_types::DiagnosticSeverity::HINT) => "hint".to_string(),
        Some(_) => "unknown".to_string(),
        None => "unknown".to_string(),
    }
}

pub(super) fn host_status_to_sdk(
    status: agena_plugin_host::status::PluginStatus,
) -> HostPluginStatus {
    HostPluginStatus {
        plugin_id: status.plugin_id,
        kind: status.kind.to_string(),
        state: status.state.to_string(),
        pid: status.pid,
        restart_count: status.restart_count,
        last_exit_code: status.last_exit_code,
        last_restart_at_ms: status.last_restart_at_ms,
        last_failure: status.last_failure,
    }
}

use super::{
    AskUserRequest, AskUserToolInput, HostPermissionCheckResponse, HostPluginStatus,
    HostSchedulerJob, HostSession, MonitorError, MonitorEvent, MonitorHandle, MonitorReadResponse,
    PluginError, PluginPermissionDecision, PluginStorageError, ToolDescriptor, ToolInvokeOutput,
    UserInputOption, UserInputQuestion,
};
use agena_domain::{ProcessStatus, ProcessStream};
use agena_plugin_sdk::ToolInput;
