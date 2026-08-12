use super::replies::{operation_from_part, operation_id_from_part};
use super::{
    AppError, AttachmentItem, ExecutionControlError, HashSet, PermissionAction, PermissionMode,
    PermissionReplyKind, PermissionScope, PersistedPermissionRule, ResolvedPendingTool,
    RunAbortReason, SessionManager, SessionPendingTool, TimeRange, ToolError, ToolInvocation,
    ToolInvocationExecution, ToolOutput, UserInputReplyKind, Utc,
};
use crate::session::Session;
use agena_domain::{PermissionReply, UserInputReply, UserInputRequest};
use agena_tool::ToolHumanRenderer;

/// Default lifetime of an interactive user-input request when the caller does
/// not specify `auto_resolution_ms`. Every interactive request is bounded so a
/// host/plugin `ask_user` (for example the workflow plan approval) can never
/// wedge a session forever when no client replies.
pub(super) const DEFAULT_USER_INPUT_TIMEOUT_MS: u64 = 10 * 60 * 1000;

/// Hard ceiling applied to any caller-supplied `auto_resolution_ms`. A plugin
/// may choose a shorter deadline, never a longer one.
pub(super) const MAX_USER_INPUT_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1000;

/// Resolve the effective auto-resolution deadline for a user-input request.
/// `None` becomes the system default and every value is capped, so the
/// invariant "no interactive request blocks indefinitely" holds at the
/// session-manager layer regardless of the calling plugin or tool.
pub(super) fn effective_user_input_timeout_ms(requested: Option<u64>) -> Option<u64> {
    Some(
        requested
            .unwrap_or(DEFAULT_USER_INPUT_TIMEOUT_MS)
            .min(MAX_USER_INPUT_TIMEOUT_MS),
    )
}

/// Derive the human-facing detail Markdown from a compact tool result payload.
///
/// This is the runtime's single derivation entry: the durable record stores
/// only the compact `data` (serialized `ToolResult`), and the human view is
/// produced here via the unified renderer (`ToolResultRender`). It is called
/// at snapshot load and on lazy detail fetches, never during persistence.
///
/// `tool_name` lets the renderer reconstruct the `ToolPayloadOutput`
/// discriminant (the compact payload is stored without its `tool` tag).
/// `command` (when known) lets the human view show `$ command` for shell runs.
pub(super) fn derive_operation_markdown(
    tool_name: &str,
    data: &serde_json::Value,
    command: Option<&str>,
) -> String {
    crate::tool::render_tool_payload_markdown_with_name(
        tool_name,
        data,
        &crate::tool::RenderContext {
            workspace_root: std::path::Path::new(""),
            live_tail: None,
            command,
            read_managed: &|path| std::fs::read_to_string(path).ok(),
        },
    )
}

pub(super) fn execution_control_to_app_error(err: ExecutionControlError) -> AppError {
    match err {
        ExecutionControlError::NoActiveExecution(id) => AppError::NoActiveExecution(id),
        ExecutionControlError::AlreadyActive(id) => AppError::ExecutionAlreadyActive(id),
        ExecutionControlError::SteerClosed => {
            AppError::Internal("steer channel closed for session".to_string())
        }
        ExecutionControlError::InvalidTransition(message) => AppError::Internal(message),
    }
}

pub(super) fn run_abort_reason(error: &AppError) -> RunAbortReason {
    match error {
        AppError::Cancelled => RunAbortReason::UserCancelled,
        AppError::Provider(_)
        | AppError::ProviderClassified { .. }
        | AppError::HttpStatus { .. }
        | AppError::Http(_)
        | AppError::EmptyResponse => RunAbortReason::ProviderError,
        _ => RunAbortReason::Internal,
    }
}

