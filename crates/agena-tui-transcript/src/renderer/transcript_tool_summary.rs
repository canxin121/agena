pub(crate) fn tool_execution_preview(
    part: &MessagePartResource,
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
    push_markdown(out, prefix, text, width);
}

pub(crate) fn tool_output_copy_text(
    part: &MessagePartResource,
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
        PartExecutionStatusResource::Failed => agena_tui_components::theme::danger_color(),
        PartExecutionStatusResource::Cancelled => agena_tui_components::theme::muted_color(),
    }
}

pub(crate) fn activity_status_icon(status: PartExecutionStatusResource) -> &'static str {
    match status {
        PartExecutionStatusResource::Pending => "○",
        PartExecutionStatusResource::InProgress => transcript_spinner_placeholder(),
        PartExecutionStatusResource::Completed => "●",
        PartExecutionStatusResource::Failed => "×",
        PartExecutionStatusResource::Cancelled => "–",
    }
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

pub(crate) fn tool_execution_status_summary(part: &MessagePartResource, label: &str) -> String {
    format!("{} {label}", activity_status_icon(part.status))
}

pub(crate) fn tool_execution_collapsed_summary(
    part: &MessagePartResource,
    tool: &OperationPartResource,
    _: &I18n,
) -> String {
    tool_execution_compact_summary(part.status, tool)
}

