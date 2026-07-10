use super::*;
use agena::message::{ExecutionStatus, FileChangeKind, MessageStatus, OperationBlock, RequestPart};
use agena_tui_components::{line_plain_text, trim_empty_line_edges};
use textwrap::{Options as WrapOptions, WordSplitter, wrap};
use tui_markdown::from_str as markdown_to_text;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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
            if let PartContent::Text(text) = transcript_part_content(part) {
                let blocks = markdown_blocks(text.text.as_str());
                for (block_index, block) in blocks.iter().enumerate() {
                    if should_suppress_markdown_block(blocks.as_slice(), block_index) {
                        continue;
                    }
                    if block.leading_blank_line && lines.len() > header_start.saturating_add(1) {
                        lines.push(RenderedLine::plain("  ".to_string(), Style::default()));
                    }

                    // Keep the message header outside Markdown selections.  A selected code
                    // block or list should be exactly that block, both visually and on copy.
                    let start_line = lines.len();
                    render_markdown_block(&mut lines, "  ", &block, width);
                    if lines.len() > start_line {
                        nodes.push(RenderedTranscriptNode {
                            key: TranscriptNodeKey::MarkdownBlock {
                                message_id: message.id,
                                part_id: part.id,
                                block_index,
                            },
                            kind: block.kind,
                            start_line,
                            end_line: lines.len(),
                            copy_text: block.copy_text.clone(),
                            toggleable: false,
                            expanded: true,
                        });
                        attached_header = true;
                    }
                }
                continue;
            }

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

