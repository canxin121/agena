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

#[cfg(test)]
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

/// Whether a pending request has already been shown to the user on some
/// client. User-input requests persist this acknowledgement on the request
/// part (`presented_at`), so a presented-but-unanswered request is surfaced
/// through the awaiting-input hint instead of a forced modal. Permission
/// requests have no durable presentation field: they remain eligible for
/// auto-open until replied, so closing one can never lose it.
pub(crate) fn pending_interactive_request_is_presented(
    request: &PendingInteractiveRequestResource,
) -> bool {
    match &request.request {
        PendingInteractiveRequest::UserInput { request } => request.presented_at.is_some(),
        PendingInteractiveRequest::Permission { .. } => false,
    }
}

/// The auto-open candidate set: outstanding requests that have neither been
/// presented (durably) nor locally guarded in this session. Never-presented
/// requests must always pop up; presented-but-unanswered ones stay visible
/// through the persistent awaiting-input hint and can be reopened manually.
pub(crate) fn first_auto_open_pending_interactive_request<'a>(
    requests: &'a [PendingInteractiveRequestResource],
    seen_permission_request_ids: &BTreeSet<String>,
    seen_user_input_request_ids: &BTreeSet<String>,
) -> Option<&'a PendingInteractiveRequestResource> {
    requests.iter().find(|request| {
        !pending_interactive_request_is_seen(
            request,
            seen_permission_request_ids,
            seen_user_input_request_ids,
        ) && !pending_interactive_request_is_presented(request)
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

/// Whether to skip an incoming execution update, honoring terminal finality.
///
/// A terminal execution (`active_execution == None`) delivered for a session
/// whose execution the TUI does not know yet is the server's current durable
/// truth: the server reports `active_execution == None` only when no run is
/// registered, so applying it cannot regress a newer run. It must therefore
/// never be dropped merely because a live-only event advanced the local
/// watermark past the server's durable `latest_event_seq` — dropping it is
/// what leaves the transcript stuck on an InProgress reply with the final
/// body rendered as a collapsible "Text" card.
///
/// When the TUI already knows a *running* execution, an older terminal must
/// not clobber it (a newer run may have started since the server snapshot);
/// the plain staleness check still protects that path.
pub(crate) fn execution_update_is_stale_with_terminal(
    current_latest_event_seq: Option<i64>,
    incoming_latest_event_seq: Option<i64>,
    incoming_is_terminal: bool,
    current_execution_absent: bool,
) -> bool {
    if incoming_is_terminal && current_execution_absent {
        return false;
    }
    execution_update_is_stale(current_latest_event_seq, incoming_latest_event_seq)
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
    PermissionScope, SessionExecutionResource, SessionResource, ui_text,
};

#[cfg(test)]
mod terminal_staleness_tests {
    use super::execution_update_is_stale_with_terminal;

    #[test]
    fn terminal_update_is_never_stale_when_no_execution_is_known_locally() {
        // The local watermark sits ahead (live-only events inflated it) and
        // the incoming terminal snapshot carries an older durable seq. It is
        // still the server's current truth (no run registered), so it must be
        // applied or the reply stays stuck on an InProgress "Text" card.
        assert!(!execution_update_is_stale_with_terminal(
            Some(100),
            Some(90),
            true,
            true,
        ));
        assert!(!execution_update_is_stale_with_terminal(
            Some(100),
            None,
            true,
            true,
        ));
        assert!(!execution_update_is_stale_with_terminal(
            None,
            Some(90),
            true,
            true,
        ));
    }

    #[test]
    fn older_terminal_cannot_clobber_a_locally_running_execution() {
        // A genuinely older terminal snapshot must not regress a newer run
        // the TUI already knows about.
        assert!(execution_update_is_stale_with_terminal(
            Some(100),
            Some(90),
            true,
            false,
        ));
    }

    #[test]
    fn running_updates_keep_the_plain_staleness_rules() {
        assert!(execution_update_is_stale_with_terminal(
            Some(100),
            Some(90),
            false,
            false,
        ));
        assert!(!execution_update_is_stale_with_terminal(
            Some(100),
            Some(110),
            false,
            false,
        ));
        assert!(!execution_update_is_stale_with_terminal(
            None,
            Some(90),
            false,
            false,
        ));
    }
}
