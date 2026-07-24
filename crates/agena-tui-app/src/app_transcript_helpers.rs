pub(crate) fn message_sort_key(message: &MessageResource) -> (i64, i64) {
    (message.created_at.timestamp_millis(), message.id)
}

pub(crate) fn merge_message_resources(
    current: &MessageResource,
    incoming: &MessageResource,
) -> MessageResource {
    let mut merged = if incoming.updated_at >= current.updated_at {
        incoming.clone()
    } else {
        current.clone()
    };

    let current_parts_score = message_parts_score(current.parts.as_ref());
    let incoming_parts_score = message_parts_score(incoming.parts.as_ref());
    merged.parts = if (incoming.parts.is_none() && current.parts.is_some())
        || current_parts_score > incoming_parts_score
    {
        current.parts.clone()
    } else {
        incoming.parts.clone()
    };

    if message_status_rank(current.state) > message_status_rank(merged.state) {
        merged.state = current.state;
    }
    if current.updated_at > merged.updated_at {
        merged.updated_at = current.updated_at;
    }
    if merged.usage.is_none() {
        merged.usage = current.usage.clone();
    }
    if let Some(parts) = merged.parts.as_mut() {
        parts.sort_by_key(|part| part.part_index);
        merged.part_count = parts.len() as u64;
    } else {
        merged.part_count = merged.part_count.max(current.part_count);
    }

    merged
}

pub(crate) fn message_parts_score(parts: Option<&Vec<MessagePartResource>>) -> usize {
    parts
        .map(|parts| parts.iter().map(message_part_score).sum())
        .unwrap_or(0)
}

pub(crate) fn message_part_score(part: &MessagePartResource) -> usize {
    let mut score = 0;
    if part.content.is_some() {
        score += 1;
    }
    if part
        .summary
        .as_deref()
        .is_some_and(|summary| !summary.trim().is_empty())
    {
        score += 4;
    }
    match part.content.as_ref() {
        Some(MessagePartDetailResource::Text(text)) if !text.text.trim().is_empty() => score += 16,
        Some(MessagePartDetailResource::Reasoning(reasoning))
            if !reasoning.summary.is_empty() || !reasoning.raw_content.is_empty() =>
        {
            score += 16;
        }
        Some(MessagePartDetailResource::Operation(operation)) => {
            if !operation.model_output.text.trim().is_empty() {
                score += 16;
            } else if !operation.title.trim().is_empty() || operation.error.is_some() {
                score += 8;
            }
        }
        Some(MessagePartDetailResource::Attachment(_))
        | Some(MessagePartDetailResource::Request(_))
        | Some(MessagePartDetailResource::Error(_)) => {
            score += 16;
        }
        _ => {}
    }
    score
}

pub(crate) fn message_status_rank(status: MessageStatus) -> u8 {
    match status {
        MessageStatus::Pending => 0,
        MessageStatus::InProgress => 1,
        MessageStatus::Completed => 2,
        MessageStatus::Failed => 3,
        MessageStatus::Cancelled => 4,
    }
}

