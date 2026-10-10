pub(crate) fn push_expanded_diff_text(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    width: u16,
    i18n: &I18n,
) {
    use super::{DiffRowKind, transcript_diff_files};
    let sanitized = sanitize_terminal_text(text);
    let palette = agena_tui_components::theme::active_palette();
    let prefix_width = UnicodeWidthStr::width(prefix);
    for file in transcript_diff_files(&sanitized) {
        let path = if !file.old_path.is_empty() && file.old_path != file.path {
            format!("{} → {}", file.old_path, file.path)
        } else if file.path.is_empty() {
            ui_text::t(i18n, "transcript-diff-changes")
        } else {
            file.path.clone()
        };
        let title = vec![
            Span::styled(
                format!("{} ", file.change.unwrap_or('M')),
                Style::default().fg(palette.muted),
            ),
            Span::styled(path.clone(), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("  +{}", file.additions),
                Style::default().fg(palette.success),
            ),
            Span::styled(
                format!(" −{}", file.deletions),
                Style::default().fg(palette.danger),
            ),
        ];
        let available = (width as usize).saturating_sub(prefix_width).max(1);
        for line in wrap_rich_line(&title, available, available) {
            let copy = line
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>();
            let mut spans = vec![Span::raw(prefix.to_owned())];
            spans.extend(line.spans);
            out.push(
                RenderedLine::rich(Line::from(spans)).with_copy_projection(copy, prefix_width),
            );
        }
        let language = std::path::Path::new(&file.path)
            .extension()
            .and_then(|value| value.to_str());
        let source = file
            .rows
            .iter()
            .filter(|row| {
                matches!(
                    row.kind,
                    DiffRowKind::Added | DiffRowKind::Removed | DiffRowKind::Context
                )
            })
            .map(|row| row.text.as_str())
            .collect::<Vec<_>>();
        let highlighted =
            language.map(|language| syntax_highlight_lines(language, &source, palette));
        let mut highlight_index = 0;
        let max_line = file
            .rows
            .iter()
            .flat_map(|row| [row.old_line, row.new_line])
            .flatten()
            .max()
            .unwrap_or(1);
        let digits = max_line.to_string().len();
        let gutter = (digits + 2).min(available.saturating_sub(1));
        let body_width = available.saturating_sub(gutter).max(1);
        for row in &file.rows {
            if matches!(row.kind, DiffRowKind::Hunk | DiffRowKind::Note) {
                push_wrapped_line(
                    out,
                    prefix,
                    prefix,
                    &row.text,
                    Style::default().fg(palette.muted),
                    width,
                );
                continue;
            }
            let (marker, color) = match row.kind {
                DiffRowKind::Added => ("+", palette.success),
                DiffRowKind::Removed => ("-", palette.danger),
                _ => (" ", palette.muted),
            };
            let background = if row.kind == DiffRowKind::Context {
                palette.code_bg
            } else {
                diff_background(palette.code_bg, color)
            };
            let mut spans = highlighted
                .as_ref()
                .and_then(|lines| lines.get(highlight_index))
                .cloned()
                .unwrap_or_else(|| {
                    vec![Span::styled(
                        row.text.clone(),
                        Style::default().fg(palette.code_fg),
                    )]
                });
            highlight_index += 1;
            for span in &mut spans {
                span.style = span.style.bg(background);
            }
            let number = if row.kind == DiffRowKind::Removed {
                row.old_line
            } else {
                row.new_line
            }
            .unwrap_or(0);
            let navigation_unit = out.len();
            for (index, body) in wrap_rich_line(&spans, body_width, body_width)
                .into_iter()
                .enumerate()
            {
                let body_text = body
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>();
                let mut line = vec![Span::raw(prefix.to_owned())];
                if gutter > 0 {
                    let label = if index == 0 {
                        format!("{number:>digits$} ")
                    } else {
                        " ".repeat(digits + 1)
                    };
                    line.push(Span::styled(
                        truncate_display_width(&label, gutter - 1),
                        Style::default().fg(palette.muted).bg(background),
                    ));
                    line.push(Span::styled(
                        if index == 0 { marker } else { " " },
                        Style::default().fg(color).bg(background),
                    ));
                }
                line.extend(body.spans);
                let padding = body_width.saturating_sub(UnicodeWidthStr::width(body_text.as_str()));
                line.push(Span::styled(
                    " ".repeat(padding),
                    Style::default().bg(background),
                ));
                out.push(
                    RenderedLine::rich(Line::from(line))
                        .with_copy_projection(
                            if index == 0 {
                                format!("{marker}{body_text}")
                            } else {
                                body_text
                            },
                            prefix_width + gutter.saturating_sub(1),
                        )
                        .with_navigation_unit(navigation_unit, format!("{marker}{}", row.text)),
                );
            }
        }
    }
}

fn diff_background(
    base: ratatui::style::Color,
    accent: ratatui::style::Color,
) -> ratatui::style::Color {
    use ratatui::style::Color;
    match (base, accent) {
        (Color::Rgb(r, g, b), Color::Rgb(ar, ag, ab)) => {
            let blend = |value: u8, accent: u8| ((value as u16 * 9 + accent as u16) / 10) as u8;
            Color::Rgb(blend(r, ar), blend(g, ag), blend(b, ab))
        }
        _ => base,
    }
}

