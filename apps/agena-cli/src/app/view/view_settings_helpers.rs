pub(in crate::app) fn selection_highlight_style() -> Style {
    agena_tui_components::theme::selection_style()
}

pub(in crate::app) fn apply_line_highlight(line: Line<'static>) -> Line<'static> {
    let style = selection_highlight_style();
    let spans = line
        .spans
        .into_iter()
        .map(|span| {
            let mut span_style = span.style;
            if style.fg.is_some() {
                span_style.fg = style.fg;
            }
            if style.bg.is_some() {
                span_style.bg = style.bg;
            }
            span_style = span_style.add_modifier(style.add_modifier);
            span_style = span_style.remove_modifier(style.sub_modifier);
            Span::styled(span.content, span_style)
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

pub(in crate::app) fn sanitize_display_text(text: impl AsRef<str>) -> String {
    sanitize_terminal_text(text.as_ref())
}

pub(in crate::app) fn sanitize_display_str(text: &str) -> String {
    sanitize_display_text(text)
}

pub(in crate::app) fn settings_item_detail_title(
    i18n: &I18n,
    dialog: &SettingsStudioOverlay,
) -> String {
    let detail_label = ui_text::t(i18n, "overlay-workbench-details");
    dialog
        .state
        .selected_item()
        .map(|item| format!("{detail_label}: {}", item.label))
        .unwrap_or(detail_label)
}

pub(in crate::app) fn settings_item_detail_text(
    i18n: &I18n,
    dialog: &SettingsStudioOverlay,
) -> Text<'static> {
    dialog
        .state
        .selected_item()
        .map(|item| {
            let mut lines = vec![Line::from(Span::styled(
                sanitize_display_text(item.detail.as_str()),
                Style::default(),
            ))];
            if let Some(current_value) = item.current_value.as_deref() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    sanitize_display_text(ui_text::t(i18n, "settings-detail-values-heading")),
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(sanitize_display_text(i18n.text_args(
                    "overlay-settings-detail-current",
                    &crate::fl_args!("value" => current_value.to_string()),
                ))));
            }
            if let Some(effective_value) = item.effective_value.as_deref() {
                lines.push(Line::from(sanitize_display_text(i18n.text_args(
                    "overlay-settings-edit-effective-value",
                    &crate::fl_args!("value" => effective_value.to_string()),
                ))));
            }
            if !item.source_rows.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    sanitize_display_text(ui_text::t(i18n, "settings-detail-sources-heading")),
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                for row in &item.source_rows {
                    lines.push(Line::from(sanitize_display_text(format!(
                        "{}: {}",
                        row.label, row.value
                    ))));
                }
            }
            if let Some(path) = item.path.as_deref() {
                lines.push(Line::from(""));
                lines.push(Line::from(sanitize_display_text(i18n.text_args(
                    "overlay-settings-detail-path",
                    &crate::fl_args!("path" => path.to_string()),
                ))));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(sanitize_display_text(
                settings_item_action_hint(i18n, item),
            )));
            Text::from(lines)
        })
        .unwrap_or_else(|| Text::from(ui_text::t(i18n, "overlay-settings-empty-detail")))
}

pub(in crate::app) fn settings_item_action_hint(i18n: &I18n, item: &SettingsStudioItem) -> String {
    match &item.action {
        SettingsPickerAction::OpenPluginWorkbench
        | SettingsPickerAction::OpenTerminalDiagnostics
        | SettingsPickerAction::RefreshProviderClientVersions => {
            ui_text::t(i18n, "settings-detail-action-screen")
        }
        SettingsPickerAction::OpenSessionEffectivePermissionView(_) => {
            ui_text::t(i18n, "settings-detail-action-readonly")
        }
        SettingsPickerAction::OpenConfigFile => ui_text::t(i18n, "settings-detail-action-file"),
        _ => ui_text::t(i18n, "overlay-settings-detail-action"),
    }
}

pub(in crate::app) fn settings_section_group_label(
    i18n: &I18n,
    section: SettingsStudioSectionId,
) -> String {
    let key = match section {
        SettingsStudioSectionId::ModelsProviders
        | SettingsStudioSectionId::Agents
        | SettingsStudioSectionId::Permissions
        | SettingsStudioSectionId::PluginsTools => "overlay-settings-group-core",
        SettingsStudioSectionId::RuntimeSession => "overlay-settings-group-application",
        SettingsStudioSectionId::Interface => "overlay-settings-group-application",
        SettingsStudioSectionId::Diagnostics => "overlay-settings-group-system",
    };
    ui_text::t(i18n, key)
}

pub(in crate::app) fn settings_table_columns(columns: &[(&str, usize)], width: u16) -> String {
    format_fixed_columns(columns, width, |text| sanitize_display_text(text))
}
use super::{
    I18n, Line, Modifier, SettingsPickerAction, SettingsStudioItem, SettingsStudioOverlay,
    SettingsStudioSectionId, Span, Style, Text, format_fixed_columns, sanitize_terminal_text,
    ui_text,
};
