pub(in crate::app) fn permission_studio_table_label(
    item: &PermissionStudioItem,
    section_id: PermissionStudioSectionId,
    max_width: usize,
) -> String {
    if section_id != PermissionStudioSectionId::PathRules {
        return item.label.clone();
    }

    let (pattern, access) = match &item.action {
        PermissionStudioAction::EditMode(PermissionStudioModeTarget::PathRuleRead { pattern }) => {
            (pattern.as_str(), "read")
        }
        PermissionStudioAction::EditMode(PermissionStudioModeTarget::PathRuleWrite { pattern }) => {
            (pattern.as_str(), "write")
        }
        _ => return item.label.clone(),
    };
    compact_permission_path_rule_label(pattern, access, max_width)
}

pub(in crate::app) fn compact_permission_path_rule_label(
    pattern: &str,
    access: &str,
    max_width: usize,
) -> String {
    let suffix = format!(" · {access}");
    let suffix_width = UnicodeWidthStr::width(suffix.as_str());
    if max_width <= suffix_width {
        return truncate_display_text(suffix.as_str(), max_width);
    }
    let path_budget = max_width.saturating_sub(suffix_width);
    format!(
        "{}{}",
        compact_permission_path_pattern(pattern, path_budget),
        suffix
    )
}

/// Keeps the right-most path components visible because those usually identify a rule.
///
/// A normal left truncation turns paths beneath one workspace into indistinguishable rows.
/// This preserves a meaningful root marker when it fits and replaces only the shared middle
/// with an ellipsis: `/…/generated/client/**` or `<workspace>/…/src/**`.
pub(in crate::app) fn compact_permission_path_pattern(pattern: &str, max_width: usize) -> String {
    let pattern = sanitize_display_text(pattern);
    if UnicodeWidthStr::width(pattern.as_str()) <= max_width {
        return pattern;
    }
    if max_width == 0 {
        return String::new();
    }

    let (root, remainder) = if let Some(remainder) = pattern.strip_prefix("<workspace>/") {
        ("<workspace>", remainder)
    } else if let Some(remainder) = pattern.strip_prefix("~/") {
        ("~", remainder)
    } else if let Some(remainder) = pattern.strip_prefix('/') {
        ("/", remainder)
    } else {
        ("", pattern.as_str())
    };
    let components = remainder
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    if components.len() < 2 {
        return ellipsize_from_start(pattern.as_str(), max_width);
    }

    let root_ellipsis = match root {
        "<workspace>" => "<workspace>/…",
        "~" => "~/…",
        "/" => "/…",
        _ => "…",
    };
    for start in 0..components.len() {
        let tail = components[start..].join("/");
        let candidate = format!("{root_ellipsis}/{tail}");
        if UnicodeWidthStr::width(candidate.as_str()) <= max_width {
            return candidate;
        }
    }
    ellipsize_from_start(components.last().copied().unwrap_or_default(), max_width)
}

pub(in crate::app) fn ellipsize_from_start(text: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let content_width = max_width.saturating_sub(1);
    let mut tail = String::new();
    let mut width = 0_usize;
    for character in text.chars().rev() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width.saturating_add(character_width) > content_width {
            break;
        }
        tail.insert(0, character);
        width = width.saturating_add(character_width);
    }
    format!("…{tail}")
}

pub(in crate::app) fn settings_compact_pad_to_width(text: &str, width: usize) -> String {
    let cleaned = sanitize_display_text(text);
    let clipped = truncate_display_text(cleaned.as_str(), width);
    let padding = width.saturating_sub(clipped.width());
    format!("{clipped}{}", " ".repeat(padding))
}

pub(in crate::app) fn user_input_review_answer_preview(i18n: &I18n, values: &[String]) -> String {
    if values.is_empty() {
        ui_text::t(i18n, "overlay-user-input-unanswered")
    } else {
        truncate_display_text(values.join(", ").as_str(), 72)
    }
}

pub(in crate::app) fn user_input_option_description_preview(
    description: &str,
    width: u16,
) -> String {
    truncate_display_text(
        sanitize_display_text(description).as_str(),
        width.max(1) as usize,
    )
}

pub(in crate::app) fn user_input_custom_values_preview(
    i18n: &I18n,
    values: &[String],
    width: u16,
) -> String {
    if values.is_empty() {
        ui_text::t(i18n, "overlay-user-input-custom-empty")
    } else {
        truncate_display_text(values.join(", ").as_str(), width.max(1) as usize)
    }
}

