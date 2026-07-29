use crate::{DetailTextLine, Style, sanitize_terminal_text};

pub(crate) fn app_detail_labeled_line(
    label: impl Into<String>,
    value: impl Into<String>,
) -> DetailTextLine<'static> {
    let label = label.into();
    let value = value.into();
    DetailTextLine::labeled(
        label,
        sanitize_terminal_text(value.as_str()),
        Style::default().fg(agena_tui_components::theme::muted_color()),
        Style::default(),
    )
}

pub(crate) fn app_detail_plain_line(text: impl Into<String>) -> DetailTextLine<'static> {
    let text = text.into();
    DetailTextLine::plain(sanitize_terminal_text(text.as_str()), Style::default())
}
