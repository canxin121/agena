pub(in crate::app) fn transcript_should_follow_tail(
    cursor_line: usize,
    line_count: usize,
    viewport_at_bottom: bool,
) -> bool {
    line_count == 0 || (viewport_at_bottom && cursor_line >= line_count.saturating_sub(1))
}

impl RenderedLine {
    pub(in crate::app) fn plain(text: impl Into<String>, style: Style) -> Self {
        let text = text.into();
        Self {
            rich_line: Some(Line::from(Span::styled(text.clone(), style))),
            text,
            style,
        }
    }

    pub(in crate::app) fn rich(line: Line<'static>) -> Self {
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let style = line.style;
        Self {
            text,
            style,
            rich_line: Some(line),
        }
    }

    pub(in crate::app) fn dim(text: impl Into<String>) -> Self {
        Self::plain(
            text,
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )
    }
}

pub(in crate::app) fn message_sort_key(message: &MessageResource) -> (i64, i64) {
    (message.created_at.timestamp_millis(), message.id)
}

pub(in crate::app) fn merge_message_resources(
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

pub(in crate::app) fn message_parts_score(parts: Option<&Vec<MessagePart>>) -> usize {
    parts
        .map(|parts| parts.iter().map(message_part_score).sum())
        .unwrap_or(0)
}

pub(in crate::app) fn message_part_score(part: &MessagePart) -> usize {
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
        Some(PartContent::Text(text)) if !text.text.trim().is_empty() => score += 16,
        Some(PartContent::Reasoning(reasoning))
            if !reasoning.summary.is_empty() || !reasoning.raw_content.is_empty() =>
        {
            score += 16;
        }
        Some(PartContent::Operation(operation)) => {
            if operation.output_text().is_some() {
                score += 16;
            } else if operation.title().is_some() || operation.error_message().is_some() {
                score += 8;
            }
        }
        Some(PartContent::Attachment(_))
        | Some(PartContent::Request(_))
        | Some(PartContent::Error(_)) => {
            score += 16;
        }
        _ => {}
    }
    score
}

pub(in crate::app) fn message_status_rank(status: MessageStatus) -> u8 {
    match status {
        MessageStatus::Pending => 0,
        MessageStatus::InProgress => 1,
        MessageStatus::Completed => 2,
        MessageStatus::Failed => 3,
        MessageStatus::Cancelled => 4,
    }
}

pub(in crate::app) fn assistant_message_text(message: &MessageResource) -> Option<String> {
    let parts = message.parts.as_ref()?;
    let text = parts
        .iter()
        .filter_map(|part| match part.content.as_ref()? {
            PartContent::Text(text) if !text.synthetic && !text.ignored => {
                let trimmed = text.text.trim();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}

pub(in crate::app) fn pending_interactive_kind_from_request(
    request: &PendingInteractiveRequest,
) -> PendingInteractiveKind {
    match request {
        PendingInteractiveRequest::Permission { .. } => PendingInteractiveKind::Permission,
        PendingInteractiveRequest::UserInput { .. } => PendingInteractiveKind::UserInput,
    }
}

pub(in crate::app) fn pending_interactive_request_id(request: &PendingInteractiveRequest) -> &str {
    request.request_id()
}

pub(in crate::app) fn pending_interactive_request_matches_kind(
    request: &PendingInteractiveRequest,
    kind: PendingInteractiveKind,
) -> bool {
    pending_interactive_kind_from_request(request) == kind
}

pub(in crate::app) fn pending_interactive_request_is_seen(
    request: &PendingInteractiveRequest,
    seen_permission_request_ids: &BTreeSet<String>,
    seen_user_input_request_ids: &BTreeSet<String>,
) -> bool {
    let request_id = pending_interactive_request_id(request);
    match pending_interactive_kind_from_request(request) {
        PendingInteractiveKind::Permission => seen_permission_request_ids.contains(request_id),
        PendingInteractiveKind::UserInput => seen_user_input_request_ids.contains(request_id),
    }
}

pub(in crate::app) fn first_unseen_pending_interactive_request<'a>(
    requests: &'a [PendingInteractiveRequest],
    seen_permission_request_ids: &BTreeSet<String>,
    seen_user_input_request_ids: &BTreeSet<String>,
) -> Option<&'a PendingInteractiveRequest> {
    requests.iter().find(|request| {
        !pending_interactive_request_is_seen(
            request,
            seen_permission_request_ids,
            seen_user_input_request_ids,
        )
    })
}

pub(in crate::app) fn first_pending_interactive_request_by_kind<'a>(
    requests: &'a [PendingInteractiveRequest],
    kind: PendingInteractiveKind,
) -> Option<&'a PendingInteractiveRequest> {
    requests
        .iter()
        .find(|request| pending_interactive_request_matches_kind(request, kind))
}

pub(in crate::app) fn pending_interactive_kind(
    requests: &[PendingInteractiveRequest],
) -> Option<PendingInteractiveKind> {
    requests.first().map(pending_interactive_kind_from_request)
}

pub(in crate::app) fn pending_interactive_kind_for_execution(
    execution: &SessionExecutionResource,
) -> Option<PendingInteractiveKind> {
    pending_interactive_kind(execution.pending_interactive_requests.as_slice())
}

