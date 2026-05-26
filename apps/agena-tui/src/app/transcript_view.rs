use super::*;
use agena::message::{ExecutionStatus, FileChangeKind, MessageStatus, OperationBlock, RequestPart};
use agena_tui_components::{line_plain_text, trim_empty_line_edges};
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

    let parts = transcript_message_parts(message);
    if parts.is_empty() {
        lines.push(RenderedLine::dim(format!(
            "  {}",
            ui_text::t(i18n, "message-empty")
        )));
    } else {
        for part in parts {
            render_part(part, width, &mut lines, i18n);
        }
    }

    lines
}

pub(super) fn render_message_detailed(
    message: &MessageResource,
    width: u16,
    i18n: &I18n,
    defaults: TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
) -> RenderedMessageBlock {
    let mut lines = Vec::new();
    let mut nodes = Vec::new();
    let header_start = lines.len();
    push_message_header(&mut lines, message, width, i18n);

    let parts = transcript_message_parts(message);
    if parts.is_empty() {
        lines.push(RenderedLine::dim(format!(
            "  {}",
            ui_text::t(i18n, "message-empty")
        )));
        nodes.push(RenderedTranscriptNode {
            key: TranscriptNodeKey::MessagePart {
                message_id: message.id,
                part_id: None,
            },
            kind: TranscriptNodeKind::Message,
            start_line: header_start,
            end_line: lines.len(),
            copy_text: String::new(),
            toggleable: false,
            expanded: true,
        });
    } else {
        let mut attached_header = false;
        for part in parts {
            let start_line = if attached_header {
                lines.len()
            } else {
                header_start
            };
            let node =
                render_part_node(message, part, width, &mut lines, i18n, defaults, expansions);
            if lines.len() > start_line {
                nodes.push(RenderedTranscriptNode {
                    key: node.key,
                    kind: node.kind,
                    start_line,
                    end_line: lines.len(),
                    copy_text: node.copy_text,
                    toggleable: node.toggleable,
                    expanded: node.expanded,
                });
                attached_header = true;
            }
        }
    }

    RenderedMessageBlock { lines, nodes }
}

#[derive(Debug, Clone)]
pub(super) struct RenderedMessageBlock {
    pub lines: Vec<RenderedLine>,
    pub nodes: Vec<RenderedTranscriptNode>,
}

#[derive(Debug, Clone)]
struct RenderedNodeDraft {
    key: TranscriptNodeKey,
    kind: TranscriptNodeKind,
    copy_text: String,
    toggleable: bool,
    expanded: bool,
}

pub(super) fn rewind_message_preview(message: &MessageResource, i18n: &I18n) -> String {
    let preview = transcript_message_parts(message)
        .iter()
        .find_map(|part| preview_for_part(part, i18n))
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
        ui_text::transcript_export_default_title(i18n)
    };

    let mut out = vec![format!("# {title}"), String::new()];
    if let Some(session_id) = session_id {
        out.push(ui_text::transcript_export_session_id_line(i18n, session_id));
    }
    out.push(ui_text::transcript_export_exported_at_line(
        i18n,
        Local::now(),
    ));
    out.push(ui_text::transcript_export_messages_loaded_line(
        i18n,
        messages.len(),
    ));
    out.push(ui_text::transcript_export_older_messages_omitted_line(
        i18n,
        has_more_older,
    ));
    if let Some(execution) = execution {
        if let Some(parent_id) = execution.session.parent_id {
            out.push(ui_text::transcript_export_parent_session_line(
                i18n, parent_id,
            ));
        }
        out.push(ui_text::transcript_export_child_sessions_line(
            i18n,
            execution.session.child_session_count,
        ));
    }
    out.push(String::new());

    if messages.is_empty() {
        out.push(ui_text::transcript_export_empty_line(i18n));
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
    tool_output_preview_with_limits(text, TOOL_CARD_PREVIEW_LINES, TOOL_CARD_PREVIEW_CHARS)
}

