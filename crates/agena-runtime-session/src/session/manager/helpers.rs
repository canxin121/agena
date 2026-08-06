use super::{
    AppError, AttachmentItem, ExecutionControlError, ExecutionStatus, HashSet, HistoryToolCallId,
    Message, MessageMetadata, MessagePart, OperationBlock, PartContent, PermissionAction,
    PermissionMode, PermissionReplyKind, PermissionScope, PersistedPermissionRule, RequestPart,
    ReservedMessageIds, ResolvedPendingTool, Role, RunAbortReason, SessionPendingTool,
    SessionStore, TimeRange, ToolError, ToolInvocation, ToolInvocationExecution, ToolOutput,
    UserInputReplyKind, Utc,
};
use crate::session::Session;
use agena_domain::{PermissionReply, UserInputReply, UserInputRequest};

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

pub(super) fn tool_call_id_for(resolved: &ResolvedPendingTool) -> HistoryToolCallId {
    HistoryToolCallId::new(resolved.operation_id.clone())
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

    let activity_id = session
        .part(&normalized_pending.part)
        .and_then(|part| part.activity_id);

    Ok(ResolvedPendingTool {
        pending: normalized_pending,
        operation_id: record.operation_id,
        call_id: record.call_id,
        invocation: record.invocation,
        advertised_tool_identity: record.advertised_tool_identity,
        prepared_shell_command: None,
        lifecycle: record.lifecycle,
        session_runtime: session.runtime.clone(),
        activity_id,
    })
}

pub(super) fn operation_blocks_from_tool_output(
    invocation: &ToolInvocation,
    details: &ToolOutput,
    attachments: &[AttachmentItem],
    output_text: &str,
) -> Vec<OperationBlock> {
    let payload_tool_name = payload_tool_name_for_invocation(invocation);

    // Structured human blocks take precedence over the flat text blob: shell
    // commands, file diffs, path lists, and diagnostics each render as a
    // first-class visual card. Only tools without a structured presentation
    // fall back to the raw text.
    let mut blocks = Vec::new();

    // Shell/process executions render as a command card: the command line and
    // its output, exit code, and stderr shown as a distinct human block.
    if let Some(block) = shell_command_block_from_invocation(invocation, details) {
        blocks.push(block);
    }

    match crate::tool::ToolPayloadOutput::from_tool_output(payload_tool_name.as_str(), details) {
        Some(crate::tool::ToolPayloadOutput::ApplyPatch { changes, diff, .. })
            if !changes.is_empty() || !diff.trim().is_empty() =>
        {
            if !changes.is_empty() {
                blocks.push(OperationBlock::FileChanges { changes });
            }
            // The unified diff is the human view of what changed before/after;
            // show it as a dedicated diff card rather than hiding it in the
            // structured payload.
            if !diff.trim().is_empty() {
                blocks.push(OperationBlock::Diff {
                    diff,
                    language: Some("diff".to_owned()),
                });
            }
        }
        // Path-list tools (glob, directory reads) render as human-readable
        // Markdown lists rather than a flat text blob so the terminal shows
        // each path as a first-class line.
        Some(crate::tool::ToolPayloadOutput::Glob { .. }) => {
            if let Some(markdown) = path_list_human_block(payload_tool_name.as_str(), details) {
                blocks.push(markdown);
            }
        }
        Some(crate::tool::ToolPayloadOutput::Read { .. }) => {
            if let Some(markdown) = directory_read_human_block(payload_tool_name.as_str(), details)
            {
                blocks.push(markdown);
            }
        }
        Some(crate::tool::ToolPayloadOutput::CronList { .. }) => {
            if let Some(markdown) = cron_list_human_block(payload_tool_name.as_str(), details) {
                blocks.push(markdown);
            }
        }
        Some(crate::tool::ToolPayloadOutput::ToolSearch { .. }) => {
            if let Some(markdown) = tool_search_human_block(payload_tool_name.as_str(), details) {
                blocks.push(markdown);
            }
        }
        Some(crate::tool::ToolPayloadOutput::LspDiagnostics { .. }) => {
            if let Some(markdown) = lsp_diagnostics_human_block(payload_tool_name.as_str(), details)
            {
                blocks.push(markdown);
            }
        }
        _ => {}
    }

    if let Some(block) = structured_web_search_results_block(payload_tool_name.as_str(), details) {
        blocks.push(block);
    }
    if let Some(block) = structured_web_crawl_results_block(payload_tool_name.as_str(), details) {
        blocks.push(block);
    }

    for block in crate::message::tool_output_content_blocks(details) {
        blocks.push(block);
    }

    for attachment in attachments {
        blocks.push(OperationBlock::Media {
            mime_type: attachment.mime.clone(),
            artifact: agena_domain::ArtifactRef {
                uri: attachment_source_uri(&attachment.source),
                mime: attachment.mime.clone(),
                name: attachment
                    .filename
                    .clone()
                    .or_else(|| attachment.title.clone()),
                size_bytes: attachment.size_bytes,
                sha256: attachment.sha256.clone(),
            },
        });
    }

    // Only fall back to the flat text when no structured presentation was
    // produced, so glob/search/diagnostic lists are not duplicated.
    if blocks.is_empty() && !output_text.trim().is_empty() {
        blocks.extend(text_result_blocks(output_text));
    }

    dedupe_operation_blocks(blocks)
}