pub(in crate::app) fn execution_update_is_stale(
    current_latest_event_seq: Option<i64>,
    incoming_latest_event_seq: Option<i64>,
) -> bool {
    match (current_latest_event_seq, incoming_latest_event_seq) {
        (Some(current), Some(incoming)) => incoming < current,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

pub(in crate::app) fn permission_overlay_matches_pending_request(
    overlay: &PermissionOverlay,
    session_id: Option<i64>,
    execution: Option<&SessionExecutionResource>,
) -> bool {
    if session_id != Some(overlay.session_id) {
        return false;
    }

    first_pending_interactive_request_by_kind(
        execution
            .map(|resource| resource.pending_interactive_requests.as_slice())
            .unwrap_or(&[]),
        PendingInteractiveKind::Permission,
    )
    .and_then(PendingInteractiveRequest::as_permission)
    .is_some_and(|request| request.request_id == overlay.request.request_id)
}

pub(in crate::app) fn user_input_overlay_matches_pending_request(
    overlay: &UserInputOverlay,
    session_id: Option<i64>,
    execution: Option<&SessionExecutionResource>,
) -> bool {
    if session_id != Some(overlay.session_id) {
        return false;
    }

    first_pending_interactive_request_by_kind(
        execution
            .map(|resource| resource.pending_interactive_requests.as_slice())
            .unwrap_or(&[]),
        PendingInteractiveKind::UserInput,
    )
    .and_then(PendingInteractiveRequest::as_user_input)
    .is_some_and(|request| request.request_id == overlay.request.request_id)
}

pub(in crate::app) fn execution_wait_state_key(
    execution: &SessionExecutionResource,
) -> Option<&'static str> {
    match execution.pending_interactive_requests.first() {
        Some(PendingInteractiveRequest::Permission { .. }) => Some("session-awaiting-approval"),
        Some(PendingInteractiveRequest::UserInput { .. }) => Some("session-awaiting-user-input"),
        None if execution.blocked => Some("session-blocked"),
        None => None,
    }
}

pub(in crate::app) fn execution_pending_flash_key(
    execution: &SessionExecutionResource,
) -> Option<&'static str> {
    match execution.pending_interactive_requests.first() {
        Some(PendingInteractiveRequest::Permission { .. }) => {
            Some("flash-session-awaiting-approval")
        }
        Some(PendingInteractiveRequest::UserInput { .. }) => {
            Some("flash-session-awaiting-user-input")
        }
        None => None,
    }
}

pub(in crate::app) fn pending_interactive_counts_for_execution(
    execution: &SessionExecutionResource,
) -> (usize, usize) {
    execution.pending_interactive_requests.iter().fold(
        (0, 0),
        |(permission_count, user_input_count), request| match request {
            PendingInteractiveRequest::Permission { .. } => {
                (permission_count + 1, user_input_count)
            }
            PendingInteractiveRequest::UserInput { .. } => (permission_count, user_input_count + 1),
        },
    )
}

pub(in crate::app) fn composer_input_is_active(
    focus: Focus,
    has_text_or_items: bool,
    has_auxiliary_input_ui: bool,
) -> bool {
    focus == Focus::Composer && (has_text_or_items || has_auxiliary_input_ui)
}

pub(in crate::app) fn preferred_visible_session_selection(
    session: &SessionResource,
    visible_sessions: &[SessionResource],
) -> Option<i64> {
    [
        Some(session.id),
        session.parent_id,
        (session.root_id != session.id).then_some(session.root_id),
    ]
    .into_iter()
    .flatten()
    .find(|candidate| visible_sessions.iter().any(|item| item.id == *candidate))
}

