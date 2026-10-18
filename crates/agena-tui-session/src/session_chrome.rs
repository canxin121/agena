//! Compact session rows shared by the home screen and resume picker.
use agena_tui::sanitize_picker_text;
use agena_tui_components::{theme, truncate_display_text};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

pub(crate) fn session_line(
    width: u16,
    label: &str,
    summary: &str,
    pinned: bool,
    favorite: bool,
    indented: bool,
) -> Line<'static> {
    let prefix = if indented { "  " } else { "" };
    let available = usize::from(width).saturating_sub(prefix.len() + 4);
    let summary_width = if available >= 64 {
        (available / 3).min(28)
    } else {
        0
    };
    let title_width = available.saturating_sub(summary_width + usize::from(summary_width > 0) * 2);
    let title = truncate_display_text(&sanitize_picker_text(label), title_width);
    let padding = title_width.saturating_sub(Line::from(title.as_str()).width());
    let mut spans = vec![
        Span::raw(prefix),
        Span::styled(if pinned { "◆ " } else { "  " }, theme::accent_color()),
        Span::styled(if favorite { "★ " } else { "  " }, theme::warning_color()),
        Span::raw(title),
    ];
    if summary_width > 0 {
        spans.push(Span::raw(" ".repeat(padding + 2)));
        spans.push(Span::styled(
            truncate_display_text(&sanitize_picker_text(summary), summary_width),
            theme::muted_style(),
        ));
    }
    Line::from(spans)
}

pub(crate) fn render_details(frame: &mut Frame<'_>, area: Rect, title: &str, detail: &str) {
    if area.is_empty() {
        return;
    }
    let lines = vec![
        Line::from(Span::styled(
            truncate_display_text(&sanitize_picker_text(title), usize::from(area.width)),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            truncate_display_text(&sanitize_picker_text(detail), usize::from(area.width)),
            theme::muted_style(),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}
