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

pub(in crate::app) fn settings_compact_sections_text(
    i18n: &I18n,
    dialog: &SettingsStudioOverlay,
    width: u16,
) -> Text<'static> {
    let mut lines = vec![Line::from(Span::styled(
        sanitize_display_text(ui_text::t(i18n, "overlay-settings-sections")),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(""));
    if dialog.state.sections().is_empty() {
        lines.push(Line::from(sanitize_display_text(ui_text::t(
            i18n,
            "overlay-settings-empty-section",
        ))));
        return Text::from(lines);
    }
    let content_width = width.max(1) as usize;
    let mut previous_group = String::new();
    for (index, section) in dialog.state.sections().iter().enumerate() {
        let group = settings_section_group_label(i18n, section.id);
        if group != previous_group {
            if !previous_group.is_empty() {
                lines.push(Line::from(""));
            }
            previous_group = group.clone();
            lines.push(Line::from(Span::styled(
                sanitize_display_text(settings_compact_pad_to_width(group.as_str(), content_width)),
                Style::default()
                    .fg(agena_tui_components::theme::muted_color())
                    .add_modifier(Modifier::BOLD),
            )));
        }
        let focused = dialog.state.focus() == SettingsStudioFocus::Navigation;
        let selected = index == dialog.state.selected_section_index();
        let marker = if selected && focused { "> " } else { "  " };
        let label = format!("{marker}{}  {}", section.label, section.items.len());
        let line = settings_compact_pad_to_width(label.as_str(), content_width);
        let style = if selected && focused {
            selection_highlight_style()
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(sanitize_display_text(line), style)));
    }
    Text::from(lines)
}

pub(in crate::app) fn settings_compact_editor_text(
    i18n: &I18n,
    dialog: &SettingsStudioOverlay,
    current_section: Option<&SettingsStudioSection>,
    width: u16,
    height: u16,
) -> Text<'static> {
    let mut lines = Vec::new();
    let Some(section) = current_section else {
        lines.push(Line::from(sanitize_display_text(ui_text::t(
            i18n,
            "overlay-settings-empty-section",
        ))));
        return Text::from(lines);
    };

    lines.push(Line::from(Span::styled(
        sanitize_display_text(section.label.as_str()),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        sanitize_display_text(section.description.as_str()),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    )));
    lines.push(Line::from(""));

    let fixed_rows = 3usize;
    let visible_item_count = height
        .saturating_sub(fixed_rows as u16)
        .saturating_div(2)
        .max(1) as usize;

    if section.items.is_empty() {
        lines.push(Line::from(sanitize_display_text(ui_text::t(
            i18n,
            "overlay-settings-empty-items",
        ))));
    } else {
        let (start, end) = settings_compact_visible_range(
            section.items.len(),
            dialog.state.selected_item_index(),
            visible_item_count,
        );
        for (index, item) in section.items[start..end].iter().enumerate() {
            let index = start + index;
            let focused = dialog.state.focus() == SettingsStudioFocus::Items;
            let selected = index == dialog.state.selected_item_index();
            let marker = if selected && focused { ">> " } else { "   " };
            let style = if selected && focused {
                selection_highlight_style()
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                settings_compact_item_title_line(item, marker, width),
                style,
            )));
            lines.push(Line::from(Span::styled(
                settings_compact_item_subtitle_line(item, "   ", width),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )));
        }
    }

    Text::from(lines)
}

pub(in crate::app) fn settings_compact_item_title_line(
    item: &SettingsStudioItem,
    marker: &str,
    width: u16,
) -> String {
    let width = width.max(1) as usize;
    let marker = sanitize_display_text(marker);
    let label = sanitize_display_text(item.label.as_str());
    let value = sanitize_display_text(item.value.as_str());
    if value.trim().is_empty() || width <= UnicodeWidthStr::width(marker.as_str()) + 4 {
        return truncate_display_text(format!("{marker}{label}").as_str(), width);
    }

    let marker_width = UnicodeWidthStr::width(marker.as_str());
    let label_width = UnicodeWidthStr::width(label.as_str());
    let value_width = UnicodeWidthStr::width(value.as_str());
    let full_width = marker_width
        .saturating_add(label_width)
        .saturating_add(value_width)
        .saturating_add(2);
    if full_width <= width {
        let gap = width
            .saturating_sub(marker_width)
            .saturating_sub(label_width)
            .saturating_sub(value_width)
            .max(1);
        return format!("{marker}{label}{}{}", " ".repeat(gap), value);
    }

    let available = width.saturating_sub(marker_width);
    let gap_width = if available > 2 { 2 } else { 1 };
    let content_budget = available.saturating_sub(gap_width);
    if content_budget <= 1 {
        return truncate_display_text(format!("{marker}{label}").as_str(), width);
    }

    let min_value_budget = value_width.min(16).min(content_budget.saturating_sub(1));
    let preferred_label_budget = label_width.min(24);
    let mut label_budget = preferred_label_budget.min(content_budget.saturating_sub(1));
    let mut value_budget = content_budget.saturating_sub(label_budget);
    if value_budget < min_value_budget {
        let min_label_budget = label_width.min(12).min(content_budget.saturating_sub(1));
        let reclaim = min_value_budget
            .saturating_sub(value_budget)
            .min(label_budget.saturating_sub(min_label_budget));
        label_budget = label_budget.saturating_sub(reclaim);
        value_budget = value_budget.saturating_add(reclaim);
    }

    let label = truncate_display_text(label.as_str(), label_budget.max(1));
    let value = truncate_display_text(value.as_str(), value_budget.max(1));
    let label_width = UnicodeWidthStr::width(label.as_str());
    let value_width = UnicodeWidthStr::width(value.as_str());
    let gap = width
        .saturating_sub(marker_width)
        .saturating_sub(value_width)
        .saturating_sub(label_width)
        .max(1);
    format!("{marker}{label}{}{}", " ".repeat(gap), value)
}

