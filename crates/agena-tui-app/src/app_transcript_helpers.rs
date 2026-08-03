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

pub(crate) fn permission_overlay_choice(
    page: PermissionPromptPage,
    selected: usize,
) -> PermissionOverlayChoice {
    match page {
        PermissionPromptPage::Action => match selected {
            0 => PermissionOverlayChoice::OpenScope(PermissionPromptDecision::Allow),
            1 => PermissionOverlayChoice::OpenScope(PermissionPromptDecision::Deny),
            2 => PermissionOverlayChoice::EditRule,
            3 => PermissionOverlayChoice::AutoApprove,
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
    BTreeSet, I18n, PendingInteractiveKind, PendingInteractiveRequest,
    PendingInteractiveRequestResource, PermissionAction, PermissionOverlay,
    PermissionOverlayChoice, PermissionPromptDecision, PermissionPromptPage, PermissionReplyKind,
    PermissionScope, SessionExecutionResource, SessionResource, UserInputOverlay, ui_text,
};