/// Resolve a pending tool ref to its decoded operation payload. The tool part
/// is the durable `tool_call` part itself; the operation (with its call id,
/// invocation, advertised identity, and lifecycle) rides in the part's
/// canonical content under `extra.operation` (v2 has no in-memory message
/// record — the store is the single source of truth).
pub(super) fn resolve_pending_tool(
    session: &Session,
    pending_tool: &SessionPendingTool,
) -> Result<ResolvedPendingTool, AppError> {
    let normalized_part = session
        .resolve_part_ref(&pending_tool.part)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "pending tool part not found: part={}",
                pending_tool.part.part_id
            ))
        })?;
    let normalized_pending = SessionPendingTool {
        part: normalized_part,
    };
    let part = session.part(&normalized_pending.part).ok_or_else(|| {
        AppError::Internal(format!(
            "pending tool part not found: part={}",
            normalized_pending.part.part_id
        ))
    })?;
    let operation = operation_from_part(part).ok_or_else(|| {
        AppError::Internal(format!(
            "pending tool payload missing: part={}",
            normalized_pending.part.part_id
        ))
    })?;
    let advertised_tool_identity = operation.advertised_tool_identity().map(ToOwned::to_owned);

    Ok(ResolvedPendingTool {
        pending: normalized_pending,
        operation_id: operation_id_from_part(part).unwrap_or_default(),
        call_id: operation.call_id,
        invocation: operation.invocation,
        advertised_tool_identity,
        prepared_shell_command: None,
        lifecycle: operation.lifecycle,
        session_runtime: session.runtime.clone(),
    })
}

pub(super) fn operation_blocks_from_tool_output(
    invocation: &ToolInvocation,
    details: &ToolOutput,
    attachments: &[AttachmentItem],
    output_text: &str,
) -> Vec<agena_domain::ViewBlock> {
    // Human view ownership lives with the tools crate: the built-in renderer
    // maps the structured payload to ViewBlocks (the same contract TUI/Web
    // consume for activity v2). The legacy per-tool human-block derivation is
    // removed.
    let _ = attachments;
    let command = invocation
        .input
        .get("command")
        .and_then(|value| value.as_text())
        .map(ToOwned::to_owned);
    let cwd = invocation
        .input
        .get("workdir")
        .and_then(|value| value.as_text())
        .map(ToOwned::to_owned);
    let mut renderer = crate::tool::human_view::BuiltinHumanRenderer::new(invocation.name.as_str());
    if let Some(command) = command {
        renderer = renderer.with_command(command);
    }
    if let Some(cwd) = cwd {
        renderer = renderer.with_cwd(cwd);
    }
    let raw = agena_domain::RawOutput {
        payload: details.to_json_payload(),
        text: output_text.to_owned(),
        ..agena_domain::RawOutput::default()
    };
    let ctx = agena_tool::RenderContext {
        workspace_root: std::path::PathBuf::new(),
        command: None,
    };
    renderer
        .render_human(&ctx, &raw)
        .unwrap_or_else(|_| crate::activity::projection::fallback_human_view(&raw))
}
pub(super) fn payload_tool_name_for_invocation(invocation: &ToolInvocation) -> String {
    crate::tool::ToolPayloadInput::from_invocation(invocation)
        .map(|payload| payload.tool_name().to_string())
        .unwrap_or_else(|| invocation.name.clone())
}

pub(super) fn custom_payload_value(details: &ToolOutput) -> Option<serde_json::Value> {
    details.to_json_payload()
}