pub(in crate::app) fn settings_compact_item_subtitle_line(
    item: &SettingsStudioItem,
    indent: &str,
    width: u16,
) -> String {
    let width = width.max(1) as usize;
    let indent = sanitize_display_text(indent);
    let indent_width = UnicodeWidthStr::width(indent.as_str()).min(width);
    let budget = width.saturating_sub(indent_width).max(1);
    format!(
        "{indent}{}",
        truncate_display_text(sanitize_display_text(item.detail.as_str()).as_str(), budget)
    )
}

pub(in crate::app) fn settings_compact_item_detail_title(
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

pub(in crate::app) fn settings_compact_item_detail_text(
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
        SettingsPickerAction::OpenPluginPolicyStudio
        | SettingsPickerAction::OpenPluginWorkbench => {
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
        SettingsStudioSectionId::ConfigProviders
        | SettingsStudioSectionId::ConfigAgents
        | SettingsStudioSectionId::ConfigPermission
        | SettingsStudioSectionId::ConfigPlugins => "overlay-settings-group-core",
        SettingsStudioSectionId::ConfigRuntime
        | SettingsStudioSectionId::ConfigSession
        | SettingsStudioSectionId::ConfigHarnesses
        | SettingsStudioSectionId::ConfigTracing
        | SettingsStudioSectionId::ConfigUi => "overlay-settings-group-application",
        SettingsStudioSectionId::RuntimeOverrides | SettingsStudioSectionId::RuntimeRules => {
            "overlay-settings-group-session"
        }
        SettingsStudioSectionId::Catalogs | SettingsStudioSectionId::Files => {
            "overlay-settings-group-system"
        }
    };
    ui_text::t(i18n, key)
}

pub(in crate::app) fn settings_compact_visible_range(
    item_count: usize,
    selected_index: usize,
    max_visible: usize,
) -> (usize, usize) {
    if item_count == 0 {
        return (0, 0);
    }
    let max_visible = max_visible.max(1).min(item_count);
    let selected_index = selected_index.min(item_count.saturating_sub(1));
    let start = selected_index
        .saturating_sub(max_visible / 2)
        .min(item_count.saturating_sub(max_visible));
    (start, start + max_visible)
}

pub(in crate::app) fn settings_compact_vertical_divider(height: u16) -> Text<'static> {
    Text::from((0..height).map(|_| Line::from("│")).collect::<Vec<_>>())
}

pub(in crate::app) fn settings_compact_fixed_columns(
    columns: &[(&str, usize)],
    width: u16,
) -> String {
    let mut out = String::new();
    for (index, (text, size)) in columns.iter().enumerate() {
        if index > 0 {
            out.push_str("  ");
        }
        let remaining = width.saturating_sub(out.width() as u16) as usize;
        if remaining == 0 {
            break;
        }
        let size = (*size).min(remaining);
        let cleaned = sanitize_display_text(text);
        let clipped = truncate_display_text(cleaned.as_str(), size);
        out.push_str(clipped.as_str());
        let padding = size.saturating_sub(clipped.width());
        out.push_str(" ".repeat(padding).as_str());
    }
    out
}
use super::{
    I18n, Line, Modifier, SettingsPickerAction, SettingsStudioFocus, SettingsStudioItem,
    SettingsStudioOverlay, SettingsStudioSection, SettingsStudioSectionId, Span, Style, Text,
    UnicodeWidthStr, sanitize_terminal_text, settings_compact_pad_to_width, truncate_display_text,
    ui_text,
};