pub(crate) fn assistant_message_text(message: &MessageResource) -> Option<String> {
    let parts = message.parts.as_ref()?;
    let text = parts
        .iter()
        .filter_map(|part| match part.content.as_ref()? {
            MessagePartDetailResource::Text(text) if !text.synthetic && !text.ignored => {
                let trimmed = text.text.trim();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}

pub(crate) fn rewind_message_composer_text(message: &MessageResource) -> String {
    message
        .parts
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|part| match part.content.as_ref()? {
            MessagePartDetailResource::Text(text)
                if !text.synthetic && !text.ignored && !text.text.trim().is_empty() =>
            {
                Some(text.text.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn pending_interactive_kind_from_request(
    request: &PendingInteractiveRequestResource,
) -> PendingInteractiveKind {
    match &request.request {
        PendingInteractiveRequest::Permission { .. } => PendingInteractiveKind::Permission,
        PendingInteractiveRequest::UserInput { .. } => PendingInteractiveKind::UserInput,
    }
}

pub(crate) fn pending_interactive_request_id(request: &PendingInteractiveRequestResource) -> &str {
    request.request.request_id()
}

pub(crate) fn pending_interactive_request_matches_kind(
    request: &PendingInteractiveRequestResource,
    kind: PendingInteractiveKind,
) -> bool {
    pending_interactive_kind_from_request(request) == kind
}

pub(crate) fn pending_interactive_request_is_seen(
    request: &PendingInteractiveRequestResource,
    seen_permission_request_ids: &BTreeSet<String>,
    seen_user_input_request_ids: &BTreeSet<String>,
) -> bool {
    let request_id = pending_interactive_request_id(request);
    match pending_interactive_kind_from_request(request) {
        PendingInteractiveKind::Permission => seen_permission_request_ids.contains(request_id),
        PendingInteractiveKind::UserInput => seen_user_input_request_ids.contains(request_id),
    }
}

pub(crate) fn first_unseen_pending_interactive_request<'a>(
    requests: &'a [PendingInteractiveRequestResource],
    seen_permission_request_ids: &BTreeSet<String>,
    seen_user_input_request_ids: &BTreeSet<String>,
) -> Option<&'a PendingInteractiveRequestResource> {
    requests.iter().find(|request| {
        !pending_interactive_request_is_seen(
            request,
            seen_permission_request_ids,
            seen_user_input_request_ids,
        )
    })
}

pub(crate) fn first_pending_interactive_request_by_kind(
    requests: &[PendingInteractiveRequestResource],
    kind: PendingInteractiveKind,
) -> Option<&PendingInteractiveRequestResource> {
    requests
        .iter()
        .find(|request| pending_interactive_request_matches_kind(request, kind))
}

pub(crate) fn pending_interactive_kind(
    requests: &[PendingInteractiveRequestResource],
) -> Option<PendingInteractiveKind> {
    requests.first().map(pending_interactive_kind_from_request)
}

pub(crate) fn pending_interactive_kind_for_execution(
    execution: &SessionExecutionResource,
) -> Option<PendingInteractiveKind> {
    pending_interactive_kind(execution.pending_interactive_requests.as_slice())
}

pub(crate) fn execution_update_is_stale(
    current_latest_event_seq: Option<i64>,
    incoming_latest_event_seq: Option<i64>,
) -> bool {
    match (current_latest_event_seq, incoming_latest_event_seq) {
        (Some(current), Some(incoming)) => incoming < current,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

pub(crate) fn permission_overlay_matches_pending_request(
    overlay: &PermissionOverlay,
    _session_id: Option<i64>,
    execution: Option<&SessionExecutionResource>,
) -> bool {
    execution.is_some_and(|execution| {
        execution
            .pending_interactive_requests
            .iter()
            .filter(|request| request.session_id == overlay.session_id)
            .filter_map(|request| request.request.as_permission())
            .any(|request| request.request_id == overlay.request.request_id)
    })
}

pub(crate) fn user_input_overlay_matches_pending_request(
    overlay: &UserInputOverlay,
    _session_id: Option<i64>,
    execution: Option<&SessionExecutionResource>,
) -> bool {
    execution.is_some_and(|execution| {
        execution
            .pending_interactive_requests
            .iter()
            .filter(|request| request.session_id == overlay.session_id)
            .filter_map(|request| request.request.as_user_input())
            .any(|request| request.request_id == overlay.request.request_id)
    })
}

pub(crate) fn execution_pending_flash_key(
    execution: &SessionExecutionResource,
) -> Option<&'static str> {
    match execution
        .pending_interactive_requests
        .first()
        .map(|resource| &resource.request)
    {
        Some(PendingInteractiveRequest::Permission { .. }) => {
            Some("flash-session-awaiting-approval")
        }
        Some(PendingInteractiveRequest::UserInput { .. }) => {
            Some("flash-session-awaiting-user-input")
        }
        None => None,
    }
}

pub(crate) fn pending_interactive_counts_for_execution(
    execution: &SessionExecutionResource,
) -> (usize, usize) {
    execution.pending_interactive_requests.iter().fold(
        (0, 0),
        |(permission_count, user_input_count), request| match &request.request {
            PendingInteractiveRequest::Permission { .. } => {
                (permission_count + 1, user_input_count)
            }
            PendingInteractiveRequest::UserInput { .. } => (permission_count, user_input_count + 1),
        },
    )
}

pub(crate) fn composer_input_is_active(
    focus: Focus,
    has_text_or_items: bool,
    has_auxiliary_input_ui: bool,
) -> bool {
    focus == Focus::Composer && (has_text_or_items || has_auxiliary_input_ui)
}

pub(crate) fn preferred_visible_session_selection(
    session: &SessionResource,
    visible_sessions: &[agena_tui_session::session_list::SessionListItem],
) -> Option<i64> {
    [
        Some(session.id),
        session.parent_id,
        (session.root_id != session.id).then_some(session.root_id),
    ]
    .into_iter()
    .flatten()
    .find(|candidate| {
        visible_sessions
            .iter()
            .any(|item| item.session_id == *candidate)
    })
}

pub(crate) fn permission_request_fingerprint(request: &PermissionRequest) -> String {
    json!({
        "action": &request.action,
        "related_actions": &request.related_actions,
        "requested_actions": &request.requested_actions,
        "reason": &request.reason,
        "explanation": &request.explanation,
        "source": &request.source,
        "scope": &request.scope,
        "operator": &request.operator,
        "risk": request.risk,
        "trace": &request.trace,
    })
    .to_string()
}

pub(crate) fn permission_overlay_choice(
    page: PermissionPromptPage,
    selected: usize,
) -> PermissionOverlayChoice {
    match page {
        PermissionPromptPage::Action => match selected {
            0 => PermissionOverlayChoice::OpenScope(PermissionPromptDecision::Allow),
            1 => PermissionOverlayChoice::OpenScope(PermissionPromptDecision::Deny),
            2 => PermissionOverlayChoice::EditRule,
            _ => PermissionOverlayChoice::Details,
        },
        PermissionPromptPage::Scope(PermissionPromptDecision::Allow) => match selected {
            0 => PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::AllowOnce,
                scope: None,
            },
            1 => PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::AllowAlways,
                scope: Some(PermissionScope::Session),
            },
            2 => PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::AllowAlways,
                scope: Some(PermissionScope::Workspace),
            },
            3 => PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::AllowAlways,
                scope: Some(PermissionScope::Global),
            },
            _ => PermissionOverlayChoice::Details,
        },
        PermissionPromptPage::Scope(PermissionPromptDecision::Deny) => match selected {
            0 => PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::DenyOnce,
                scope: None,
            },
            1 => PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::DenyAlways,
                scope: Some(PermissionScope::Session),
            },
            2 => PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::DenyAlways,
                scope: Some(PermissionScope::Workspace),
            },
            3 => PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::DenyAlways,
                scope: Some(PermissionScope::Global),
            },
            _ => PermissionOverlayChoice::Details,
        },
        PermissionPromptPage::Details(_) => {
            unreachable!("permission details do not have selectable choices")
        }
    }
}

