pub(crate) struct PermissionStudioColumns {
    label: usize,
    value: usize,
    source: usize,
}

pub(crate) fn permission_studio_columns(
    i18n: &I18n,
    dialog: &crate::PermissionStudioOverlay,
    width: usize,
) -> PermissionStudioColumns {
    let items = dialog
        .state
        .sections()
        .iter()
        .flat_map(|section| &section.items)
        .filter(|item| item.source.is_some());
    let value = items
        .clone()
        .map(|item| UnicodeWidthStr::width(sanitize_display_text(&item.value).as_str()))
        .max()
        .unwrap_or(0)
        .min(width / 3 + 1);
    let source = items
        .filter_map(|item| item.source)
        .map(|source| {
            UnicodeWidthStr::width(crate::permission_studio_source_label(i18n, source).as_str())
        })
        .max()
        .unwrap_or(0)
        .min(width / 3);
    PermissionStudioColumns {
        label: width.saturating_sub(value + source + 2),
        value,
        source,
    }
}

pub(crate) fn permission_studio_dense_line(
    i18n: &I18n,
    item: &PermissionStudioItem,
    section_id: PermissionStudioSectionId,
    columns: &PermissionStudioColumns,
    width: usize,
) -> ratatui::text::Line<'static> {
    use ratatui::{
        style::{Modifier, Style},
        text::{Line, Span},
    };
    let muted = Style::default().fg(agena_tui_components::theme::muted_color());
    let value_style = Style::default().add_modifier(Modifier::BOLD);
    if let Some(source) = item.source {
        let label = permission_studio_table_label(item, section_id, columns.label);
        return Line::from(vec![
            Span::raw(permission_studio_cell(&label, columns.label)),
            Span::raw(" "),
            Span::styled(
                permission_studio_cell(&item.value, columns.value),
                value_style,
            ),
            Span::raw(" "),
            Span::styled(
                permission_studio_cell(
                    &crate::permission_studio_source_label(i18n, source),
                    columns.source,
                ),
                muted,
            ),
        ]);
    }
    if item.value.is_empty() {
        return Line::from(Span::styled(
            super::truncate_display_text(&sanitize_display_text(&item.label), width),
            muted,
        ));
    }
    let label_width = columns.label.min(width / 2);
    Line::from(vec![
        Span::raw(permission_studio_cell(&item.label, label_width)),
        Span::raw(" "),
        Span::styled(
            permission_studio_cell(&item.value, width.saturating_sub(label_width + 1)),
            value_style,
        ),
    ])
}

fn permission_studio_cell(text: &str, width: usize) -> String {
    let text = super::truncate_display_text(&sanitize_display_text(text), width);
    let padding = width.saturating_sub(UnicodeWidthStr::width(text.as_str()));
    format!("{text}{}", " ".repeat(padding))
}

pub(crate) fn permission_studio_table_label(
    item: &PermissionStudioItem,
    section_id: PermissionStudioSectionId,
    max_width: usize,
) -> String {
    if section_id != PermissionStudioSectionId::PathRules {
        return item.label.clone();
    }

    let pattern = match &item.action {
        PermissionStudioAction::EditMode(PermissionStudioModeTarget::PathRule { pattern }) => {
            pattern
        }
        _ => return item.label.clone(),
    };
    compact_permission_path_pattern(pattern, max_width)
}

/// Keeps the right-most path components visible because those usually identify a rule.
///
/// A normal left truncation turns paths beneath one workspace into indistinguishable rows.
/// This preserves a meaningful root marker when it fits and replaces only the shared middle
/// with an ellipsis: `/…/generated/client` or `<workspace>/…/src`.
pub(crate) fn compact_permission_path_pattern(pattern: &str, max_width: usize) -> String {
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

pub(crate) fn ellipsize_from_start(text: &str, max_width: usize) -> String {
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

pub(crate) fn provider_studio_main_field_display(
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

pub(crate) fn provider_studio_detail_text_spec() -> DetailTextSpec<'static> {
    DetailTextSpec::label_width(16)
}

use super::{
    DetailTextSpec, I18n, PermissionStudioAction, PermissionStudioItem, PermissionStudioModeTarget,
    PermissionStudioSectionId, ProviderDraftSecretSourceKind, ProviderStudioField,
    ProviderStudioOverlay, UnicodeWidthChar, UnicodeWidthStr, provider_draft_auth_mode_label,
    provider_draft_auth_subtype_label, provider_studio_auth_login_kind_label,
    provider_studio_main_field_value, sanitize_display_text,
};
use crate::ui_text;