pub(in crate::app) fn provider_studio_main_field_display(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> String {
    let value = match field {
        ProviderStudioField::AuthMode => {
            provider_draft_auth_mode_label(i18n, &dialog.draft.auth_kind)
        }
        ProviderStudioField::AuthLoginMethod => dialog
            .draft
            .interactive_login_kind()
            .map(|kind| provider_studio_auth_login_kind_label(i18n, kind))
            .unwrap_or_else(|| provider_studio_main_field_value(i18n, dialog, field)),
        ProviderStudioField::AuthSubtype => {
            provider_draft_auth_subtype_label(i18n, &dialog.draft.auth_kind)
        }
        _ => provider_studio_main_field_value(i18n, dialog, field),
    };
    match field {
        ProviderStudioField::ApiKeyValue
            if matches!(
                dialog.draft.auth.secret_source_kind,
                ProviderDraftSecretSourceKind::Inline
            ) && !value.trim().is_empty() =>
        {
            "********".to_owned()
        }
        ProviderStudioField::ApiKeyValue
        | ProviderStudioField::RefreshToken
        | ProviderStudioField::AccessToken
        | ProviderStudioField::AccessKeyId
        | ProviderStudioField::SecretAccessKey
        | ProviderStudioField::SessionToken
            if !value.trim().is_empty() =>
        {
            "********".to_owned()
        }
        _ if value.trim().is_empty() => ui_text::t(i18n, "value-unset"),
        _ => value,
    }
}

pub(in crate::app) fn provider_studio_detail_text_spec() -> DetailTextSpec<'static> {
    DetailTextSpec::label_width(16)
}

pub(in crate::app) fn permission_overlay_body_lines(
    i18n: &I18n,
    dialog: &PermissionOverlay,
) -> Vec<Line<'static>> {
    if matches!(dialog.page, PermissionOverlayPage::Details(_)) {
        return permission_overlay_details_lines(i18n, dialog);
    }

    let mut lines = Vec::new();
    append_permission_primary_action_lines(i18n, &mut lines, &dialog.request.action);
    let requested_actions = permission_requested_actions_for_display(
        Some(&dialog.request.action),
        dialog.request.requested_actions.as_slice(),
    );
    append_permission_secondary_action_lines(
        i18n,
        &mut lines,
        "overlay-permission-requested-actions",
        requested_actions.as_slice(),
    );
    let related_actions = permission_related_actions_for_display(
        Some(&dialog.request.action),
        dialog.request.related_actions.as_slice(),
        dialog.request.requested_actions.as_slice(),
    );
    append_permission_secondary_action_lines(
        i18n,
        &mut lines,
        "overlay-permission-related-actions",
        related_actions.as_slice(),
    );
    append_permission_field_line(
        i18n,
        &mut lines,
        "overlay-permission-field-reason",
        permission_request_explanation(&dialog.request),
        Style::default(),
    );
    lines.push(Line::from(Span::styled(
        i18n.text_args(
            "overlay-permission-fact-risk",
            &crate::fl_args!("value" => permission_risk_label(i18n, dialog.request.risk)),
        ),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    )));
    lines
}

pub(in crate::app) fn permission_overlay_choice_lines(
    i18n: &I18n,
    dialog: &PermissionOverlay,
) -> Vec<Line<'static>> {
    let heading = match dialog.page {
        PermissionOverlayPage::Action => "overlay-permission-decision-heading",
        PermissionOverlayPage::Scope(_) => "overlay-permission-scope-heading",
        PermissionOverlayPage::Details(_) => return Vec::new(),
    };
    let mut lines = vec![Line::from(Span::styled(
        sanitize_display_text(ui_text::t(i18n, heading)),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for (index, label) in permission_overlay_choices(i18n, dialog.page)
        .into_iter()
        .enumerate()
    {
        let selected = index == dialog.selection.selected;
        let style = if selected {
            selection_highlight_style()
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{}{}", if selected { ">> " } else { "   " }, label),
            style,
        )));
    }
    lines
}

pub(in crate::app) fn append_permission_primary_action_lines(
    i18n: &I18n,
    lines: &mut Vec<Line<'static>>,
    action: &PermissionAction,
) {
    match action {
        PermissionAction::Tool {
            tool_name,
            qualifier,
        } => {
            append_permission_field_line(
                i18n,
                lines,
                "overlay-permission-field-tool",
                tool_name,
                Style::default().add_modifier(Modifier::BOLD),
            );
            if let Some(qualifier) = qualifier
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                append_permission_field_line(
                    i18n,
                    lines,
                    "overlay-permission-field-target",
                    qualifier,
                    Style::default(),
                );
            }
        }
        PermissionAction::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        } => {
            append_permission_field_line(
                i18n,
                lines,
                "overlay-permission-field-access",
                permission_rule_path_access_kind_display(i18n, access_kind),
                Style::default().add_modifier(Modifier::BOLD),
            );
            append_permission_field_line(
                i18n,
                lines,
                "overlay-permission-field-path",
                target_path,
                Style::default().add_modifier(Modifier::BOLD),
            );
            if !workspace_root.trim().is_empty() && workspace_root != target_path {
                append_permission_field_line(
                    i18n,
                    lines,
                    "overlay-permission-field-workspace",
                    workspace_root,
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                );
            }
        }
        PermissionAction::NetworkAccess { target, host, port } => {
            let endpoint = if target.trim().is_empty() {
                match port {
                    Some(port) => format!("{host}:{port}"),
                    None => host.clone(),
                }
            } else {
                target.clone()
            };
            append_permission_field_line(
                i18n,
                lines,
                "overlay-permission-field-network",
                endpoint,
                Style::default().add_modifier(Modifier::BOLD),
            );
            let host_label = match port {
                Some(port) => format!("{host}:{port}"),
                None => host.clone(),
            };
            if host_label != target.trim() {
                append_permission_field_line(
                    i18n,
                    lines,
                    "overlay-permission-field-host",
                    host_label,
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                );
            }
        }
    }
}

