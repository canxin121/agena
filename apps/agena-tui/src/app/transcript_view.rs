use super::*;
use agena::message::{ExecutionStatus, FileChangeKind, MessageStatus, OperationBlock, RequestPart};
use textwrap::{Options as WrapOptions, WordSplitter, wrap};
use tui_markdown::from_str as markdown_to_text;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub(super) fn render_message(
    message: &MessageResource,
    width: u16,
    i18n: &I18n,
) -> Vec<RenderedLine> {
    let mut lines = Vec::new();
    push_message_header(&mut lines, message, width, i18n);

    match message.parts.as_ref() {
        Some(parts) if parts.is_empty() => {
            lines.push(RenderedLine::dim(format!(
                "  {}",
                ui_text::t(i18n, "message-empty")
            )));
        }
        Some(parts) => {
            for part in parts {
                render_part(part, width, &mut lines, i18n);
            }
        }
        None => {
            lines.push(RenderedLine::dim(format!(
                "  {}",
                ui_text::message_parts_not_loaded(i18n, message.part_count as usize),
            )));
        }
    }

    lines
}

pub(super) fn rewind_message_preview(message: &MessageResource, i18n: &I18n) -> String {
    let preview = message
        .parts
        .as_ref()
        .and_then(|parts| parts.iter().find_map(|part| preview_for_part(part, i18n)))
        .or_else(|| {
            message
                .parts
                .is_none()
                .then(|| ui_text::message_parts_not_loaded(i18n, message.part_count as usize))
        })
        .unwrap_or_else(|| ui_text::t(i18n, "message-empty"));
    truncate_display_width(preview.as_str(), 64)
}

pub(super) fn render_transcript_export_markdown(
    i18n: &I18n,
    session_id: Option<i64>,
    session_title: &str,
    execution: Option<&SessionExecutionResource>,
    messages: &[MessageResource],
    has_more_older: bool,
) -> String {
    if session_id.is_none() && messages.is_empty() {
        return String::new();
    }

    let title = if !session_title.trim().is_empty() {
        session_title.trim().to_string()
    } else if let Some(session_id) = session_id {
        ui_text::session_fallback_title(i18n, session_id)
    } else {
        "Agena Transcript Export".to_string()
    };

    let mut out = vec![format!("# {title}"), String::new()];
    if let Some(session_id) = session_id {
        out.push(format!("- Session ID: {session_id}"));
    }
    out.push(format!(
        "- Exported At: {}",
        Local::now().format("%Y-%m-%d %H:%M:%S %z")
    ));
    out.push(format!("- Messages Loaded: {}", messages.len()));
    out.push(format!(
        "- Older Messages Omitted: {}",
        if has_more_older { "yes" } else { "no" }
    ));
    if let Some(execution) = execution {
        if let Some(parent_id) = execution.session.parent_id {
            out.push(format!("- Parent Session: #{parent_id}"));
        }
        out.push(format!(
            "- Child Sessions: {}",
            execution.session.child_session_count
        ));
    }
    out.push(String::new());

    if messages.is_empty() {
        out.push("_No messages loaded in this session._".to_string());
        return out.join("\n");
    }

    for message in messages {
        let timestamp = format_timestamp(message.created_at);
        out.push(format!(
            "## {} · {} · {}",
            ui_text::role_label(i18n, message.role),
            ui_text::message_state_label(i18n, message.state),
            timestamp,
        ));
        out.push(String::new());
        out.push("~~~~text".to_string());
        out.extend(
            render_message(message, u16::MAX, i18n)
                .into_iter()
                .map(|line| line.text),
        );
        out.push("~~~~".to_string());
        out.push(String::new());
    }

    out.join("\n")
}

