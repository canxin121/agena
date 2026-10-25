//! Fixed columns measured in terminal cells, shared by headers and all rows.
use super::SessionHubItem;
use agena_tui::{i18n::I18n, sanitize_picker_text};
use agena_tui_components::{theme, truncate_display_text_with_suffix};
use ratatui::text::{Line, Span};

pub(super) struct HubColumns {
    name: usize,
    status: usize,
    updated: usize,
    separator: &'static str,
}

impl HubColumns {
    /// `width` excludes the single-cell selection gutter. Even narrow panes
    /// retain status and time; they never silently drop the metadata columns.
    pub(super) fn new(width: u16) -> Self {
        let width = usize::from(width);
        let (status, updated, separator) = if width >= 40 {
            (8, 11, " │ ")
        } else if width >= 5 {
            let metadata = (width - 2) / 3;
            (metadata.min(8), metadata.min(11), "│")
        } else {
            (0, 0, "")
        };
        Self {
            name: width.saturating_sub(status + updated + Line::from(separator).width() * 2),
            status,
            updated,
            separator,
        }
    }

    pub(super) fn header(&self, i18n: &I18n, indented: bool) -> Line<'static> {
        let prefix = if indented && self.name > 0 { " " } else { "" };
        let name = format!("{prefix}{}", i18n.text("hub-column-name"));
        self.line(
            vec![Span::raw(cell(&name, self.name))],
            &i18n.text("hub-column-status"),
            &i18n.text("hub-column-updated"),
        )
    }

    pub(super) fn item(&self, item: &SessionHubItem, indented: bool) -> Line<'static> {
        let prefix = if indented && self.name > 0 { " " } else { "" };
        // Markers are suffixes inside the name cell. Unmarked sessions have
        // no marker placeholders, and toggling a marker never shifts columns.
        let markers = format!(
            "{}{}",
            if item.pinned { "◆" } else { "" },
            if item.favorite { "★" } else { "" }
        );
        let suffix = if markers.is_empty() {
            String::new()
        } else {
            format!(" {markers}")
        };
        let suffix = truncate_display_text_with_suffix(
            &suffix,
            self.name.saturating_sub(prefix.len() + 1),
            "",
        );
        let title_width = self
            .name
            .saturating_sub(prefix.len() + Line::from(suffix.as_str()).width());
        let title =
            truncate_display_text_with_suffix(&sanitize_picker_text(&item.label), title_width, "…");
        let mut name = vec![Span::raw(format!("{prefix}{title}"))];
        if !suffix.is_empty() {
            name.push(Span::styled(suffix, theme::accent_color()));
        }
        let used = name.iter().map(Span::width).sum::<usize>();
        name.push(Span::raw(" ".repeat(self.name.saturating_sub(used))));
        self.line(name, &item.status, &item.updated)
    }

    pub(super) fn action(&self, label: &str) -> Line<'static> {
        self.line(vec![Span::raw(cell(label, self.name))], "", "")
    }

    fn line(&self, mut name: Vec<Span<'static>>, status: &str, updated: &str) -> Line<'static> {
        if self.status > 0 {
            name.push(Span::styled(self.separator, theme::muted_style()));
            name.push(Span::styled(
                cell(status, self.status),
                theme::muted_style(),
            ));
            name.push(Span::styled(self.separator, theme::muted_style()));
            name.push(Span::styled(
                cell(updated, self.updated),
                theme::muted_style(),
            ));
        }
        Line::from(name)
    }
}

fn cell(text: &str, width: usize) -> String {
    let text = truncate_display_text_with_suffix(&sanitize_picker_text(text), width, "…");
    let padding = width.saturating_sub(Line::from(text.as_str()).width());
    format!("{text}{}", " ".repeat(padding))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn separators(line: &Line<'_>) -> Vec<usize> {
        let mut offset = 0;
        let mut positions = Vec::new();
        for span in &line.spans {
            if matches!(span.content.as_ref(), " │ " | "│") {
                positions.push(offset);
            }
            offset += span.width();
        }
        positions
    }

    #[test]
    fn multilingual_names_and_markers_keep_header_and_metadata_aligned() {
        for width in [25, 38, 48, 78, 118, 218] {
            let columns = HubColumns::new(width);
            for locale in ["en-US", "zh-CN", "ja-JP", "de-DE"] {
                let i18n = I18n::resolve(Some(locale), None);
                let header = columns.header(&i18n, true);
                assert_eq!(header.width(), usize::from(width));
                for label in [
                    "Short",
                    "中文项目：检查终端的列对齐与空间利用",
                    "Cafe\u{301} 👩‍💻 long workspace title that must be clipped inside its cell",
                ] {
                    for (pinned, favorite) in
                        [(false, false), (true, false), (false, true), (true, true)]
                    {
                        let mut item = super::super::tests::item(1, 1);
                        item.label = label.to_owned();
                        item.pinned = pinned;
                        item.favorite = favorite;
                        let line = columns.item(&item, true);
                        assert_eq!(line.width(), usize::from(width), "{line:?}");
                        assert_eq!(separators(&line), separators(&header), "{line:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn unmarked_names_start_after_only_one_tree_cell() {
        let columns = HubColumns::new(78);
        let mut item = super::super::tests::item(1, 1);
        item.label = "A session".to_owned();
        let line = columns.item(&item, true);
        assert!(line.spans[0].content.starts_with(" A session"));
        item.pinned = true;
        item.favorite = true;
        let marked = columns.item(&item, true);
        assert_eq!(marked.spans[0].content, line.spans[0].content);
        assert!(marked.spans.iter().any(|span| span.content == " ◆★"));
        assert_eq!(separators(&line), separators(&marked));
    }

    #[test]
    fn tiny_terminals_and_oversized_metadata_stay_inside_their_cells() {
        let mut item = super::super::tests::item(1, 1);
        item.pinned = true;
        item.favorite = true;
        item.status = "等待用户处理一个非常长的交互请求".to_owned();
        item.updated = "2026-10-25 23:59:59 UTC+08:00".to_owned();
        for width in 0..220 {
            let columns = HubColumns::new(width);
            let line = columns.item(&item, true);
            assert_eq!(line.width(), usize::from(width), "width={width} {line:?}");
            assert_eq!(
                separators(&line),
                separators(&columns.header(&I18n::english(), true))
            );
        }
    }
}