/// Build a `Command` human block for a shell execution. The command line is
/// read from the invocation input so the human view shows `$ command` plus the
/// captured stdout/stderr and exit code instead of a bare text blob.
fn shell_command_block_from_invocation(
    invocation: &ToolInvocation,
    details: &ToolOutput,
) -> Option<OperationBlock> {
    let payload_tool_name = payload_tool_name_for_invocation(invocation);
    let action =
        match crate::tool::ToolPayloadOutput::from_tool_output(payload_tool_name.as_str(), details)
        {
            Some(crate::tool::ToolPayloadOutput::Shell { action, .. }) => action,
            _ => return None,
        };
    if action != "run" {
        return None;
    }
    // Extract the command line from the invocation input.
    let command = invocation
        .input
        .get("command")
        .and_then(|value| value.as_text())
        .map(ToOwned::to_owned);
    let command = match command {
        Some(command) if !command.trim().is_empty() => command,
        _ => return None,
    };
    let cwd = invocation
        .input
        .get("workdir")
        .and_then(|value| value.as_text())
        .map(ToOwned::to_owned);
    let (exit_code, stdout, stderr) =
        match crate::tool::ToolPayloadOutput::from_tool_output(payload_tool_name.as_str(), details)
        {
            Some(crate::tool::ToolPayloadOutput::Shell {
                exit_code, output, ..
            }) => (exit_code, output.clone(), None),
            _ => (None, None, None),
        };
    Some(OperationBlock::Command {
        command,
        cwd,
        exit_code,
        stdout,
        stderr,
    })
}

pub(super) fn payload_tool_name_for_invocation(invocation: &ToolInvocation) -> String {
    crate::tool::ToolPayloadInput::from_invocation(invocation)
        .map(|payload| payload.tool_name().to_string())
        .unwrap_or_else(|| invocation.name.clone())
}