pub(crate) fn tool_invocation_label(invocation: &agena_domain::ToolInvocation) -> String {
    let input = serde_json::Value::from(invocation.input.clone());
    if let Some(function_name) = tool_api_display_name(invocation.name.as_str())
        && let Some(tool_name) = input.get("tool").and_then(serde_json::Value::as_str)
        && !tool_name.trim().is_empty()
    {
        return format!("{function_name} · {}", tool_name.trim());
    }
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "description",
        "action",
        "id",
        "expression",
        "notebook_path",
    ] {
        if let Some(value) = input.get(key).and_then(serde_json::Value::as_str)
            && !value.trim().is_empty()
        {
            return format!("{} {}", invocation.name, value.trim());
        }
    }
    invocation.name.clone()
}

pub(crate) fn tool_api_display_name(name: &str) -> Option<&'static str> {
    match name {
        "tools_list" => Some("tools.list"),
        "tools_search" => Some("tools.search"),
        "tools_help" => Some("tools.help"),
        "tools_tags" => Some("tools.tags"),
        "tools_call" => Some("tools.call"),
        _ => None,
    }
}
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use super::{
    I18n, RenderedLine, push_wrapped_line, sanitize_terminal_text, syntax_highlight_lines,
    truncate_display_width, wrap_rich_line,
};
use crate::ui_text;

#[cfg(test)]
mod tests {
    use super::super::{DiffRowKind, transcript_diff_files};
    use super::*;

    #[test]
    fn diff_renders_file_stats_numbers_colors_and_wrapped_copy_without_headers() {
        let diff = "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -98,3 +98,3 @@\n fn main() {\n-    println!(\"old\");\n+    println!(\"new 内容 and a long line\");\n }\n";
        let mut lines = Vec::new();
        push_expanded_diff_text(&mut lines, "  ", diff, 40, &I18n::english());
        let text = lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("src/main.rs  +1 −1"), "{text}");
        assert!(text.contains("99 -"), "{text}");
        assert!(text.contains("99 +"), "{text}");
        assert!(!text.contains("--- a/"));
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= 40)
        );
        let added = lines
            .iter()
            .find(|line| line.text.contains("99 +"))
            .unwrap();
        assert!(
            added
                .rich_line
                .as_ref()
                .unwrap()
                .spans
                .iter()
                .any(|span| span.content == "+"
                    && span.style.fg == Some(agena_tui_components::theme::success_color()))
        );
        assert!(added.navigation_copy_text.contains("+    println!"));
        assert!(!added.copy_text.contains("99"));
    }

    #[test]
    fn parses_multifile_hunks_and_source_that_looks_like_headers() {
        let files = transcript_diff_files(
            "--- a/old.rs\n+++ b/new.rs\n@@ -10,2 +20,2 @@\n--- source\n+++ source\n context\n--- a/deleted.py\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n\\ No newline at end of file\n",
        );
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "new.rs");
        assert_eq!(files[0].change, Some('R'));
        assert_eq!((files[0].additions, files[0].deletions), (1, 1));
        assert_eq!(files[0].rows[1].text, "-- source");
        assert_eq!(files[0].rows[1].old_line, Some(10));
        assert_eq!(files[0].rows[2].new_line, Some(20));
        assert_eq!(files[1].path, "deleted.py");
        assert_eq!(files[1].change, Some('D'));
        assert_eq!(files[1].rows.last().unwrap().kind, DiffRowKind::Note);
    }

    #[test]
    fn legacy_add_diff_and_blank_lines_keep_visible_gutters() {
        let mut lines = Vec::new();
        push_expanded_diff_text(
            &mut lines,
            "",
            "+++ b/新.txt\n@@\n+\n+你好\n",
            12,
            &I18n::english(),
        );
        assert!(lines.iter().any(|line| line.text.starts_with("1 +")));
        assert!(lines.iter().any(|line| line.text.contains("2 +你好")));
    }

    #[test]
    fn terminal_buffer_shows_empty_file_operations_and_colored_diff_gutters() {
        use ratatui::{Terminal, backend::TestBackend, widgets::Paragraph};

        let diff = "diff --git a/empty b/empty\nnew file mode 100644\n\
diff --git a/gone b/gone\ndeleted file mode 100644\n\
diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n\
diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";
        for width in [32, 80] {
            let mut lines = Vec::new();
            push_expanded_diff_text(&mut lines, "  ", diff, width, &I18n::english());
            let rich = lines
                .into_iter()
                .map(|line| line.rich_line.unwrap())
                .collect::<Vec<_>>();
            let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
            terminal
                .draw(|frame| frame.render_widget(Paragraph::new(rich), frame.area()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let screen = buffer
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            for label in [
                "A empty  +0 −0",
                "D gone  +0 −0",
                "R old → new  +0 −0",
                "M a.rs  +1 −1",
                "1 -old",
                "1 +new",
            ] {
                assert!(screen.contains(label), "{screen}");
            }
            let added = buffer
                .content
                .iter()
                .find(|cell| {
                    cell.symbol() == "+"
                        && cell.fg == agena_tui_components::theme::success_color()
                        && cell.bg != ratatui::style::Color::Reset
                })
                .unwrap();
            let removed = buffer
                .content
                .iter()
                .find(|cell| {
                    cell.symbol() == "-" && cell.fg == agena_tui_components::theme::danger_color()
                })
                .unwrap();
            assert_ne!(added.bg, removed.bg);
        }
    }
}