/// Whether a completed tool execution launched work that keeps running after
/// the tool call returned (a monitored shell process or a delegated task).
/// Such operations must keep rendering in-progress on their transcript part
/// until the background work actually finishes; the returned marker correlates
/// the part with the runtime completion signal.
pub(super) fn background_operation_from_execution(
    invocation: &ToolInvocation,
    details: &ToolOutput,
) -> Option<crate::part::BackgroundOperation> {
    // Delegated task: `agena.tasks.create` returns immediately while the child
    // session keeps running in the background. The generated task id lives in
    // the output payload (`tasks[0].task_id`), not the input (the caller may
    // omit `task_id` and let the tool generate one).
    if matches!(
        invocation.name.as_str(),
        "agena.tasks.create" | "tasks.create" | "agena_tasks_create"
    ) {
        let task_id = custom_payload_value(details)
            .and_then(|value| value.get("tasks").cloned())
            .and_then(|value| value.as_array().cloned())
            .and_then(|tasks| tasks.into_iter().next())
            .and_then(|task| task.get("task_id").cloned())
            .and_then(|task_id| task_id.as_str().map(str::to_owned));
        return task_id.map(|task_id| crate::part::BackgroundOperation {
            kind: "task".to_string(),
            id: task_id,
        });
    }
    // Monitored shell process: the payload carries `action: "run"`,
    // `background: true`, and the process id.
    let payload_tool_name = payload_tool_name_for_invocation(invocation);
    if let Some(crate::tool::ToolPayloadOutput::Shell {
        action,
        background,
        process_id,
        ..
    }) = crate::tool::ToolPayloadOutput::from_tool_output(payload_tool_name.as_str(), details)
    {
        if action == "run" && background {
            if let Some(process_id) = process_id {
                return Some(crate::part::BackgroundOperation {
                    kind: "shell".to_string(),
                    id: process_id,
                });
            }
        }
    }
    None
}

/// Find the pending tool whose decoded operation carries `call_id`. v2 has no
/// in-memory message record — the durable `tool_call` part is the record and
/// the call id rides inside its operation payload, so this resolves by
/// decoding each pending tool part.
pub(super) fn pending_tool_by_call_id(
    session: &Session,
    call_id: i64,
) -> Option<SessionPendingTool> {
    session.pending_tools().into_iter().find(|pending| {
        session
            .part(&pending.part)
            .and_then(operation_from_part)
            .is_some_and(|operation| operation.call_id == call_id)
    })
}

pub(super) fn completed_lifecycle(lifecycle: &TimeRange) -> TimeRange {
    TimeRange {
        start_ms: lifecycle.start_ms,
        end_ms: Some(Utc::now().timestamp_millis()),
    }
}

pub(super) fn tool_name(invocation: &ToolInvocation) -> String {
    let ToolInvocation { name, .. } = invocation;
    name.clone()
}

pub(super) fn text_result_blocks(output_text: &str) -> Vec<agena_domain::ViewBlock> {
    if output_text.trim().is_empty() {
        Vec::new()
    } else {
        vec![agena_domain::ViewBlock::Text {
            id: None,
            text: output_text.to_string(),
        }]
    }
}

pub(super) fn persisted_mode_for_reply(kind: PermissionReplyKind) -> Option<PermissionMode> {
    match kind {
        PermissionReplyKind::AllowAlways => Some(PermissionMode::Allow),
        PermissionReplyKind::DenyAlways => Some(PermissionMode::Deny),
        PermissionReplyKind::AllowOnce | PermissionReplyKind::DenyOnce => None,
        // AutoApprove is downgraded before the reply is recorded; it never
        // persists a rule. Kept explicit for exhaustiveness.
        PermissionReplyKind::AutoApprove => None,
    }
}

pub(super) async fn persisted_rules_for_reply(
    manager: &SessionManager,
    session_id: i64,
    actions: &[PermissionAction],
    reply: &PermissionReply,
    operator: Option<&str>,
) -> Result<Vec<PersistedPermissionRule>, AppError> {
    let Some(mode) = persisted_mode_for_reply(reply.kind) else {
        return Ok(Vec::new());
    };
    let scope = reply.scope.unwrap_or(PermissionScope::Session);
    let workspace_id = match scope {
        PermissionScope::Session | PermissionScope::Global => None,
        PermissionScope::Workspace => Some(manager.current_workspace_id().await?),
    };
    let session_rule_id = match scope {
        PermissionScope::Session => Some(session_id),
        PermissionScope::Workspace | PermissionScope::Global => None,
    };
    let mut seen = HashSet::new();
    let mut rules = Vec::new();
    for action in actions {
        let action_key = permission_action_key(action)?;
        if !seen.insert(action_key.clone()) {
            continue;
        }
        rules.push(PersistedPermissionRule {
            id: None,
            created_at_ms: None,
            updated_at_ms: None,
            action_key,
            mode,
            scope,
            session_id: session_rule_id,
            workspace_id,
            source: "permission_reply".to_string(),
            reason: reply.reason.clone(),
            operator: operator.map(str::to_string),
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        });
    }
    Ok(rules)
}