pub(super) fn tool_output_preview(text: &str) -> ToolOutputPreview {
    let normalized = trim_empty_line_edges(sanitize_terminal_text(text).as_str());
    if normalized.is_empty() {
        return ToolOutputPreview {
            text: String::new(),
            omitted_lines: 0,
        };
    }

    let total_lines = normalized.split('\n').count();
    let mut preview = String::new();
    let mut used_chars = 0_usize;
    let mut included_lines = 0_usize;
    let mut truncated = false;

    for (index, line) in normalized.split('\n').enumerate() {
        if index >= TOOL_CARD_PREVIEW_LINES {
            truncated = true;
            break;
        }

        let separator_chars = usize::from(index > 0);
        let line_chars = line.chars().count();
        if used_chars
            .saturating_add(separator_chars)
            .saturating_add(line_chars)
            > TOOL_CARD_PREVIEW_CHARS
        {
            if index > 0 {
                preview.push('\n');
            }
            let remaining = TOOL_CARD_PREVIEW_CHARS
                .saturating_sub(used_chars)
                .saturating_sub(separator_chars);
            preview.extend(line.chars().take(remaining));
            included_lines = index + 1;
            truncated = true;
            break;
        }

        if index > 0 {
            preview.push('\n');
            used_chars += 1;
        }
        preview.push_str(line);
        used_chars += line_chars;
        included_lines = index + 1;
    }

    let mut omitted_lines = if truncated {
        total_lines.saturating_sub(included_lines)
    } else {
        0
    };
    if truncated && omitted_lines == 0 {
        omitted_lines = 1;
    }

    ToolOutputPreview {
        text: preview,
        omitted_lines,
    }
}

pub(super) fn sanitize_terminal_text(text: &str) -> String {
    let stripped = strip_terminal_ansi_sequences(text).replace('\r', "");
    stripped
        .chars()
        .filter_map(|ch| match ch {
            '\n' | '\t' => Some(ch),
            '\u{200e}' | '\u{200f}' => None,
            '\u{202a}'..='\u{202e}' => None,
            '\u{2066}'..='\u{2069}' => None,
            ch if ch.is_control() => Some(' '),
            _ => Some(ch),
        })
        .collect()
}

fn push_message_header(
    out: &mut Vec<RenderedLine>,
    message: &MessageResource,
    width: u16,
    i18n: &I18n,
) {
    let role = ui_text::role_label(i18n, message.role);
    let header = if message.state == MessageStatus::Completed {
        role
    } else {
        format!(
            "{role} {}",
            ui_text::message_state_label(i18n, message.state)
        )
    };
    let header_style = style_for_role(message.role).add_modifier(Modifier::BOLD);

    if UnicodeWidthStr::width(header.as_str()) <= width.max(1) as usize {
        out.push(RenderedLine::plain(header, header_style));
    } else {
        push_wrapped_line(out, "", "", header.as_str(), header_style, width);
    }
}

fn render_part(part: &MessagePart, width: u16, out: &mut Vec<RenderedLine>, i18n: &I18n) {
    match part.content.as_ref() {
        Some(PartContent::Text(text)) => push_markdown(out, "  ", text.text.as_str(), width),
        Some(PartContent::Reasoning(reasoning)) => {
            let summary = if !reasoning.summary.is_empty() {
                reasoning.summary.join(" ")
            } else {
                reasoning.raw_content.join(" ")
            };
            push_section_heading(
                out,
                "  thinking",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
                width,
            );
            push_multiline(
                out,
                "    ",
                summary.as_str(),
                Style::default().fg(Color::DarkGray),
                width,
            );
        }
        Some(PartContent::Operation(tool)) => render_tool_execution(part, tool, out, width, i18n),
        Some(PartContent::Error(error)) => {
            push_multiline(
                out,
                "  ",
                &i18n.text_args(
                    "message-error",
                    &crate::fl_args!(
                        "code" => error.code.as_str(),
                        "message" => error.message.as_str(),
                    ),
                ),
                Style::default().fg(Color::Red),
                width,
            );
        }
        Some(PartContent::Attachment(attachment)) => {
            push_section_heading(
                out,
                &format!("  {}", ui_text::t(i18n, "message-attachments")),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
                width,
            );
            for item in &attachment.attachments {
                let label = item
                    .title
                    .as_ref()
                    .or(item.filename.as_ref())
                    .cloned()
                    .unwrap_or_else(|| item.mime.clone());
                push_label_value(out, "    - ", label.as_str(), Style::default(), width);
            }
        }
        Some(PartContent::Request(RequestPart::Permission(permission))) => {
            render_permission_request(permission, out, width, i18n)
        }
        Some(PartContent::Request(RequestPart::UserInput(request))) => {
            render_user_input_request(request, out, width, i18n)
        }
        None => {
            let fallback = part
                .summary
                .clone()
                .unwrap_or_else(|| ui_text::t(i18n, "message-part-detail-unavailable"));
            push_multiline(
                out,
                "  ",
                fallback.as_str(),
                Style::default().fg(Color::DarkGray),
                width,
            );
        }
    }
}