pub(in crate::app) fn permission_request_fingerprint(request: &PermissionRequest) -> String {
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

pub(in crate::app) fn permission_overlay_choice(
    page: PermissionOverlayPage,
    selected: usize,
) -> PermissionOverlayChoice {
    match page {
        PermissionOverlayPage::Action => match selected {
            0 => PermissionOverlayChoice::OpenScope(PermissionOverlayDecision::Allow),
            1 => PermissionOverlayChoice::OpenScope(PermissionOverlayDecision::Deny),
            2 => PermissionOverlayChoice::EditRule,
            _ => PermissionOverlayChoice::Details,
        },
        PermissionOverlayPage::Scope(PermissionOverlayDecision::Allow) => match selected {
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
        PermissionOverlayPage::Scope(PermissionOverlayDecision::Deny) => match selected {
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
        PermissionOverlayPage::Details(_) => {
            unreachable!("permission details do not have selectable choices")
        }
    }
}

pub(in crate::app) fn permission_overlay_choice_label(
    i18n: &I18n,
    choice: PermissionOverlayChoice,
) -> String {
    match choice {
        PermissionOverlayChoice::OpenScope(PermissionOverlayDecision::Allow) => {
            ui_text::t(i18n, "overlay-permission-choice-allow")
        }
        PermissionOverlayChoice::OpenScope(PermissionOverlayDecision::Deny) => {
            ui_text::t(i18n, "overlay-permission-choice-deny")
        }
        PermissionOverlayChoice::EditRule => {
            ui_text::t(i18n, "overlay-permission-choice-edit-rule")
        }
        PermissionOverlayChoice::Details => ui_text::t(i18n, "overlay-permission-details-title"),
        PermissionOverlayChoice::Reply {
            kind: PermissionReplyKind::AllowOnce,
            ..
        }
        | PermissionOverlayChoice::Reply {
            kind: PermissionReplyKind::DenyOnce,
            ..
        } => ui_text::t(i18n, "overlay-permission-choice-once"),
        PermissionOverlayChoice::Reply {
            scope: Some(PermissionScope::Session),
            ..
        } => ui_text::t(i18n, "overlay-permission-choice-session"),
        PermissionOverlayChoice::Reply {
            scope: Some(PermissionScope::Workspace),
            ..
        } => ui_text::t(i18n, "overlay-permission-choice-workspace"),
        PermissionOverlayChoice::Reply {
            scope: Some(PermissionScope::Global),
            ..
        } => ui_text::t(i18n, "overlay-permission-choice-global"),
        PermissionOverlayChoice::Reply { .. } => ui_text::t(i18n, "overlay-permission-choice-once"),
    }
}

pub(in crate::app) fn permission_overlay_reply_label(
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

pub(in crate::app) fn permission_overlay_decision_label(
    i18n: &I18n,
    decision: PermissionOverlayDecision,
) -> String {
    ui_text::t(
        i18n,
        match decision {
            PermissionOverlayDecision::Allow => "value-allow",
            PermissionOverlayDecision::Deny => "value-deny",
        },
    )
}

pub(in crate::app) fn permission_overlay_choices(
    i18n: &I18n,
    page: PermissionOverlayPage,
) -> Vec<String> {
    let count = match page {
        PermissionOverlayPage::Action => 4,
        PermissionOverlayPage::Scope(_) => 5,
        PermissionOverlayPage::Details(_) => 0,
    };
    (0..count)
        .map(|selected| {
            permission_overlay_choice_label(i18n, permission_overlay_choice(page, selected))
        })
        .collect()
}

pub(in crate::app) fn permission_overlay_title(i18n: &I18n, page: PermissionOverlayPage) -> String {
    let base = ui_text::t(i18n, "overlay-permission-title");
    match page {
        PermissionOverlayPage::Action => base,
        PermissionOverlayPage::Scope(PermissionOverlayDecision::Allow) => {
            format!(
                "{base} · {}",
                permission_overlay_decision_label(i18n, PermissionOverlayDecision::Allow)
            )
        }
        PermissionOverlayPage::Scope(PermissionOverlayDecision::Deny) => {
            format!(
                "{base} · {}",
                permission_overlay_decision_label(i18n, PermissionOverlayDecision::Deny)
            )
        }
        PermissionOverlayPage::Details(_) => {
            format!(
                "{base} · {}",
                ui_text::t(i18n, "overlay-permission-details-title")
            )
        }
    }
}

pub(in crate::app) fn permission_overlay_footer(
    i18n: &I18n,
    page: PermissionOverlayPage,
) -> String {
    match page {
        PermissionOverlayPage::Action => ui_text::t(i18n, "overlay-permission-footer-action"),
        PermissionOverlayPage::Scope(_) => ui_text::t(i18n, "overlay-permission-footer-scope"),
        PermissionOverlayPage::Details(_) => ui_text::t(i18n, "overlay-permission-footer-details"),
    }
}

pub(in crate::app) fn permission_action_label(i18n: &I18n, action: &PermissionAction) -> String {
    match action {
        PermissionAction::Tool {
            tool_name,
            qualifier,
        } => {
            let base = i18n.text_args(
                "overlay-permission-action-tool",
                &crate::fl_args!("tool" => tool_name.clone()),
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
            &crate::fl_args!(
                "access" => access_kind.clone(),
                "path" => target_path.clone(),
            ),
        ),
        PermissionAction::NetworkAccess { target, host, port } => i18n.text_args(
            "overlay-permission-action-network",
            &crate::fl_args!(
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

pub(in crate::app) fn permission_requested_actions_for_display<'a>(
    primary: Option<&'a PermissionAction>,
    requested: &'a [PermissionAction],
) -> Vec<&'a PermissionAction> {
    if requested.is_empty() {
        return Vec::new();
    }
    if requested.len() == 1 && primary.is_some_and(|primary| requested.first() == Some(&primary)) {
        return Vec::new();
    }
    requested.iter().collect()
}

pub(in crate::app) fn permission_related_actions_for_display<'a>(
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
use crate::app::{
    BTreeSet, Focus, I18n, Line, MessagePart, MessageResource, MessageStatus, PartContent,
    PendingInteractiveKind, PendingInteractiveRequest, PermissionAction, PermissionOverlay,
    PermissionOverlayChoice, PermissionOverlayDecision, PermissionOverlayPage, PermissionReplyKind,
    PermissionRequest, PermissionScope, RenderedLine, SessionExecutionResource, SessionResource,
    Span, Style, UserInputOverlay, json, ui_text,
};