/// Build the one-line presentation for a folded operation.  The operation's
/// title is deliberately not used as the primary label here: providers often
/// send generic titles such as "Tool tools.call" or "Apply patch".  The
/// invocation contains the execution-tool name and arguments, which are more
/// useful when scanning a busy transcript.
pub(crate) fn tool_execution_compact_summary(
    status: PartExecutionStatusResource,
    tool: &OperationPartResource,
) -> String {
    let (name, input) = compact_tool_identity(&tool.invocation);
    let mut parts = vec![name.clone()];
    if let Some(subject) = compact_tool_subject(name.as_str(), &input, tool) {
        parts.push(subject);
    }
    if let Some(outcome) = compact_tool_outcome(status, name.as_str(), tool)
        && !parts
            .iter()
            .any(|part| normalized_tool_text(part) == normalized_tool_text(outcome.as_str()))
    {
        parts.push(outcome);
    }
    tool_execution_status_summary_for_status(status, parts.join(" · ").as_str())
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

pub(crate) fn compact_tool_subject(
    name: &str,
    input: &serde_json::Value,
    tool: &OperationPartResource,
) -> Option<String> {
    match name {
        "fs.apply_patch" => compact_patch_summary(input, tool),
        "fs.read" => compact_read_subject(input),
        "fs.glob" => compact_glob_subject(input),
        "fs.grep" => compact_grep_subject(input),
        "web.search" => compact_string_field(input, "query").map(compact_query),
        "web.fetch" | "web.crawl" => compact_string_field(input, "url").map(compact_url),
        "shell.run" => compact_string_field(input, "command").map(compact_command),
        "shell.logs" | "shell.stop" => compact_string_field(input, "process_id"),
        "mcp.tools.call" | "mcp.call" => compact_mcp_subject(input),
        _ => compact_generic_subject(input),
    }
}

pub(crate) fn compact_patch_summary(
    input: &serde_json::Value,
    tool: &OperationPartResource,
) -> Option<String> {
    let patch = compact_string_field(input, "patch");
    let changes = apply_patch_details(&tool.details)
        .map(|payload| payload.changes)
        .or_else(|| {
            tool.blocks.iter().find_map(|block| match block {
                OperationBlockResource::FileChanges { changes } => Some(changes.clone()),
                _ => None,
            })
        })
        .filter(|changes| !changes.is_empty());
    let path_preview = changes
        .as_deref()
        .map(compact_file_change_preview)
        .or_else(|| patch.as_deref().and_then(compact_patch_path_preview));
    let diff = apply_patch_details(&tool.details)
        .map(|payload| payload.diff)
        .filter(|diff| !diff.trim().is_empty())
        .or_else(|| {
            tool.blocks.iter().find_map(|block| match block {
                OperationBlockResource::Diff { diff, .. } if !diff.trim().is_empty() => {
                    Some(diff.clone())
                }
                _ => None,
            })
        })
        .or(patch);
    let stats = diff
        .as_deref()
        .map(|diff| diff_stats(diff, changes.as_deref()))
        .unwrap_or(DiffStats {
            file_count: changes.as_ref().map_or(0, Vec::len),
            additions: 0,
            deletions: 0,
            renames: 0,
            line_count: 0,
        });
    let delta = compact_diff_delta(stats.additions, stats.deletions);
    match (path_preview, delta) {
        (Some(paths), Some(delta)) => Some(format!("{paths} · {delta}")),
        (Some(paths), None) => Some(paths),
        (None, Some(delta)) => Some(delta),
        (None, None) => Some("patch".to_string()),
    }
}

pub(crate) fn compact_read_subject(input: &serde_json::Value) -> Option<String> {
    let path =
        compact_string_field(input, "file_path").or_else(|| compact_string_field(input, "path"))?;
    let offset = input.get("offset").and_then(serde_json::Value::as_u64);
    let limit = input.get("limit").and_then(serde_json::Value::as_u64);
    match (offset, limit) {
        (Some(offset), Some(limit)) => Some(format!("{path}:{offset}–{}", offset + limit)),
        (Some(offset), None) => Some(format!("{path}:{offset}")),
        _ => Some(path),
    }
}

pub(crate) fn compact_glob_subject(input: &serde_json::Value) -> Option<String> {
    let pattern = compact_string_field(input, "pattern")?;
    compact_string_field(input, "path")
        .filter(|path| !path.is_empty())
        .map(|path| format!("{pattern} in {path}"))
        .or(Some(pattern))
}

pub(crate) fn compact_grep_subject(input: &serde_json::Value) -> Option<String> {
    let pattern = compact_string_field(input, "pattern")?;
    let mut subject = format!("/{pattern}/");
    if let Some(path) = compact_string_field(input, "path").filter(|path| !path.is_empty()) {
        subject.push_str(" in ");
        subject.push_str(path.as_str());
    }
    if let Some(include) =
        compact_string_field(input, "include").filter(|include| !include.is_empty())
    {
        subject.push_str(" · ");
        subject.push_str(include.as_str());
    }
    Some(subject)
}

pub(crate) fn compact_mcp_subject(input: &serde_json::Value) -> Option<String> {
    let server = compact_string_field(input, "server");
    let name = compact_string_field(input, "name");
    match (server, name) {
        (Some(server), Some(name)) => Some(format!("{server}/{name}")),
        (Some(server), None) => Some(server),
        (None, Some(name)) => Some(name),
        (None, None) => None,
    }
}

pub(crate) fn compact_generic_subject(input: &serde_json::Value) -> Option<String> {
    for key in [
        "command",
        "file_path",
        "path",
        "query",
        "url",
        "tool",
        "name",
        "server",
        "process_id",
        "pattern",
        "id",
        "action",
        "description",
    ] {
        if let Some(value) = compact_string_field(input, key) {
            return Some(match key {
                "command" => compact_command(value),
                "query" => compact_query(value),
                "url" => compact_url(value),
                _ => value,
            });
        }
    }
    None
}

pub(crate) fn compact_string_field(input: &serde_json::Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn compact_query(query: String) -> String {
    format!("“{}”", concise_text(query.as_str(), 72))
}

pub(crate) fn compact_url(url: String) -> String {
    concise_text(
        url.trim_start_matches("https://")
            .trim_start_matches("http://"),
        96,
    )
}

pub(crate) fn compact_command(command: String) -> String {
    concise_text(command.as_str(), 96)
}

pub(crate) fn concise_text(text: &str, max_width: usize) -> String {
    let normalized = sanitize_terminal_text(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    truncate_display_width(normalized.as_str(), max_width)
}

pub(crate) fn compact_patch_path_preview(patch: &str) -> Option<String> {
    let mut entries = Vec::new();
    for line in patch.lines() {
        let (marker, path) = if let Some(path) = line.strip_prefix("*** Add File: ") {
            ("A", path)
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            ("M", path)
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            ("D", path)
        } else if let Some(path) = line.strip_prefix("+++ b/") {
            ("M", path)
        } else {
            continue;
        };
        let entry = format!("{marker} {}", path.trim());
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    compact_preview_entries(entries)
}

pub(crate) fn compact_file_change_preview(
    changes: &[agena_api::message_part::FileChangeRecordResource],
) -> String {
    let entries = changes
        .iter()
        .map(|change| {
            format!(
                "{} {}",
                file_change_marker(change.kind),
                file_change_display_path(change)
            )
        })
        .collect::<Vec<_>>();
    compact_preview_entries(entries).unwrap_or_else(|| "patch".to_string())
}

pub(crate) fn compact_preview_entries(mut entries: Vec<String>) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let omitted = entries.len().saturating_sub(2);
    entries.truncate(2);
    if omitted > 0 {
        entries.push(format!("+{omitted} files"));
    }
    Some(entries.join(", "))
}

pub(crate) fn compact_diff_delta(additions: usize, deletions: usize) -> Option<String> {
    let mut parts = Vec::new();
    if additions > 0 {
        parts.push(format!("+{additions}"));
    }
    if deletions > 0 {
        parts.push(format!("−{deletions}"));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

pub(crate) fn compact_tool_outcome(
    status: PartExecutionStatusResource,
    name: &str,
    tool: &OperationPartResource,
) -> Option<String> {
    match status {
        PartExecutionStatusResource::Failed => tool
            .error_message()
            .or_else(|| (!tool.summary.trim().is_empty()).then_some(tool.summary.as_str()))
            .or_else(|| {
                (!tool.model_output.text.trim().is_empty())
                    .then_some(tool.model_output.text.as_str())
            })
            .map(|message| concise_text(message, 96)),
        PartExecutionStatusResource::Cancelled => Some("cancelled".to_string()),
        PartExecutionStatusResource::Pending
            if tool
                .title
                .to_ascii_lowercase()
                .contains("awaiting permission") =>
        {
            Some("awaiting approval".to_string())
        }
        PartExecutionStatusResource::Pending | PartExecutionStatusResource::InProgress => None,
        PartExecutionStatusResource::Completed => compact_completed_outcome(name, tool),
    }
}

pub(crate) fn compact_completed_outcome(
    name: &str,
    tool: &OperationPartResource,
) -> Option<String> {
    if name != "fs.apply_patch" {
        if let Some(exit_code) = tool.blocks.iter().find_map(|block| match block {
            OperationBlockResource::Command { exit_code, .. } => *exit_code,
            _ => None,
        }) {
            return (exit_code != 0).then(|| format!("exit {exit_code}"));
        }
        if let Some(result_count) = tool.blocks.iter().find_map(|block| match block {
            OperationBlockResource::SearchResults { results, .. } => Some(results.len()),
            _ => None,
        }) {
            return Some(format!("{result_count} results"));
        }
        if let Some(changes) = tool.blocks.iter().find_map(|block| match block {
            OperationBlockResource::FileChanges { changes } => Some(changes.len()),
            _ => None,
        }) {
            return Some(format!("{changes} files changed"));
        }
    }

    let payload = serde_json::Value::from(tool.details.payload.clone());
    let count = payload
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| payload.get("matches").and_then(serde_json::Value::as_u64))
        .or_else(|| {
            payload
                .get("matches")
                .and_then(serde_json::Value::as_array)
                .map(|items| items.len() as u64)
        })
        .or_else(|| {
            payload
                .get("results")
                .and_then(serde_json::Value::as_array)
                .map(|items| items.len() as u64)
        });
    count.map(|count| match name {
        "fs.glob" => format!("{count} paths"),
        "fs.grep" => format!("{count} matches"),
        _ => format!("{count} items"),
    })
}

pub(crate) fn tool_execution_status_summary_for_status(
    status: PartExecutionStatusResource,
    label: &str,
) -> String {
    format!("{} {label}", activity_status_icon(status))
}
use super::{
    Color, DiffStats, I18n, Line, RenderedLine, Style, UnicodeWidthStr, apply_patch_details,
    diff_stats, file_change_display_path, file_change_marker, normalized_tool_text,
    operation_block_copy_text, push_markdown, push_multiline, push_wrapped_line,
    sanitize_terminal_text, should_render_tool_model_output, tool_api_display_name,
    tool_display_label, tool_invocation_label, tool_output_preview, truncate_display_width,
};
use crate::{
    MessagePartResource, OperationBlockResource, OperationPartResource,
    PartExecutionStatusResource, ToolInvocationResource,
};
