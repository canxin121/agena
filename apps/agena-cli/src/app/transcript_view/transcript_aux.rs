pub(in crate::app) fn push_limited_diff_text(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    width: u16,
    i18n: &I18n,
) {
    let preview = tool_output_preview_with_limits(
        text,
        TOOL_EXPANDED_PREVIEW_LINES,
        TOOL_EXPANDED_PREVIEW_CHARS,
    );
    for raw_line in preview.text.lines() {
        push_wrapped_line(
            out,
            prefix,
            prefix,
            raw_line,
            diff_line_style(raw_line),
            width,
        );
    }
    if preview.omitted_lines > 0 {
        push_multiline(
            out,
            prefix,
            &i18n.text_args(
                "message-tool-output-collapsed",
                &crate::fl_args!("lines" => preview.omitted_lines as i64),
            ),
            Style::default().fg(agena_tui_components::theme::muted_color()),
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
    let input = serde_json::Value::from(invocation.input.clone());
    if let Some(gateway_name) = gateway_model_tool_name(invocation.name.as_str())
        && let Some(target) = input.get("tool").and_then(serde_json::Value::as_str)
        && !target.trim().is_empty()
    {
        return format!("{gateway_name} · {}", target.trim());
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

pub(in crate::app) fn gateway_model_tool_name(name: &str) -> Option<&'static str> {
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
    I18n, RenderedLine, Style, TOOL_EXPANDED_PREVIEW_CHARS, TOOL_EXPANDED_PREVIEW_LINES,
    ToolInvocation, push_multiline, push_wrapped_line, tool_output_preview_with_limits,
};
