use super::{
    AppError, AttachmentItem, ExecutionStatus, HOST_PERMISSION_REQUEST_SEQUENCE, HashSet,
    HistoryToolCallId, IpAddr, Message, MessageMetadata, MessagePart, MessageStatus,
    OperationBlock, Ordering, PartContent, PermissionAction, PermissionDecision, PermissionMode,
    PermissionReply, PermissionReplyKind, PermissionRiskLevel, PermissionScope,
    PersistedPermissionRule, RequestPart, ReservedMessageIds, ResolvedPendingTool, Role,
    RunControlError, SessionPendingTool, SessionStore, TimeRange, ToolError, ToolInvocation,
    ToolInvocationExecution, ToolOutput, UserInputReply, UserInputReplyKind, UserInputRequest, Utc,
};
use crate::session::Session;

pub(super) fn host_permission_grant_matches_action(
    granted_actions: &[PermissionAction],
    requested_action: &PermissionAction,
) -> bool {
    if granted_actions
        .iter()
        .any(|granted| granted == requested_action)
    {
        return true;
    }

    // A plugin can resolve an approved hostname before opening its connection.
    // Keep the explicit approval bound to this invocation, but do not ask again
    // merely because the resolver reports the corresponding public IP address.
    // Private and loopback addresses never use this shortcut.
    is_public_network_access(requested_action)
        && granted_actions
            .iter()
            .any(|granted| matches!(granted, PermissionAction::NetworkAccess { .. }))
}

pub(super) fn is_public_network_access(action: &PermissionAction) -> bool {
    let PermissionAction::NetworkAccess { host, .. } = action else {
        return false;
    };
    let Ok(address) = host.parse::<IpAddr>() else {
        return false;
    };
    match address {
        IpAddr::V4(address) => {
            !address.is_private()
                && !address.is_loopback()
                && !address.is_link_local()
                && address.octets()[0] != 0
                && address.octets()[0] < 224
        }
        IpAddr::V6(address) => {
            !address.is_loopback()
                && !address.is_unique_local()
                && !address.is_unicast_link_local()
                && !address.is_unspecified()
        }
    }
}

pub(super) fn permission_subject(action: &PermissionAction) -> serde_json::Value {
    match action {
        PermissionAction::Tool { tool_name, .. } => {
            serde_json::json!({
                "kind": "tool",
                "tool_name": tool_name,
            })
        }
        PermissionAction::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        } => serde_json::json!({
            "kind": "path_access",
            "access_kind": access_kind,
            "workspace_root": workspace_root,
            "target_path": target_path,
        }),
        PermissionAction::NetworkAccess { target, host, port } => serde_json::json!({
            "kind": "network_access",
            "target": target,
            "host": host,
            "port": port,
        }),
    }
}

pub(super) fn run_control_to_app_error(err: RunControlError) -> AppError {
    match err {
        RunControlError::NoActiveRun(id) => {
            AppError::Internal(format!("no in-flight run for session {id}"))
        }
        RunControlError::SteerClosed => {
            AppError::Internal("steer channel closed for session".to_string())
        }
    }
}

pub(super) fn is_user_cancelled_error(err: &AppError) -> bool {
    matches!(err, AppError::Internal(message) if message == "run cancelled by user")
}

