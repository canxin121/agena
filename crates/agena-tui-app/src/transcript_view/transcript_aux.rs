pub(crate) fn push_expanded_diff_text(
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

pub(crate) fn diff_line_style(line: &str) -> Style {
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
    match name.trim() {
        "agena.tools.list" | "tools.list" | "tools_list" => Some("tools.list"),
        "agena.tools.search" | "tools.search" | "tools_search" => Some("tools.search"),
        "agena.tools.help" | "tools.help" | "tools_help" => Some("tools.help"),
        "agena.tools.tags" | "tools.tags" | "tools_tags" => Some("tools.tags"),
        "agena.tools.call" | "tools.call" | "tools_call" => Some("tools.call"),
        _ => None,
    }
}
use super::{
    RenderedLine, Style, push_wrapped_line, sanitize_terminal_text, trim_empty_line_edges,
};
