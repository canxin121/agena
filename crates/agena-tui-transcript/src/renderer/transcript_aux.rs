pub(crate) fn push_expanded_diff_text(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    width: u16,
) {
    let sanitized = sanitize_terminal_text(text);
    let normalized = trim_empty_line_edges(sanitized.as_str());
    let diff_lines = normalized.lines().collect::<Vec<_>>();

    let available = width.max(1) as usize;
    let prefix_width = UnicodeWidthStr::width(prefix);
    let card_width = available.saturating_sub(prefix_width);
    let palette = agena_tui_components::theme::active_palette();
    let code_uses_terminal_defaults = palette.code_bg == ratatui::style::Color::Reset;
    let code_muted = if code_uses_terminal_defaults {
        palette.code_fg
    } else {
        palette.muted
    };
    let code_accent = if code_uses_terminal_defaults {
        palette.code_fg
    } else {
        palette.accent
    };
    if card_width < 16 {
        for raw_line in &diff_lines {
            push_wrapped_line(
                out,
                prefix,
                prefix,
                raw_line,
                diff_line_style(raw_line),
                width,
            );
        }
        return;
    }

    let language = diff_target_language(&diff_lines);
    let label = truncate_display_width(
        &language.as_deref().map_or_else(
            || "diff".to_string(),
            |language| format!("diff · {language}"),
        ),
        card_width.saturating_sub(7).max(1),
    );
    let top_start = format!("┌─ {label} ");
    let top_fill = "─".repeat(
        card_width
            .saturating_sub(UnicodeWidthStr::width(top_start.as_str()))
            .saturating_sub(1),
    );
    out.push(
        RenderedLine::rich(Line::from(vec![
            Span::raw(prefix.to_string()),
            Span::styled("┌─ ", Style::default().fg(code_muted).bg(palette.code_bg)),
            Span::styled(
                label,
                Style::default()
                    .fg(code_accent)
                    .bg(palette.code_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {top_fill}┐"),
                Style::default().fg(code_muted).bg(palette.code_bg),
            ),
        ]))
        .with_copy_projection(String::new(), prefix_width),
    );

    let body_width = card_width.saturating_sub(2).max(1);
    let highlighted = language.as_deref().map(|language| {
        let content_lines = diff_lines
            .iter()
            .filter(|line| {
                matches!(
                    diff_line_kind(line),
                    DiffLineKind::Added | DiffLineKind::Removed | DiffLineKind::Context
                )
            })
            .map(|line| &line[line.len().min(1)..])
            .collect::<Vec<_>>();
        syntax_highlight_lines(language, &content_lines, palette)
    });
    let mut highlighted_index = 0_usize;

    for raw_line in &diff_lines {
        let navigation_unit = out.len();
        let kind = diff_line_kind(raw_line);
        let spans = diff_card_line_spans(
            raw_line,
            kind,
            highlighted.as_deref(),
            &mut highlighted_index,
            palette,
        );
        for body in wrap_rich_line(&spans, body_width, body_width) {
            let body_display_width = body
                .spans
                .iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>();
            let body_copy_text = body
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            let padding = " ".repeat(body_width.saturating_sub(body_display_width));
            let mut line_spans = vec![
                Span::raw(prefix.to_string()),
                Span::styled("│", Style::default().fg(code_muted).bg(palette.code_bg)),
            ];
            line_spans.extend(body.spans);
            line_spans.extend([
                Span::styled(padding, Style::default().bg(palette.code_bg)),
                Span::styled("│", Style::default().fg(code_muted).bg(palette.code_bg)),
            ]);
            out.push(
                RenderedLine::rich(Line::from(line_spans))
                    .with_copy_projection(body_copy_text, prefix_width.saturating_add(1))
                    .with_navigation_unit(navigation_unit, *raw_line),
            );
        }
    }
    if diff_lines.is_empty() {
        out.push(
            RenderedLine::rich(Line::from(vec![
                Span::raw(prefix.to_string()),
                Span::styled("│", Style::default().fg(code_muted).bg(palette.code_bg)),
                Span::styled(
                    "  (empty)".to_string() + &" ".repeat(card_width.saturating_sub(11)),
                    Style::default().fg(palette.code_fg).bg(palette.code_bg),
                ),
                Span::styled("│", Style::default().fg(code_muted).bg(palette.code_bg)),
            ]))
            .with_copy_projection(String::new(), prefix_width),
        );
    }
    out.push(
        RenderedLine::rich(Line::from(vec![
            Span::raw(prefix.to_string()),
            Span::styled(
                format!("└{}┘", "─".repeat(card_width.saturating_sub(2))),
                Style::default().fg(code_muted).bg(palette.code_bg),
            ),
        ]))
        .with_copy_projection(String::new(), prefix_width),
    );
}

pub(crate) fn diff_line_style(line: &str) -> Style {
    match diff_line_kind(line) {
        DiffLineKind::Header => {
            Style::default().fg(agena_tui_components::theme::accent_color())
        }
        DiffLineKind::Hunk => {
            Style::default().fg(agena_tui_components::theme::warning_color())
        }
        DiffLineKind::Added => {
            Style::default().fg(agena_tui_components::theme::success_color())
        }
        DiffLineKind::Removed => {
            Style::default().fg(agena_tui_components::theme::danger_color())
        }
        DiffLineKind::Context => {
            Style::default().fg(agena_tui_components::theme::muted_color())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffLineKind {
    Header,
    Hunk,
    Added,
    Removed,
    Context,
}

fn diff_line_kind(line: &str) -> DiffLineKind {
    if line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("new file mode ")
        || line.starts_with("deleted file mode ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
        || line.starts_with("similarity index ")
        || line.starts_with("Binary files ")
        || line.starts_with("GIT binary patch")
        || line.starts_with("old mode ")
        || line.starts_with("new mode ")
    {
        DiffLineKind::Header
    } else if line.starts_with("@@") {
        DiffLineKind::Hunk
    } else if line.starts_with('+') {
        DiffLineKind::Added
    } else if line.starts_with('-') {
        DiffLineKind::Removed
    } else {
        DiffLineKind::Context
    }
}

fn diff_card_line_spans(
    line: &str,
    kind: DiffLineKind,
    highlighted: Option<&[Vec<Span<'static>>]>,
    highlighted_index: &mut usize,
    palette: agena_tui_components::ThemePalette,
) -> Vec<Span<'static>> {
    let bg = palette.code_bg;
    match kind {
        DiffLineKind::Header => vec![Span::styled(
            line.to_string(),
            Style::default()
                .fg(agena_tui_components::theme::accent_color())
                .bg(bg),
        )],
        DiffLineKind::Hunk => vec![Span::styled(
            line.to_string(),
            Style::default()
                .fg(agena_tui_components::theme::warning_color())
                .bg(bg),
        )],
        DiffLineKind::Added | DiffLineKind::Removed | DiffLineKind::Context => {
            let marker = &line[..line.len().min(1)];
            let content = &line[marker.len()..];
            let marker_style = match kind {
                DiffLineKind::Added => Style::default()
                    .fg(agena_tui_components::theme::success_color())
                    .bg(bg),
                DiffLineKind::Removed => Style::default()
                    .fg(agena_tui_components::theme::danger_color())
                    .bg(bg),
                _ => Style::default()
                    .fg(agena_tui_components::theme::muted_color())
                    .bg(bg),
            };
            let mut spans = vec![Span::styled(marker.to_string(), marker_style)];
            if let Some(highlighted) = highlighted
                && let Some(content_spans) = highlighted.get(*highlighted_index)
            {
                *highlighted_index += 1;
                spans.extend(content_spans.iter().cloned());
            } else if !content.is_empty() {
                let content_style = match kind {
                    DiffLineKind::Added => Style::default()
                        .fg(agena_tui_components::theme::success_color())
                        .bg(bg),
                    DiffLineKind::Removed => Style::default()
                        .fg(agena_tui_components::theme::danger_color())
                        .bg(bg),
                    _ => Style::default()
                        .fg(agena_tui_components::theme::muted_color())
                        .bg(bg),
                };
                spans.push(Span::styled(content.to_string(), content_style));
            }
            spans
        }
    }
}

fn diff_target_language(diff_lines: &[&str]) -> Option<String> {
    for line in diff_lines {
        let path = if let Some(rest) = line.strip_prefix("+++ b/") {
            Some(rest)
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            Some(rest)
        } else if let Some(rest) = line.strip_prefix("diff --git ") {
            rest.rsplit(" b/").next().map(str::trim)
        } else {
            None
        };
        if let Some(path) = path
            && let Some(extension) = std::path::Path::new(path)
                .extension()
                .and_then(|extension| extension.to_str())
            && !extension.is_empty()
        {
            return Some(extension.to_string());
        }
    }
    None
}

pub(crate) fn tool_invocation_label(invocation: &crate::ToolInvocationResource) -> String {
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
    RenderedLine, push_wrapped_line, sanitize_terminal_text, syntax_highlight_lines,
    trim_empty_line_edges, truncate_display_width, wrap_rich_line,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn span_fg(line: &RenderedLine, needle: &str) -> Option<ratatui::style::Color> {
        line.rich_line
            .as_ref()?
            .spans
            .iter()
            .find(|span| span.content.contains(needle))
            .map(|span| span.style.fg.unwrap_or(ratatui::style::Color::Reset))
    }

    #[test]
    fn expanded_diff_renders_a_highlighted_diff_card() {
        let diff = concat!(
            "diff --git a/src/main.rs b/src/main.rs\n",
            "index 123..456 100644\n",
            "--- a/src/main.rs\n",
            "+++ b/src/main.rs\n",
            "@@ -1,3 +1,4 @@\n",
            " fn main() {\n",
            "-    println!(\"old\");\n",
            "+    println!(\"new\");\n",
            " }\n",
        );
        let mut lines = Vec::new();
        push_expanded_diff_text(&mut lines, "  ", diff, 48);

        let rendered = lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
        assert!(
            rendered.first().is_some_and(|line| line.contains("┌─ diff · rs")),
            "card label should include the detected language: {rendered:?}"
        );
        assert!(
            rendered.last().is_some_and(|line| line.contains('└')),
            "card needs a bottom border: {rendered:?}"
        );

        let added = lines
            .iter()
            .find(|line| line.text.contains("println!(\"new\")"))
            .expect("added line");
        let plus_marker = added
            .rich_line
            .as_ref()
            .and_then(|line| line.spans.iter().find(|span| span.content == "+"))
            .expect("+ marker");
        assert_eq!(
            plus_marker.style.fg,
            Some(agena_tui_components::theme::success_color())
        );

        let removed = lines
            .iter()
            .find(|line| line.text.contains("println!(\"old\")"))
            .expect("removed line");
        let minus_marker = removed
            .rich_line
            .as_ref()
            .and_then(|line| line.spans.iter().find(|span| span.content == "-"))
            .expect("- marker");
        assert_eq!(
            minus_marker.style.fg,
            Some(agena_tui_components::theme::danger_color())
        );

        // The added content is syntax-highlighted with the file language
        // (rust), so at least one token is not the flat diff success color.
        let content_fgs = added
            .rich_line
            .as_ref()
            .expect("rich line")
            .spans
            .iter()
            .filter(|span| span.content != "+" && !span.content.trim().is_empty())
            .map(|span| span.style.fg)
            .collect::<Vec<_>>();
        assert!(
            content_fgs
                .iter()
                .any(|fg| *fg != Some(agena_tui_components::theme::success_color())),
            "added content should be syntax-highlighted: {content_fgs:?}"
        );

        assert_eq!(
            span_fg(
                lines
                    .iter()
                    .find(|line| line.text.contains("diff --git"))
                    .expect("header line"),
                "diff --git"
            ),
            Some(agena_tui_components::theme::accent_color())
        );
        assert_eq!(
            span_fg(
                lines
                    .iter()
                    .find(|line| line.text.contains("@@"))
                    .expect("hunk line"),
                "@@"
            ),
            Some(agena_tui_components::theme::warning_color())
        );
    }

    #[test]
    fn diff_target_language_detects_the_added_file() {
        assert_eq!(
            diff_target_language(&["diff --git a/x b/src/main.rs"]),
            Some("rs".to_string())
        );
        assert_eq!(
            diff_target_language(&["+++ b/app/models/user.py"]),
            Some("py".to_string())
        );
        assert_eq!(diff_target_language(&["+++ /dev/null"]), None);
        assert_eq!(diff_target_language(&["diff --git a/x b/y"]), None);
        assert_eq!(diff_target_language(&["plain text"]), None);
    }

    #[test]
    fn diff_line_kind_classifies_git_diff_lines() {
        assert_eq!(diff_line_kind("diff --git a/x b/x"), DiffLineKind::Header);
        assert_eq!(diff_line_kind("index 123..456 100644"), DiffLineKind::Header);
        assert_eq!(diff_line_kind("--- a/x"), DiffLineKind::Header);
        assert_eq!(diff_line_kind("+++ b/x"), DiffLineKind::Header);
        assert_eq!(diff_line_kind("new file mode 100644"), DiffLineKind::Header);
        assert_eq!(diff_line_kind("@@ -1,3 +1,4 @@"), DiffLineKind::Hunk);
        assert_eq!(diff_line_kind("+let x = 1;"), DiffLineKind::Added);
        assert_eq!(diff_line_kind("-let x = 1;"), DiffLineKind::Removed);
        assert_eq!(diff_line_kind(" fn main() {"), DiffLineKind::Context);
        assert_eq!(diff_line_kind(""), DiffLineKind::Context);
    }
}
