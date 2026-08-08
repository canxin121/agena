//! Title rendering helpers for panels and dialogs.

use unicode_width::UnicodeWidthStr;

use crate::truncate_display_text;

pub fn title_with_summary(title: &str, summary: &str, width: u16) -> String {
    let title = title.trim().to_string();
    let summary = summary.trim().to_string();
    if summary.is_empty() {
        return format!(" {title} ");
    }

    let max_summary_width = width
        .saturating_sub(UnicodeWidthStr::width(title.as_str()) as u16)
        .saturating_sub(7) as usize;
    if max_summary_width < 8 {
        format!(" {title} ")
    } else {
        format!(
            " {} · {} ",
            title,
            truncate_display_text(summary.as_str(), max_summary_width)
        )
    }
}