pub(crate) fn permission_overlay_reply_label(
    i18n: &I18n,
    kind: PermissionReplyKind,
    scope: Option<PermissionScope>,
) -> String {
    match (kind, scope) {
        (PermissionReplyKind::AllowAlways, Some(PermissionScope::Session)) => {
            ui_text::t(i18n, "overlay-permission-choice-allow-always-session")
        }
        (PermissionReplyKind::AllowAlways, Some(PermissionScope::Workspace)) => {
            ui_text::t(i18n, "overlay-permission-choice-allow-always-workspace")
        }
        (PermissionReplyKind::AllowAlways, Some(PermissionScope::Global)) => {
            ui_text::t(i18n, "overlay-permission-choice-allow-always-global")
        }
        (PermissionReplyKind::DenyAlways, Some(PermissionScope::Session)) => {
            ui_text::t(i18n, "overlay-permission-choice-deny-always-session")
        }
        (PermissionReplyKind::DenyAlways, Some(PermissionScope::Workspace)) => {
            ui_text::t(i18n, "overlay-permission-choice-deny-always-workspace")
        }
        (PermissionReplyKind::DenyAlways, Some(PermissionScope::Global)) => {
            ui_text::t(i18n, "overlay-permission-choice-deny-always-global")
        }
        _ => ui_text::permission_reply_label(i18n, kind),
    }
}

pub(crate) fn permission_action_label(i18n: &I18n, action: &PermissionAction) -> String {
    match action {
        PermissionAction::Tool {
            tool_name,
            qualifier,
        } => {
            let base = i18n.text_args(
                "overlay-permission-action-tool",
                &agena_tui::fl_args!("tool" => tool_name.clone()),
            );
            qualifier
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("{base} · {value}"))
                .unwrap_or(base)
        }
        PermissionAction::PathAccess {
            access_kind,
            target_path,
            ..
        } => i18n.text_args(
            "overlay-permission-action-path",
            &agena_tui::fl_args!(
                "access" => access_kind.clone(),
                "path" => target_path.clone(),
            ),
        ),
        PermissionAction::NetworkAccess { target, host, port } => i18n.text_args(
            "overlay-permission-action-network",
            &agena_tui::fl_args!(
                "target" => if target.trim().is_empty() {
                    match port {
                        Some(port) => format!("{host}:{port}"),
                        None => host.clone(),
                    }
                } else {
                    target.clone()
                }
            ),
        ),
    }
}

pub(crate) fn permission_requested_actions_for_display<'a>(
    primary: Option<&'a PermissionAction>,
    requested: &'a [PermissionAction],
) -> Vec<&'a PermissionAction> {
    if requested.is_empty() {
        return Vec::new();
    }
    if requested.len() == 1 && primary.is_some_and(|primary| requested.first() == Some(primary)) {
        return Vec::new();
    }
    requested.iter().collect()
}

pub(crate) fn permission_related_actions_for_display<'a>(
    primary: Option<&'a PermissionAction>,
    related: &'a [PermissionAction],
    requested: &'a [PermissionAction],
) -> Vec<&'a PermissionAction> {
    related
        .iter()
        .filter(|action| {
            !primary.is_some_and(|primary| *action == primary)
                && !requested.iter().any(|candidate| candidate == *action)
        })
        .collect()
}
use crate::{
    BTreeSet, I18n, MessagePartDetailResource, MessagePartResource, MessageResource, MessageStatus,
    PendingInteractiveKind, PendingInteractiveRequest, PendingInteractiveRequestResource,
    PermissionAction, PermissionOverlay, PermissionOverlayChoice, PermissionPromptDecision,
    PermissionPromptPage, PermissionReplyKind, PermissionRequest, PermissionScope,
    SessionExecutionResource, SessionResource, UserInputOverlay, json, ui_text,
};
use agena_tui::main_focus::Focus;