pub(super) fn custom_payload_value(details: &ToolOutput) -> Option<serde_json::Value> {
    details.to_json_payload()
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct StructuredWebSearchPayload {
    query: String,
    #[serde(default)]
    results: Vec<StructuredWebSearchResult>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct StructuredWebSearchResult {
    title: String,
    url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    snippet: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct StructuredWebCrawlPayload {
    #[serde(default)]
    documents: Vec<StructuredWebCrawlDocument>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct StructuredWebCrawlDocument {
    title: String,
    url: String,
    #[serde(default)]
    depth: u32,
    #[serde(default)]
    chunk_count: usize,
}

pub(super) fn structured_web_search_results_block(
    payload_tool_name: &str,
    details: &ToolOutput,
) -> Option<OperationBlock> {
    if payload_tool_name != "web_search" {
        return None;
    }

    let payload: StructuredWebSearchPayload =
        serde_json::from_value(custom_payload_value(details)?).ok()?;
    if payload.results.is_empty() {
        return None;
    }

    Some(OperationBlock::SearchResults {
        query: Some(payload.query),
        results: payload
            .results
            .into_iter()
            .map(|result| {
                let snippet = result
                    .snippet
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| {
                        let description = result.description.trim();
                        (!description.is_empty()).then(|| description.to_string())
                    });
                agena_domain::SearchResultItem {
                    title: result.title,
                    uri: result.url,
                    snippet,
                    score: None,
                }
            })
            .collect(),
    })
}

pub(super) fn structured_web_crawl_results_block(
    payload_tool_name: &str,
    details: &ToolOutput,
) -> Option<OperationBlock> {
    if payload_tool_name != "web.crawl"
        && payload_tool_name != "agena_web__crawl"
        && payload_tool_name != "crawl"
    {
        return None;
    }

    let payload: StructuredWebCrawlPayload =
        serde_json::from_value(custom_payload_value(details)?).ok()?;
    if payload.documents.is_empty() {
        return None;
    }

    Some(OperationBlock::SearchResults {
        query: None,
        results: payload
            .documents
            .into_iter()
            .map(|document| agena_domain::SearchResultItem {
                title: document.title,
                uri: document.url,
                snippet: Some(format!(
                    "depth {} · {} chunk(s)",
                    document.depth, document.chunk_count
                )),
                score: None,
            })
            .collect(),
    })
}

/// Human-friendly Markdown list for glob results.
///
/// Path lists are inherently ordered and line-oriented; a Markdown list keeps
/// each path as a first-class visual row instead of a flat text blob.
fn path_list_human_block(payload_tool_name: &str, details: &ToolOutput) -> Option<OperationBlock> {
    let crate::tool::ToolPayloadOutput::Glob {
        paths, truncated, ..
    } = crate::tool::ToolPayloadOutput::from_tool_output(payload_tool_name, details)?
    else {
        return None;
    };
    if paths.is_empty() {
        return None;
    }
    let mut body = paths
        .iter()
        .map(|path| format!("- `{path}`"))
        .collect::<Vec<_>>()
        .join("\n");
    if truncated {
        body.push_str("\n_…more matches available_");
    }
    Some(OperationBlock::Markdown { text: body })
}

/// Human-friendly Markdown list for directory reads.
///
/// Directory listings store a newline-joined preview of entry names, while
/// file previews carry `N: line` numbering. A listing with no line-number
/// pattern is a directory and renders as a Markdown list.
fn directory_read_human_block(
    payload_tool_name: &str,
    details: &ToolOutput,
) -> Option<OperationBlock> {
    let crate::tool::ToolPayloadOutput::Read {
        preview: Some(preview),
        truncated,
        ..
    } = crate::tool::ToolPayloadOutput::from_tool_output(payload_tool_name, details)?
    else {
        return None;
    };
    let preview = preview.trim();
    if preview.is_empty() {
        return None;
    }
    // File previews always number their lines (`1: content`); directory
    // listings are bare entry names. A single-entry directory has no newline
    // and no `N:` prefix, so fall through to the list branch.
    let looks_like_file = preview.lines().any(|line| {
        let Some((left, _)) = line.split_once(':') else {
            return false;
        };
        left.trim().parse::<u32>().is_ok()
    });
    if looks_like_file {
        return None;
    }
    let entries = preview.lines().collect::<Vec<_>>();
    let mut body = entries
        .iter()
        .map(|entry| format!("- `{entry}`"))
        .collect::<Vec<_>>()
        .join("\n");
    if truncated {
        body.push_str("\n_…more entries available_");
    }
    Some(OperationBlock::Markdown { text: body })
}

/// Human-friendly Markdown for scheduled-job listings.
fn cron_list_human_block(payload_tool_name: &str, details: &ToolOutput) -> Option<OperationBlock> {
    let crate::tool::ToolPayloadOutput::CronList { jobs } =
        crate::tool::ToolPayloadOutput::from_tool_output(payload_tool_name, details)?
    else {
        return None;
    };
    if jobs.is_empty() {
        return None;
    }
    let body = jobs
        .iter()
        .map(|job| {
            let state = if job.paused {
                "⏸ paused"
            } else if job.completed {
                "✓ completed"
            } else {
                "▶ active"
            };
            let next = job
                .next_fire_at
                .as_deref()
                .map(|next| format!(" · next {next}"))
                .unwrap_or_default();
            let expr = job
                .expression
                .as_deref()
                .map(|expr| format!(" `{expr}`"))
                .unwrap_or_default();
            format!("- **{state}**{expr}{next} · {}", job.prompt)
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(OperationBlock::Markdown { text: body })
}

/// Human-friendly Markdown for tool-search results.
fn tool_search_human_block(
    payload_tool_name: &str,
    details: &ToolOutput,
) -> Option<OperationBlock> {
    let crate::tool::ToolPayloadOutput::ToolSearch { results } =
        crate::tool::ToolPayloadOutput::from_tool_output(payload_tool_name, details)?
    else {
        return None;
    };
    if results.is_empty() {
        return None;
    }
    let body = results
        .iter()
        .map(|name| format!("- `{name}`"))
        .collect::<Vec<_>>()
        .join("\n");
    Some(OperationBlock::Markdown { text: body })
}

/// Human-friendly Markdown for LSP diagnostics, using severity markers.
fn lsp_diagnostics_human_block(
    payload_tool_name: &str,
    details: &ToolOutput,
) -> Option<OperationBlock> {
    let crate::tool::ToolPayloadOutput::LspDiagnostics { entries } =
        crate::tool::ToolPayloadOutput::from_tool_output(payload_tool_name, details)?
    else {
        return None;
    };
    if entries.is_empty() {
        return None;
    }
    let body = entries
        .iter()
        .map(|entry| {
            if entry.contains("[error]") {
                format!("- ❌ `{entry}`")
            } else if entry.contains("[warning]") {
                format!("- ⚠️ `{entry}`")
            } else {
                format!("- `{entry}`")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(OperationBlock::Markdown { text: body })
}

pub(super) fn attachment_source_uri(source: &crate::message::AttachmentSource) -> String {
    match source {
        crate::message::AttachmentSource::Url { url }
        | crate::message::AttachmentSource::DataUrl { url } => url.clone(),
        crate::message::AttachmentSource::LocalPath { path } => path.clone(),
        crate::message::AttachmentSource::Base64 { .. } => {
            "data:application/octet-stream;base64".to_string()
        }
        crate::message::AttachmentSource::FileId { file_id } => format!("file:{file_id}"),
    }
}

pub(super) fn dedupe_operation_blocks(blocks: Vec<OperationBlock>) -> Vec<OperationBlock> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::with_capacity(blocks.len());
    for block in blocks {
        let key = serde_json::to_string(&block).unwrap_or_else(|_| format!("{:?}", block));
        if seen.insert(key) {
            deduped.push(block);
        }
    }
    deduped
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

pub(super) fn text_result_blocks(output_text: &str) -> Vec<OperationBlock> {
    if output_text.trim().is_empty() {
        Vec::new()
    } else {
        vec![OperationBlock::Text {
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
    store: &SessionStore,
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
        PermissionScope::Workspace => Some(store.current_workspace_id().await?),
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

pub(super) fn permission_scope_label(scope: PermissionScope) -> String {
    match scope {
        PermissionScope::Session => "session".to_string(),
        PermissionScope::Workspace => "workspace".to_string(),
        PermissionScope::Global => "global".to_string(),
    }
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
    for question in &request.questions {
        if let Some(answer) = answers.get(question.id.as_str()) {
            lines.push(format!("- {}: {}", question.id, answer.join(", ")));
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
            let question = request.questions.first().ok_or_else(|| {
                AppError::Internal("host user input request is missing its question".to_string())
            })?;
            let answer = answers
                .get(question.id.as_str())
                .and_then(|values| values.first())
                .cloned()
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "host user input reply missing answer for question {}",
                        question.id
                    ))
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

    for question in &request.questions {
        let raw_answers = reply
            .answers
            .get(question.id.as_str())
            .cloned()
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "missing answer for user input question {}",
                    question.id
                ))
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
                "missing answer for user input question {}",
                question.id
            )));
        }
        if !question.multiple && normalized.len() != 1 {
            return Err(AppError::Internal(format!(
                "question {} accepts exactly one answer",
                question.id
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
                "unsupported answer '{}' for question {}",
                answer, question.id
            )));
        }

        answers.insert(question.id.clone(), normalized);
    }

    for answer_id in reply.answers.keys() {
        if !request
            .questions
            .iter()
            .any(|question| question.id == *answer_id)
        {
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
            OperationBlock::Markdown { text }
                if text.contains("- `src/a.rs`") && text.contains("- `src/b.rs`")
        )));
        // No raw text blob duplicated alongside the list.
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, OperationBlock::Text { .. }))
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
            OperationBlock::Markdown { text } if text.contains("- `src/`")
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
        // Numbered file preview is not a directory list.
        assert!(
            !blocks
                .iter()
                .any(|block| matches!(block, OperationBlock::Markdown { .. }))
        );
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block, OperationBlock::Text { .. }))
        );
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
            OperationBlock::Markdown { text } if text.contains("▶ active") && text.contains("run backup")
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
            OperationBlock::Markdown { text }
                if text.contains("❌") && text.contains("⚠️")
        )));
    }
}