pub(super) fn render_message_export(
    message: &MessageResource,
    width: u16,
    i18n: &I18n,
    defaults: TranscriptDetailDefaults,
) -> Vec<RenderedLine> {
    render_message_detailed(
        message,
        width,
        i18n,
        defaults,
        &std::collections::BTreeMap::new(),
    )
    .lines
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownBlock {
    kind: TranscriptNodeKind,
    source: String,
    copy_text: String,
    leading_blank_line: bool,
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
            render_message_export(
                message,
                u16::MAX,
                i18n,
                TranscriptDetailDefaults {
                    tool_output_expanded: true,
                    thinking_expanded: false,
                },
            )
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

fn render_tool_execution(
    part: &MessagePart,
    tool: &OperationPart,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) {
    let label = tool_display_label(tool);
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

    let failure_text = if part.status == ExecutionStatus::Failed {
        tool.error_message()
            .map(str::trim)
            .filter(|text| !text.is_empty())
    } else {
        None
    };

    if let Some(error_message) = failure_text {
        push_multiline(
            out,
            "    ",
            error_message,
            Style::default().fg(Color::Red),
            width,
        );
    }

    if should_render_tool_model_output(tool, failure_text) {
        if expanded {
            render_limited_tool_text_block(
                out,
                "    ",
                tool.model_output.text.as_str(),
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

    let apply_patch = apply_patch_details(&tool.details);
    if let Some(changes) = apply_patch
        .as_ref()
        .filter(|payload| !payload.changes.is_empty())
        .map(|payload| payload.changes.as_slice())
    {
        render_file_changes(changes, out, width, i18n);
    }

    if let Some(diff) = apply_patch
        .as_ref()
        .map(|payload| payload.diff.as_str())
        .filter(|diff| !diff.trim().is_empty())
    {
        let stats = diff_stats(
            diff,
            apply_patch
                .as_ref()
                .map(|payload| payload.changes.as_slice()),
        );
        push_label_value(
            out,
            "    ",
            &ui_text::operation_diff_summary(
                i18n,
                stats.file_count,
                stats.additions,
                stats.deletions,
                stats.renames,
                stats.line_count,
            ),
            Style::default().fg(Color::DarkGray),
            width,
        );
        push_limited_diff_text(out, "    ", diff, width, i18n);
    }

    render_operation_blocks(
        tool.blocks.as_slice(),
        out,
        width,
        i18n,
        expanded,
        failure_text,
        apply_patch
            .as_ref()
            .is_some_and(|payload| !payload.changes.is_empty()),
    );
}

fn render_operation_blocks(
    blocks: &[OperationBlock],
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
    skipped_text: Option<&str>,
    skip_file_changes: bool,
) {
    for block in blocks {
        match block {
            OperationBlock::Text { text } => {
                if skipped_text.is_some_and(|candidate| text.trim() == candidate) {
                    continue;
                }
                if expanded {
                    render_limited_tool_text_block(out, "    ", text, width, i18n);
                } else {
                    push_collapsible_text(out, "    ", text, Style::default(), width, i18n);
                }
            }
            OperationBlock::Markdown { text } => {
                if skipped_text.is_some_and(|candidate| text.trim() == candidate) {
                    continue;
                }
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
                    push_limited_diff_text(out, "    ", diff, width, i18n);
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
                if !skip_file_changes {
                    render_file_changes(changes, out, width, i18n)
                }
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
                    media_artifact_label(artifact).as_str(),
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
        push_label_value(
            out,
            "      - ",
            &file_change_list_item_text(entry, i18n),
            file_change_style(entry.kind),
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

fn tool_status_key_and_color(status: ExecutionStatus) -> (&'static str, Color) {
    match status {
        ExecutionStatus::Pending => ("message-tool-pending", Color::Magenta),
        ExecutionStatus::InProgress => ("message-tool-running", Color::Magenta),
        ExecutionStatus::Completed => ("message-tool-done", Color::Green),
        ExecutionStatus::Failed => ("message-tool-failed", Color::Red),
        ExecutionStatus::Cancelled => ("message-tool-cancelled", Color::DarkGray),
    }
}

fn tool_execution_collapsed_summary(
    part: &MessagePart,
    tool: &OperationPart,
    i18n: &I18n,
) -> String {
    let base_label = tool_display_label(tool);
    let label = apply_patch_details(&tool.details)
        .filter(|payload| !payload.changes.is_empty())
        .map(|payload| {
            format!(
                "{} · {}",
                base_label,
                ui_text::file_changes_preview(
                    i18n,
                    payload.changes.len(),
                    summarize_change_paths(i18n, payload.changes.as_slice(), 3).as_str(),
                )
            )
        })
        .unwrap_or(base_label);
    ui_text::message_tool_summary(i18n, part.status, label.as_str())
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
            .map(|change| file_change_list_item_text(change, i18n))
            .collect::<Vec<_>>()
            .join("\n"),
        OperationBlock::ResourceLink { uri, title, .. }
        | OperationBlock::Citation { uri, title, .. } => {
            title.clone().unwrap_or_else(|| uri.clone())
        }
        OperationBlock::Image { url, .. }
        | OperationBlock::Audio { url, .. }
        | OperationBlock::File { url, .. } => url.clone(),
        OperationBlock::Media { artifact, .. } => media_artifact_label(artifact),
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

fn media_artifact_label(artifact: &agena::message::ArtifactRef) -> String {
    let uri = artifact.uri.trim();
    if uri.starts_with("file://")
        || uri.starts_with('/')
        || uri.starts_with("./")
        || uri.starts_with("../")
    {
        return uri.to_string();
    }

    artifact
        .name
        .clone()
        .unwrap_or_else(|| artifact.uri.clone())
}

fn tool_display_label(tool: &OperationPart) -> String {
    if tool.title.trim().is_empty() {
        tool_invocation_label(&tool.invocation)
    } else {
        tool.title.clone()
    }
}

fn should_render_tool_model_output(tool: &OperationPart, skipped_text: Option<&str>) -> bool {
    let model_output = normalized_tool_text(tool.model_output.text.as_str());
    if model_output.is_empty() {
        return false;
    }
    if tool.invocation.name == "agena_web__search"
        && tool
            .blocks
            .iter()
            .any(|block| matches!(block, OperationBlock::SearchResults { .. }))
    {
        return false;
    }
    let tool_label = normalized_tool_text(tool_display_label(tool).as_str());
    if tool_label == model_output {
        return false;
    }
    if let Some(prefix) = tool_label.strip_suffix(model_output.as_str())
        && prefix.chars().last().is_some_and(|ch| ch.is_whitespace())
        && !prefix.trim().is_empty()
    {
        return false;
    }
    if skipped_text.is_some_and(|candidate| normalized_tool_text(candidate) == model_output) {
        return false;
    }
    !tool
        .blocks
        .iter()
        .filter_map(operation_text_block_text)
        .any(|text| normalized_tool_text(text) == model_output)
}

fn operation_text_block_text(block: &OperationBlock) -> Option<&str> {
    match block {
        OperationBlock::Text { text } | OperationBlock::Markdown { text } => Some(text.as_str()),
        _ => None,
    }
}

fn normalized_tool_text(text: &str) -> String {
    let sanitized = sanitize_terminal_text(text);
    trim_empty_line_edges(sanitized.as_str()).to_string()
}

fn render_limited_tool_text_block(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    width: u16,
    i18n: &I18n,
) {
    if tool_text_looks_like_markdown(text) {
        push_limited_markdown(out, prefix, text, width, i18n);
    } else {
        push_limited_tool_text(out, prefix, text, Style::default(), width, i18n);
    }
}

fn tool_text_looks_like_markdown(text: &str) -> bool {
    let normalized = normalized_tool_text(text);
    if normalized.is_empty() {
        return false;
    }
    let lines = normalized.lines().collect::<Vec<_>>();
    let unordered_item_count = lines
        .iter()
        .filter(|line| is_markdown_unordered_list_item(line))
        .count();
    let ordered_item_count = lines
        .iter()
        .filter(|line| is_markdown_ordered_list_item(line))
        .count();

    lines.iter().any(|line| {
        let trimmed = line.trim_start();
        markdown_fence_delimiter(trimmed).is_some()
            || trimmed.starts_with("#")
            || trimmed.starts_with("> ")
            || trimmed.starts_with("- [")
            || trimmed.starts_with("* [")
            || trimmed.starts_with("+ [")
    }) || lines
        .windows(2)
        .any(|window| is_markdown_table_header(window[0], window[1]))
        || unordered_item_count >= 2
        || ordered_item_count >= 2
}

fn is_markdown_unordered_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    ["- ", "* ", "+ "]
        .into_iter()
        .any(|prefix| trimmed.starts_with(prefix) && trimmed.len() > prefix.len())
}

fn is_markdown_ordered_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    digit_count > 0
        && trimmed
            .chars()
            .nth(digit_count)
            .is_some_and(|delimiter| delimiter == '.')
        && trimmed
            .chars()
            .nth(digit_count + 1)
            .is_some_and(|separator| separator == ' ')
}

/// Splits a Markdown text part into independently navigable transcript blocks.
///
/// This deliberately follows the renderer's lightweight Markdown recognition
/// rather than trying to implement a second full Markdown parser.  The source
/// remains Markdown so rendering stays consistent, while fenced code copies as
/// plain code (without its fence) and every other block copies its source.
fn markdown_blocks(text: &str) -> Vec<MarkdownBlock> {
    let sanitized = sanitize_terminal_text(text);
    let markdown = trim_empty_line_edges(sanitized.as_str());
    if markdown.is_empty() {
        return Vec::new();
    }

    let lines = markdown.lines().collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let mut index = 0_usize;
    let mut leading_blank_line = false;

    while index < lines.len() {
        if lines[index].trim().is_empty() {
            leading_blank_line = true;
            index += 1;
            continue;
        }

        let start = index;
        let kind;
        let copy_range;

        if let Some(opening_fence) = markdown_fence_delimiter(lines[index]) {
            kind = TranscriptNodeKind::MarkdownCode;
            index += 1;
            let body_start = index;
            while index < lines.len() {
                if markdown_fence_delimiter(lines[index]).is_some_and(|closing_fence| {
                    closing_fence.marker == opening_fence.marker
                        && closing_fence.len >= opening_fence.len
                }) {
                    break;
                }
                index += 1;
            }
            let body_end = index;
            if index < lines.len() {
                index += 1;
            }
            copy_range = body_start..body_end;
        } else if markdown_heading(lines[index]).is_some() {
            kind = TranscriptNodeKind::MarkdownParagraph;
            index += 1;
            copy_range = start..index;
        } else if is_markdown_quote_line(lines[index]) {
            kind = TranscriptNodeKind::MarkdownParagraph;
            index += 1;
            while index < lines.len() && is_markdown_quote_line(lines[index]) {
                index += 1;
            }
            copy_range = start..index;
        } else if is_markdown_thematic_break(lines[index]) {
            kind = TranscriptNodeKind::MarkdownParagraph;
            index += 1;
            copy_range = start..index;
        } else if index + 1 < lines.len()
            && is_markdown_table_header(lines[index], lines[index + 1])
        {
            kind = TranscriptNodeKind::MarkdownTable;
            index += 2;
            while index < lines.len() && looks_like_markdown_table_row(lines[index]) {
                index += 1;
            }
            copy_range = start..index;
        } else if is_markdown_list_item(lines[index]) {
            kind = TranscriptNodeKind::MarkdownList;
            index += 1;
            while index < lines.len() {
                let line = lines[index];
                if line.trim().is_empty() {
                    // A blank line only remains part of the list when it
                    // precedes another item or an indented continuation.
                    let Some(next) = lines.get(index + 1) else {
                        break;
                    };
                    if is_markdown_list_item(next) || is_indented_markdown_line(next) {
                        index += 1;
                        continue;
                    }
                    break;
                }
                if is_markdown_list_item(line) || is_indented_markdown_line(line) {
                    index += 1;
                    continue;
                }
                break;
            }
            copy_range = start..index;
        } else {
            kind = TranscriptNodeKind::MarkdownParagraph;
            index += 1;
            while index < lines.len()
                && !lines[index].trim().is_empty()
                && markdown_fence_delimiter(lines[index]).is_none()
                && markdown_heading(lines[index]).is_none()
                && !is_markdown_quote_line(lines[index])
                && !is_markdown_thematic_break(lines[index])
                && !(index + 1 < lines.len()
                    && is_markdown_table_header(lines[index], lines[index + 1]))
                && !is_markdown_list_item(lines[index])
            {
                index += 1;
            }
            copy_range = start..index;
        }

        let source = lines[start..index].join("\n");
        if source.trim().is_empty() {
            continue;
        }
        let copy_text = lines[copy_range].join("\n");
        blocks.push(MarkdownBlock {
            kind,
            source,
            copy_text,
            leading_blank_line,
        });
        leading_blank_line = false;
    }

    blocks
}

fn is_markdown_list_item(line: &str) -> bool {
    is_markdown_unordered_list_item(line) || is_markdown_ordered_list_item(line)
}

fn is_indented_markdown_line(line: &str) -> bool {
    line.starts_with("  ") || line.starts_with('\t')
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let text = trimmed.get(level..)?.strip_prefix(' ')?.trim();
    let text = text.trim_end_matches('#').trim_end();
    (!text.is_empty()).then_some((level, text))
}

fn is_markdown_quote_line(line: &str) -> bool {
    markdown_quote_depth_and_text(line).is_some()
}

fn markdown_quote_depth_and_text(line: &str) -> Option<(usize, &str)> {
    let mut depth = 0_usize;
    let mut rest = line.trim_start();
    while let Some(after_marker) = rest.strip_prefix('>') {
        depth += 1;
        rest = after_marker.strip_prefix(' ').unwrap_or(after_marker);
    }
    (depth > 0).then_some((depth, rest))
}

fn strip_markdown_quote_level(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn is_markdown_thematic_break(line: &str) -> bool {
    let mut marker = None;
    let mut count = 0_usize;
    for ch in line.chars().filter(|ch| !ch.is_whitespace()) {
        if !matches!(ch, '-' | '*' | '_') || marker.is_some_and(|value| value != ch) {
            return false;
        }
        marker = Some(ch);
        count += 1;
    }
    count >= 3
}

fn render_markdown_block(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    block: &MarkdownBlock,
    width: u16,
) {
    match block.kind {
        TranscriptNodeKind::MarkdownCode => {
            push_markdown_code_block(out, prefix, &block.source, width)
        }
        TranscriptNodeKind::MarkdownList => push_markdown_list(out, prefix, &block.source, width),
        TranscriptNodeKind::MarkdownTable => {
            let table_lines = block.source.lines().collect::<Vec<_>>();
            push_markdown_table(out, prefix, table_lines.as_slice(), width);
        }
        TranscriptNodeKind::MarkdownParagraph => {
            if let Some((level, text)) = markdown_heading(&block.source) {
                push_markdown_heading(out, prefix, level, text, width);
            } else if block.source.lines().all(is_markdown_quote_line) {
                push_markdown_quote(out, prefix, &block.source, width);
            } else if is_markdown_thematic_break(&block.source) {
                push_markdown_rule(out, prefix, width);
            } else {
                push_markdown(out, prefix, &block.source, width);
            }
        }
        TranscriptNodeKind::Message | TranscriptNodeKind::Reasoning | TranscriptNodeKind::Tool => {
            push_markdown(out, prefix, &block.source, width);
        }
    }
}

fn should_suppress_markdown_block(blocks: &[MarkdownBlock], index: usize) -> bool {
    let Some(block) = blocks.get(index) else {
        return false;
    };
    let Some(previous) = index.checked_sub(1).and_then(|index| blocks.get(index)) else {
        return false;
    };
    // A heading already carries its own visual rule.  Markdown examples often
    // place `---` right after every heading, which otherwise produces two
    // adjacent separators with no semantic value in the transcript.
    markdown_heading(previous.source.as_str()).is_some()
        && is_markdown_thematic_break(block.source.as_str())
}

fn push_markdown_heading(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    level: usize,
    text: &str,
    width: u16,
) {
    let marker = match level {
        1 => "══",
        2 => "──",
        _ => "›",
    };
    let style = Style::default()
        .fg(if level <= 2 { Color::Cyan } else { Color::Blue })
        .add_modifier(Modifier::BOLD);
    let available = width.max(1) as usize;
    let start = format!("{prefix}{marker} {text} ");
    if level <= 2 && UnicodeWidthStr::width(start.as_str()) < available {
        let fill = "─".repeat(available.saturating_sub(UnicodeWidthStr::width(start.as_str())));
        out.push(RenderedLine::plain(format!("{start}{fill}"), style));
    } else {
        push_wrapped_line(
            out,
            prefix,
            prefix,
            format!("{marker} {text}").as_str(),
            style,
            width,
        );
    }
}

fn push_markdown_quote(out: &mut Vec<RenderedLine>, prefix: &str, source: &str, width: u16) {
    let inner_source = source
        .lines()
        .filter_map(strip_markdown_quote_level)
        .collect::<Vec<_>>()
        .join("\n");
    let inner_prefix = format!("{prefix}│ ");
    let blocks = markdown_blocks(inner_source.as_str());
    for block in blocks {
        if block.leading_blank_line {
            out.push(RenderedLine::plain(
                inner_prefix.to_string(),
                Style::default().fg(Color::DarkGray),
            ));
        }
        render_markdown_block(out, inner_prefix.as_str(), &block, width);
    }
}

fn push_markdown_rule(out: &mut Vec<RenderedLine>, prefix: &str, width: u16) {
    let available = (width.max(1) as usize).saturating_sub(UnicodeWidthStr::width(prefix));
    push_single_line(
        out,
        prefix,
        "─".repeat(available.clamp(3, 24)).as_str(),
        Style::default().fg(Color::DarkGray),
        width,
    );
}

fn push_markdown_code_block(out: &mut Vec<RenderedLine>, prefix: &str, source: &str, width: u16) {
    let source_lines = source.lines().collect::<Vec<_>>();
    let opening = source_lines.first().copied().unwrap_or_default();
    let language = code_block_language(opening);
    let has_closing_fence = source_lines
        .last()
        .is_some_and(|line| source_lines.len() > 1 && markdown_fence_delimiter(line).is_some());
    let code_lines = if has_closing_fence {
        &source_lines[1..source_lines.len().saturating_sub(1)]
    } else {
        &source_lines[1..]
    };

    let available = width.max(1) as usize;
    let prefix_width = UnicodeWidthStr::width(prefix);
    let card_width = available.saturating_sub(prefix_width);
    if card_width < 16 {
        push_single_line(
            out,
            prefix,
            format!("[{language}]").as_str(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            width,
        );
        for line in code_lines {
            push_single_line(out, prefix, line, Style::default(), width);
        }
        return;
    }

    let label = truncate_display_width(language.as_str(), card_width.saturating_sub(7).max(1));
    let top_start = format!("┌─ {label} ");
    let top_fill = "─".repeat(
        card_width
            .saturating_sub(UnicodeWidthStr::width(top_start.as_str()))
            .saturating_sub(1),
    );
    out.push(RenderedLine::rich(Line::from(vec![
        Span::raw(prefix.to_string()),
        Span::styled("┌─ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            label,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {top_fill}┐"),
            Style::default().fg(Color::DarkGray),
        ),
    ])));

    let line_count_width = code_lines.len().max(1).to_string().len();
    let gutter_width = line_count_width.saturating_add(1);
    let body_width = card_width.saturating_sub(gutter_width).saturating_sub(2);
    for (index, line) in code_lines.iter().enumerate() {
        let number = format!("{:>width$} ", index + 1, width = line_count_width);
        let body = truncate_code_line(line.replace('\t', "    ").as_str(), body_width);
        let padding = " ".repeat(body_width.saturating_sub(UnicodeWidthStr::width(body.as_str())));
        out.push(RenderedLine::rich(Line::from(vec![
            Span::raw(prefix.to_string()),
            Span::styled("│", Style::default().fg(Color::DarkGray)),
            Span::styled(number, Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{body}{padding}"), Style::default()),
            Span::styled("│", Style::default().fg(Color::DarkGray)),
        ])));
    }
    if code_lines.is_empty() {
        out.push(RenderedLine::rich(Line::from(vec![
            Span::raw(prefix.to_string()),
            Span::styled("│", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "  (empty)".to_string() + &" ".repeat(card_width.saturating_sub(11)),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("│", Style::default().fg(Color::DarkGray)),
        ])));
    }
    out.push(RenderedLine::plain(
        format!("{prefix}└{}┘", "─".repeat(card_width.saturating_sub(2))),
        Style::default().fg(Color::DarkGray),
    ));
}

fn code_block_language(opening: &str) -> String {
    let Some(fence) = markdown_fence_delimiter(opening) else {
        return "code".to_string();
    };
    let language = opening
        .trim_start()
        .trim_start_matches(fence.marker)
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("code");
    if language.is_empty() {
        "code".to_string()
    } else {
        language.to_string()
    }
}

fn truncate_code_line(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    format!("{}…", truncate_display_width(text, width.saturating_sub(1)))
}

fn push_markdown_list(out: &mut Vec<RenderedLine>, prefix: &str, source: &str, width: u16) {
    for line in source.lines() {
        if line.trim().is_empty() {
            out.push(RenderedLine::plain(prefix.to_string(), Style::default()));
            continue;
        }
        if let Some((indent, marker, text)) = markdown_list_item_parts(line) {
            let depth = indent / 2;
            let marker = display_list_marker(marker, text, depth);
            let text = markdown_inline_text(display_list_text(text).as_str());
            let list_prefix = format!("{prefix}{}{} ", "  ".repeat(depth), marker);
            let continuation = format!(
                "{prefix}{}{}",
                "  ".repeat(depth),
                " ".repeat(UnicodeWidthStr::width(marker.as_str()) + 1)
            );
            push_wrapped_line(
                out,
                list_prefix.as_str(),
                continuation.as_str(),
                text.as_str(),
                Style::default(),
                width,
            );
        } else {
            let indent = line.len().saturating_sub(line.trim_start().len()) / 2;
            push_wrapped_line(
                out,
                format!("{prefix}{}", "  ".repeat(indent)).as_str(),
                format!("{prefix}{}", "  ".repeat(indent)).as_str(),
                line.trim(),
                Style::default().fg(Color::DarkGray),
                width,
            );
        }
    }
}

fn markdown_list_item_parts(line: &str) -> Option<(usize, &str, &str)> {
    let indent = line.len().saturating_sub(line.trim_start().len());
    let trimmed = line.trim_start();
    for marker in ["-", "*", "+"] {
        if let Some(text) = trimmed
            .strip_prefix(marker)
            .and_then(|rest| rest.strip_prefix(' '))
        {
            return Some((indent, marker, text));
        }
    }
    let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits > 0
        && trimmed
            .get(digits..)
            .is_some_and(|rest| rest.starts_with(". "))
    {
        Some((indent, &trimmed[..digits + 1], &trimmed[digits + 2..]))
    } else {
        None
    }
}

fn display_list_marker(marker: &str, text: &str, depth: usize) -> String {
    if marker.ends_with('.') {
        marker.to_string()
    } else if text.starts_with("[x] ") || text.starts_with("[X] ") {
        "●".to_string()
    } else if text.starts_with("[ ] ") {
        "○".to_string()
    } else {
        ["•", "◦", "▪"][depth.min(2)].to_string()
    }
}

fn display_list_text(text: &str) -> String {
    text.strip_prefix("[ ] ")
        .or_else(|| text.strip_prefix("[x] "))
        .or_else(|| text.strip_prefix("[X] "))
        .unwrap_or(text)
        .to_string()
}

fn markdown_inline_text(text: &str) -> String {
    let rendered = markdown_to_text(text);
    let plain = rendered
        .lines
        .iter()
        .map(|line| line_plain_text(&owned_line(line)))
        .collect::<Vec<_>>()
        .join(" ");
    sanitize_terminal_text(plain.as_str()).trim().to_string()
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

    // Three cells need two inner separators plus the two outer borders.
    let separator_width = column_count
        .saturating_sub(1)
        .saturating_mul(3)
        .saturating_add(2);
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

    push_table_border(
        out,
        prefix,
        column_widths.as_slice(),
        "┌",
        "┬",
        "┐",
        separator_style,
    );
    render_table_row(
        out,
        prefix,
        rows[0].as_slice(),
        column_widths.as_slice(),
        alignments.as_slice(),
        header_style,
    );
    push_table_border(
        out,
        prefix,
        column_widths.as_slice(),
        "├",
        "┼",
        "┤",
        separator_style,
    );
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
    push_table_border(
        out,
        prefix,
        column_widths.as_slice(),
        "└",
        "┴",
        "┘",
        separator_style,
    );
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
        let mut spans = vec![
            Span::raw(prefix.to_string()),
            Span::styled("│", Style::default().fg(Color::DarkGray)),
        ];
        for (column_index, width) in widths.iter().enumerate() {
            if column_index > 0 {
                spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
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
        spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
        out.push(RenderedLine::rich(Line::from(spans)));
    }
}

fn push_table_border(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    widths: &[usize],
    left: &str,
    middle: &str,
    right: &str,
    style: Style,
) {
    let mut spans = vec![
        Span::raw(prefix.to_string()),
        Span::styled(left.to_string(), style),
    ];
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(format!("─{middle}─"), style));
        }
        spans.push(Span::styled("─".repeat(*width), style));
    }
    spans.push(Span::styled(right.to_string(), style));
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

#[derive(Debug, Clone, Default)]
struct ApplyPatchDisplay {
    changes: Vec<agena::message::FileChangeRecord>,
    diff: String,
}

#[derive(Debug, Clone, Copy, Default)]
struct DiffStats {
    file_count: usize,
    additions: usize,
    deletions: usize,
    renames: usize,
    line_count: usize,
}

fn apply_patch_details(details: &agena::message::ToolOutput) -> Option<ApplyPatchDisplay> {
    let changes: Vec<agena::message::FileChangeRecord> = details
        .payload
        .get("changes")
        .cloned()
        .and_then(|value| serde_json::from_value(serde_json::Value::from(value)).ok())
        .unwrap_or_default();
    let diff = details
        .payload
        .get("diff")
        .and_then(agena::message::StructuredValue::as_text)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();

    if details.payload.get("operation_id").is_none() && changes.is_empty() && diff.is_empty() {
        return None;
    }

    Some(ApplyPatchDisplay { changes, diff })
}

fn diff_stats(diff: &str, changes: Option<&[agena::message::FileChangeRecord]>) -> DiffStats {
    let mut file_count = diff
        .lines()
        .filter(|line| line.starts_with("diff --git "))
        .count();
    let line_count = diff.lines().count();
    let mut additions = 0usize;
    let mut deletions = 0usize;
    for line in diff.lines() {
        if line.starts_with("+++ ") || line.starts_with("--- ") {
            continue;
        }
        if line.starts_with('+') {
            additions += 1;
        } else if line.starts_with('-') {
            deletions += 1;
        }
    }
    let renames = if let Some(changes) = changes {
        file_count = file_count.max(changes.len());
        changes
            .iter()
            .filter(|change| change.kind == FileChangeKind::Moved)
            .count()
    } else {
        0
    };
    DiffStats {
        file_count,
        additions,
        deletions,
        renames,
        line_count,
    }
}

fn summarize_change_paths(
    i18n: &I18n,
    changes: &[agena::message::FileChangeRecord],
    preview_limit: usize,
) -> String {
    let mut preview = changes
        .iter()
        .take(preview_limit)
        .map(file_change_display_path)
        .collect::<Vec<_>>();
    let omitted = changes.len().saturating_sub(preview.len());
    if omitted > 0 {
        preview.push(ui_text::file_changes_more(i18n, omitted));
    }
    preview.join(", ")
}

fn file_change_display_path(change: &agena::message::FileChangeRecord) -> String {
    if change.kind == FileChangeKind::Moved {
        change
            .from_path
            .as_ref()
            .map(|from_path| format!("{from_path} -> {}", change.path))
            .unwrap_or_else(|| change.path.clone())
    } else {
        change.path.clone()
    }
}

fn file_change_marker(kind: FileChangeKind) -> &'static str {
    match kind {
        FileChangeKind::Added => "A",
        FileChangeKind::Updated => "M",
        FileChangeKind::Deleted => "D",
        FileChangeKind::Moved => "R",
    }
}

fn file_change_style(kind: FileChangeKind) -> Style {
    match kind {
        FileChangeKind::Added => Style::default().fg(Color::Green),
        FileChangeKind::Updated => Style::default().fg(Color::Yellow),
        FileChangeKind::Deleted => Style::default().fg(Color::Red),
        FileChangeKind::Moved => Style::default().fg(Color::Cyan),
    }
}

fn file_change_list_item_text(change: &agena::message::FileChangeRecord, i18n: &I18n) -> String {
    format!(
        "{} {} ({})",
        file_change_marker(change.kind),
        file_change_display_path(change),
        ui_text::file_change_kind_label(i18n, change.kind)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_blocks_make_code_lists_and_tables_independently_selectable() {
        let blocks = markdown_blocks(
            "Introduction.\n\n```rust\nlet answer = 42;\n```\n\n- first\n- second\n\n| name | value |\n| --- | ---: |\n| answer | 42 |\n\nConclusion.",
        );

        assert_eq!(blocks.len(), 5);
        assert_eq!(blocks[0].kind, TranscriptNodeKind::MarkdownParagraph);
        assert_eq!(blocks[1].kind, TranscriptNodeKind::MarkdownCode);
        assert_eq!(blocks[1].source, "```rust\nlet answer = 42;\n```");
        assert_eq!(blocks[1].copy_text, "let answer = 42;");
        assert_eq!(blocks[2].kind, TranscriptNodeKind::MarkdownList);
        assert_eq!(blocks[2].copy_text, "- first\n- second");
        assert_eq!(blocks[3].kind, TranscriptNodeKind::MarkdownTable);
        assert_eq!(
            blocks[3].copy_text,
            "| name | value |\n| --- | ---: |\n| answer | 42 |"
        );
        assert_eq!(blocks[4].kind, TranscriptNodeKind::MarkdownParagraph);
    }

    #[test]
    fn markdown_blocks_keep_multiline_list_items_together() {
        let blocks = markdown_blocks(
            "- first item\n  continuation\n\n  still the first item\n- second item\n\nAfter the list.",
        );

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, TranscriptNodeKind::MarkdownList);
        assert_eq!(
            blocks[0].copy_text,
            "- first item\n  continuation\n\n  still the first item\n- second item"
        );
        assert_eq!(blocks[1].kind, TranscriptNodeKind::MarkdownParagraph);
    }

    #[test]
    fn code_blocks_render_as_bounded_numbered_cards_without_wrapping_code() {
        let block = markdown_blocks("```rust\nlet a_very_long_identifier = 42;\n```")
            .pop()
            .expect("code block");
        let mut lines = Vec::new();
        render_markdown_block(&mut lines, "  ", &block, 28);

        assert!(
            lines
                .first()
                .is_some_and(|line| line.text.contains("┌─ rust"))
        );
        assert!(lines.get(1).is_some_and(|line| line.text.contains("1 ")));
        assert!(lines.get(1).is_some_and(|line| line.text.contains('…')));
        assert!(lines.last().is_some_and(|line| line.text.ends_with('┘')));
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= 28)
        );
    }

    #[test]
    fn headings_quotes_lists_and_tables_have_distinct_terminal_chrome() {
        let mut heading = Vec::new();
        let heading_block = markdown_blocks("# Overview").pop().expect("heading block");
        render_markdown_block(&mut heading, "  ", &heading_block, 36);
        assert!(heading[0].text.contains("══ Overview"));

        let mut quote = Vec::new();
        let quote_block = markdown_blocks("> quoted context\n> remains visually distinct")
            .pop()
            .expect("quote block");
        render_markdown_block(&mut quote, "  ", &quote_block, 36);
        assert!(quote.iter().all(|line| line.text.starts_with("  │ ")));

        let mut list = Vec::new();
        let list_block = markdown_blocks("- [ ] pending\n  - nested\n- [x] complete")
            .pop()
            .expect("list block");
        render_markdown_block(&mut list, "  ", &list_block, 36);
        assert!(list.iter().any(|line| line.text.contains("○ pending")));
        assert!(list.iter().any(|line| line.text.contains("◦ nested")));
        assert!(list.iter().any(|line| line.text.contains("● complete")));

        let mut table = Vec::new();
        let table_block = markdown_blocks("| key | value |\n| --- | ---: |\n| answer | 42 |")
            .pop()
            .expect("table block");
        render_markdown_block(&mut table, "  ", &table_block, 36);
        assert!(table.first().is_some_and(|line| line.text.contains('┌')));
        assert!(
            table
                .iter()
                .any(|line| line.text.contains('│') && line.text.contains("key"))
        );
        assert!(table.last().is_some_and(|line| line.text.contains('└')));
    }

    #[test]
    fn quote_blocks_preserve_inline_markdown_and_render_each_nesting_level() {
        let block = markdown_blocks(
            "> **保持简单，保持愚蠢。**  \n> —— Unix 哲学\n>\n> > 嵌套引用\n> > > 三层嵌套",
        )
        .pop()
        .expect("quote block");
        let mut lines = Vec::new();
        render_markdown_block(&mut lines, "  ", &block, 52);

        assert!(
            lines
                .iter()
                .any(|line| line.text.contains("保持简单，保持愚蠢。"))
        );
        assert!(lines.iter().all(|line| !line.text.contains("**")));
        assert!(
            lines
                .iter()
                .any(|line| line.text.starts_with("  │ │ ") && line.text.contains("嵌套引用"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.text.starts_with("  │ │ │ ") && line.text.contains("三层嵌套"))
        );
    }

    #[test]
    fn thematic_rule_immediately_after_heading_is_suppressed() {
        let blocks = markdown_blocks("## 💬 引用\n\n---\n\n> 内容");

        assert_eq!(blocks.len(), 3);
        assert!(!should_suppress_markdown_block(blocks.as_slice(), 0));
        assert!(should_suppress_markdown_block(blocks.as_slice(), 1));
        assert!(!should_suppress_markdown_block(blocks.as_slice(), 2));
    }
}

fn push_limited_diff_text(
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
            Style::default().fg(Color::DarkGray),
            width,
        );
    }
}

fn diff_line_style(line: &str) -> Style {
    if line.starts_with("diff --git ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
        || line.starts_with("new file mode ")
        || line.starts_with("deleted file mode ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
    {
        Style::default().fg(Color::Cyan)
    } else if line.starts_with("@@") {
        Style::default().fg(Color::Yellow)
    } else if line.starts_with('+') {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::DarkGray)
    }
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
