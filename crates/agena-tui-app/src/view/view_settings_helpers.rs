pub(crate) fn selection_highlight_style() -> Style {
    agena_tui_components::theme::selection_style()
}

/// A dragged text range is an overlay, not the permanent navigation cursor.
/// Give it a separate semantic color and underline so both states remain
/// distinguishable even when they overlap on the same terminal row.
pub(crate) fn text_range_highlight_style() -> Style {
    let palette = agena_tui_components::theme::active_palette();
    Style::default()
        .fg(palette.modal_fg)
        .bg(palette.special)
        .add_modifier(Modifier::UNDERLINED)
}

/// A semantic block provides context around the permanent primary line. Use
/// the same readable colors without bolding every row; the primary line is
/// layered on top with the normal bold selection style.
pub(crate) fn apply_block_highlight(line: Line<'static>, width: u16) -> Line<'static> {
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
pub(crate) fn apply_line_cell_highlight(
    line: Line<'static>,
    range: std::ops::Range<usize>,
    width: u16,
) -> Line<'static> {
    // Both Visual-line selection and atomic graphical pointer selections use
    // an unbounded end column. Resolve it against the viewport before adding
    // padding, including rows whose only visible content is a native image.
    let range = range.start.min(usize::from(width))..range.end.min(usize::from(width));
    if range.start >= range.end {
        return line;
    }
    apply_line_cell_highlight_with_style(line, range, text_range_highlight_style())
}

/// Highlight the single grapheme occupied by the keyboard cursor. Unlike a
/// pointer range this intentionally uses the primary selection appearance,
/// while preserving rich-text styles around that one terminal-cell span.
pub(crate) fn apply_cursor_cell_highlight(
    line: Line<'static>,
    range: std::ops::Range<usize>,
) -> Line<'static> {
    apply_line_cell_highlight_with_style(line, range, selection_highlight_style())
}

fn apply_line_cell_highlight_with_style(
    line: Line<'static>,
    range: std::ops::Range<usize>,
    highlight: Style,
) -> Line<'static> {
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

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn visual_line_and_pointer_ranges_highlight_the_same_full_graphical_rows() {
        for width in [1, 24, 40, 80] {
            let line = Line::from(if width > 1 { "  " } else { "" });
            let keyboard = apply_line_cell_highlight(line.clone(), 0..usize::MAX, width);
            let pointer = apply_line_cell_highlight(line, 0..usize::from(width), width);
            assert_eq!(keyboard, pointer);
            assert_eq!(
                UnicodeWidthStr::width(keyboard.to_string().as_str()),
                usize::from(width)
            );
            assert!(
                keyboard
                    .spans
                    .iter()
                    .all(|span| span.style.add_modifier.contains(Modifier::UNDERLINED))
            );
        }
    }

    #[test]
    fn viewport_clipping_preserves_text_and_does_not_allocate_offscreen_padding() {
        let line = Line::from(vec![
            Span::raw("前缀 "),
            Span::styled("formula", Style::default().add_modifier(Modifier::BOLD)),
        ]);
        let selected = apply_line_cell_highlight(line.clone(), 100..usize::MAX, 24);
        assert_eq!(selected, line);
        let selected = apply_line_cell_highlight(line, 5..usize::MAX, 24);
        assert_eq!(UnicodeWidthStr::width(selected.to_string().as_str()), 24);
        assert!(selected.spans.iter().any(|span| {
            span.content == "formula"
                && span
                    .style
                    .add_modifier
                    .contains(Modifier::BOLD | Modifier::UNDERLINED)
        }));
    }
}

pub(crate) fn sanitize_display_text(text: impl AsRef<str>) -> String {
    sanitize_terminal_text(text.as_ref())
}

pub(crate) fn settings_item_detail_title(i18n: &I18n, dialog: &SettingsStudioOverlay) -> String {
    let detail_label = ui_text::t(i18n, "overlay-workbench-details");
    dialog
        .state
        .selected_item()
        .map(|item| format!("{detail_label}: {}", item.label))
        .unwrap_or(detail_label)
}

pub(crate) fn settings_item_detail_text(
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
                    &agena_tui::fl_args!("value" => current_value.to_string()),
                ))));
            }
            if let Some(effective_value) = item.effective_value.as_deref() {
                lines.push(Line::from(sanitize_display_text(i18n.text_args(
                    "overlay-settings-edit-effective-value",
                    &agena_tui::fl_args!("value" => effective_value.to_string()),
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
                    &agena_tui::fl_args!("path" => path.to_string()),
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

pub(crate) fn settings_item_action_hint(
    i18n: &I18n,
    item: &SettingsStudioItem<SettingsPickerAction>,
) -> String {
    match &item.action {
        SettingsPickerAction::OpenPluginWorkbench
        | SettingsPickerAction::OpenSkillsWorkbench
        | SettingsPickerAction::OpenTerminalDiagnostics => {
            ui_text::t(i18n, "settings-detail-action-screen")
        }
        SettingsPickerAction::OpenSessionEffectivePermissionView(_) => {
            ui_text::t(i18n, "settings-detail-action-readonly")
        }
        SettingsPickerAction::OpenConfigFile => ui_text::t(i18n, "settings-detail-action-file"),
        _ => ui_text::t(i18n, "overlay-settings-detail-action"),
    }
}

pub(crate) fn settings_table_columns(columns: &[(&str, usize)], width: u16) -> String {
    format_fixed_columns(columns, width, |text| sanitize_display_text(text))
}

use super::{
    I18n, Line, Modifier, SettingsPickerAction, SettingsStudioItem, SettingsStudioOverlay, Span,
    Style, Text, format_fixed_columns, sanitize_terminal_text,
};
use crate::ui_text;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