pub(super) fn permission_action_key(action: &PermissionAction) -> Result<String, AppError> {
    serde_json::to_string(action).map_err(AppError::from)
}

pub(super) fn tool_error_to_app_error(err: ToolError) -> AppError {
    match err {
        ToolError::Cancelled | ToolError::Shell(agena_tool::ShellError::Cancelled) => {
            AppError::Cancelled
        }
        ToolError::PolicyDenied(denial) => AppError::PolicyDenied(denial),
        ToolError::UserDeclined(decline) => AppError::UserDeclined(decline),
        ToolError::CapabilityUnavailable(unavailable) => {
            AppError::CapabilityUnavailable(unavailable)
        }
        ToolError::ToolUnavailable(unavailable) => AppError::ToolUnavailable(unavailable),
        ToolError::UserInputRequired(_) => {
            AppError::Internal("unexpected unresolved user input request".to_string())
        }
        other => AppError::Tool(Box::new(other)),
    }
}

pub(super) fn ask_user_title(request: &UserInputRequest) -> String {
    if !request.title.trim().is_empty() {
        return request.title.trim().to_string();
    }
    match request.questions.len() {
        0 => "Ask user".to_string(),
        1 => {
            let header = request.questions[0].header.trim();
            if header.is_empty() {
                "Ask user".to_string()
            } else {
                format!("Ask: {header}")
            }
        }
        count => format!("Ask user ({count})"),
    }
}

/// Build the opaque correlation id for a host `ask_user` request.
///
/// The `host-input:{session}:{call}:{seq}` string is user-visible (it
/// round-trips through reply payloads and present URLs) and baked into stored
/// rows and e2e fixtures, so its format is stable. It is now an OPAQUE id only:
/// it carries no semantic weight — origin (host `ask_user` vs plugin
/// `interaction.ask`) is the typed [`agena_domain::UserInputSource`] carried on
/// the request, never a prefix check on this id.
pub(super) fn host_user_input_request_id(
    session_id: i64,
    call_id: i64,
    sequence_index: usize,
) -> String {
    format!("host-input:{session_id}:{call_id}:{sequence_index}")
}

pub(super) fn user_input_execution(
    request: &UserInputRequest,
    reply: &UserInputReply,
) -> Result<ToolInvocationExecution, AppError> {
    let answers = validate_user_input_reply(request, reply)?;
    let mut lines = vec!["Answers:".to_string()];
    for (index, _question) in request.questions.iter().enumerate() {
        if let Some(answer) = answers.get(index.to_string().as_str()) {
            lines.push(format!("- {index}: {}", answer.join(", ")));
        }
    }

    let selection_count: usize = answers.values().map(Vec::len).sum();
    let mut view = crate::tool::ToolExecutionView::simple(
        "Ask user",
        format!("{selection_count} answers"),
        lines.join("\n"),
    );
    view.metadata
        .insert("answer_count".to_string(), selection_count.to_string());
    view.metadata.insert(
        "question_count".to_string(),
        request.questions.len().to_string(),
    );

    Ok(ToolInvocationExecution::new(
        crate::tool::ToolPayloadOutput::AskUser {
            answers,
            timed_out: false,
        }
        .into_tool_output(),
        view,
    ))
}