fn render_tool_execution(
    part: &MessagePart,
    tool: &OperationPart,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    let label = if tool.title.trim().is_empty() {
        tool_invocation_label(&tool.invocation)
    } else {
        tool.title.clone()
    };
    let (message_key, color) = match part.status {
        ExecutionStatus::Pending => ("message-tool-pending", Color::Magenta),
        ExecutionStatus::InProgress => ("message-tool-running", Color::Magenta),
        ExecutionStatus::Completed => ("message-tool-done", Color::Green),
        ExecutionStatus::Failed => ("message-tool-failed", Color::Red),
        ExecutionStatus::Cancelled => ("message-tool-failed", Color::DarkGray),
    };
    push_multiline(
        out,
        "  ",
        &i18n.text_args(message_key, &crate::fl_args!("label" => label)),
        Style::default().fg(color),
        width,
    );

    if part.status == ExecutionStatus::Failed
        && let Some(error_message) = tool.error_message()
        && !error_message.trim().is_empty()
    {
        push_multiline(
            out,
            "    ",
            error_message,
            Style::default().fg(Color::Red),
            width,
        );
    }

    if !tool.model_output.text.trim().is_empty() {
        push_tool_output_preview(
            out,
            "    ",
            tool.model_output.text.as_str(),
            Style::default(),
            width,
            i18n,
        );
    }

    if let Some(diff) = apply_patch_diff(&tool.details) {
        push_label_value(
            out,
            "    diff: ",
            &format!("{} lines", diff.lines().count()),
            Style::default().fg(Color::DarkGray),
            width,
        );
        push_tool_output_preview(
            out,
            "    ",
            diff.as_str(),
            Style::default().fg(Color::DarkGray),
            width,
            i18n,
        );
    }

    render_operation_blocks(tool.blocks.as_slice(), out, width, i18n);
}

fn render_operation_blocks(
    blocks: &[OperationBlock],
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    for block in blocks {
        match block {
            OperationBlock::Text { text } => {
                push_multiline(out, "    ", text, Style::default(), width);
            }
            OperationBlock::Markdown { text } => {
                push_markdown(out, "    ", text, width);
            }
            OperationBlock::Command {
                command,
                exit_code,
                stdout,
                stderr,
                ..
            } => {
                push_label_value(
                    out,
                    "    $ ",
                    command.as_str(),
                    Style::default().fg(Color::Magenta),
                    width,
                );
                if let Some(stdout) = stdout
                    && !stdout.trim().is_empty()
                {
                    push_tool_output_preview(out, "      ", stdout, Style::default(), width, i18n);
                }
                if let Some(stderr) = stderr
                    && !stderr.trim().is_empty()
                {
                    push_tool_output_preview(
                        out,
                        "      ",
                        stderr,
                        Style::default().fg(Color::Red),
                        width,
                        i18n,
                    );
                }
                if let Some(exit_code) = exit_code
                    && *exit_code != 0
                {
                    push_label_value(
                        out,
                        "      ",
                        &format!("exit {exit_code}"),
                        Style::default().fg(Color::DarkGray),
                        width,
                    );
                }
            }
            OperationBlock::Diff { diff, .. } => {
                push_tool_output_preview(
                    out,
                    "    ",
                    diff,
                    Style::default().fg(Color::DarkGray),
                    width,
                    i18n,
                );
            }
            OperationBlock::FileChanges { changes } => {
                render_file_changes(changes, out, width, i18n)
            }
            OperationBlock::Checklist { items } => render_checklist(items, out, width, i18n),
            OperationBlock::SearchResults { query, results } => {
                let heading = query
                    .as_deref()
                    .map(|query| format!("search: {query}"))
                    .unwrap_or_else(|| "search results".to_string());
                push_section_heading(
                    out,
                    &format!("    {heading}"),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                    width,
                );
                for result in results {
                    push_label_value(
                        out,
                        "      - ",
                        result.title.as_str(),
                        Style::default(),
                        width,
                    );
                    push_multiline(
                        out,
                        "        ",
                        result.uri.as_str(),
                        Style::default().fg(Color::DarkGray),
                        width,
                    );
                    if let Some(snippet) = &result.snippet
                        && !snippet.trim().is_empty()
                    {
                        push_multiline(out, "        ", snippet, Style::default(), width);
                    }
                }
            }
            OperationBlock::ResourceLink { uri, title, .. }
            | OperationBlock::Citation { uri, title, .. } => {
                push_label_value(
                    out,
                    "    - ",
                    title.as_deref().unwrap_or(uri.as_str()),
                    Style::default().fg(Color::DarkGray),
                    width,
                );
            }
            OperationBlock::Image { url, .. }
            | OperationBlock::Audio { url, .. }
            | OperationBlock::File { url, .. } => {
                push_label_value(
                    out,
                    "    - ",
                    url.as_str(),
                    Style::default().fg(Color::DarkGray),
                    width,
                );
            }
            OperationBlock::EmbeddedResource { uri, text, .. } => {
                push_label_value(
                    out,
                    "    - ",
                    uri.as_str(),
                    Style::default().fg(Color::DarkGray),
                    width,
                );
                if let Some(text) = text
                    && !text.trim().is_empty()
                {
                    push_tool_output_preview(out, "      ", text, Style::default(), width, i18n);
                }
            }
            OperationBlock::Media { artifact, .. } => {
                push_label_value(
                    out,
                    "    - ",
                    artifact.name.as_deref().unwrap_or(artifact.uri.as_str()),
                    Style::default().fg(Color::DarkGray),
                    width,
                );
            }
            OperationBlock::Progress { message, .. } => {
                push_multiline(
                    out,
                    "    ",
                    message,
                    Style::default().fg(Color::DarkGray),
                    width,
                );
            }
            OperationBlock::NestedTask {
                task_id,
                title,
                status,
            } => {
                let title = title.as_deref().unwrap_or(task_id.as_str());
                push_label_value(
                    out,
                    "    - ",
                    &format!(
                        "{} ({})",
                        title,
                        ui_text::execution_status_label(i18n, *status)
                    ),
                    Style::default().fg(Color::DarkGray),
                    width,
                );
            }
            OperationBlock::Json { .. }
            | OperationBlock::Table { .. }
            | OperationBlock::Log { .. }
            | OperationBlock::Custom { .. } => {}
        }
    }
}

