pub(crate) fn first_non_empty_preview_line(text: &str) -> Option<String> {
    sanitize_terminal_text(text)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

pub(crate) fn push_section_heading(
    out: &mut Vec<RenderedLine>,
    heading: &str,
    style: Style,
    width: u16,
) {
    push_wrapped_line(out, "", "", heading, style, width);
}

pub(crate) fn push_label_value(
    out: &mut Vec<RenderedLine>,
    label: &str,
    value: &str,
    style: Style,
    width: u16,
) {
    let continuation = " ".repeat(UnicodeWidthStr::width(label));
    push_wrapped_line(out, label, continuation.as_str(), value, style, width);
}

/// One collapsed run of activities, rendered from the shared fold vocabulary.
/// Server-projected folds and the client's visible-budget collapse must look
/// and copy identically.
pub(crate) fn fold_marker_summary(i18n: &I18n, hidden_count: usize) -> String {
    i18n.text_args(
        "message-activity-run-collapsed",
        &agena_tui::fl_args!("count" => hidden_count as i64, "load" => super::COLLAPSED_ACTIVITY_VISIBLE_COUNT as i64),
    )
}

/// Push the collapsed-run marker row and return the text it carries, which is
/// also the row's copy projection.
pub(crate) fn push_fold_marker_row(
    out: &mut Vec<RenderedLine>,
    i18n: &I18n,
    hidden_count: usize,
    width: u16,
) -> String {
    let summary = fold_marker_summary(i18n, hidden_count);
    push_single_line(
        out,
        "",
        summary.as_str(),
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
    summary
}

pub(crate) fn push_single_line(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    style: Style,
    width: u16,
) {
    let available_width = width.max(1) as usize;
    let prefix_width = UnicodeWidthStr::width(prefix);
    if prefix_width >= available_width {
        out.push(
            RenderedLine::plain(truncate_display_width(prefix, available_width), style)
                .with_copy_projection(String::new(), available_width),
        );
        return;
    }
    let body = truncate_display_width(text, available_width.saturating_sub(prefix_width));
    out.push(
        RenderedLine::plain(format!("{prefix}{body}"), style)
            .with_copy_projection(body, prefix_width),
    );
}

pub(crate) fn push_collapsible_text(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    style: Style,
    width: u16,
    i18n: &I18n,
) {
    let preview = tool_output_preview(text);
    push_multiline(out, prefix, preview.text.as_str(), style, width);
    if preview.omitted_lines > 0 {
        push_multiline(
            out,
            prefix,
            &i18n.text_args(
                "message-tool-output-collapsed",
                &agena_tui::fl_args!("lines" => preview.omitted_lines as i64),
            ),
            Style::default().fg(agena_tui_components::theme::muted_color()),
            width,
        );
    }
}

pub(crate) fn push_expanded_tool_text(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    style: Style,
    width: u16,
) {
    push_multiline(out, prefix, text, style, width);
}

pub(crate) fn push_expanded_markdown(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    width: u16,
    i18n: &I18n,
) {
    push_markdown_document(out, prefix, text, width, i18n);
}

pub(crate) fn tool_status_color(status: PartExecutionStatusResource) -> Color {
    match status {
        PartExecutionStatusResource::Pending | PartExecutionStatusResource::InProgress => {
            agena_tui_components::theme::special_color()
        }
        PartExecutionStatusResource::Completed => agena_tui_components::theme::success_color(),
        PartExecutionStatusResource::PolicyDenied => agena_tui_components::theme::warning_color(),
        PartExecutionStatusResource::UserDeclined => agena_tui_components::theme::muted_color(),
        PartExecutionStatusResource::CapabilityUnavailable
        | PartExecutionStatusResource::ToolUnavailable => {
            agena_tui_components::theme::warning_color()
        }
        PartExecutionStatusResource::Failed => agena_tui_components::theme::danger_color(),
        PartExecutionStatusResource::Cancelled => agena_tui_components::theme::muted_color(),
    }
}

pub(crate) fn activity_status_icon(status: PartExecutionStatusResource) -> &'static str {
    match status {
        PartExecutionStatusResource::Pending => "○",
        PartExecutionStatusResource::InProgress => transcript_spinner_placeholder(),
        PartExecutionStatusResource::Completed => "●",
        PartExecutionStatusResource::PolicyDenied => "⊘",
        PartExecutionStatusResource::UserDeclined => "–",
        PartExecutionStatusResource::CapabilityUnavailable
        | PartExecutionStatusResource::ToolUnavailable => "◇",
        PartExecutionStatusResource::Failed => "×",
        PartExecutionStatusResource::Cancelled => "–",
    }
}

enum ActivitySummary<'a> {
    Markdown(&'a str),
    Literal(&'a str),
}

/// Render the shared one-line Activity chrome with a literal title and
/// Markdown-aware summary. Status belongs to the icon, title carries heading emphasis,
/// and the compact summary stays visually subordinate instead of tinting and
/// bolding the entire row as one undifferentiated string.
pub(crate) fn push_activity_headline(
    out: &mut Vec<RenderedLine>,
    status: PartExecutionStatusResource,
    expanded: bool,
    toggleable: bool,
    title: &str,
    summary: &str,
    width: u16,
    i18n: &I18n,
) {
    push_activity_headline_content(
        out,
        status,
        expanded,
        toggleable,
        title,
        ActivitySummary::Markdown(summary),
        width,
        i18n,
    );
}

/// File and command names are literal identifiers, including Markdown syntax.
pub(crate) fn push_literal_activity_headline(
    out: &mut Vec<RenderedLine>,
    status: PartExecutionStatusResource,
    expanded: bool,
    toggleable: bool,
    title: &str,
    summary: &str,
    width: u16,
    i18n: &I18n,
) {
    push_activity_headline_content(
        out,
        status,
        expanded,
        toggleable,
        title,
        ActivitySummary::Literal(summary),
        width,
        i18n,
    );
}

fn push_activity_headline_content(
    out: &mut Vec<RenderedLine>,
    status: PartExecutionStatusResource,
    expanded: bool,
    toggleable: bool,
    title: &str,
    summary: ActivitySummary<'_>,
    width: u16,
    i18n: &I18n,
) {
    let disclosure = if toggleable && expanded { "▾" } else { "▸" };
    let icon = activity_status_icon(status);
    let chrome = format!("{disclosure} {icon} ");
    let content_width = usize::from(width).saturating_sub(UnicodeWidthStr::width(chrome.as_str()));
    let summary_style = Style::default().fg(agena_tui_components::theme::muted_color());
    let summary = match summary {
        ActivitySummary::Markdown(summary) => {
            let source = crate::part_headline::first_content_line(summary).unwrap_or_default();
            markdown_inline_line(&source, summary_style, i18n)
                .unwrap_or_else(|| Line::from(Span::styled(source, summary_style)))
        }
        ActivitySummary::Literal(summary) => Line::from(Span::styled(
            sanitize_terminal_text(summary)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            summary_style,
        )),
    };
    let summary_text = summary
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let (title, bounded_summary) = bounded_title_summary_parts(title, &summary_text, content_width);

    let title_style = Style::default().add_modifier(Modifier::BOLD);
    // Titles contain shell syntax and paths. Interpreting `$...$`, backticks
    // or underscores as prose changes the command the user sees and copies.
    let mut content_spans = vec![Span::styled(title.clone(), title_style)];
    if !bounded_summary.is_empty() {
        content_spans.push(Span::styled(" · ", summary_style));
        content_spans.extend(
            truncate_rich_line(summary, UnicodeWidthStr::width(bounded_summary.as_str())).spans,
        );
    }
    let mut spans = vec![
        Span::styled(
            disclosure,
            Style::default().fg(agena_tui_components::theme::muted_color()),
        ),
        Span::raw(" "),
        Span::styled(icon, Style::default().fg(tool_status_color(status))),
        Span::raw(" "),
    ];
    spans.extend(content_spans);
    let line = truncate_rich_line(Line::from(spans), usize::from(width.max(1)));
    let visible = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let copy_text = visible.strip_prefix(&chrome).unwrap_or_default().to_owned();
    out.push(RenderedLine::rich(line).with_copy_projection(
        copy_text,
        UnicodeWidthStr::width(chrome.as_str()).min(usize::from(width)),
    ));
}

pub(crate) const fn transcript_spinner_placeholder() -> &'static str {
    "\u{e000}"
}

pub fn current_spinner_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

pub fn spinner_frame(elapsed_millis: u128) -> &'static str {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    FRAMES[((elapsed_millis / 100) % FRAMES.len() as u128) as usize]
}

