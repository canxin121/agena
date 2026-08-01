pub(crate) fn tool_execution_preview(
    part: &TranscriptEntryPart,
    tool: &OperationPartResource,
    _i18n: &I18n,
) -> String {
    let label = tool_invocation_label(&tool.invocation);
    format!("{} {label}", activity_status_icon(part.status))
}

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
) {
    push_markdown_document(out, prefix, text, width);
}

pub(crate) fn tool_output_copy_text(
    part: &TranscriptEntryPart,
    tool: &OperationPartResource,
    i18n: &I18n,
) -> String {
    let label = tool_display_label(tool);
    let mut sections = vec![tool_execution_preview(part, tool, i18n), label];
    if should_render_tool_model_output(tool, tool.error_message()) {
        sections.push(tool.model_output.text.trim().to_string());
    }
    if let Some(diff) = apply_patch_details(&tool.details).map(|payload| payload.diff)
        && !diff.trim().is_empty()
    {
        sections.push(diff.trim().to_string());
    }
    let operation_blocks = tool
        .blocks
        .iter()
        .map(|block| operation_block_copy_text(block, i18n))
        .collect::<Vec<_>>();
    if !operation_blocks.is_empty() {
        sections.push(operation_blocks.join("\n\n"));
    }
    sections
        .into_iter()
        .filter(|section| !section.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
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

/// Render the shared one-line Activity chrome with Markdown-aware title and
/// summary spans. Status belongs to the icon, title carries heading emphasis,
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
) {
    let disclosure = if toggleable && expanded { "▾" } else { "▸" };
    let icon = activity_status_icon(status);
    let chrome = format!("  {disclosure} {icon} ");
    let content_width = usize::from(width).saturating_sub(UnicodeWidthStr::width(chrome.as_str()));
    let (title, summary) =
        bounded_title_summary_parts(title, if expanded { "" } else { summary }, content_width);

    let title_style = Style::default().add_modifier(Modifier::BOLD);
    let summary_style = Style::default().fg(agena_tui_components::theme::muted_color());
    let mut content_spans = markdown_inline_line(title.as_str(), title_style)
        .map(|line| line.spans)
        .unwrap_or_else(|| vec![Span::styled(title.clone(), title_style)]);
    if !summary.is_empty() {
        content_spans.push(Span::styled(" · ", summary_style));
        content_spans.extend(
            markdown_inline_line(summary.as_str(), summary_style)
                .map(|line| line.spans)
                .unwrap_or_else(|| vec![Span::styled(summary.clone(), summary_style)]),
        );
    }
    let copy_text = content_spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let mut spans = vec![
        Span::raw("  "),
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
    out.push(
        RenderedLine::rich(line)
            .with_copy_projection(copy_text, UnicodeWidthStr::width(chrome.as_str())),
    );
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

/// Compose a one-line `title · summary` within a display-width budget.
///
/// The title has priority and is only truncated when it cannot fit by itself.
/// A summary is included when at least a useful fragment can fit in the
/// remaining space. Both fields are already durably bounded by the producer
/// contract; this final bound only adapts them to the current viewport.
#[cfg(test)]
pub(crate) fn bounded_title_summary(title: &str, summary: &str, max_width: usize) -> String {
    let (title, summary) = bounded_title_summary_parts(title, summary, max_width);
    if summary.is_empty() {
        title
    } else {
        format!("{title} · {summary}")
    }
}

#[cfg(test)]
pub(crate) fn tool_execution_compact_summary(
    status: PartExecutionStatusResource,
    tool: &OperationPartResource,
    width: u16,
) -> String {
    const COLLAPSED_PREFIX_WIDTH: usize = 4;
    const STATUS_WIDTH: usize = 2;
    let content_width = usize::from(width)
        .saturating_sub(COLLAPSED_PREFIX_WIDTH)
        .saturating_sub(STATUS_WIDTH);
    format!(
        "{} {}",
        activity_status_icon(status),
        bounded_title_summary(
            tool_display_label(tool).as_str(),
            tool.summary.as_str(),
            content_width,
        )
    )
}

pub(crate) fn bounded_title_summary_parts(
    title: &str,
    summary: &str,
    max_width: usize,
) -> (String, String) {
    const SEPARATOR: &str = " · ";
    const MIN_SUMMARY_WIDTH: usize = 8;

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
        return (
            truncate_with_ellipsis(summary.as_str(), max_width),
            String::new(),
        );
    }
    let title_width = UnicodeWidthStr::width(title.as_str());
    if title_width > max_width {
        return (
            truncate_with_ellipsis(title.as_str(), max_width),
            String::new(),
        );
    }
    if summary.is_empty()
        || normalized_tool_text(title.as_str()) == normalized_tool_text(summary.as_str())
    {
        return (title, String::new());
    }

    let separator_width = UnicodeWidthStr::width(SEPARATOR);
    let summary_width = max_width
        .saturating_sub(title_width)
        .saturating_sub(separator_width);
    if summary_width < MIN_SUMMARY_WIDTH {
        return (title, String::new());
    }
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
    let content_width = max_width - 1;
    let mut width = 0_usize;
    let mut bounded = String::new();
    for grapheme in value.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width.saturating_add(grapheme_width) > content_width {
            break;
        }
        bounded.push_str(grapheme);
        width = width.saturating_add(grapheme_width);
    }
    bounded.truncate(bounded.trim_end().len());
    bounded.push('…');
    bounded
}

/// Return the execution-tool name plus its arguments, unwrapping `tools.call` so a
/// user sees `fs.grep` or `web.search`, rather than the Tool API implementation
/// that happened to dispatch it.
pub(crate) fn compact_tool_identity(
    invocation: &ToolInvocationResource,
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
    match name.trim() {
        "agena_web__fetch" | "web.fetch" | "web_fetch" | "fetch" => "web.fetch".to_string(),
        "agena_web__search" | "web.search" | "web_search" | "search" => "web.search".to_string(),
        "agena_fs_read" | "agena.fs.read" | "read" => "fs.read".to_string(),
        "agena_fs_glob" | "agena.fs.glob" | "glob" => "fs.glob".to_string(),
        "agena_fs_grep" | "agena.fs.grep" | "grep" => "fs.grep".to_string(),
        "agena_fs_apply_patch" | "agena.fs.apply_patch" | "apply_patch" => {
            "fs.apply_patch".to_string()
        }
        "agena_shell_run" | "agena.shell.run" | "agena_process_run" | "agena.process.run" => {
            "shell.run".to_string()
        }
        "agena_shell_list" | "agena.shell.list" | "agena_process_list" | "agena.process.list" => {
            "shell.list".to_string()
        }
        "agena_shell_logs" | "agena.shell.logs" | "agena_process_logs" | "agena.process.logs" => {
            "shell.logs".to_string()
        }
        "agena_shell_stop" | "agena.shell.stop" | "agena_process_stop" | "agena.process.stop" => {
            "shell.stop".to_string()
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
    Color, I18n, Line, Modifier, RenderedLine, Span, Style, UnicodeWidthStr, apply_patch_details,
    normalized_tool_text, operation_block_copy_text, push_markdown_document, push_multiline,
    push_wrapped_line, sanitize_terminal_text, should_render_tool_model_output,
    tool_api_display_name, tool_display_label, tool_invocation_label, tool_output_preview,
    truncate_display_width,
};
use crate::{
    OperationPartResource, PartExecutionStatusResource, ToolInvocationResource,
    TranscriptEntryPart, truncate_rich_line,
};
use unicode_segmentation::UnicodeSegmentation;