fn render_file_changes(
    changes: &[agena::message::FileChangeEntry],
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    if changes.is_empty() {
        return;
    }
    push_section_heading(
        out,
        &format!("    {}", ui_text::t(i18n, "message-file-changes")),
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        width,
    );
    for entry in changes {
        let path = if entry.kind == FileChangeKind::Moved {
            entry
                .from_path
                .as_ref()
                .map(|from_path| format!("{from_path} -> {}", entry.path))
                .unwrap_or_else(|| entry.path.clone())
        } else {
            entry.path.clone()
        };
        push_label_value(
            out,
            "      - ",
            &format!(
                "{} ({})",
                path,
                ui_text::file_change_kind_label(i18n, entry.kind)
            ),
            Style::default(),
            width,
        );
    }
}

fn render_checklist(
    items: &[agena::message::TodoItem],
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    if items.is_empty() {
        return;
    }
    push_section_heading(
        out,
        &format!("    {}", ui_text::t(i18n, "message-todo-list")),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        width,
    );
    for item in items {
        push_label_value(
            out,
            "      - ",
            &format!(
                "[{}|{}] {}",
                ui_text::todo_status_label(i18n, item.status),
                ui_text::todo_priority_label(i18n, item.priority),
                item.content
            ),
            Style::default(),
            width,
        );
    }
}

fn render_permission_request(
    permission: &agena::message::PermissionRequestPart,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    push_section_heading(
        out,
        "  permission",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        width,
    );
    push_multiline(
        out,
        "    ",
        ui_text::permission_summary(i18n, permission).as_str(),
        Style::default().fg(Color::Magenta),
        width,
    );
}

fn render_user_input_request(
    request: &agena::message::UserInputRequestPart,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    push_multiline(
        out,
        "  ",
        &i18n.text_args(
            "message-awaiting-user-input",
            &crate::fl_args!("request_id" => request.request.request_id.as_str()),
        ),
        Style::default().fg(Color::Magenta),
        width,
    );
    for question in &request.request.questions {
        push_multiline(
            out,
            "    ",
            &ui_text::message_question_line(i18n, question.question.as_str(), question.id.as_str()),
            Style::default(),
            width,
        );
    }
}