pub fn refresh_spinner_line(mut line: Line<'static>, frame: &str) -> Line<'static> {
    for span in &mut line.spans {
        if span.content.contains(transcript_spinner_placeholder()) {
            let mut refreshed = String::with_capacity(span.content.len());
            for character in span.content.chars() {
                if character == '\u{e000}' {
                    refreshed.push_str(frame);
                } else {
                    refreshed.push(character);
                }
            }
            span.content = refreshed.into();
        }
    }
    line
}

pub(crate) fn bounded_title_summary_parts(
    title: &str,
    summary: &str,
    max_width: usize,
) -> (String, String) {
    const SEPARATOR: &str = " · ";
    const MIN_TITLE_WIDTH: usize = 8;
    const MIN_SUMMARY_WIDTH: usize = 6;

    let title = sanitize_terminal_text(title)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let summary = sanitize_terminal_text(summary)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if max_width == 0 {
        return (String::new(), String::new());
    }
    if title.is_empty() {
        return (truncate_with_ellipsis(&summary, max_width), String::new());
    }
    let separator_width = UnicodeWidthStr::width(SEPARATOR);
    if summary.is_empty()
        || normalized_tool_text(&title) == normalized_tool_text(&summary)
        || max_width < MIN_TITLE_WIDTH + separator_width + MIN_SUMMARY_WIDTH
    {
        return (truncate_with_ellipsis(&title, max_width), String::new());
    }
    // Reserve useful subject text even when the title contains a long path or
    // command. Both pieces stay on one physical line at every disclosure state.
    let reserved_summary =
        UnicodeWidthStr::width(summary.as_str()).min((max_width / 3).clamp(MIN_SUMMARY_WIDTH, 32));
    let title = truncate_with_ellipsis(&title, max_width - separator_width - reserved_summary);
    let summary_width = max_width - UnicodeWidthStr::width(title.as_str()) - separator_width;
    (
        title,
        truncate_with_ellipsis(summary.as_str(), summary_width),
    )
}

fn truncate_with_ellipsis(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let mut bounded = agena_tui_components::text::clip_display_width(value, max_width - 1);
    bounded.truncate(bounded.trim_end().len());
    bounded.push('…');
    bounded
}

/// Return the execution-tool name plus its arguments, unwrapping `tools.call` so a
/// user sees `fs.grep` or `web.search`, rather than the Tool API implementation
/// that happened to dispatch it.
pub(crate) fn compact_tool_identity(
    invocation: &agena_domain::ToolInvocation,
) -> (String, serde_json::Value) {
    let input = serde_json::Value::from(invocation.input.clone());
    if tool_api_display_name(invocation.name.as_str()) == Some("tools.call")
        && let Some(tool_name) = input.get("tool").and_then(serde_json::Value::as_str)
        && !tool_name.trim().is_empty()
    {
        let tool_input = input
            .get("input")
            .cloned()
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()));
        return (compact_tool_name(tool_name), tool_input);
    }
    (compact_tool_name(invocation.name.as_str()), input)
}

