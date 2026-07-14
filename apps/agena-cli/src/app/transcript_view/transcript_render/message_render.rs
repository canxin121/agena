use super::super::transcript_ast::{MarkdownNode, render_attachment_image};
use super::super::{
    ExecutionStatus, I18n, Local, MessagePart, MessageResource, MessageStatus, Modifier,
    PartContent, RenderedLine, RenderedTranscriptNode, RequestPart, SessionExecutionResource,
    Style, TOOL_CARD_PREVIEW_CHARS, TOOL_CARD_PREVIEW_LINES, ToolOutputPreview,
    TranscriptDetailDefaults, TranscriptNodeKey, TranscriptNodeKind, UnicodeWidthStr,
    activity_status_icon, concise_text, format_timestamp, push_label_value, push_markdown,
    push_multiline, push_section_heading, push_single_line, push_wrapped_line,
    render_message_detailed, strip_terminal_ansi_sequences, style_for_role, tool_output_copy_text,
    transcript_message_parts, transcript_part_content, transcript_spinner_placeholder,
    trim_empty_line_edges, truncate_display_width, ui_text,
};
use super::operation_render::render_tool_execution;
use super::request_render::{
    preview_for_part, render_permission_request, render_user_input_request,
};

/// Export and pager output is a document, not an infinitely wide terminal.
/// Keeping this width bounded prevents visual rules and code-card borders from
/// expanding to `u16::MAX`-sized lines while remaining comfortable to read in
/// both terminal pagers and text editors.
pub(in crate::app) const TRANSCRIPT_EXPORT_WIDTH: u16 = 120;

pub(in crate::app) fn interactive_request_is_embedded_in_operation(
    parts: &[MessagePart],
    index: usize,
) -> bool {
    let Some(request_part) = parts.get(index) else {
        return false;
    };
    let Some(operation_id) = request_part.operation_id.as_deref() else {
        return false;
    };
    matches!(
        transcript_part_content(request_part),
        PartContent::Request(RequestPart::Permission(_))
            | PartContent::Request(RequestPart::UserInput(_))
    ) && parts
        .iter()
        .enumerate()
        .any(|(candidate_index, candidate)| {
            candidate_index != index
                && candidate.operation_id.as_deref() == Some(operation_id)
                && matches!(
                    transcript_part_content(candidate),
                    PartContent::Operation(_)
                )
        })
}

pub(in crate::app) fn render_message_export(
    message: &MessageResource,
    i18n: &I18n,
    defaults: TranscriptDetailDefaults,
) -> Vec<RenderedLine> {
    crate::math_render::with_text_math_rendering(|| {
        render_message_detailed(
            message,
            TRANSCRIPT_EXPORT_WIDTH,
            i18n,
            defaults,
            &std::collections::BTreeMap::new(),
        )
        .lines
    })
}

#[derive(Debug, Clone)]
pub(in crate::app) struct RenderedMessageBlock {
    pub lines: Vec<RenderedLine>,
    pub nodes: Vec<RenderedTranscriptNode>,
}

#[derive(Debug, Clone)]
pub(in crate::app) struct RenderedNodeDraft {
    key: TranscriptNodeKey,
    kind: TranscriptNodeKind,
    copy_text: String,
    toggleable: bool,
    expanded: bool,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn append_rendered_part_node(
    message: &MessageResource,
    part: &MessagePart,
    width: u16,
    lines: &mut Vec<RenderedLine>,
    nodes: &mut Vec<RenderedTranscriptNode>,
    i18n: &I18n,
    defaults: TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
) {
    // Like Markdown blocks, non-text parts start after the message header so
    // selecting the first activity part never highlights `assistant`.
    let start_line = lines.len();
    let node = render_part_node(message, part, width, lines, i18n, defaults, expansions);
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
    }
}

pub(in crate::app) fn collapsed_activity_run_end(
    parts: &[MessagePart],
    start: usize,
) -> Option<usize> {
    is_activity_part(parts.get(start)?).then(|| {
        let mut end = start.saturating_add(1);
        while parts.get(end).is_some_and(|part| {
            is_activity_part(part) || is_invisible_activity_run_bridge(parts, end)
        }) {
            end = end.saturating_add(1);
        }
        end
    })
}

pub(in crate::app) const COLLAPSED_ACTIVITY_VISIBLE_COUNT: usize = 5;

fn is_invisible_activity_run_bridge(parts: &[MessagePart], index: usize) -> bool {
    interactive_request_is_embedded_in_operation(parts, index)
        || matches!(
            parts.get(index).map(transcript_part_content),
            Some(PartContent::Text(text)) if text.ignored || text.text.trim().is_empty()
        )
}