fn preview_for_part(part: &MessagePart, i18n: &I18n) -> Option<String> {
    match part.content.as_ref() {
        Some(PartContent::Text(text)) => first_non_empty_preview_line(text.text.as_str()),
        Some(PartContent::Reasoning(reasoning)) => {
            let summary = if !reasoning.summary.is_empty() {
                reasoning.summary.join(" ")
            } else {
                reasoning.raw_content.join(" ")
            };
            first_non_empty_preview_line(summary.as_str())
        }
        Some(PartContent::Operation(tool)) => Some(tool_execution_preview(part, tool)),
        Some(PartContent::Error(error)) => Some(format!("{}: {}", error.code, error.message)),
        Some(PartContent::Attachment(attachment)) => attachment.attachments.first().map(|item| {
            item.title
                .as_ref()
                .or(item.filename.as_ref())
                .cloned()
                .unwrap_or_else(|| item.mime.clone())
        }),
        Some(PartContent::Request(RequestPart::Permission(permission))) => {
            Some(ui_text::permission_summary(i18n, permission))
        }
        Some(PartContent::Request(RequestPart::UserInput(request))) => request
            .request
            .questions
            .first()
            .map(|question| question.question.clone()),
        None => part.summary.clone(),
    }
}

fn tool_execution_preview(part: &MessagePart, tool: &OperationPart) -> String {
    let label = if tool.title.trim().is_empty() {
        tool_invocation_label(&tool.invocation)
    } else {
        tool.title.clone()
    };
    match part.status {
        ExecutionStatus::Pending => format!("tool pending {label}"),
        ExecutionStatus::InProgress => format!("tool running {label}"),
        ExecutionStatus::Completed => format!("tool done {label}"),
        ExecutionStatus::Failed => format!("tool failed {label}"),
        ExecutionStatus::Cancelled => format!("tool cancelled {label}"),
    }
}

fn first_non_empty_preview_line(text: &str) -> Option<String> {
    sanitize_terminal_text(text)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn push_section_heading(out: &mut Vec<RenderedLine>, heading: &str, style: Style, width: u16) {
    push_wrapped_line(out, "", "", heading, style, width);
}

fn push_label_value(
    out: &mut Vec<RenderedLine>,
    label: &str,
    value: &str,
    style: Style,
    width: u16,
) {
    let continuation = " ".repeat(UnicodeWidthStr::width(label));
    push_wrapped_line(out, label, continuation.as_str(), value, style, width);
}

fn push_tool_output_preview(
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
                &crate::fl_args!("lines" => preview.omitted_lines as i64),
            ),
            Style::default().fg(Color::DarkGray),
            width,
        );
    }
}

fn push_multiline(out: &mut Vec<RenderedLine>, prefix: &str, text: &str, style: Style, width: u16) {
    let sanitized = sanitize_terminal_text(text);
    let normalized = trim_empty_line_edges(sanitized.as_str());
    if normalized.is_empty() {
        return;
    }
    for raw_line in normalized.split('\n') {
        push_wrapped_line(out, prefix, prefix, raw_line, style, width);
    }
}

fn trim_empty_line_edges(text: &str) -> String {
    let lines = text.split('\n').collect::<Vec<_>>();
    let Some(first_non_empty) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let last_non_empty = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .unwrap_or(first_non_empty);
    lines[first_non_empty..=last_non_empty].join("\n")
}

fn push_markdown(out: &mut Vec<RenderedLine>, prefix: &str, text: &str, width: u16) {
    let sanitized = sanitize_terminal_text(text);
    let markdown = trim_empty_line_edges(sanitized.as_str());
    if markdown.is_empty() {
        return;
    }

    let rendered = markdown_to_text(markdown.as_str());
    for line in rendered.lines {
        push_wrapped_rich_line(out, prefix, prefix, owned_line(&line), width);
    }
}