pub(crate) fn compact_tool_name(name: &str) -> String {
    if let Some(display_name) = tool_api_display_name(name) {
        return display_name.to_owned();
    }
    if let Some(name) = agena_domain::ToolPermissionConfig::canonical_shell_tool_name(name.trim()) {
        return name.strip_prefix("agena.").unwrap_or(name).to_owned();
    }
    match name.trim() {
        "agena_web__fetch" | "web.fetch" | "web_fetch" | "fetch" => "web.fetch".to_string(),
        "agena_web__search" | "web.search" | "web_search" | "search" => "web.search".to_string(),
        "agena_fs_read" | "agena.fs.read" | "read" => "fs.read".to_string(),
        "agena_fs_glob" | "agena.fs.glob" | "glob" => "fs.glob".to_string(),
        "agena_fs_grep" | "agena.fs.grep" | "grep" => "fs.grep".to_string(),
        "agena_fs_apply_patch" | "agena.fs.apply_patch" | "apply_patch" => {
            "fs.apply_patch".to_string()
        }
        other => other.strip_prefix("agena.").unwrap_or(other).to_string(),
    }
}

pub(crate) fn concise_text(text: &str, max_width: usize) -> String {
    let normalized = sanitize_terminal_text(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    truncate_display_width(normalized.as_str(), max_width)
}

use super::transcript_ast::markdown_inline_line;
use super::{
    Color, I18n, Line, Modifier, RenderedLine, Span, Style, UnicodeWidthStr, normalized_tool_text,
    push_markdown_document, push_multiline, push_wrapped_line, sanitize_terminal_text,
    tool_api_display_name, tool_output_preview, truncate_display_width,
};
use crate::{PartExecutionStatusResource, truncate_rich_line};

#[cfg(test)]
mod terminal_title_tests {
    use super::*;

    #[test]
    fn shell_headline_preserves_command_syntax() {
        let command =
            r#"Open interactive terminal · for i in $(seq 1 10); do echo "$i"; sleep 1; done"#;
        let mut lines = Vec::new();
        push_activity_headline(
            &mut lines,
            PartExecutionStatusResource::Completed,
            true,
            true,
            command,
            "",
            160,
            &I18n::english(),
        );
        assert!(lines[0].text.ends_with(command), "{}", lines[0].text);
        assert_eq!(lines[0].copy_text, command);
    }
}
