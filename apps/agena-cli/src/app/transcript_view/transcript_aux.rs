pub(in crate::app) fn push_expanded_diff_text(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    width: u16,
) {
    let sanitized = sanitize_terminal_text(text);
    let normalized = trim_empty_line_edges(sanitized.as_str());
    for raw_line in normalized.lines() {
        push_wrapped_line(
            out,
            prefix,
            prefix,
            raw_line,
            diff_line_style(raw_line),
            width,
        );
    }
}

pub(in crate::app) fn diff_line_style(line: &str) -> Style {
    if line.starts_with("diff --git ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
        || line.starts_with("new file mode ")
        || line.starts_with("deleted file mode ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
    {
        Style::default().fg(agena_tui_components::theme::accent_color())
    } else if line.starts_with("@@") {
        Style::default().fg(agena_tui_components::theme::warning_color())
    } else if line.starts_with('+') {
        Style::default().fg(agena_tui_components::theme::success_color())
    } else if line.starts_with('-') {
        Style::default().fg(agena_tui_components::theme::danger_color())
    } else {
        Style::default().fg(agena_tui_components::theme::muted_color())
    }
}

pub(in crate::app) fn tool_invocation_label(invocation: &ToolInvocation) -> String {
    invocation.display_label(" · ", true)
}
use super::{
    RenderedLine, Style, ToolInvocation, push_wrapped_line, sanitize_terminal_text,
    trim_empty_line_edges,
};
