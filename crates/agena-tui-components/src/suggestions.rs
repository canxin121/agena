use std::{borrow::Cow, cmp::min};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use crate::truncate_display_text_with_suffix;

pub struct SuggestionPopupItem<'a> {
    pub prefix: Option<Cow<'a, str>>,
    pub label: Cow<'a, str>,
    pub detail: Option<Cow<'a, str>>,
}

pub struct SuggestionPopupSpec<'a> {
    pub items: &'a [SuggestionPopupItem<'a>],
    pub selected: usize,
    pub max_visible_rows: usize,
    pub selected_marker: Cow<'a, str>,
    pub unselected_marker: Cow<'a, str>,
    pub max_label_width: usize,
    pub detail_gap: usize,
    pub base_style: Style,
    pub selected_style: Style,
    pub prefix_style: Style,
    pub selected_prefix_style: Option<Style>,
    pub label_style: Style,
    pub detail_style: Style,
    pub pad_selected_row: bool,
}

pub struct QuerySuggestionPopupSpec<'a> {
    pub prompt_label: Cow<'a, str>,
    pub query: Cow<'a, str>,
    pub empty_message: Cow<'a, str>,
    pub prompt_style: Style,
    pub query_style: Style,
    pub empty_style: Style,
    pub results: SuggestionPopupSpec<'a>,
}

pub fn render_suggestion_popup(frame: &mut Frame, area: Rect, spec: &SuggestionPopupSpec<'_>) {
    if area.width == 0 || area.height == 0 || spec.items.is_empty() {
        return;
    }

    let lines = build_suggestion_popup_lines(area.width as usize, area.height as usize, spec);
    if lines.is_empty() {
        return;
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

pub fn render_query_suggestion_popup(
    frame: &mut Frame,
    area: Rect,
    spec: &QuerySuggestionPopupSpec<'_>,
) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    let header = Line::from(vec![
        Span::styled(spec.prompt_label.as_ref().to_string(), spec.prompt_style),
        Span::styled(spec.query.as_ref().to_string(), spec.query_style),
    ]);
    frame.render_widget(Paragraph::new(header), rows[0]);

    if rows.len() > 1 && rows[1].height > 0 {
        if spec.results.items.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    spec.empty_message.as_ref().to_string(),
                    spec.empty_style,
                ))),
                rows[1],
            );
        } else {
            render_suggestion_popup(frame, rows[1], &spec.results);
        }
    }

    let cursor_x = area
        .x
        .saturating_add(UnicodeWidthStr::width(spec.prompt_label.as_ref()) as u16)
        .saturating_add(UnicodeWidthStr::width(spec.query.as_ref()) as u16);
    Some((cursor_x, area.y))
}

fn build_suggestion_popup_lines<'a>(
    width: usize,
    height: usize,
    spec: &SuggestionPopupSpec<'a>,
) -> Vec<Line<'a>> {
    if width == 0 || height == 0 || spec.items.is_empty() {
        return Vec::new();
    }

    let selected = min(spec.selected, spec.items.len().saturating_sub(1));
    let visible_rows = min(height, min(spec.max_visible_rows.max(1), spec.items.len()));
    let start = selected.saturating_add(1).saturating_sub(visible_rows);

    spec.items
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_rows)
        .map(|(index, item)| {
            let is_selected = index == selected;
            let base_style = if is_selected {
                spec.selected_style
            } else {
                spec.base_style
            };
            let label_style = if is_selected {
                spec.selected_style
            } else {
                spec.label_style
            };
            let detail_style = if is_selected {
                spec.selected_style
            } else {
                spec.detail_style
            };
            let prefix_style = if is_selected {
                spec.selected_prefix_style.unwrap_or(spec.selected_style)
            } else {
                spec.prefix_style
            };

            let marker = if is_selected {
                spec.selected_marker.as_ref()
            } else {
                spec.unselected_marker.as_ref()
            };
            let marker_width = UnicodeWidthStr::width(marker);
            let prefix = item.prefix.as_deref().unwrap_or("");
            let prefix_width = UnicodeWidthStr::width(prefix);
            let label_max = min(
                spec.max_label_width,
                width
                    .saturating_sub(marker_width)
                    .saturating_sub(prefix_width),
            );
            let label = truncate_display_text_with_suffix(item.label.as_ref(), label_max, "…");
            let label_width = UnicodeWidthStr::width(label.as_str());
            let detail_max = width
                .saturating_sub(marker_width)
                .saturating_sub(prefix_width)
                .saturating_sub(label_width)
                .saturating_sub(spec.detail_gap);

            let mut spans = vec![Span::styled(marker.to_string(), base_style)];
            if !prefix.is_empty() {
                spans.push(Span::styled(prefix.to_string(), prefix_style));
            }
            spans.push(Span::styled(label, label_style));
            let mut used_width = marker_width
                .saturating_add(prefix_width)
                .saturating_add(label_width);
            if let Some(detail) = item.detail.as_ref() {
                let detail = truncate_display_text_with_suffix(detail.as_ref(), detail_max, "…");
                if !detail.is_empty() {
                    let detail_width = UnicodeWidthStr::width(detail.as_str());
                    spans.push(Span::styled(" ".repeat(spec.detail_gap), base_style));
                    spans.push(Span::styled(detail, detail_style));
                    used_width = used_width
                        .saturating_add(spec.detail_gap)
                        .saturating_add(detail_width);
                }
            }
            if is_selected && spec.pad_selected_row && used_width < width {
                spans.push(Span::styled(" ".repeat(width - used_width), base_style));
            }

            Line::from(spans)
        })
        .collect()
}
