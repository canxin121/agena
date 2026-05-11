use super::*;
use textwrap::{Options as WrapOptions, WordSplitter, wrap};
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

    if let Some(usage) = &message.usage {
        push_label_value(
            &mut lines,
            "  usage: ",
            &ui_text::message_usage(
                i18n,
                usage.input_tokens,
                usage.output_tokens,
                usage.reasoning_tokens,
            ),
            Style::default().fg(Color::DarkGray),
            width,
        );
    }

    if let Some(finish) = &message.finish
        && !finish.trim().is_empty()
    {
        push_label_value(
            &mut lines,
            "  finish: ",
            &ui_text::message_finish(i18n, finish),
            Style::default().fg(Color::DarkGray),
            width,
        );
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
    let total_lines = text.split('\n').count();
    let mut preview = String::new();
    let mut used_chars = 0_usize;
    let mut included_lines = 0_usize;
    let mut truncated = false;

    for (index, line) in text.split('\n').enumerate() {
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
        .map(|ch| match ch {
            '\n' | '\t' => ch,
            '\u{200e}' | '\u{200f}' => ' ',
            '\u{202a}'..='\u{202e}' => ' ',
            '\u{2066}'..='\u{2069}' => ' ',
            ch if ch.is_control() => ' ',
            _ => ch,
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
    let state = ui_text::message_state_label(i18n, message.state);
    let header = format!("[{role}] {state}");
    let metadata = format!("{} | #{}", format_timestamp(message.created_at), message.id);
    let combined = format!("{header} | {metadata}");
    let header_style = style_for_role(message.role).add_modifier(Modifier::BOLD);

    if UnicodeWidthStr::width(combined.as_str()) <= width.max(1) as usize {
        out.push(RenderedLine {
            text: combined,
            style: header_style,
        });
    } else {
        push_wrapped_line(out, "", "", header.as_str(), header_style, width);
        push_wrapped_line(
            out,
            "  ",
            "  ",
            metadata.as_str(),
            Style::default().fg(Color::DarkGray),
            width,
        );
    }
}

fn render_part(part: &MessagePart, width: u16, out: &mut Vec<RenderedLine>, i18n: &I18n) {
    match part.content.as_ref() {
        Some(PartContent::Text(text)) => {
            push_multiline(out, "  ", text.text.as_str(), Style::default(), width)
        }
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
        Some(PartContent::ToolExecution(tool)) => render_tool_execution(tool, out, width, i18n),
        Some(PartContent::CommandExecution(command)) => {
            push_label_value(
                out,
                "  command: ",
                &format!("$ {}", command.command),
                Style::default().fg(Color::Magenta),
                width,
            );
            if let Some(output) = &command.output
                && !output.trim().is_empty()
            {
                push_section_heading(
                    out,
                    "    output",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                    width,
                );
                push_multiline(
                    out,
                    "      ",
                    output,
                    Style::default().fg(Color::Gray),
                    width,
                );
            }
            push_label_value(
                out,
                "  status: ",
                &i18n.text_args(
                    "message-command-status",
                    &crate::fl_args!(
                        "status" => ui_text::execution_status_label(i18n, command.status),
                        "exit" => command.exit_code.unwrap_or(-1),
                    ),
                ),
                Style::default().fg(Color::DarkGray),
                width,
            );
        }
        Some(PartContent::FileChange(change)) => {
            push_section_heading(
                out,
                &format!("  {}", ui_text::t(i18n, "message-file-changes")),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
                width,
            );
            for entry in &change.changes {
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
                    "    - ",
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
        Some(PartContent::WebSearch(search)) => {
            push_label_value(
                out,
                "  search: ",
                search.query.as_str(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                width,
            );
            for result in &search.results {
                push_label_value(
                    out,
                    "    - ",
                    result.title.as_str(),
                    Style::default(),
                    width,
                );
                push_multiline(
                    out,
                    "      ",
                    result.url.as_str(),
                    Style::default().fg(Color::DarkGray),
                    width,
                );
                if let Some(snippet) = &result.snippet
                    && !snippet.trim().is_empty()
                {
                    push_multiline(out, "      ", snippet, Style::default(), width);
                }
            }
        }
        Some(PartContent::TodoList(todo)) => {
            push_section_heading(
                out,
                &format!("  {}", ui_text::t(i18n, "message-todo-list")),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                width,
            );
            for item in &todo.items {
                push_label_value(
                    out,
                    "    - ",
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
        Some(PartContent::PermissionRequest(permission)) => {
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
        Some(PartContent::UserInputRequest(request)) => {
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
                    &ui_text::message_question_line(
                        i18n,
                        question.question.as_str(),
                        question.id.as_str(),
                    ),
                    Style::default(),
                    width,
                );
            }
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
    tool: &ToolExecutionPart,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    match tool {
        ToolExecutionPart::Pending {
            invocation, title, ..
        } => {
            let label = if title.trim().is_empty() {
                tool_invocation_label(invocation)
            } else {
                title.clone()
            };
            push_multiline(
                out,
                "  ",
                &i18n.text_args("message-tool-pending", &crate::fl_args!("label" => label)),
                Style::default().fg(Color::Magenta),
                width,
            );
        }
        ToolExecutionPart::InProgress {
            invocation,
            title,
            output_text,
            ..
        } => {
            let label = if title.trim().is_empty() {
                tool_invocation_label(invocation)
            } else {
                title.clone()
            };
            push_multiline(
                out,
                "  ",
                &i18n.text_args("message-tool-running", &crate::fl_args!("label" => label)),
                Style::default().fg(Color::Magenta),
                width,
            );
            if !output_text.trim().is_empty() {
                push_section_heading(
                    out,
                    "    output",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                    width,
                );
                push_tool_output_preview(out, "      ", output_text, Style::default(), width, i18n);
            }
        }
        ToolExecutionPart::Completed {
            invocation,
            output_text,
            blocks,
            details,
            ..
        } => {
            push_multiline(
                out,
                "  ",
                &i18n.text_args(
                    "message-tool-done",
                    &crate::fl_args!("label" => tool_invocation_label(invocation)),
                ),
                Style::default().fg(Color::Green),
                width,
            );
            if !output_text.trim().is_empty() {
                push_section_heading(
                    out,
                    "    output",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                    width,
                );
                push_tool_output_preview(out, "      ", output_text, Style::default(), width, i18n);
            }
            if let Some(diff) = apply_patch_diff(details) {
                push_label_value(
                    out,
                    "    diff: ",
                    &format!("{} lines", diff.lines().count()),
                    Style::default().fg(Color::DarkGray),
                    width,
                );
                push_tool_output_preview(
                    out,
                    "      ",
                    diff.as_str(),
                    Style::default().fg(Color::DarkGray),
                    width,
                    i18n,
                );
            }
            if !blocks.is_empty() {
                push_multiline(
                    out,
                    "    ",
                    &ui_text::message_tool_result_blocks(i18n, blocks.len()),
                    Style::default().fg(Color::DarkGray),
                    width,
                );
            }
        }
        ToolExecutionPart::Failed {
            invocation,
            error_message,
            output_text,
            ..
        } => {
            push_multiline(
                out,
                "  ",
                &i18n.text_args(
                    "message-tool-failed",
                    &crate::fl_args!("label" => tool_invocation_label(invocation)),
                ),
                Style::default().fg(Color::Red),
                width,
            );
            if !error_message.trim().is_empty() {
                push_section_heading(
                    out,
                    "    error",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    width,
                );
                push_multiline(
                    out,
                    "      ",
                    error_message,
                    Style::default().fg(Color::Red),
                    width,
                );
            }
            if !output_text.trim().is_empty() {
                push_section_heading(
                    out,
                    "    output",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                    width,
                );
                push_tool_output_preview(out, "      ", output_text, Style::default(), width, i18n);
            }
        }
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
        Some(PartContent::ToolExecution(tool)) => Some(tool_execution_preview(tool)),
        Some(PartContent::CommandExecution(command)) => Some(format!("$ {}", command.command)),
        Some(PartContent::FileChange(change)) => change.changes.first().map(|entry| {
            let path = if entry.kind == FileChangeKind::Moved {
                entry
                    .from_path
                    .as_ref()
                    .map(|from_path| format!("{from_path} -> {}", entry.path))
                    .unwrap_or_else(|| entry.path.clone())
            } else {
                entry.path.clone()
            };
            format!(
                "{} {}",
                ui_text::file_change_kind_label(i18n, entry.kind),
                path
            )
        }),
        Some(PartContent::WebSearch(search)) => Some(format!("search: {}", search.query)),
        Some(PartContent::TodoList(todo)) => todo.items.first().map(|item| item.content.clone()),
        Some(PartContent::Error(error)) => Some(format!("{}: {}", error.code, error.message)),
        Some(PartContent::Attachment(attachment)) => attachment.attachments.first().map(|item| {
            item.title
                .as_ref()
                .or(item.filename.as_ref())
                .cloned()
                .unwrap_or_else(|| item.mime.clone())
        }),
        Some(PartContent::PermissionRequest(permission)) => {
            Some(ui_text::permission_summary(i18n, permission))
        }
        Some(PartContent::UserInputRequest(request)) => request
            .request
            .questions
            .first()
            .map(|question| question.question.clone()),
        None => part.summary.clone(),
    }
}

fn tool_execution_preview(tool: &ToolExecutionPart) -> String {
    match tool {
        ToolExecutionPart::Pending {
            invocation, title, ..
        } => {
            let label = if title.trim().is_empty() {
                tool_invocation_label(invocation)
            } else {
                title.clone()
            };
            format!("tool pending {label}")
        }
        ToolExecutionPart::InProgress {
            invocation, title, ..
        } => {
            let label = if title.trim().is_empty() {
                tool_invocation_label(invocation)
            } else {
                title.clone()
            };
            format!("tool running {label}")
        }
        ToolExecutionPart::Completed { invocation, .. } => {
            format!("tool done {}", tool_invocation_label(invocation))
        }
        ToolExecutionPart::Failed { invocation, .. } => {
            format!("tool failed {}", tool_invocation_label(invocation))
        }
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
    for raw_line in sanitized.split('\n') {
        push_wrapped_line(out, prefix, prefix, raw_line, style, width);
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
        out.push(RenderedLine {
            text: initial_prefix.to_string(),
            style,
        });
        return;
    }

    let initial = format!("{initial_prefix}{text}");
    if width <= 1 || UnicodeWidthStr::width(initial.as_str()) <= width as usize {
        out.push(RenderedLine {
            text: initial,
            style,
        });
        return;
    }

    let initial_width = UnicodeWidthStr::width(initial_prefix);
    let continuation_width = UnicodeWidthStr::width(continuation_prefix);
    let available_width = width as usize;
    if available_width <= initial_width.max(continuation_width).saturating_add(1) {
        out.push(RenderedLine {
            text: initial,
            style,
        });
        return;
    }

    let options = WrapOptions::new(available_width)
        .initial_indent(initial_prefix)
        .subsequent_indent(continuation_prefix)
        .break_words(false)
        .word_splitter(WordSplitter::NoHyphenation);
    let wrapped = wrap(text, options);
    if wrapped.is_empty() {
        out.push(RenderedLine {
            text: initial_prefix.to_string(),
            style,
        });
        return;
    }

    out.extend(wrapped.into_iter().map(|segment| RenderedLine {
        text: segment.into_owned(),
        style,
    }));
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
    match details.as_first_party()? {
        FirstPartyToolOutput::ApplyPatch { diff, .. } if !diff.trim().is_empty() => Some(diff),
        _ => None,
    }
}

fn tool_invocation_label(invocation: &ToolInvocation) -> String {
    if let Some(input) = invocation.as_first_party() {
        return match input {
            FirstPartyToolInput::Bash(input) => format!("bash {}", input.command),
            FirstPartyToolInput::Read(input) => format!("read {}", input.file_path),
            FirstPartyToolInput::ViewFile(input) => format!("view_file {}", input.path),
            FirstPartyToolInput::ApplyPatch(_) => "apply_patch".to_string(),
            FirstPartyToolInput::Glob(input) => format!("glob {}", input.pattern),
            FirstPartyToolInput::Grep(input) => format!("grep {}", input.pattern),
            FirstPartyToolInput::Task(input) => format!("task {}", input.description),
            FirstPartyToolInput::ToolSearch(input) => format!("tool_search {}", input.query),
            FirstPartyToolInput::TodoWrite(_) => "todo_write".to_string(),
            FirstPartyToolInput::AskUser(_) => "ask_user".to_string(),
            FirstPartyToolInput::Monitor(input) => match input {
                agena::message::MonitorToolInput::Start { command, .. } => {
                    format!("monitor start {command}")
                }
                agena::message::MonitorToolInput::List {} => "monitor list".to_string(),
                agena::message::MonitorToolInput::Read { monitor_id, .. } => {
                    format!("monitor read {monitor_id}")
                }
                agena::message::MonitorToolInput::Stop { monitor_id } => {
                    format!("monitor stop {monitor_id}")
                }
            },
            FirstPartyToolInput::WebFetch(input) => format!("web_fetch {}", input.url),
            FirstPartyToolInput::WebSearch(input) => format!("web_search {}", input.query),
            FirstPartyToolInput::EnterPlanMode(_) => "enter_plan_mode".to_string(),
            FirstPartyToolInput::ExitPlanMode(_) => "exit_plan_mode".to_string(),
            FirstPartyToolInput::EnterWorktree(input) => match (&input.name, &input.path) {
                (Some(n), _) => format!("enter_worktree name={n}"),
                (_, Some(p)) => format!("enter_worktree path={p}"),
                _ => "enter_worktree".to_string(),
            },
            FirstPartyToolInput::ExitWorktree(input) => format!("exit_worktree {}", input.action),
            FirstPartyToolInput::CronCreate(input) => {
                format!("cron_create {}", input.expression)
            }
            FirstPartyToolInput::CronList(_) => "cron_list".to_string(),
            FirstPartyToolInput::CronDelete(input) => format!("cron_delete {}", input.id),
            FirstPartyToolInput::ScheduleWakeup(input) => {
                format!("schedule_wakeup +{}s", input.delay_seconds)
            }
            FirstPartyToolInput::LspDefinition(input) => {
                format!(
                    "lsp_definition {}:{}:{}",
                    input.file_path, input.line, input.character
                )
            }
            FirstPartyToolInput::LspReferences(input) => {
                format!(
                    "lsp_references {}:{}:{}",
                    input.file_path, input.line, input.character
                )
            }
            FirstPartyToolInput::LspHover(input) => {
                format!(
                    "lsp_hover {}:{}:{}",
                    input.file_path, input.line, input.character
                )
            }
            FirstPartyToolInput::LspDiagnostics(input) => {
                format!("lsp_diagnostics {}", input.file_path)
            }
            FirstPartyToolInput::NotebookEdit(input) => {
                format!("notebook_edit {}", input.notebook_path)
            }
            FirstPartyToolInput::PowerShell(input) => format!("powershell {}", input.command),
        };
    }
    let ToolInvocation { name, .. } = invocation;
    name.clone()
}