pub(in crate::app) fn is_activity_part(part: &MessagePart) -> bool {
    matches!(
        transcript_part_content(part),
        PartContent::Reasoning(_) | PartContent::Operation(_)
    )
}

pub(in crate::app) fn activity_part_copy_text(part: &MessagePart, i18n: &I18n) -> Option<String> {
    match transcript_part_content(part) {
        PartContent::Reasoning(reasoning) => Some(reasoning.preferred_text()),
        PartContent::Operation(tool) => Some(tool_output_copy_text(part, tool, i18n)),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct MarkdownBlock {
    pub(in crate::app) kind: TranscriptNodeKind,
    pub(in crate::app) source: String,
    pub(in crate::app) copy_text: String,
    pub(in crate::app) leading_blank_line: bool,
    pub(in crate::app) parsed: MarkdownNode,
}

pub(in crate::app) fn rewind_message_preview(message: &MessageResource, i18n: &I18n) -> String {
    let preview = transcript_message_parts(message)
        .iter()
        .find_map(|part| preview_for_part(part, i18n))
        .unwrap_or_else(|| ui_text::t(i18n, "message-empty"));
    truncate_display_width(preview.as_str(), 64)
}

pub(in crate::app) fn render_transcript_export_markdown(
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

    let title = truncate_display_width(
        title.as_str(),
        usize::from(TRANSCRIPT_EXPORT_WIDTH).saturating_sub(2),
    );
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
                i18n,
                TranscriptDetailDefaults {
                    activity_expanded: true,
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

pub(in crate::app) fn tool_output_preview(text: &str) -> ToolOutputPreview {
    tool_output_preview_with_limits(text, TOOL_CARD_PREVIEW_LINES, TOOL_CARD_PREVIEW_CHARS)
}

pub(in crate::app) fn tool_output_preview_with_limits(
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

pub(in crate::app) fn sanitize_terminal_text(text: &str) -> String {
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

pub(in crate::app) fn push_message_header(
    out: &mut Vec<RenderedLine>,
    message: &MessageResource,
    width: u16,
    i18n: &I18n,
) {
    let role = ui_text::role_label(i18n, message.role);
    let header = match message.state {
        MessageStatus::Completed => role,
        MessageStatus::Pending => format!("{role} ○"),
        MessageStatus::InProgress => format!("{role} {}", transcript_spinner_placeholder()),
        MessageStatus::Failed => format!("{role} ×"),
        MessageStatus::Cancelled => format!("{role} –"),
    };
    let header_style = style_for_role(message.role).add_modifier(Modifier::BOLD);

    if UnicodeWidthStr::width(header.as_str()) <= width.max(1) as usize {
        out.push(RenderedLine::plain(header, header_style));
    } else {
        push_wrapped_line(out, "", "", header.as_str(), header_style, width);
    }
}

pub(in crate::app) fn render_part_node(
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
            let key = TranscriptNodeKey::ActivityPart {
                message_id: message.id,
                part_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.activity_expanded);
            let summary = reasoning.preferred_text();
            if expanded {
                push_section_heading(
                    out,
                    "  thinking",
                    Style::default()
                        .fg(agena_tui_components::theme::muted_color())
                        .add_modifier(Modifier::BOLD),
                    width,
                );
                push_multiline(
                    out,
                    "    ",
                    summary.as_str(),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    width,
                );
            } else {
                push_single_line(
                    out,
                    "  ",
                    thinking_collapsed_summary(part.status, summary.as_str()).as_str(),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    width,
                );
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: summary,
                toggleable: true,
                expanded,
            }
        }
        PartContent::Operation(tool) => {
            let key = TranscriptNodeKey::ActivityPart {
                message_id: message.id,
                part_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.activity_expanded);
            render_tool_execution(part, tool, out, width, i18n, expanded);
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
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
            push_multiline(
                out,
                "  ",
                &text,
                Style::default().fg(agena_tui_components::theme::danger_color()),
                width,
            );
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
                    .fg(agena_tui_components::theme::special_color())
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
                if !render_attachment_image(out, "    ", item, width) {
                    push_label_value(out, "    - ", label.as_str(), Style::default(), width);
                }
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

pub(in crate::app) fn thinking_collapsed_summary(status: ExecutionStatus, text: &str) -> String {
    let normalized = trim_empty_line_edges(sanitize_terminal_text(text).as_str());
    let preview = normalized
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let additional_content = normalized
        .lines()
        .skip_while(|line| line.trim().is_empty())
        .skip(1)
        .any(|line| !line.trim().is_empty());
    let suffix = if additional_content { " …" } else { "" };
    format!(
        "{} thinking · {}{suffix}",
        activity_status_icon(status),
        concise_text(preview, 112)
    )
}
