use super::{
    AppError, AttachmentItem, ExecutionControlError, ExecutionStatus, HashSet, Message,
    MessageMetadata, MessagePart, PartContent, PermissionAction, PermissionMode,
    PermissionReplyKind, PermissionScope, PersistedPermissionRule, RequestPart, ReservedMessageIds,
    ResolvedPendingTool, Role, RunAbortReason, SessionManager, SessionPendingTool, TimeRange,
    ToolError, ToolInvocation, ToolInvocationExecution, ToolOutput, UserInputReplyKind, Utc,
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

pub(super) fn build_message(
    ids: ReservedMessageIds,
    role: Role,
    message_state: ExecutionStatus,
    parts: Vec<PartContent>,
    metadata: MessageMetadata,
) -> Result<Message, AppError> {
    // Every part needs a reserved id; a caller that reserves fewer than it
    // builds used to index past the end of `part_ids` and abort the process.
    // Enforce the invariant here so a mismatch surfaces as an internal error
    // instead of a crash.
    if ids.part_ids.len() < parts.len() {
        return Err(AppError::Internal(format!(
            "reserved {} part ids for {} parts (message {})",
            ids.part_ids.len(),
            parts.len(),
            ids.message_id
        )));
    }
    let created_at = Utc::now();
    let parts = parts
        .into_iter()
        .enumerate()
        .map(|(idx, content)| {
            MessagePart::from_content_with_index(
                ids.part_ids[idx],
                ids.message_id,
                idx as i32,
                created_at,
                part_status(&content),
                content,
            )
        })
        .collect();
    Ok(Message {
        id: ids.message_id,
        role,
        state: message_state,
        parts,
        created_at,
        metadata,
        provider_state: None,
        usage: None,
    })
}

pub(super) fn resolve_pending_tool(
    session: &Session,
    pending_tool: &SessionPendingTool,
) -> Result<ResolvedPendingTool, AppError> {
    let normalized_part = session
        .resolve_part_ref(&pending_tool.part)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "pending tool part not found: message={}, part={}",
                pending_tool.part.message_id, pending_tool.part.part_id
            ))
        })?;
    let normalized_pending = SessionPendingTool {
        part: normalized_part,
    };
    let record = session
        .pending_tool_record(&normalized_pending)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "pending tool payload missing: message={}, part={}",
                pending_tool.part.message_id, pending_tool.part.part_id
            ))
        })?;

    Ok(ResolvedPendingTool {
        pending: normalized_pending,
        operation_id: record.operation_id,
        call_id: record.call_id,
        invocation: record.invocation,
        advertised_tool_identity: record.advertised_tool_identity,
        prepared_shell_command: None,
        lifecycle: record.lifecycle,
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

pub(super) fn part_status(content: &PartContent) -> ExecutionStatus {
    match content {
        PartContent::Activity(crate::message::RuntimeActivity::Operation(tool)) => tool.status(),
        PartContent::Activity(crate::message::RuntimeActivity::Interaction(
            RequestPart::UserInput(request),
        )) => request.status(),
        _ => ExecutionStatus::Completed,
    }
}

pub(super) fn build_request_part(
    part_id: i64,
    message_id: i64,
    operation_id: &str,
    request: RequestPart,
) -> MessagePart {
    let mut part = MessagePart::from_content(
        part_id,
        message_id,
        Utc::now(),
        request.status(),
        PartContent::Activity(crate::message::RuntimeActivity::Interaction(request)),
    );
    part.operation_id = Some(operation_id.to_string());
    part
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
    fn build_message_rejects_fewer_reserved_part_ids_than_parts() {
        // The original crash: indexing past the end of part_ids aborted the
        // process. The guard must turn the mismatch into a clean error.
        let ids = ReservedMessageIds {
            message_id: 7,
            part_ids: vec![100],
        };
        let err = build_message(
            ids,
            Role::User,
            ExecutionStatus::Completed,
            vec![PartContent::text("one"), PartContent::text("two")],
            MessageMetadata::default(),
        )
        .expect_err("fewer reserved ids than parts must error");
        match err {
            AppError::Internal(message) => {
                assert!(message.contains("1 part ids"), "got: {message}");
                assert!(message.contains("2 parts"), "got: {message}");
            }
            other => panic!("expected internal error, got {other:?}"),
        }
    }

    #[test]
    fn build_message_succeeds_with_exactly_matching_ids() {
        let ids = ReservedMessageIds {
            message_id: 8,
            part_ids: vec![200, 201],
        };
        let message = build_message(
            ids,
            Role::User,
            ExecutionStatus::Completed,
            vec![PartContent::text("one"), PartContent::text("two")],
            MessageMetadata::default(),
        )
        .expect("matching ids build cleanly");
        assert_eq!(message.id, 8);
        assert_eq!(message.parts.len(), 2);
        assert_eq!(message.parts[0].id, 200);
        assert_eq!(message.parts[1].id, 201);
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