fn push_wrapped_line(
    out: &mut Vec<RenderedLine>,
    initial_prefix: &str,
    continuation_prefix: &str,
    text: &str,
    style: Style,
    width: u16,
) {
    if text.is_empty() {
        out.push(RenderedLine::plain(initial_prefix.to_string(), style));
        return;
    }

    let initial = format!("{initial_prefix}{text}");
    if width <= 1 || UnicodeWidthStr::width(initial.as_str()) <= width as usize {
        out.push(RenderedLine::plain(initial, style));
        return;
    }

    let initial_width = UnicodeWidthStr::width(initial_prefix);
    let continuation_width = UnicodeWidthStr::width(continuation_prefix);
    let available_width = width as usize;
    if available_width <= initial_width.max(continuation_width).saturating_add(1) {
        out.push(RenderedLine::plain(initial, style));
        return;
    }

    let options = WrapOptions::new(available_width)
        .initial_indent(initial_prefix)
        .subsequent_indent(continuation_prefix)
        .break_words(false)
        .word_splitter(WordSplitter::NoHyphenation);
    let wrapped = wrap(text, options);
    if wrapped.is_empty() {
        out.push(RenderedLine::plain(initial_prefix.to_string(), style));
        return;
    }

    out.extend(
        wrapped
            .into_iter()
            .map(|segment| RenderedLine::plain(segment.into_owned(), style)),
    );
}

fn push_wrapped_rich_line(
    out: &mut Vec<RenderedLine>,
    initial_prefix: &str,
    continuation_prefix: &str,
    line: Line<'static>,
    width: u16,
) {
    if line.spans.is_empty() {
        out.push(RenderedLine::rich(Line::from(Span::raw(
            initial_prefix.to_string(),
        ))));
        return;
    }

    let plain_text = line_plain_text(&line);
    let available_width = width.max(1) as usize;
    let initial_prefix_width = UnicodeWidthStr::width(initial_prefix);
    let continuation_prefix_width = UnicodeWidthStr::width(continuation_prefix);
    let initial_total_width =
        initial_prefix_width.saturating_add(UnicodeWidthStr::width(plain_text.as_str()));
    if initial_total_width <= available_width
        || available_width
            <= initial_prefix_width
                .max(continuation_prefix_width)
                .saturating_add(1)
    {
        out.push(RenderedLine::rich(prefix_rich_line(initial_prefix, line)));
        return;
    }

    let wrapped_lines = wrap_rich_line(
        line.spans.as_slice(),
        available_width.saturating_sub(initial_prefix_width).max(1),
        available_width
            .saturating_sub(continuation_prefix_width)
            .max(1),
    );
    if wrapped_lines.is_empty() {
        out.push(RenderedLine::rich(prefix_rich_line(initial_prefix, line)));
        return;
    }

    for (index, wrapped_line) in wrapped_lines.into_iter().enumerate() {
        let prefix = if index == 0 {
            initial_prefix
        } else {
            continuation_prefix
        };
        out.push(RenderedLine::rich(prefix_rich_line(prefix, wrapped_line)));
    }
}