pub(super) fn build_message(
    ids: ReservedMessageIds,
    role: Role,
    message_state: MessageStatus,
    parts: Vec<PartContent>,
    metadata: MessageMetadata,
) -> Message {
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
    Message {
        id: ids.message_id,
        role,
        state: message_state,
        parts,
        created_at,
        metadata,
        provider_state: None,
        usage: None,
    }
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
) -> Vec<OperationBlock> {
    let mut blocks = text_result_blocks(output_text);

    let payload_tool_name = payload_tool_name_for_invocation(invocation);
    match crate::tool::ToolPayloadOutput::from_tool_output(payload_tool_name.as_str(), details) {
        Some(crate::tool::ToolPayloadOutput::ApplyPatch { changes, .. }) if !changes.is_empty() => {
            blocks.push(OperationBlock::FileChanges { changes });
        }
        _ => {}
    }

    if let Some(block) = structured_web_search_results_block(payload_tool_name.as_str(), details) {
        blocks.push(block);
    }
    if let Some(block) = structured_web_crawl_results_block(payload_tool_name.as_str(), details) {
        blocks.push(block);
    }

    for block in details.content_blocks() {
        blocks.push(block);
    }

    for attachment in attachments {
        blocks.push(OperationBlock::Media {
            mime_type: attachment.mime.clone(),
            artifact: crate::message::ArtifactRef {
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

    dedupe_operation_blocks(blocks)
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
                crate::message::SearchResultItem {
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
            .map(|document| crate::message::SearchResultItem {
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
        PartContent::Operation(tool) => tool.status(),
        PartContent::Request(RequestPart::Permission(permission)) => permission.status(),
        PartContent::Request(RequestPart::UserInput(request)) => request.status(),
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
        PartContent::Request(request),
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
        ToolError::PermissionDenied(reason) | ToolError::PermissionAsk(reason) => {
            AppError::Internal(reason)
        }
        ToolError::UserInputRequired(_) => {
            AppError::Internal("unexpected unresolved user input request".to_string())
        }
        other => AppError::Internal(other.to_string()),
    }
}

pub(super) fn apply_advisory_permission_decision(
    base: PermissionDecision,
    advice: crate::plugin::PermissionDecision,
    explanation: &str,
) -> PermissionDecision {
    match (base, advice) {
        (PermissionDecision::Deny { reason }, _) => PermissionDecision::Deny { reason },
        (_, crate::plugin::PermissionDecision::Deny) => PermissionDecision::Deny {
            reason: if explanation.trim().is_empty() {
                "denied by plugin advice".to_string()
            } else {
                explanation.to_string()
            },
        },
        (PermissionDecision::Ask { reason }, _) => PermissionDecision::Ask { reason },
        (PermissionDecision::Allow, crate::plugin::PermissionDecision::Prompt) => {
            PermissionDecision::Ask {
                reason: if explanation.trim().is_empty() {
                    "permission requires confirmation".to_string()
                } else {
                    explanation.to_string()
                },
            }
        }
        (PermissionDecision::Allow, crate::plugin::PermissionDecision::Allow) => {
            PermissionDecision::Allow
        }
    }
}

pub(super) fn risk_for_permission_decision(decision: &PermissionDecision) -> PermissionRiskLevel {
    match decision {
        PermissionDecision::Allow => PermissionRiskLevel::Low,
        PermissionDecision::Ask { .. } => PermissionRiskLevel::Medium,
        PermissionDecision::Deny { .. } => PermissionRiskLevel::High,
    }
}

pub(super) fn plugin_risk_to_core(
    risk: crate::plugin::sdk::PermissionRiskLevel,
) -> PermissionRiskLevel {
    match risk {
        crate::plugin::sdk::PermissionRiskLevel::Low => PermissionRiskLevel::Low,
        crate::plugin::sdk::PermissionRiskLevel::Medium => PermissionRiskLevel::Medium,
        crate::plugin::sdk::PermissionRiskLevel::High => PermissionRiskLevel::High,
        crate::plugin::sdk::PermissionRiskLevel::Critical => PermissionRiskLevel::Critical,
    }
}

pub(super) fn max_permission_risk(
    left: PermissionRiskLevel,
    right: PermissionRiskLevel,
) -> PermissionRiskLevel {
    if permission_risk_rank(left) >= permission_risk_rank(right) {
        left
    } else {
        right
    }
}

pub(super) fn permission_risk_rank(risk: PermissionRiskLevel) -> u8 {
    match risk {
        PermissionRiskLevel::Low => 0,
        PermissionRiskLevel::Medium => 1,
        PermissionRiskLevel::High => 2,
        PermissionRiskLevel::Critical => 3,
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

pub(super) fn host_permission_request_id(session_id: i64, call_id: i64) -> String {
    let sequence = HOST_PERMISSION_REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("host-permission:{session_id}:{call_id}:{sequence}")
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

    let mut view = crate::tool::ToolExecutionView::simple("Ask user", lines.join("\n"));
    let selection_count: usize = answers.values().map(Vec::len).sum();
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
) -> Result<crate::plugin::sdk::host_api::AskUserResponse, AppError> {
    match reply.kind {
        UserInputReplyKind::Cancel => Ok(crate::plugin::sdk::host_api::AskUserResponse {
            reply: reply.reason.clone().unwrap_or_default(),
            cancelled: true,
            timed_out: false,
            answers: Default::default(),
        }),
        UserInputReplyKind::Timeout => Ok(crate::plugin::sdk::host_api::AskUserResponse {
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
            Ok(crate::plugin::sdk::host_api::AskUserResponse {
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