pub(super) fn host_user_input_response(
    request: &UserInputRequest,
    reply: &UserInputReply,
) -> Result<agena_plugin_host::sdk::host_api::AskUserResponse, AppError> {
    match reply.kind {
        UserInputReplyKind::Cancel => Ok(agena_plugin_host::sdk::host_api::AskUserResponse {
            reply: reply.reason.clone().unwrap_or_default(),
            cancelled: true,
            timed_out: false,
            answers: Default::default(),
        }),
        UserInputReplyKind::Timeout => Ok(agena_plugin_host::sdk::host_api::AskUserResponse {
            reply: reply.reason.clone().unwrap_or_default(),
            cancelled: false,
            timed_out: true,
            answers: Default::default(),
        }),
        UserInputReplyKind::Submit => {
            let answers = validate_user_input_reply(request, reply)?;
            if request.questions.is_empty() {
                return Err(AppError::Internal(
                    "host user input request is missing its question".to_string(),
                ));
            }
            let answer = answers
                .get("0")
                .and_then(|values| values.first())
                .cloned()
                .ok_or_else(|| {
                    AppError::Internal(
                        "host user input reply missing answer for question 0".to_string(),
                    )
                })?;
            Ok(agena_plugin_host::sdk::host_api::AskUserResponse {
                reply: answer,
                cancelled: false,
                timed_out: false,
                answers,
            })
        }
    }
}