fn prefix_rich_line(prefix: &str, line: Line<'static>) -> Line<'static> {
    if prefix.is_empty() {
        return line;
    }
    let mut spans = Vec::with_capacity(line.spans.len().saturating_add(1));
    spans.push(Span::raw(prefix.to_string()));
    spans.extend(line.spans);
    Line::from(spans)
}

fn owned_line(line: &Line<'_>) -> Line<'static> {
    Line::from(
        line.spans
            .iter()
            .map(|span| Span::styled(span.content.to_string(), span.style))
            .collect::<Vec<_>>(),
    )
}

fn line_plain_text(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

#[derive(Debug, Clone)]
struct StyledGrapheme {
    text: String,
    style: Style,
    width: usize,
    whitespace: bool,
}

fn wrap_rich_line(
    spans: &[Span<'static>],
    initial_width: usize,
    continuation_width: usize,
) -> Vec<Line<'static>> {
    let tokens = spans
        .iter()
        .flat_map(|span| {
            let style = span.style;
            span.content
                .as_ref()
                .graphemes(true)
                .map(move |grapheme| StyledGrapheme {
                    text: grapheme.to_string(),
                    style,
                    width: UnicodeWidthStr::width(grapheme),
                    whitespace: grapheme.chars().all(char::is_whitespace),
                })
        })
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return vec![Line::default()];
    }

    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut current_width = 0_usize;
    let mut width_limit = initial_width.max(1);
    let mut last_break_index = None;

    for token in tokens {
        let mut pending = Some(token);
        while let Some(token) = pending.take() {
            let token_fits =
                current.is_empty() || current_width.saturating_add(token.width) <= width_limit;
            if token_fits {
                if token.whitespace {
                    last_break_index = Some(current.len());
                }
                current_width = current_width.saturating_add(token.width);
                current.push(token);
                continue;
            }

            if let Some(break_index) = last_break_index.filter(|index| *index > 0) {
                let line_tokens = current[..break_index].to_vec();
                let mut carry = current[break_index + 1..].to_vec();
                while carry.first().is_some_and(|grapheme| grapheme.whitespace) {
                    carry.remove(0);
                }
                lines.push(styled_tokens_to_line(line_tokens));
                current = carry;
                current_width = current.iter().map(|grapheme| grapheme.width).sum();
                width_limit = continuation_width.max(1);
                last_break_index = current.iter().rposition(|grapheme| grapheme.whitespace);
                pending = Some(token);
                continue;
            }

            if current.is_empty() {
                current_width = current_width.saturating_add(token.width);
                current.push(token);
                continue;
            }

            lines.push(styled_tokens_to_line(current));
            current = Vec::new();
            current_width = 0;
            width_limit = continuation_width.max(1);
            last_break_index = None;
            pending = Some(token);
        }
    }

    if !current.is_empty() {
        lines.push(styled_tokens_to_line(current));
    }
    if lines.is_empty() {
        lines.push(Line::default());
    }
    lines
}

fn styled_tokens_to_line(tokens: Vec<StyledGrapheme>) -> Line<'static> {
    let mut spans = Vec::<Span<'static>>::new();
    for token in tokens {
        if let Some(last) = spans.last_mut()
            && last.style == token.style
        {
            last.content = format!("{}{}", last.content.as_ref(), token.text).into();
        } else {
            spans.push(Span::styled(token.text, token.style));
        }
    }
    Line::from(spans)
}

fn strip_terminal_ansi_sequences(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != 0x1b {
            let ch = text[index..].chars().next().unwrap_or_default();
            out.push(ch);
            index += ch.len_utf8();
            continue;
        }

        index += 1;
        if index >= bytes.len() {
            break;
        }

        match bytes[index] {
            b'[' => {
                index += 1;
                while index < bytes.len() {
                    let byte = bytes[index];
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            b']' => {
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        0x07 => {
                            index += 1;
                            break;
                        }
                        0x1b if bytes.get(index + 1) == Some(&b'\\') => {
                            index += 2;
                            break;
                        }
                        _ => index += 1,
                    }
                }
            }
            _ => {
                index += 1;
            }
        }
    }

    out
}

fn apply_patch_diff(details: &agena::message::ToolOutput) -> Option<String> {
    details
        .payload
        .get("diff")
        .and_then(agena::message::StructuredValue::as_text)
        .map(str::trim)
        .filter(|diff| !diff.is_empty())
        .map(str::to_string)
}

fn tool_invocation_label(invocation: &ToolInvocation) -> String {
    let input = serde_json::Value::from(invocation.input.clone());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_empty_line_edges_removes_outer_blank_lines_only() {
        let text = "\n\nfirst line\n\nsecond line\n\n";
        assert_eq!(trim_empty_line_edges(text), "first line\n\nsecond line");
    }

    #[test]
    fn push_multiline_ignores_outer_blank_lines() {
        let mut out = Vec::new();
        push_multiline(&mut out, "  ", "\n\nhello\nworld\n\n", Style::default(), 80);
        let rendered = out.into_iter().map(|line| line.text).collect::<Vec<_>>();
        assert_eq!(rendered, vec!["  hello".to_string(), "  world".to_string()]);
    }

    #[test]
    fn tool_output_preview_ignores_outer_blank_lines() {
        let preview = tool_output_preview("\n\nalpha\nbeta\n\n");
        assert_eq!(preview.text, "alpha\nbeta");
        assert_eq!(preview.omitted_lines, 0);
    }

    #[test]
    fn push_markdown_emits_rich_transcript_lines() {
        let mut out = Vec::new();
        push_markdown(&mut out, "  ", "# hello\n\n**world**", 40);
        assert!(!out.is_empty());
        assert!(out.iter().all(|line| line.rich_line.is_some()));
        assert!(out.iter().any(|line| line.text.contains("hello")));
        assert!(out.iter().any(|line| line.text.contains("world")));
    }
}