pub(in crate::app) fn append_permission_field_line(
    i18n: &I18n,
    lines: &mut Vec<Line<'static>>,
    label_key: &str,
    value: impl AsRef<str>,
    value_style: Style,
) {
    lines.push(Line::from(Span::styled(
        sanitize_display_text(ui_text::t(i18n, label_key)),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    )));
    lines.push(Line::from(Span::styled(
        format!("  {}", sanitize_display_text(value)),
        value_style,
    )));
}

pub(in crate::app) fn append_permission_secondary_action_lines(
    i18n: &I18n,
    lines: &mut Vec<Line<'static>>,
    heading_key: &str,
    actions: &[&PermissionAction],
) {
    if actions.is_empty() {
        return;
    }
    lines.push(Line::from(Span::styled(
        sanitize_display_text(ui_text::t(i18n, heading_key)),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    )));
    lines.extend(actions.iter().map(|action| {
        Line::from(Span::styled(
            format!(
                "  {}",
                sanitize_display_text(permission_action_label(i18n, action))
            ),
            Style::default(),
        ))
    }));
}

pub(in crate::app) fn permission_request_explanation(request: &PermissionRequest) -> &str {
    let explanation = request.explanation.trim();
    if explanation.is_empty() {
        request.reason.trim()
    } else {
        explanation
    }
}

pub(in crate::app) fn permission_overlay_details_lines(
    i18n: &I18n,
    dialog: &PermissionOverlay,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    append_permission_field_line(
        i18n,
        &mut lines,
        "overlay-permission-detail-request-id",
        &dialog.request.request_id,
        Style::default().fg(agena_tui_components::theme::muted_color()),
    );
    if let Some(source) = dialog.request.source.as_deref() {
        append_permission_field_line(
            i18n,
            &mut lines,
            "overlay-permission-detail-source",
            source,
            Style::default().fg(agena_tui_components::theme::muted_color()),
        );
    }
    if let Some(scope) = dialog.request.scope {
        append_permission_field_line(
            i18n,
            &mut lines,
            "overlay-permission-detail-scope",
            permission_request_scope_label(i18n, scope),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        );
    }
    if let Some(operator) = dialog.request.operator.as_deref() {
        append_permission_field_line(
            i18n,
            &mut lines,
            "overlay-permission-detail-operator",
            operator,
            Style::default().fg(agena_tui_components::theme::muted_color()),
        );
    }
    if !dialog.request.trace.is_empty() {
        lines.push(Line::from(Span::styled(
            sanitize_display_text(ui_text::t(i18n, "overlay-permission-detail-trace")),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )));
        lines.extend(dialog.request.trace.iter().map(|step| {
            Line::from(Span::styled(
                format!(
                    "  {}",
                    sanitize_display_text(permission_trace_step_label(i18n, step))
                ),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            ))
        }));
    }
    lines
}

pub(in crate::app) fn permission_request_scope_label(
    i18n: &I18n,
    scope: PermissionScope,
) -> String {
    match scope {
        PermissionScope::Session => ui_text::t(i18n, "value-session"),
        PermissionScope::Workspace => ui_text::t(i18n, "value-workspace"),
        PermissionScope::Global => ui_text::t(i18n, "value-global"),
    }
}
use super::{
    DetailTextSpec, I18n, Line, Modifier, PermissionAction, PermissionOverlay,
    PermissionOverlayPage, PermissionRequest, PermissionScope, PermissionStudioAction,
    PermissionStudioItem, PermissionStudioModeTarget, PermissionStudioSectionId,
    ProviderDraftSecretSourceKind, ProviderStudioField, ProviderStudioOverlay, Span, Style,
    UnicodeWidthChar, UnicodeWidthStr, permission_action_label, permission_overlay_choices,
    permission_related_actions_for_display, permission_requested_actions_for_display,
    permission_risk_label, permission_rule_path_access_kind_display, permission_trace_step_label,
    provider_draft_auth_mode_label, provider_draft_auth_subtype_label,
    provider_studio_auth_login_kind_label, provider_studio_main_field_value, sanitize_display_text,
    selection_highlight_style, truncate_display_text, ui_text,
};
