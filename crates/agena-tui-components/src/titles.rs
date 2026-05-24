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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_with_summary_omits_summary_when_space_is_tight() {
        assert_eq!(title_with_summary("Catalog", "query text", 12), " Catalog ");
    }

    #[test]
    fn title_with_summary_truncates_wide_summary_text() {
        let title = title_with_summary("Catalog", "这是一个很长的摘要内容", 24);
        assert!(title.starts_with(" Catalog · "));
        assert!(title.ends_with("... "));
    }
}