pub(super) fn validate_user_input_reply(
    request: &UserInputRequest,
    reply: &UserInputReply,
) -> Result<std::collections::BTreeMap<String, Vec<String>>, AppError> {
    let mut answers = std::collections::BTreeMap::new();

    for (index, question) in request.questions.iter().enumerate() {
        let raw_answers = reply
            .answers
            .get(index.to_string().as_str())
            .cloned()
            .ok_or_else(|| {
                AppError::Internal(format!("missing answer for user input question {index}"))
            })?;
        let mut normalized = Vec::new();
        for value in raw_answers {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !normalized
                .iter()
                .any(|existing: &String| existing == trimmed)
            {
                normalized.push(trimmed.to_string());
            }
        }

        if normalized.is_empty() {
            return Err(AppError::Internal(format!(
                "missing answer for user input question {index}"
            )));
        }
        if !question.multiple && normalized.len() != 1 {
            return Err(AppError::Internal(format!(
                "question {index} accepts exactly one answer"
            )));
        }

        let allowed = question
            .options
            .iter()
            .map(|option| option.label.trim())
            .filter(|label| !label.is_empty())
            .collect::<std::collections::HashSet<_>>();
        if !question.allow_custom
            && let Some(answer) = normalized
                .iter()
                .find(|value| !allowed.contains(value.as_str()))
        {
            return Err(AppError::Internal(format!(
                "unsupported answer '{}' for question {index}",
                answer
            )));
        }

        answers.insert(index.to_string(), normalized);
    }

    for answer_id in reply.answers.keys() {
        let is_valid_index = answer_id
            .parse::<usize>()
            .is_ok_and(|index| index < request.questions.len());
        if !is_valid_index {
            return Err(AppError::Internal(format!(
                "unexpected answer for unknown user input question {answer_id}"
            )));
        }
    }

    Ok(answers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ToolPayloadOutput;

    fn invocation_named(name: &str) -> ToolInvocation {
        ToolInvocation::new(name, agena_domain::StructuredObject::default())
    }

    fn output(payload: ToolPayloadOutput) -> ToolOutput {
        payload.into_tool_output()
    }

    #[test]
    fn glob_produces_a_human_markdown_list_not_a_flat_blob() {
        let invocation = invocation_named("glob");
        let details = output(ToolPayloadOutput::Glob {
            count: Some(2),
            paths: vec!["src/a.rs".to_owned(), "src/b.rs".to_owned()],
            truncated: false,
        });
        let blocks =
            operation_blocks_from_tool_output(&invocation, &details, &[], "src/a.rs\nsrc/b.rs");
        assert!(blocks.iter().any(|block| matches!(
            block,
            agena_domain::ViewBlock::Markdown { text, .. }
                if text.contains("- src/a.rs") && text.contains("- src/b.rs")
        )));
        // No raw text blob duplicated alongside the list.
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, agena_domain::ViewBlock::Text { .. }))
        );
    }

    #[test]
    fn directory_read_produces_a_markdown_list() {
        let invocation = invocation_named("read");
        let details = output(ToolPayloadOutput::Read {
            preview: Some("src/\ntarget/".to_owned()),
            truncated: false,
            loaded_paths: vec![".".to_owned()],
            attachment: None,
        });
        let blocks = operation_blocks_from_tool_output(&invocation, &details, &[], "src/\ntarget/");
        assert!(blocks.iter().any(|block| matches!(
            block,
            agena_domain::ViewBlock::Markdown { text, .. }
                if text.contains("### preview") && text.contains("src/")
        )));
    }

    #[test]
    fn file_read_stays_a_plain_text_card() {
        let invocation = invocation_named("read");
        let details = output(ToolPayloadOutput::Read {
            preview: Some("1: fn main()".to_owned()),
            truncated: false,
            loaded_paths: vec!["main.rs".to_owned()],
            attachment: None,
        });
        let blocks = operation_blocks_from_tool_output(&invocation, &details, &[], "1: fn main()");
        // Numbered file preview renders as a Markdown preview card.
        assert!(blocks.iter().any(|block| matches!(
            block,
            agena_domain::ViewBlock::Markdown { text, .. }
                if text.contains("### preview") && text.contains("1: fn main()")
        )));
    }

    #[test]
    fn cron_list_produces_a_human_markdown_list() {
        let invocation = invocation_named("cron_list");
        let job = agena_tool::CronJobSummary {
            id: "job-1".to_owned(),
            kind: "cron".to_owned(),
            expression: Some("0 9 * * *".to_owned()),
            at: None,
            prompt: "run backup".to_owned(),
            next_fire_at: Some("2026-08-05T09:00:00Z".to_owned()),
            last_fired_at: None,
            paused: false,
            completed: false,
            misfire_policy: String::new(),
            retry_max_attempts: 0,
            retry_at: None,
            run_count: 0,
            last_run_status: None,
            last_run_failure: None,
        };
        let details = output(ToolPayloadOutput::CronList { jobs: vec![job] });
        let blocks = operation_blocks_from_tool_output(&invocation, &details, &[], "1 job(s)");
        assert!(blocks.iter().any(|block| matches!(
            block,
            agena_domain::ViewBlock::Markdown { text, .. }
                if text.contains("### cron jobs") && text.contains("run backup")
        )));
    }

    #[test]
    fn lsp_diagnostics_render_with_severity_markers() {
        let invocation = invocation_named("lsp_diagnostics");
        let details = output(ToolPayloadOutput::LspDiagnostics {
            entries: vec![
                "src/main.rs:3:5 [error] undefined name".to_owned(),
                "src/main.rs:7:2 [warning] unused import".to_owned(),
            ],
        });
        let blocks = operation_blocks_from_tool_output(&invocation, &details, &[], "2 diagnostics");
        assert!(blocks.iter().any(|block| matches!(
            block,
            agena_domain::ViewBlock::Markdown { text, .. }
                if text.contains("### diagnostics")
                    && text.contains("[error]")
                    && text.contains("[warning]")
        )));
    }

    #[test]
    fn user_input_timeout_defaults_and_caps_are_bounded() {
        // No deadline requested: the system default applies so an interactive
        // request can never block the host forever.
        assert_eq!(
            effective_user_input_timeout_ms(None),
            Some(DEFAULT_USER_INPUT_TIMEOUT_MS)
        );
        // Caller-specified deadlines are honored below the ceiling.
        assert_eq!(effective_user_input_timeout_ms(Some(1_000)), Some(1_000));
        assert_eq!(
            effective_user_input_timeout_ms(Some(DEFAULT_USER_INPUT_TIMEOUT_MS)),
            Some(DEFAULT_USER_INPUT_TIMEOUT_MS)
        );
        // Anything above the ceiling is clamped so a plugin cannot create an
        // effectively unbounded request.
        assert_eq!(
            effective_user_input_timeout_ms(Some(MAX_USER_INPUT_TIMEOUT_MS + 1)),
            Some(MAX_USER_INPUT_TIMEOUT_MS)
        );
        assert_eq!(
            effective_user_input_timeout_ms(Some(u64::MAX)),
            Some(MAX_USER_INPUT_TIMEOUT_MS)
        );
    }
}
