pub(in crate::app) fn selection_highlight_style() -> Style {
    agena_tui_components::theme::selection_style()
}

/// A dragged text range is an overlay, not the permanent navigation cursor.
/// Give it a separate semantic color and underline so both states remain
/// distinguishable even when they overlap on the same terminal row.
pub(in crate::app) fn text_range_highlight_style() -> Style {
    let palette = agena_tui_components::theme::active_palette();
    Style::default()
        .fg(palette.modal_fg)
        .bg(palette.special)
        .add_modifier(Modifier::UNDERLINED)
}

pub(in crate::app) fn apply_line_highlight(line: Line<'static>, width: u16) -> Line<'static> {
    apply_full_line_highlight(line, width, selection_highlight_style())
}

/// A semantic block provides context around the permanent primary line. Use
/// the same readable colors without bolding every row; the primary line is
/// layered on top with the normal bold selection style.
pub(in crate::app) fn apply_block_highlight(line: Line<'static>, width: u16) -> Line<'static> {
    let palette = agena_tui_components::theme::active_palette();
    apply_full_line_highlight(
        line,
        width,
        Style::default()
            .fg(palette.selection_fg)
            .bg(palette.selection_bg),
    )
}

fn apply_full_line_highlight(line: Line<'static>, width: u16, style: Style) -> Line<'static> {
    let Line {
        style: line_style,
        alignment,
        spans,
    } = line;
    let content_width = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum::<usize>();
    let padding = usize::from(width).saturating_sub(content_width);
    let (leading, trailing) = match alignment {
        Some(ratatui::layout::Alignment::Right) => (padding, 0),
        Some(ratatui::layout::Alignment::Center) => (padding / 2, padding - padding / 2),
        _ => (0, padding),
    };
    let mut highlighted = Vec::with_capacity(spans.len().saturating_add(2));
    if leading > 0 {
        highlighted.push(Span::styled(
            " ".repeat(leading),
            merge_highlight_style(Style::default(), style),
        ));
    }
    highlighted.extend(
        spans
            .into_iter()
            .map(|span| Span::styled(span.content, merge_highlight_style(span.style, style))),
    );
    if trailing > 0 {
        highlighted.push(Span::styled(
            " ".repeat(trailing),
            merge_highlight_style(Style::default(), style),
        ));
    }
    Line {
        style: line_style,
        alignment,
        spans: highlighted,
    }
}

/// Highlight only a display-cell range while retaining every rich-text span
/// style. Cell-aware splitting is required for CJK, emoji, and combining
/// sequences; slicing by UTF-8 byte or scalar index would corrupt either the
/// highlight geometry or the text itself.
pub(in crate::app) fn apply_line_cell_highlight(
    line: Line<'static>,
    range: std::ops::Range<usize>,
) -> Line<'static> {
    let highlight = text_range_highlight_style();
    let Line {
        style: line_style,
        alignment,
        spans,
    } = line;
    let mut column = 0_usize;
    let mut highlighted_spans = Vec::new();

    for span in spans {
        let base_style = span.style;
        let mut run_text = String::new();
        let mut run_highlighted = None;
        for grapheme in span.content.graphemes(true) {
            let width = UnicodeWidthStr::width(grapheme);
            let start = column;
            let end = column.saturating_add(width);
            column = end;
            let selected = start < range.end && end > range.start;
            if run_highlighted.is_some_and(|current| current != selected) {
                let style = if run_highlighted == Some(true) {
                    merge_highlight_style(base_style, highlight)
                } else {
                    base_style
                };
                highlighted_spans.push(Span::styled(std::mem::take(&mut run_text), style));
            }
            run_highlighted = Some(selected);
            run_text.push_str(grapheme);
        }
        if !run_text.is_empty() {
            let style = if run_highlighted == Some(true) {
                merge_highlight_style(base_style, highlight)
            } else {
                base_style
            };
            highlighted_spans.push(Span::styled(run_text, style));
        }
    }

    // A terminal selection may end in cells after the text. Materialize only
    // that finite trailing interval so the visual feedback matches the pointer
    // without extending multi-line selections to an unbounded width.
    if range.end != usize::MAX && range.end > column {
        let gap = range.start.saturating_sub(column);
        if gap > 0 {
            highlighted_spans.push(Span::raw(" ".repeat(gap)));
            column = column.saturating_add(gap);
        }
        let selected_cells = range.end.saturating_sub(column);
        if selected_cells > 0 {
            highlighted_spans.push(Span::styled(
                " ".repeat(selected_cells),
                merge_highlight_style(Style::default(), highlight),
            ));
        }
    }

    Line {
        style: line_style,
        alignment,
        spans: highlighted_spans,
    }
}

fn merge_highlight_style(mut base: Style, highlight: Style) -> Style {
    if highlight.fg.is_some() {
        base.fg = highlight.fg;
    }
    if highlight.bg.is_some() {
        base.bg = highlight.bg;
    }
    base = base.add_modifier(highlight.add_modifier);
    base.remove_modifier(highlight.sub_modifier)
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
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
mod cell_highlight_tests {
    use super::*;

    #[test]
    fn full_line_highlight_materializes_the_entire_visual_row() {
        let line = Line {
            style: Style::default(),
            alignment: Some(ratatui::layout::Alignment::Center),
            spans: vec![Span::raw("focus")],
        };

        let highlighted = apply_line_highlight(line, 11);
        assert_eq!(highlighted.width(), 11);
        assert_eq!(
            highlighted.alignment,
            Some(ratatui::layout::Alignment::Center)
        );
        assert_eq!(
            highlighted
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "   focus   "
        );
        let selected_style = selection_highlight_style();
        assert!(highlighted.spans.iter().all(|span| {
            span.style.bg == selected_style.bg && span.style.fg == selected_style.fg
        }));
    }

    #[test]
    fn block_context_is_full_width_but_primary_line_remains_visually_stronger() {
        let block = apply_block_highlight(Line::from("block"), 9);
        let primary = apply_line_highlight(Line::from("primary"), 9);
        let selected_style = selection_highlight_style();

        assert_eq!(block.width(), 9);
        assert_eq!(primary.width(), 9);
        assert!(block.spans.iter().all(|span| {
            span.style.bg == selected_style.bg
                && span.style.fg == selected_style.fg
                && !span.style.add_modifier.contains(Modifier::BOLD)
        }));
        assert!(
            primary
                .spans
                .iter()
                .all(|span| span.style.add_modifier.contains(Modifier::BOLD))
        );
    }

    #[test]
    fn partial_highlight_preserves_rich_styles_and_grapheme_geometry() {
        let base = Style::default().fg(ratatui::style::Color::Red);
        let line_style = Style::default().add_modifier(Modifier::ITALIC);
        let line = Line {
            style: line_style,
            alignment: Some(ratatui::layout::Alignment::Right),
            spans: vec![Span::styled("a你e\u{301}z", base)],
        };

        let highlighted = apply_line_cell_highlight(line, 2..4);
        assert_eq!(highlighted.style, line_style);
        assert_eq!(
            highlighted.alignment,
            Some(ratatui::layout::Alignment::Right)
        );
        assert_eq!(
            highlighted
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "a你e\u{301}z"
        );
        let selected_style = text_range_highlight_style();
        assert!(highlighted.spans.iter().any(|span| {
            span.content.contains('你')
                && span.style.bg == selected_style.bg
                && span.style.fg == selected_style.fg
        }));
        assert!(highlighted.spans.iter().any(|span| {
            span.content.contains('z') && span.style.fg == base.fg && span.style.bg == base.bg
        }));
    }

    #[test]
    fn pointer_range_and_navigation_cursor_use_distinct_styles() {
        let cursor = selection_highlight_style();
        let range = text_range_highlight_style();
        assert_ne!(cursor.bg, range.bg);
        assert!(cursor.add_modifier.contains(Modifier::BOLD));
        assert!(range.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn pointer_range_layers_over_the_navigation_cursor_without_hiding_it() {
        let cursor = selection_highlight_style();
        let range = text_range_highlight_style();
        let highlighted =
            apply_line_cell_highlight(apply_line_highlight(Line::from("abcdef"), 6), 2..4);

        let selected = highlighted
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "cd")
            .expect("selected middle span");
        assert_eq!(selected.style.bg, range.bg);
        assert!(selected.style.add_modifier.contains(Modifier::BOLD));
        assert!(selected.style.add_modifier.contains(Modifier::UNDERLINED));

        let unselected = highlighted
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "ab")
            .expect("unselected leading span");
        assert_eq!(unselected.style.bg, cursor.bg);
        assert!(unselected.style.add_modifier.contains(Modifier::BOLD));
        assert!(!unselected.style.add_modifier.contains(Modifier::UNDERLINED));
    }
}