fn tool_output_preview_with_limits(
    text: &str,
    max_lines: usize,
    max_chars: usize,
) -> ToolOutputPreview {
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
        if index >= max_lines {
            truncated = true;
            break;
        }

        let separator_chars = usize::from(index > 0);
        let line_chars = line.chars().count();
        if used_chars
            .saturating_add(separator_chars)
            .saturating_add(line_chars)
            > max_chars
        {
            if index > 0 {
                preview.push('\n');
            }
            let remaining = max_chars
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
    match transcript_part_content(part) {
        PartContent::Text(text) => push_markdown(out, "  ", text.text.as_str(), width),
        PartContent::Reasoning(reasoning) => {
            render_reasoning_summary(reasoning.preferred_text().as_str(), out, width, i18n, true);
        }
        PartContent::Operation(tool) => render_tool_execution(part, tool, out, width, i18n, false),
        PartContent::Error(error) => {
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
        PartContent::Attachment(attachment) => {
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
        PartContent::Request(RequestPart::Permission(permission)) => {
            render_permission_request(permission, out, width, i18n)
        }
        PartContent::Request(RequestPart::UserInput(request)) => {
            render_user_input_request(request, out, width, i18n)
        }
    }
}

fn render_part_node(
    message: &MessageResource,
    part: &MessagePart,
    width: u16,
    out: &mut Vec<RenderedLine>,
    i18n: &I18n,
    defaults: TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
) -> RenderedNodeDraft {
    match transcript_part_content(part) {
        PartContent::Text(text) => {
            push_markdown(out, "  ", text.text.as_str(), width);
            RenderedNodeDraft {
                key: TranscriptNodeKey::MessagePart {
                    message_id: message.id,
                    part_id: Some(part.id),
                },
                kind: TranscriptNodeKind::Message,
                copy_text: text.text.clone(),
                toggleable: false,
                expanded: true,
            }
        }
        PartContent::Reasoning(reasoning) => {
            let key = TranscriptNodeKey::Reasoning {
                message_id: message.id,
                part_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.thinking_expanded);
            let summary = reasoning.preferred_text();
            push_section_heading(
                out,
                "  thinking",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
                width,
            );
            if expanded {
                push_multiline(
                    out,
                    "    ",
                    summary.as_str(),
                    Style::default().fg(Color::DarkGray),
                    width,
                );
            } else {
                push_collapsible_text(
                    out,
                    "    ",
                    summary.as_str(),
                    Style::default().fg(Color::DarkGray),
                    width,
                    i18n,
                );
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Reasoning,
                copy_text: summary,
                toggleable: true,
                expanded,
            }
        }
        PartContent::Operation(tool) => {
            let key = TranscriptNodeKey::Tool {
                message_id: message.id,
                part_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.tool_output_expanded);
            render_tool_execution(part, tool, out, width, i18n, expanded);
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Tool,
                copy_text: tool_output_copy_text(part, tool, i18n),
                toggleable: true,
                expanded,
            }
        }
        PartContent::Error(error) => {
            let text = i18n.text_args(
                "message-error",
                &crate::fl_args!(
                    "code" => error.code.as_str(),
                    "message" => error.message.as_str(),
                ),
            );
            push_multiline(out, "  ", &text, Style::default().fg(Color::Red), width);
            RenderedNodeDraft {
                key: TranscriptNodeKey::MessagePart {
                    message_id: message.id,
                    part_id: Some(part.id),
                },
                kind: TranscriptNodeKind::Message,
                copy_text: text,
                toggleable: false,
                expanded: true,
            }
        }
        PartContent::Attachment(attachment) => {
            push_section_heading(
                out,
                &format!("  {}", ui_text::t(i18n, "message-attachments")),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
                width,
            );
            let mut labels = Vec::new();
            for item in &attachment.attachments {
                let label = item
                    .title
                    .as_ref()
                    .or(item.filename.as_ref())
                    .cloned()
                    .unwrap_or_else(|| item.mime.clone());
                push_label_value(out, "    - ", label.as_str(), Style::default(), width);
                labels.push(label);
            }
            RenderedNodeDraft {
                key: TranscriptNodeKey::MessagePart {
                    message_id: message.id,
                    part_id: Some(part.id),
                },
                kind: TranscriptNodeKind::Message,
                copy_text: labels.join("\n"),
                toggleable: false,
                expanded: true,
            }
        }
        PartContent::Request(RequestPart::Permission(permission)) => {
            render_permission_request(permission, out, width, i18n);
            RenderedNodeDraft {
                key: TranscriptNodeKey::MessagePart {
                    message_id: message.id,
                    part_id: Some(part.id),
                },
                kind: TranscriptNodeKind::Message,
                copy_text: ui_text::permission_summary(i18n, permission),
                toggleable: false,
                expanded: true,
            }
        }
        PartContent::Request(RequestPart::UserInput(request)) => {
            render_user_input_request(request, out, width, i18n);
            RenderedNodeDraft {
                key: TranscriptNodeKey::MessagePart {
                    message_id: message.id,
                    part_id: Some(part.id),
                },
                kind: TranscriptNodeKind::Message,
                copy_text: request
                    .request
                    .questions
                    .iter()
                    .map(|question| question.question.clone())
                    .collect::<Vec<_>>()
                    .join("\n"),
                toggleable: false,
                expanded: true,
            }
        }
    }
}

fn render_reasoning_summary(
    summary: &str,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) {
    push_section_heading(
        out,
        "  thinking",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
        width,
    );
    if expanded {
        push_multiline(
            out,
            "    ",
            summary,
            Style::default().fg(Color::DarkGray),
            width,
        );
    } else {
        push_collapsible_text(
            out,
            "    ",
            summary,
            Style::default().fg(Color::DarkGray),
            width,
            i18n,
        );
    }
}

fn render_tool_execution(
    part: &MessagePart,
    tool: &OperationPart,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) {
    let label = if tool.title.trim().is_empty() {
        tool_invocation_label(&tool.invocation)
    } else {
        tool.title.clone()
    };
    let (message_key, color) = tool_status_key_and_color(part.status);
    if !expanded {
        push_single_line(
            out,
            "  ",
            tool_execution_collapsed_summary(part, tool, i18n).as_str(),
            Style::default().fg(color),
            width,
        );
        return;
    }
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
        if expanded {
            push_limited_tool_text(
                out,
                "    ",
                tool.model_output.text.as_str(),
                Style::default(),
                width,
                i18n,
            );
        } else {
            push_collapsible_text(
                out,
                "    ",
                tool.model_output.text.as_str(),
                Style::default(),
                width,
                i18n,
            );
        }
    }

    if let Some(diff) = apply_patch_diff(&tool.details) {
        push_label_value(
            out,
            "    ",
            &ui_text::operation_diff_summary(i18n, diff.lines().count()),
            Style::default().fg(Color::DarkGray),
            width,
        );
        if expanded {
            push_limited_tool_text(
                out,
                "    ",
                diff.as_str(),
                Style::default().fg(Color::DarkGray),
                width,
                i18n,
            );
        } else {
            push_collapsible_text(
                out,
                "    ",
                diff.as_str(),
                Style::default().fg(Color::DarkGray),
                width,
                i18n,
            );
        }
    }

    render_operation_blocks(tool.blocks.as_slice(), out, width, i18n, expanded);
}

fn render_operation_blocks(
    blocks: &[OperationBlock],
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) {
    for block in blocks {
        match block {
            OperationBlock::Text { text } => {
                if expanded {
                    push_limited_tool_text(out, "    ", text, Style::default(), width, i18n);
                } else {
                    push_collapsible_text(out, "    ", text, Style::default(), width, i18n);
                }
            }
            OperationBlock::Markdown { text } => {
                if expanded {
                    push_limited_markdown(out, "    ", text, width, i18n);
                } else {
                    push_collapsible_text(out, "    ", text, Style::default(), width, i18n);
                }
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
                    if expanded {
                        push_limited_tool_text(
                            out,
                            "      ",
                            stdout,
                            Style::default(),
                            width,
                            i18n,
                        );
                    } else {
                        push_collapsible_text(out, "      ", stdout, Style::default(), width, i18n);
                    }
                }
                if let Some(stderr) = stderr
                    && !stderr.trim().is_empty()
                {
                    if expanded {
                        push_limited_tool_text(
                            out,
                            "      ",
                            stderr,
                            Style::default().fg(Color::Red),
                            width,
                            i18n,
                        );
                    } else {
                        push_collapsible_text(
                            out,
                            "      ",
                            stderr,
                            Style::default().fg(Color::Red),
                            width,
                            i18n,
                        );
                    }
                }
                if let Some(exit_code) = exit_code
                    && *exit_code != 0
                {
                    push_label_value(
                        out,
                        "      ",
                        &ui_text::operation_command_exit_line(i18n, *exit_code),
                        Style::default().fg(Color::DarkGray),
                        width,
                    );
                }
            }
            OperationBlock::Diff { diff, .. } => {
                if expanded {
                    push_limited_tool_text(
                        out,
                        "    ",
                        diff,
                        Style::default().fg(Color::DarkGray),
                        width,
                        i18n,
                    );
                } else {
                    push_collapsible_text(
                        out,
                        "    ",
                        diff,
                        Style::default().fg(Color::DarkGray),
                        width,
                        i18n,
                    );
                }
            }
            OperationBlock::FileChanges { changes } => {
                render_file_changes(changes, out, width, i18n)
            }
            OperationBlock::Checklist { items } => render_checklist(items, out, width, i18n),
            OperationBlock::SearchResults { query, results } => {
                let heading = ui_text::operation_search_heading(i18n, query.as_deref());
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
                    if expanded {
                        push_limited_tool_text(out, "      ", text, Style::default(), width, i18n);
                    } else {
                        push_collapsible_text(out, "      ", text, Style::default(), width, i18n);
                    }
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
                if expanded {
                    push_limited_tool_text(
                        out,
                        "    ",
                        message,
                        Style::default().fg(Color::DarkGray),
                        width,
                        i18n,
                    );
                } else {
                    push_collapsible_text(
                        out,
                        "    ",
                        message,
                        Style::default().fg(Color::DarkGray),
                        width,
                        i18n,
                    );
                }
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
                    &ui_text::operation_nested_task_summary(i18n, title, *status),
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
    changes: &[agena::message::FileChangeRecord],
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
    permission: &agena::message::InteractiveRequestPart<
        agena::permission::PermissionRequest,
        agena::permission::PermissionReply,
    >,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    push_section_heading(
        out,
        &format!("  {}", ui_text::message_permission_heading(i18n)),
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
    request: &agena::message::InteractiveRequestPart<
        agena::message::UserInputRequest,
        agena::message::UserInputReply,
    >,
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
    match transcript_part_content(part) {
        PartContent::Text(text) => first_non_empty_preview_line(text.text.as_str()),
        PartContent::Reasoning(reasoning) => {
            let summary = reasoning.preferred_text();
            first_non_empty_preview_line(summary.as_str())
        }
        PartContent::Operation(tool) => Some(tool_execution_preview(part, tool, i18n)),
        PartContent::Error(error) => Some(ui_text::message_error_text(
            i18n,
            error.code.as_str(),
            error.message.as_str(),
        )),
        PartContent::Attachment(attachment) => attachment.attachments.first().map(|item| {
            item.title
                .as_ref()
                .or(item.filename.as_ref())
                .cloned()
                .unwrap_or_else(|| item.mime.clone())
        }),
        PartContent::Request(RequestPart::Permission(permission)) => {
            Some(ui_text::permission_summary(i18n, permission))
        }
        PartContent::Request(RequestPart::UserInput(request)) => request
            .request
            .questions
            .first()
            .map(|question| question.question.clone()),
    }
}

fn tool_execution_preview(part: &MessagePart, tool: &OperationPart, i18n: &I18n) -> String {
    ui_text::message_tool_summary(
        i18n,
        part.status,
        tool_invocation_label(&tool.invocation).as_str(),
    )
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

fn push_single_line(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    style: Style,
    width: u16,
) {
    let available_width = width.max(1) as usize;
    let prefix_width = UnicodeWidthStr::width(prefix);
    if prefix_width >= available_width {
        out.push(RenderedLine::plain(
            truncate_display_width(prefix, available_width),
            style,
        ));
        return;
    }
    let body = truncate_display_width(text, available_width.saturating_sub(prefix_width));
    out.push(RenderedLine::plain(format!("{prefix}{body}"), style));
}

fn push_collapsible_text(
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

fn push_limited_tool_text(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    style: Style,
    width: u16,
    i18n: &I18n,
) {
    let preview = tool_output_preview_with_limits(
        text,
        TOOL_EXPANDED_PREVIEW_LINES,
        TOOL_EXPANDED_PREVIEW_CHARS,
    );
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

fn push_limited_markdown(
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
    push_markdown(out, prefix, preview.text.as_str(), width);
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

fn tool_output_copy_text(part: &MessagePart, tool: &OperationPart, i18n: &I18n) -> String {
    let label = if tool.title.trim().is_empty() {
        tool_invocation_label(&tool.invocation)
    } else {
        tool.title.clone()
    };
    let mut sections = vec![tool_execution_preview(part, tool, i18n), label];
    if !tool.model_output.text.trim().is_empty() {
        sections.push(tool.model_output.text.trim().to_string());
    }
    if let Some(diff) = apply_patch_diff(&tool.details)
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

fn tool_status_key_and_color(status: ExecutionStatus) -> (&'static str, Color) {
    match status {
        ExecutionStatus::Pending => ("message-tool-pending", Color::Magenta),
        ExecutionStatus::InProgress => ("message-tool-running", Color::Magenta),
        ExecutionStatus::Completed => ("message-tool-done", Color::Green),
        ExecutionStatus::Failed => ("message-tool-failed", Color::Red),
        ExecutionStatus::Cancelled => ("message-tool-failed", Color::DarkGray),
    }
}

fn tool_execution_collapsed_summary(
    part: &MessagePart,
    tool: &OperationPart,
    i18n: &I18n,
) -> String {
    ui_text::message_tool_summary(
        i18n,
        part.status,
        tool_invocation_label(&tool.invocation).as_str(),
    )
}

fn transcript_message_parts(message: &MessageResource) -> &[MessagePart] {
    message
        .parts
        .as_deref()
        .expect("transcript messages must include full parts")
}

fn transcript_part_content(part: &MessagePart) -> &PartContent {
    part.content
        .as_ref()
        .expect("transcript message parts must include full content")
}

fn operation_block_copy_text(block: &OperationBlock, i18n: &I18n) -> String {
    match block {
        OperationBlock::Text { text }
        | OperationBlock::Markdown { text }
        | OperationBlock::Diff { diff: text, .. } => text.clone(),
        OperationBlock::Command {
            command,
            exit_code,
            stdout,
            stderr,
            ..
        } => {
            let mut parts = vec![format!("$ {command}")];
            if let Some(stdout) = stdout
                && !stdout.trim().is_empty()
            {
                parts.push(stdout.trim().to_string());
            }
            if let Some(stderr) = stderr
                && !stderr.trim().is_empty()
            {
                parts.push(stderr.trim().to_string());
            }
            if let Some(exit_code) = exit_code {
                parts.push(ui_text::operation_command_exit_line(i18n, *exit_code));
            }
            parts.join("\n")
        }
        OperationBlock::SearchResults { query, results } => {
            let mut out = Vec::new();
            if let Some(query) = query {
                out.push(ui_text::operation_search_heading(
                    i18n,
                    Some(query.as_str()),
                ));
            } else {
                out.push(ui_text::operation_search_heading(i18n, None));
            }
            for result in results {
                out.push(result.title.clone());
                out.push(result.uri.clone());
                if let Some(snippet) = &result.snippet
                    && !snippet.trim().is_empty()
                {
                    out.push(snippet.clone());
                }
            }
            out.join("\n")
        }
        OperationBlock::EmbeddedResource { uri, text, .. } => text
            .as_deref()
            .map(str::to_string)
            .unwrap_or_else(|| uri.clone()),
        OperationBlock::Checklist { items } => items
            .iter()
            .map(|item| item.content.clone())
            .collect::<Vec<_>>()
            .join("\n"),
        OperationBlock::FileChanges { changes } => changes
            .iter()
            .map(|change| change.path.clone())
            .collect::<Vec<_>>()
            .join("\n"),
        OperationBlock::ResourceLink { uri, title, .. }
        | OperationBlock::Citation { uri, title, .. } => {
            title.clone().unwrap_or_else(|| uri.clone())
        }
        OperationBlock::Image { url, .. }
        | OperationBlock::Audio { url, .. }
        | OperationBlock::File { url, .. } => url.clone(),
        OperationBlock::Media { artifact, .. } => artifact
            .name
            .clone()
            .unwrap_or_else(|| artifact.uri.clone()),
        OperationBlock::Progress { message, .. } => message.clone(),
        OperationBlock::NestedTask {
            task_id,
            title,
            status,
        } => ui_text::operation_nested_task_summary(
            i18n,
            title.as_deref().unwrap_or(task_id.as_str()),
            *status,
        ),
        OperationBlock::Json { .. }
        | OperationBlock::Table { .. }
        | OperationBlock::Log { .. }
        | OperationBlock::Custom { .. } => String::new(),
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

fn push_markdown(out: &mut Vec<RenderedLine>, prefix: &str, text: &str, width: u16) {
    let sanitized = sanitize_terminal_text(text);
    let markdown = trim_empty_line_edges(sanitized.as_str());
    if markdown.is_empty() {
        return;
    }

    let lines = markdown.lines().collect::<Vec<_>>();
    let mut chunk = Vec::<&str>::new();
    let mut active_fence = None::<MarkdownFence>;
    let mut index = 0_usize;

    while index < lines.len() {
        let line = lines[index];
        if let Some(delimiter) = markdown_fence_delimiter(line) {
            if let Some(active) = active_fence {
                if delimiter.marker == active.marker && delimiter.len >= active.len {
                    active_fence = None;
                }
            } else {
                active_fence = Some(delimiter);
            }
            chunk.push(line);
            index += 1;
            continue;
        }

        if active_fence.is_none()
            && index + 1 < lines.len()
            && is_markdown_table_header(lines[index], lines[index + 1])
        {
            flush_markdown_chunk(out, prefix, &mut chunk, width);
            let mut table_lines = vec![lines[index], lines[index + 1]];
            index += 2;
            while index < lines.len() && looks_like_markdown_table_row(lines[index]) {
                table_lines.push(lines[index]);
                index += 1;
            }
            push_markdown_table(out, prefix, table_lines.as_slice(), width);
            continue;
        }

        chunk.push(line);
        index += 1;
    }

    flush_markdown_chunk(out, prefix, &mut chunk, width);
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
    let line_style = line.style;
    let line_alignment = line.alignment;
    if line.spans.is_empty() {
        out.push(RenderedLine::rich(Line {
            style: line_style,
            alignment: line_alignment,
            spans: vec![Span::raw(initial_prefix.to_string())],
        }));
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
        out.push(RenderedLine::rich(prefix_rich_line(
            prefix,
            Line {
                style: line_style,
                alignment: line_alignment,
                spans: wrapped_line.spans,
            },
        )));
    }
}

fn prefix_rich_line(prefix: &str, line: Line<'static>) -> Line<'static> {
    if prefix.is_empty() {
        return line;
    }
    let style = line.style;
    let alignment = line.alignment;
    let mut spans = Vec::with_capacity(line.spans.len().saturating_add(1));
    spans.push(Span::raw(prefix.to_string()));
    spans.extend(line.spans);
    Line {
        style,
        alignment,
        spans,
    }
}

fn owned_line(line: &Line<'_>) -> Line<'static> {
    Line {
        style: line.style,
        alignment: line.alignment,
        spans: line
            .spans
            .iter()
            .map(|span| Span::styled(span.content.to_string(), span.style))
            .collect::<Vec<_>>(),
    }
}

fn flush_markdown_chunk(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    chunk: &mut Vec<&str>,
    width: u16,
) {
    if chunk.is_empty() {
        return;
    }
    let chunk_text = chunk.join("\n");
    let rendered = markdown_to_text(chunk_text.as_str());
    for line in rendered.lines {
        push_wrapped_rich_line(out, prefix, prefix, owned_line(&line), width);
    }
    chunk.clear();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MarkdownFence {
    marker: char,
    len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableColumnAlignment {
    Left,
    Center,
    Right,
}

fn markdown_fence_delimiter(line: &str) -> Option<MarkdownFence> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|ch| *ch == marker).count();
    (len >= 3).then_some(MarkdownFence { marker, len })
}

fn is_markdown_table_header(header: &str, delimiter: &str) -> bool {
    if !header.contains('|') {
        return false;
    }
    parse_markdown_table_alignment(delimiter).is_some()
}

fn looks_like_markdown_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.contains('|')
}

fn push_markdown_table(out: &mut Vec<RenderedLine>, prefix: &str, lines: &[&str], width: u16) {
    if lines.len() < 2 {
        for line in lines {
            push_multiline(out, prefix, line, Style::default(), width);
        }
        return;
    }

    let Some(alignments) = parse_markdown_table_alignment(lines[1]) else {
        for line in lines {
            push_multiline(out, prefix, line, Style::default(), width);
        }
        return;
    };

    let mut rows = lines
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 1)
        .map(|(_, line)| parse_markdown_table_row(line))
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return;
    }

    let column_count = rows
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or_else(|| alignments.len().max(1));
    if column_count == 0 {
        return;
    }

    let alignments = normalize_table_alignments(alignments, column_count);
    for row in &mut rows {
        row.resize(column_count, String::new());
        for cell in row.iter_mut() {
            *cell = markdown_table_cell_text(cell.as_str());
        }
    }

    let separator_width = column_count.saturating_sub(1).saturating_mul(3);
    let prefix_width = UnicodeWidthStr::width(prefix);
    let available_width = width.max(1) as usize;
    let table_width_budget = available_width.saturating_sub(prefix_width);
    let min_content_width = column_count.saturating_mul(3);
    if table_width_budget <= separator_width.saturating_add(min_content_width) {
        push_markdown_table_fallback(out, prefix, &rows, width);
        return;
    }

    let column_widths = compute_table_column_widths(
        rows.as_slice(),
        table_width_budget.saturating_sub(separator_width),
    );
    if column_widths.is_empty() {
        push_markdown_table_fallback(out, prefix, &rows, width);
        return;
    }

    let header_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let separator_style = Style::default().fg(Color::DarkGray);
    let body_style = Style::default();

    render_table_row(
        out,
        prefix,
        rows[0].as_slice(),
        column_widths.as_slice(),
        alignments.as_slice(),
        header_style,
    );
    push_table_separator(out, prefix, column_widths.as_slice(), separator_style);
    for row in rows.iter().skip(1) {
        render_table_row(
            out,
            prefix,
            row.as_slice(),
            column_widths.as_slice(),
            alignments.as_slice(),
            body_style,
        );
    }
}

fn push_markdown_table_fallback(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    rows: &[Vec<String>],
    width: u16,
) {
    if rows.is_empty() {
        return;
    }
    let header = rows[0]
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" | ");
    push_multiline(
        out,
        prefix,
        header.as_str(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        width,
    );
    for row in rows.iter().skip(1) {
        push_multiline(
            out,
            prefix,
            row.iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" | ")
                .as_str(),
            Style::default(),
            width,
        );
    }
}

fn parse_markdown_table_alignment(line: &str) -> Option<Vec<TableColumnAlignment>> {
    let cells = parse_markdown_table_row(line);
    if cells.is_empty() {
        return None;
    }

    cells
        .into_iter()
        .map(|cell| {
            let trimmed = cell.trim();
            if trimmed.is_empty()
                || !trimmed.contains('-')
                || !trimmed.chars().all(|ch| matches!(ch, '-' | ':' | ' '))
            {
                return None;
            }
            Some(match (trimmed.starts_with(':'), trimmed.ends_with(':')) {
                (true, true) => TableColumnAlignment::Center,
                (false, true) => TableColumnAlignment::Right,
                _ => TableColumnAlignment::Left,
            })
        })
        .collect()
}

fn parse_markdown_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let content = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let content = content.strip_suffix('|').unwrap_or(content);
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut escape = false;

    for ch in content.chars() {
        if escape {
            cell.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '|' => {
                cells.push(cell.trim().to_string());
                cell.clear();
            }
            _ => cell.push(ch),
        }
    }
    if escape {
        cell.push('\\');
    }
    cells.push(cell.trim().to_string());
    cells
}

fn normalize_table_alignments(
    mut alignments: Vec<TableColumnAlignment>,
    column_count: usize,
) -> Vec<TableColumnAlignment> {
    alignments.resize(column_count, TableColumnAlignment::Left);
    alignments
}

fn markdown_table_cell_text(cell: &str) -> String {
    let rendered = markdown_to_text(cell);
    let flattened = rendered
        .lines
        .iter()
        .map(|line| line_plain_text(&owned_line(line)))
        .collect::<Vec<_>>()
        .join(" ");
    sanitize_terminal_text(flattened.as_str())
        .trim()
        .to_string()
}

fn compute_table_column_widths(rows: &[Vec<String>], budget: usize) -> Vec<usize> {
    if rows.is_empty() {
        return Vec::new();
    }

    let column_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    if column_count == 0 {
        return Vec::new();
    }

    let min_width = 3_usize;
    let min_total = min_width.saturating_mul(column_count);
    if budget < min_total {
        return Vec::new();
    }

    let natural_widths = (0..column_count)
        .map(|index| {
            rows.iter()
                .filter_map(|row| row.get(index))
                .map(|cell| UnicodeWidthStr::width(cell.as_str()).max(min_width))
                .max()
                .unwrap_or(min_width)
        })
        .collect::<Vec<_>>();
    let mut widths = vec![min_width; column_count];
    let mut remaining = budget.saturating_sub(min_total);
    let mut deficits = natural_widths
        .iter()
        .map(|width| width.saturating_sub(min_width))
        .collect::<Vec<_>>();

    while remaining > 0 && deficits.iter().any(|deficit| *deficit > 0) {
        for index in 0..column_count {
            if deficits[index] == 0 || remaining == 0 {
                continue;
            }
            widths[index] += 1;
            deficits[index] -= 1;
            remaining -= 1;
        }
    }

    widths
}

fn render_table_row(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    cells: &[String],
    widths: &[usize],
    alignments: &[TableColumnAlignment],
    style: Style,
) {
    let wrapped_cells = cells
        .iter()
        .zip(widths.iter())
        .map(|(cell, width)| wrap_table_cell(cell.as_str(), *width))
        .collect::<Vec<_>>();
    let row_height = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1).max(1);

    for line_index in 0..row_height {
        let mut spans = vec![Span::raw(prefix.to_string())];
        for (column_index, width) in widths.iter().enumerate() {
            if column_index > 0 {
                spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
            }
            let text = wrapped_cells
                .get(column_index)
                .and_then(|lines| lines.get(line_index))
                .cloned()
                .unwrap_or_default();
            spans.push(Span::styled(
                pad_table_cell(
                    text.as_str(),
                    *width,
                    alignments
                        .get(column_index)
                        .copied()
                        .unwrap_or(TableColumnAlignment::Left),
                ),
                style,
            ));
        }
        out.push(RenderedLine::rich(Line::from(spans)));
    }
}

fn push_table_separator(out: &mut Vec<RenderedLine>, prefix: &str, widths: &[usize], style: Style) {
    let mut spans = vec![Span::raw(prefix.to_string())];
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("-+-", style));
        }
        spans.push(Span::styled("-".repeat(*width), style));
    }
    out.push(RenderedLine::rich(Line::from(spans)));
}

fn wrap_table_cell(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }

    let normalized = sanitize_terminal_text(text).trim().to_string();
    if normalized.is_empty() {
        return vec![String::new()];
    }

    let options = WrapOptions::new(width)
        .break_words(true)
        .word_splitter(WordSplitter::NoHyphenation);
    let wrapped = wrap(normalized.as_str(), options);
    if wrapped.is_empty() {
        return vec![String::new()];
    }

    wrapped
        .into_iter()
        .map(|segment| truncate_display_width(segment.as_ref(), width))
        .collect()
}

fn pad_table_cell(text: &str, width: usize, alignment: TableColumnAlignment) -> String {
    let visible = truncate_display_width(text, width);
    let visible_width = UnicodeWidthStr::width(visible.as_str());
    let padding = width.saturating_sub(visible_width);
    match alignment {
        TableColumnAlignment::Left => format!("{visible}{}", " ".repeat(padding)),
        TableColumnAlignment::Right => format!("{}{visible}", " ".repeat(padding)),
        TableColumnAlignment::Center => {
            let left = padding / 2;
            let right = padding.saturating_sub(left);
            format!("{}{}{}", " ".repeat(left), visible, " ".repeat(right))
        }
    }
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
    use agena::message::ReasoningPart;
    use chrono::Utc;
    use serde_json::json;

    fn transcript_message(parts: Vec<MessagePart>) -> MessageResource {
        MessageResource {
            id: 1,
            session_id: 1,
            role: MessageRole::Assistant,
            state: MessageStatus::Completed,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: Default::default(),
            usage: None,
            part_count: parts.len() as u64,
            parts: Some(parts),
        }
    }

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

    #[test]
    fn push_markdown_preserves_heading_line_style() {
        let mut out = Vec::new();
        push_markdown(&mut out, "  ", "# heading", 40);
        let heading = out
            .iter()
            .find_map(|line| line.rich_line.as_ref())
            .expect("expected heading line");
        assert_ne!(heading.style, Style::default());
        assert_eq!(out[0].style, heading.style);
    }

    #[test]
    fn push_markdown_renders_pipe_tables() {
        let mut out = Vec::new();
        push_markdown(
            &mut out,
            "  ",
            "| Name | Status |\n| --- | ---: |\n| Alice | ok |\n| Bob | waiting |",
            48,
        );
        let rendered = out.into_iter().map(|line| line.text).collect::<Vec<_>>();
        assert!(rendered.iter().any(|line| line.contains("Name")));
        assert!(rendered.iter().any(|line| line.contains("Alice")));
        assert!(rendered.iter().any(|line| line.contains("-+-")));
    }

    #[test]
    fn transcript_export_markdown_localizes_metadata_and_empty_state() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let markdown = render_transcript_export_markdown(&i18n, Some(42), "", None, &[], true);
        let first_line = markdown.lines().next().unwrap_or_default();

        assert!(first_line.contains("会话"));
        assert!(first_line.contains("42"));
        assert!(markdown.contains("会话 ID"));
        assert!(markdown.contains("42"));
        assert!(markdown.contains("已省略更早消息"));
        assert!(markdown.contains("是"));
        assert!(markdown.contains("_当前会话没有已加载消息。_"));
    }

    #[test]
    fn operation_block_copy_text_localizes_search_headings() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let text = operation_block_copy_text(
            &OperationBlock::SearchResults {
                query: Some("rust".to_string()),
                results: vec![agena::message::SearchResultItem {
                    title: "Rust".to_string(),
                    uri: "https://www.rust-lang.org".to_string(),
                    snippet: Some("Systems programming language".to_string()),
                    score: None,
                }],
            },
            &i18n,
        );
        let first_line = text.lines().next().unwrap_or_default();

        assert!(first_line.contains("搜索"));
        assert!(first_line.contains("rust"));
        assert!(text.contains("https://www.rust-lang.org"));
    }

    #[test]
    fn render_reasoning_part_does_not_inject_spaces_between_deltas() {
        let part = MessagePart::with_content(
            1,
            1,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::Reasoning(ReasoningPart {
                summary: vec![
                    "The".to_string(),
                    " user".to_string(),
                    " wants".to_string(),
                    " a".to_string(),
                    " bigger".to_string(),
                    " /m".to_string(),
                    "ore".to_string(),
                    " extensive".to_string(),
                    " table.".to_string(),
                ],
                raw_content: Vec::new(),
                encrypted_content: None,
            }),
        );

        let mut out = Vec::new();
        render_part(&part, 120, &mut out, &I18n::english());

        let rendered = out.into_iter().map(|line| line.text).collect::<Vec<_>>();
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("The user wants a bigger /more extensive table."))
        );
        assert!(!rendered.iter().any(|line| line.contains("/m ore")));
        assert!(!rendered.iter().any(|line| line.contains("The  user")));
    }

    #[test]
    fn collapsed_tool_output_renders_as_a_single_summary_line() {
        let invocation = ToolInvocation::new(
            "bash",
            serde_json::from_value(json!({ "command": "ls -la src" }))
                .expect("valid structured input"),
        );
        let tool = OperationPart::completed(
            7,
            invocation,
            "alpha\nbeta\ngamma",
            vec![OperationBlock::Command {
                command: "ls -la src".to_string(),
                cwd: None,
                exit_code: Some(0),
                stdout: Some("file-a\nfile-b\nfile-c".to_string()),
                stderr: None,
            }],
            Vec::new(),
            agena::message::ToolOutput::default(),
            agena::message::TimeRange::default(),
        );
        let part = MessagePart::with_content(
            1,
            1,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::Operation(tool.clone()),
        );

        let mut out = Vec::new();
        render_tool_execution(&part, &tool, &mut out, 120, &I18n::english(), false);

        let rendered = out.into_iter().map(|line| line.text).collect::<Vec<_>>();
        assert_eq!(
            rendered.len(),
            1,
            "collapsed tool output should stay on one line"
        );
        assert!(rendered[0].contains("tool"));
        assert!(rendered[0].contains("completed"));
        assert!(rendered[0].contains("bash"));
    }

    #[test]
    fn expanded_tool_output_caps_long_histories() {
        let stdout = (0..(TOOL_EXPANDED_PREVIEW_LINES + 5))
            .map(|index| format!("line-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let invocation = ToolInvocation::new(
            "bash",
            serde_json::from_value(json!({ "command": "tail -f log.txt" }))
                .expect("valid structured input"),
        );
        let tool = OperationPart::completed(
            8,
            invocation,
            String::new(),
            vec![OperationBlock::Command {
                command: "tail -f log.txt".to_string(),
                cwd: None,
                exit_code: Some(0),
                stdout: Some(stdout),
                stderr: None,
            }],
            Vec::new(),
            agena::message::ToolOutput::default(),
            agena::message::TimeRange::default(),
        );
        let part = MessagePart::with_content(
            1,
            1,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::Operation(tool.clone()),
        );

        let mut out = Vec::new();
        render_tool_execution(&part, &tool, &mut out, 120, &I18n::english(), true);

        let rendered = out.into_iter().map(|line| line.text).collect::<Vec<_>>();
        assert!(rendered.iter().any(|line| line.contains("line-0")));
        assert!(
            rendered.iter().any(|line| line.contains("more")),
            "expected capped expanded output to advertise hidden history"
        );
        assert!(
            !rendered
                .iter()
                .any(|line| line.contains(&format!("line-{}", TOOL_EXPANDED_PREVIEW_LINES + 4))),
            "expected lines beyond the expanded cap to stay hidden"
        );
    }

    #[test]
    fn render_tool_execution_localizes_diff_summary() {
        let invocation = ToolInvocation::new(
            "apply_patch",
            serde_json::from_value(json!({ "path": "src/app.rs" }))
                .expect("valid structured input"),
        );
        let details = agena::message::ToolOutput::from_json_payload(Some(&json!({
            "diff": "line-a\nline-b"
        })))
        .expect("valid diff payload");
        let tool = OperationPart::completed(
            9,
            invocation,
            String::new(),
            Vec::new(),
            Vec::new(),
            details,
            agena::message::TimeRange::default(),
        );
        let part = MessagePart::with_content(
            1,
            1,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::Operation(tool.clone()),
        );

        let mut out = Vec::new();
        render_tool_execution(
            &part,
            &tool,
            &mut out,
            120,
            &I18n::resolve(Some("zh-CN"), None),
            true,
        );

        let rendered = out
            .into_iter()
            .map(|line| sanitize_terminal_text(line.text.as_str()))
            .collect::<Vec<_>>();
        assert!(rendered.iter().any(|line| line.contains("diff：2 行")));
    }

    #[test]
    fn operation_block_copy_text_localizes_nested_task_status() {
        let block = OperationBlock::NestedTask {
            task_id: "sync-index".to_string(),
            title: Some("同步索引".to_string()),
            status: ExecutionStatus::Completed,
        };

        let text = sanitize_terminal_text(
            operation_block_copy_text(&block, &I18n::resolve(Some("zh-CN"), None)).as_str(),
        );

        assert_eq!(text, "同步索引（已完成）");
    }

    #[test]
    fn plain_message_parts_are_not_toggleable() {
        let message = transcript_message(vec![MessagePart::with_content(
            1,
            1,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::Text(agena::message::TextPart {
                text: "hello\nworld".to_string(),
                synthetic: false,
                ignored: false,
            }),
        )]);

        let rendered = render_message_detailed(
            &message,
            80,
            &I18n::english(),
            TranscriptDetailDefaults {
                tool_output_expanded: false,
                thinking_expanded: false,
            },
            &std::collections::BTreeMap::new(),
        );

        assert_eq!(rendered.nodes.len(), 1);
        assert!(!rendered.nodes[0].toggleable);
        assert!(rendered.nodes[0].expanded);
    }
}
