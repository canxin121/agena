use super::super::transcript_ast::{MarkdownNode, render_attachment_image};
use super::super::{
    I18n, Local, MessageStatus, Modifier, RenderedLine, RenderedTranscriptNode,
    SessionExecutionResource, Style, TOOL_CARD_PREVIEW_CHARS, TOOL_CARD_PREVIEW_LINES,
    ToolOutputPreview, TranscriptDetailDefaults, TranscriptEntry, TranscriptNodeKey,
    TranscriptNodeKind, UnicodeWidthStr, activity_status_icon, concise_text, format_timestamp,
    push_label_value, push_markdown, push_multiline, push_section_heading, push_single_line,
    push_wrapped_line, render_entry_detailed, strip_terminal_ansi_sequences, style_for_role,
    tool_output_copy_text, transcript_message_parts, transcript_part_content,
    transcript_spinner_placeholder, trim_empty_line_edges, truncate_display_width,
};
use super::operation_render::render_tool_execution;
use super::request_render::{preview_for_part, render_user_input_request};
use crate::snapshot::activity_presentation;
use crate::ui_text;
use crate::{
    MessageRequestPartResource, PartExecutionStatusResource, TranscriptActivityContent,
    TranscriptAssistantReplyLifecycle, TranscriptEntryPart, TranscriptPartContent,
};
use agena_api::resource::{
    MessageAttachment, MessageAttachmentKind, MessageAttachmentSource, MessageResource,
};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

/// Export and pager output is a document, not an infinitely wide terminal.
/// Keeping this width bounded prevents visual rules and code-card borders from
/// expanding to `u16::MAX`-sized lines while remaining comfortable to read in
/// both terminal pagers and text editors.
pub(crate) const TRANSCRIPT_EXPORT_WIDTH: u16 = 120;

pub fn render_entry_export(
    message: &TranscriptEntry,
    i18n: &I18n,
    defaults: TranscriptDetailDefaults,
) -> Vec<RenderedLine> {
    agena_tui_media::with_text_math_rendering(|| {
        render_entry_detailed(
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
pub struct RenderedMessageBlock {
    pub lines: Vec<RenderedLine>,
    pub nodes: Vec<RenderedTranscriptNode>,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderedNodeDraft {
    key: TranscriptNodeKey,
    kind: TranscriptNodeKind,
    copy_text: String,
    toggleable: bool,
    expanded: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn append_rendered_part_node(
    message: &TranscriptEntry,
    part: &TranscriptEntryPart,
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
        let atomic = node.kind.uses_atomic_navigation()
            || lines[start_line..].iter().any(|line| !line.math.is_empty());
        nodes.push(RenderedTranscriptNode {
            key: node.key,
            kind: node.kind,
            start_line,
            end_line: lines.len(),
            copy_text: node.copy_text,
            atomic,
            toggleable: node.toggleable,
            expanded: node.expanded,
        });
    }
}

pub(crate) fn collapsed_activity_run_end(
    parts: &[TranscriptEntryPart],
    start: usize,
) -> Option<usize> {
    is_activity_node(parts.get(start)?).then(|| {
        let mut end = start.saturating_add(1);
        while parts.get(end).is_some_and(|part| {
            is_activity_node(part) || is_invisible_activity_run_bridge(parts, end)
        }) {
            end = end.saturating_add(1);
        }
        end
    })
}

pub(crate) const COLLAPSED_ACTIVITY_VISIBLE_COUNT: usize = 5;

fn is_invisible_activity_run_bridge(parts: &[TranscriptEntryPart], index: usize) -> bool {
    matches!(
        parts.get(index).map(transcript_part_content),
        Some(TranscriptPartContent::Text(text)) if text.text.trim().is_empty()
    )
}

pub(crate) fn is_activity_node(part: &TranscriptEntryPart) -> bool {
    matches!(
        transcript_part_content(part),
        TranscriptPartContent::Activity(_)
    )
}

fn activity_headline(
    title: &str,
    status: PartExecutionStatusResource,
    expanded: bool,
    toggleable: bool,
) -> String {
    let disclosure = if toggleable && expanded { "▾" } else { "▸" };
    format!("{disclosure} {} {title}", activity_status_icon(status))
}

fn canonical_activity_details(
    payload: &agena_domain::ActivityPayload,
    summary: &str,
) -> Vec<String> {
    match payload {
        agena_domain::ActivityPayload::Operation(operation) => {
            let mut details = Vec::new();
            if !operation.authorization.permissions.is_empty() {
                let permissions = operation
                    .authorization
                    .permissions
                    .iter()
                    .map(|permission| {
                        let status = match permission.reply.as_ref().map(|reply| reply.kind) {
                            None => "Awaiting user approval",
                            Some(agena_domain::PermissionReplyKind::AllowOnce) => "Allowed once",
                            Some(agena_domain::PermissionReplyKind::AllowAlways) => {
                                "Allowed persistently"
                            }
                            Some(agena_domain::PermissionReplyKind::DenyOnce) => "Denied once",
                            Some(agena_domain::PermissionReplyKind::DenyAlways) => {
                                "Denied persistently"
                            }
                        };
                        let action = match &permission.request.action {
                            agena_domain::PermissionAction::Tool {
                                tool_name,
                                qualifier,
                            } => qualifier.as_deref().map_or_else(
                                || tool_name.clone(),
                                |qualifier| format!("{tool_name} · {qualifier}"),
                            ),
                            agena_domain::PermissionAction::PathAccess {
                                access_kind,
                                target_path,
                                ..
                            } => format!("{access_kind} {target_path}"),
                            agena_domain::PermissionAction::NetworkAccess { target, .. } => {
                                format!("network {target}")
                            }
                        };
                        let mut lines = vec![format!("{status} · {action}")];
                        if !permission.request.reason.trim().is_empty() {
                            lines.push(format!("Request: {}", permission.request.reason));
                        }
                        if !permission.request.explanation.trim().is_empty()
                            && permission.request.explanation.trim()
                                != permission.request.reason.trim()
                        {
                            lines.push(format!("Policy: {}", permission.request.explanation));
                        }
                        if let Some(reason) = permission
                            .reply
                            .as_ref()
                            .and_then(|reply| reply.reason.as_deref())
                            .filter(|reason| !reason.trim().is_empty())
                            && reason.trim() != permission.request.reason.trim()
                        {
                            lines.push(format!("Reply: {reason}"));
                        }
                        let provenance = [
                            permission.request.source.clone(),
                            permission.request.scope.map(|scope| format!("{scope}")),
                            Some(format!(
                                "{} risk",
                                format!("{:?}", permission.request.risk).to_ascii_lowercase()
                            )),
                        ]
                        .into_iter()
                        .flatten()
                        .filter(|value| !value.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                        if !provenance.is_empty() {
                            lines.push(provenance);
                        }
                        lines.join("\n")
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                details.push(format!("Authorization\n{permissions}"));
            }
            if !operation.invocation.input.is_empty()
                && let Ok(input) = serde_json::to_string_pretty(&serde_json::Value::from(
                    operation.invocation.input.clone(),
                ))
            {
                details.push(format!("Input\n{input}"));
            }
            if !operation.model_output_text.trim().is_empty()
                && operation.model_output_text.trim() != summary.trim()
            {
                details.push(format!("Result\n{}", operation.model_output_text));
            }
            // Named sections are the tool's explicit expanded presentation.
            // Keep them structured through the domain projection and render
            // each one exactly once, rather than reconstructing them from a
            // raw JSON payload or a duplicate Markdown block.
            for section in &operation.sections {
                let title = section.title.trim();
                let text = section.text.trim();
                if title.is_empty()
                    || text.is_empty()
                    || text == summary.trim()
                    || text == operation.model_output_text.trim()
                {
                    continue;
                }
                let detail = format!("{title}\n{text}");
                if !details.iter().any(|existing| existing == &detail) {
                    details.push(detail);
                }
            }
            // The model-visible result is the primary human-facing section.
            // Retain raw structured data only if there is neither a primary
            // result nor an explicit named section. Otherwise tools_list,
            // search and plugin tools show the same payload in multiple forms.
            if operation.model_output_text.trim().is_empty()
                && operation.sections.is_empty()
                && let Some(output) = operation.details.to_json_payload()
                && let Ok(output) = serde_json::to_string_pretty(&output)
            {
                details.push(format!("Structured result\n{output}"));
            }
            if !operation.details.managed_outputs.is_empty() {
                details.push(format!(
                    "Managed outputs\n{}",
                    operation
                        .details
                        .managed_outputs
                        .iter()
                        .map(|output| output.path.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
            details
        }
        agena_domain::ActivityPayload::SkillReference(skill) => [
            (!skill.instructions.trim().is_empty()).then(|| skill.instructions.clone()),
            Some(format!("{} · {}", skill.source, skill.content_hash)),
        ]
        .into_iter()
        .flatten()
        .collect(),
        agena_domain::ActivityPayload::SkillExecution(skill) => vec![format!(
            "execution {}{}",
            skill.execution_id,
            skill
                .parent_activity_id
                .map(|id| format!(" · parent {id}"))
                .unwrap_or_default()
        )],
        agena_domain::ActivityPayload::Progress(progress) => {
            match (progress.current, progress.total) {
                (Some(current), Some(total)) => vec![format!("{current}/{total}")],
                (Some(current), None) => vec![current.to_string()],
                (None, Some(total)) => vec![format!("total {total}")],
                (None, None) => Vec::new(),
            }
        }
        agena_domain::ActivityPayload::Checklist(checklist) => checklist
            .items
            .iter()
            .map(|item| format!("{:?} · {:?} · {}", item.status, item.priority, item.content))
            .collect(),
        agena_domain::ActivityPayload::Search(search) => search
            .results
            .iter()
            .filter_map(|result| serde_json::to_string_pretty(result).ok())
            .collect(),
        agena_domain::ActivityPayload::FileChanges(changes) => changes
            .changes
            .iter()
            .filter_map(|change| serde_json::to_string_pretty(change).ok())
            .collect(),
        agena_domain::ActivityPayload::NestedTask(task) => vec![format!(
            "task {}{}",
            task.task_id,
            task.session_id
                .map(|id| format!(" · session {id}"))
                .unwrap_or_default()
        )],
        agena_domain::ActivityPayload::Maintenance(maintenance) => {
            serde_json::to_string_pretty(maintenance)
                .ok()
                .into_iter()
                .collect()
        }
        agena_domain::ActivityPayload::Error(error) => {
            let fallback = error.problem.user.fallback.clone();
            if !fallback.is_empty() {
                vec![fallback]
            } else {
                Vec::new()
            }
        }
        agena_domain::ActivityPayload::Custom(custom) => {
            let mut details = vec![format!("schema version {}", custom.schema_version)];
            if let Ok(data) = serde_json::to_string_pretty(&custom.data) {
                details.push(data);
            }
            details
        }
        agena_domain::ActivityPayload::Resource(resource) => {
            let mut details = Vec::new();
            if let Some(media_type) = resource.media_type.as_ref() {
                details.push(media_type.clone());
            }
            if let Some(size) = resource.size_bytes {
                details.push(format!("{size} bytes"));
            }
            if let (Some(width), Some(height)) = (resource.width, resource.height) {
                details.push(format!("{width}×{height}"));
            }
            details
        }
        agena_domain::ActivityPayload::TextArtifact(artifact) => artifact
            .language
            .as_ref()
            .map(|language| format!("language {language}"))
            .into_iter()
            .collect(),
        agena_domain::ActivityPayload::Interaction(interaction) => {
            serde_json::to_string_pretty(interaction)
                .ok()
                .into_iter()
                .collect()
        }
        agena_domain::ActivityPayload::Reasoning(_) => Vec::new(),
    }
}

fn canonical_resource_attachment(resource: &agena_domain::ResourceActivity) -> MessageAttachment {
    let kind = match resource.kind {
        agena_domain::ResourceKind::Image => MessageAttachmentKind::Image,
        agena_domain::ResourceKind::Audio => MessageAttachmentKind::Audio,
        agena_domain::ResourceKind::Video => MessageAttachmentKind::Video,
        agena_domain::ResourceKind::Pdf => MessageAttachmentKind::Pdf,
        agena_domain::ResourceKind::File
        | agena_domain::ResourceKind::Directory
        | agena_domain::ResourceKind::Url
        | agena_domain::ResourceKind::Artifact => MessageAttachmentKind::File,
    };
    let (source, sha256) = match &resource.reference {
        agena_domain::ResourceReference::Artifact { sha256, uri } => (
            MessageAttachmentSource::FileId {
                file_id: uri.clone(),
            },
            Some(sha256.clone()),
        ),
        agena_domain::ResourceReference::WorkspacePath { path } => (
            MessageAttachmentSource::LocalPath { path: path.clone() },
            None,
        ),
        agena_domain::ResourceReference::Url { url } => {
            (MessageAttachmentSource::Url { url: url.clone() }, None)
        }
        agena_domain::ResourceReference::ProviderFile { file_id, .. } => (
            MessageAttachmentSource::FileId {
                file_id: file_id.clone(),
            },
            None,
        ),
    };
    MessageAttachment {
        kind,
        mime: resource.media_type.clone().unwrap_or_default(),
        source,
        filename: Some(resource.name.clone()),
        title: None,
        size_bytes: resource.size_bytes,
        sha256,
        width: resource.width,
        height: resource.height,
        duration_ms: resource.duration_ms,
        page_count: resource.page_count,
    }
}

pub(crate) fn activity_copy_text(part: &TranscriptEntryPart, i18n: &I18n) -> Option<String> {
    match transcript_part_content(part) {
        TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload)) => {
            let (_, title, summary, error) = activity_presentation(payload);
            let error_text = error.as_ref().map(|e| {
                if e.is_unexpected() {
                    format!("{} Reference: {}", e.user.fallback, e.id)
                } else {
                    e.user.fallback.clone()
                }
            });
            let details = canonical_activity_details(payload, summary.as_str());
            Some(
                [
                    Some(title),
                    (!summary.is_empty()).then_some(summary),
                    (!details.is_empty()).then_some(details.join("\n")),
                    error_text,
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n"),
            )
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(reasoning)) => {
            Some(reasoning.preferred_text())
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Operation(tool)) => {
            Some(tool_output_copy_text(part, tool, i18n))
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Attachment(attachment)) => Some(
            attachment
                .attachments
                .iter()
                .map(|item| {
                    item.title
                        .as_ref()
                        .or(item.filename.as_ref())
                        .cloned()
                        .unwrap_or_else(|| item.mime.clone())
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        TranscriptPartContent::Activity(TranscriptActivityContent::SkillReference(reference)) => {
            Some(
                reference
                    .skills
                    .iter()
                    .map(|skill| format!("Skill: {}\n{}", skill.name, skill.instructions))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            )
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Error(error)) => {
            let text = if error.problem.is_unexpected() {
                format!(
                    "{} Reference: {}",
                    error.problem.user.fallback, error.problem.id
                )
            } else {
                error.problem.user.fallback.clone()
            };
            Some(text)
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::AssistantReplyLifecycle(
            status,
        )) => Some(ui_text::t(
            i18n,
            match status {
                TranscriptAssistantReplyLifecycle::Running => "message-activity-response-running",
                TranscriptAssistantReplyLifecycle::Completed => {
                    "message-activity-response-completed"
                }
                TranscriptAssistantReplyLifecycle::Failed => "message-activity-response-failed",
                TranscriptAssistantReplyLifecycle::Cancelled => {
                    "message-activity-response-cancelled"
                }
            },
        )),
        TranscriptPartContent::Activity(TranscriptActivityContent::Request(request)) => {
            Some(match request.as_ref() {
                MessageRequestPartResource::UserInput { request, .. } => request
                    .questions
                    .iter()
                    .map(|question| question.question.clone())
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownBlock {
    pub kind: TranscriptNodeKind,
    pub source: String,
    pub copy_text: String,
    pub leading_blank_line: bool,
    pub parsed: MarkdownNode,
}

fn entry_preview(message: &TranscriptEntry, i18n: &I18n) -> String {
    let preview = transcript_message_parts(message)
        .iter()
        .find_map(|part| preview_for_part(part, i18n))
        .unwrap_or_else(|| ui_text::t(i18n, "message-empty"));
    truncate_display_width(preview.as_str(), 64)
}

pub(crate) fn render_transcript_entries_export_markdown(
    i18n: &I18n,
    session_id: Option<i64>,
    session_title: &str,
    execution: Option<&SessionExecutionResource>,
    messages: &[TranscriptEntry],
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
        if let Some(role) = message.role {
            out.push(format!(
                "## {} · {} · {}",
                ui_text::role_label(i18n, role),
                ui_text::message_state_label(i18n, message.state),
                timestamp,
            ));
        } else {
            out.push(format!(
                "## {} · {}",
                ui_text::t(i18n, "transcript-node-kind-activity"),
                timestamp,
            ));
        }
        out.push(String::new());
        out.push("~~~~text".to_string());
        out.extend(
            render_entry_export(
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

pub fn rewind_message_preview(message: &MessageResource, i18n: &I18n) -> String {
    entry_preview(&TranscriptEntry::from(message), i18n)
}

pub fn render_transcript_snapshot_export_markdown(
    i18n: &I18n,
    session_id: Option<i64>,
    session_title: &str,
    execution: Option<&SessionExecutionResource>,
    snapshot: &agena_domain::TranscriptSnapshot,
) -> String {
    let entries = crate::transcript_entries(snapshot);
    render_transcript_entries_export_markdown(
        i18n,
        session_id,
        session_title,
        execution,
        entries.as_slice(),
    )
}

pub(crate) fn tool_output_preview(text: &str) -> ToolOutputPreview {
    tool_output_preview_with_limits(text, TOOL_CARD_PREVIEW_LINES, TOOL_CARD_PREVIEW_CHARS)
}

pub(crate) fn tool_output_preview_with_limits(
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

pub(crate) fn sanitize_terminal_text(text: &str) -> String {
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

pub(crate) fn push_message_header(
    out: &mut Vec<RenderedLine>,
    message: &TranscriptEntry,
    width: u16,
    i18n: &I18n,
) {
    let role = message
        .role
        .map(|role| ui_text::role_label(i18n, role))
        .expect("only message entries render a role header");
    let header = match message.state {
        MessageStatus::Completed => role,
        MessageStatus::Pending => format!("{role} ○"),
        MessageStatus::InProgress => format!("{role} {}", transcript_spinner_placeholder()),
        MessageStatus::PolicyDenied => format!("{role} ⊘"),
        MessageStatus::UserDeclined => format!("{role} –"),
        MessageStatus::CapabilityUnavailable | MessageStatus::ToolUnavailable => {
            format!("{role} ◇")
        }
        MessageStatus::Failed => format!("{role} ×"),
        MessageStatus::Cancelled => format!("{role} –"),
    };
    let header_style =
        style_for_role(message.role.expect("message role")).add_modifier(Modifier::BOLD);

    if UnicodeWidthStr::width(header.as_str()) <= width.max(1) as usize {
        out.push(RenderedLine::plain(header, header_style));
    } else {
        push_wrapped_line(out, "", "", header.as_str(), header_style, width);
    }
}

pub(crate) fn render_part_node(
    message: &TranscriptEntry,
    part: &TranscriptEntryPart,
    width: u16,
    out: &mut Vec<RenderedLine>,
    i18n: &I18n,
    defaults: TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
) -> RenderedNodeDraft {
    match transcript_part_content(part) {
        TranscriptPartContent::UserDocument(document) => {
            let copy_text = render_user_document(document, out, width);
            RenderedNodeDraft {
                key: TranscriptNodeKey::Content {
                    entry_id: message.id,
                    content_id: Some(part.id),
                },
                kind: TranscriptNodeKind::Message,
                copy_text,
                toggleable: false,
                expanded: true,
            }
        }
        TranscriptPartContent::Text(text) => {
            push_markdown(out, "  ", text.text.as_str(), width);
            RenderedNodeDraft {
                key: TranscriptNodeKey::Content {
                    entry_id: message.id,
                    content_id: Some(part.id),
                },
                kind: TranscriptNodeKind::Message,
                copy_text: text.text.clone(),
                toggleable: false,
                expanded: true,
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.activity_expanded);
            let (_, title, summary, error) = activity_presentation(payload);
            let details = canonical_activity_details(payload, summary.as_str());
            let toggleable = !summary.trim().is_empty() || error.is_some() || !details.is_empty();
            // A persisted Activity header consists of title + compact summary.
            // The collapsed transcript must consume both, while avoiding a
            // second copy when a specialised title (for example tools_list)
            // already incorporates that same summary.
            let headline_title = if summary.trim().is_empty()
                || title.contains(summary.trim())
                || title.contains('→')
            {
                title.clone()
            } else {
                format!("{title} · {}", concise_text(summary.as_str(), 72))
            };
            push_single_line(
                out,
                "  ",
                activity_headline(headline_title.as_str(), part.status, expanded, toggleable)
                    .as_str(),
                Style::default()
                    .fg(match part.status {
                        PartExecutionStatusResource::Failed => {
                            agena_tui_components::theme::danger_color()
                        }
                        PartExecutionStatusResource::Completed => {
                            agena_tui_components::theme::success_color()
                        }
                        _ => agena_tui_components::theme::muted_color(),
                    })
                    .add_modifier(Modifier::BOLD),
                width,
            );
            if expanded
                && matches!(
                    payload.as_ref(),
                    agena_domain::ActivityPayload::Operation(_)
                )
                && !summary.trim().is_empty()
            {
                push_single_line(
                    out,
                    "    ",
                    "Result",
                    Style::default()
                        .fg(agena_tui_components::theme::muted_color())
                        .add_modifier(Modifier::BOLD),
                    width,
                );
            }
            if expanded && !summary.trim().is_empty() {
                push_multiline(
                    out,
                    if matches!(
                        payload.as_ref(),
                        agena_domain::ActivityPayload::Operation(_)
                    ) {
                        "      "
                    } else {
                        "    "
                    },
                    summary.as_str(),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    width,
                );
            }
            if expanded {
                for detail in &details {
                    push_multiline(out, "    ", detail, Style::default(), width);
                }
                if let agena_domain::ActivityPayload::Resource(resource) = payload.as_ref() {
                    let attachment = canonical_resource_attachment(resource);
                    let _ = render_attachment_image(out, "    ", &attachment, width);
                }
            }
            let error_text = error.as_ref().map(|e| {
                if e.is_unexpected() {
                    format!("{} Reference: {}", e.user.fallback, e.id)
                } else {
                    e.user.fallback.clone()
                }
            });
            if expanded
                && let Some(ref error_str) = error_text
                && error_str.trim() != summary.trim()
            {
                push_multiline(
                    out,
                    "    ",
                    error_str.as_str(),
                    Style::default().fg(agena_tui_components::theme::danger_color()),
                    width,
                );
            }
            let copy_text = [
                Some(title),
                (!summary.is_empty()).then_some(summary),
                (!details.is_empty()).then_some(details.join("\n")),
                error_text,
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text,
                toggleable,
                expanded,
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(reasoning)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.activity_expanded);
            let summary = reasoning.preferred_text();
            if expanded {
                push_section_heading(
                    out,
                    "  ▾ thinking",
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
                    "  ▸ ",
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
        TranscriptPartContent::Activity(TranscriptActivityContent::Operation(tool)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
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
        TranscriptPartContent::Activity(TranscriptActivityContent::Error(error)) => {
            let text = if error.problem.is_unexpected() {
                format!(
                    "{} Reference: {}",
                    error.problem.user.fallback, error.problem.id
                )
            } else {
                error.problem.user.fallback.clone()
            };
            push_multiline(
                out,
                "  ▸ × ",
                &text,
                Style::default().fg(agena_tui_components::theme::danger_color()),
                width,
            );
            RenderedNodeDraft {
                key: TranscriptNodeKey::Activity {
                    entry_id: message.id,
                    content_id: part.id,
                },
                kind: TranscriptNodeKind::Activity,
                copy_text: text,
                toggleable: false,
                expanded: true,
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::AssistantReplyLifecycle(
            status,
        )) => {
            let title = ui_text::t(
                i18n,
                match status {
                    TranscriptAssistantReplyLifecycle::Running => {
                        "message-activity-response-running"
                    }
                    TranscriptAssistantReplyLifecycle::Completed => {
                        "message-activity-response-completed"
                    }
                    TranscriptAssistantReplyLifecycle::Failed => "message-activity-response-failed",
                    TranscriptAssistantReplyLifecycle::Cancelled => {
                        "message-activity-response-cancelled"
                    }
                },
            );
            push_single_line(
                out,
                "  ",
                format!("▸ {title}").as_str(),
                Style::default()
                    .fg(match status {
                        TranscriptAssistantReplyLifecycle::Running => {
                            agena_tui_components::theme::special_color()
                        }
                        TranscriptAssistantReplyLifecycle::Completed => {
                            agena_tui_components::theme::success_color()
                        }
                        TranscriptAssistantReplyLifecycle::Failed
                        | TranscriptAssistantReplyLifecycle::Cancelled => {
                            agena_tui_components::theme::danger_color()
                        }
                    })
                    .add_modifier(Modifier::BOLD),
                width,
            );
            RenderedNodeDraft {
                key: TranscriptNodeKey::Activity {
                    entry_id: message.id,
                    content_id: part.id,
                },
                kind: TranscriptNodeKind::Activity,
                copy_text: title,
                toggleable: false,
                expanded: true,
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Attachment(attachment)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.activity_expanded);
            let mut labels = Vec::new();
            for item in &attachment.attachments {
                let label = item
                    .title
                    .as_ref()
                    .or(item.filename.as_ref())
                    .cloned()
                    .unwrap_or_else(|| item.mime.clone());
                labels.push(label);
            }
            push_single_line(
                out,
                "  ",
                format!(
                    "{} {}: {}",
                    if expanded { "▾" } else { "▸" },
                    ui_text::t(i18n, "message-input-activity-attachment"),
                    labels.join(", ")
                )
                .as_str(),
                Style::default()
                    .fg(agena_tui_components::theme::special_color())
                    .add_modifier(Modifier::BOLD),
                width,
            );
            if expanded {
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
                }
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: labels.join("\n"),
                toggleable: true,
                expanded,
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::SkillReference(reference)) => {
            let key = TranscriptNodeKey::Activity {
                entry_id: message.id,
                content_id: part.id,
            };
            let expanded = expansions
                .get(&key)
                .copied()
                .unwrap_or(defaults.activity_expanded);
            let mut labels = Vec::new();
            for skill in &reference.skills {
                let label = if skill.description.trim().is_empty() {
                    skill.name.clone()
                } else {
                    format!("{} — {}", skill.name, skill.description.trim())
                };
                labels.push(label);
            }
            push_single_line(
                out,
                "  ",
                format!(
                    "{} {}: {}",
                    if expanded { "▾" } else { "▸" },
                    ui_text::t(i18n, "message-input-activity-skill"),
                    labels.join(", ")
                )
                .as_str(),
                Style::default()
                    .fg(agena_tui_components::theme::special_color())
                    .add_modifier(Modifier::BOLD),
                width,
            );
            if expanded {
                for skill in &reference.skills {
                    push_label_value(
                        out,
                        "    - ",
                        skill.description.as_str(),
                        Style::default(),
                        width,
                    );
                    push_label_value(
                        out,
                        "      ",
                        format!("{} · {}", skill.source, skill.content_hash).as_str(),
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                    );
                    push_multiline(
                        out,
                        "      ",
                        skill.instructions.as_str(),
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                    );
                }
            }
            RenderedNodeDraft {
                key,
                kind: TranscriptNodeKind::Activity,
                copy_text: labels.join("\n"),
                toggleable: true,
                expanded,
            }
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Request(request)) => {
            match request.as_ref() {
                MessageRequestPartResource::UserInput { request, .. } => {
                    render_user_input_request(request, out, width, i18n);
                    RenderedNodeDraft {
                        key: TranscriptNodeKey::Activity {
                            entry_id: message.id,
                            content_id: part.id,
                        },
                        kind: TranscriptNodeKind::Activity,
                        copy_text: request
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
    }
}

#[derive(Debug)]
struct UserDocumentToken {
    text: String,
    style: Style,
    width: usize,
    newline: bool,
}

fn render_user_document(
    document: &crate::TranscriptUserDocument,
    out: &mut Vec<RenderedLine>,
    width: u16,
) -> String {
    let mut tokens = Vec::new();
    for node in &document.nodes {
        match node {
            crate::TranscriptUserDocumentNode::Text { text, .. } => {
                let sanitized = sanitize_terminal_text(text);
                tokens.extend(sanitized.graphemes(true).map(|grapheme| UserDocumentToken {
                    text: grapheme.to_owned(),
                    style: Style::default(),
                    width: UnicodeWidthStr::width(grapheme),
                    newline: grapheme == "\n",
                }));
            }
            crate::TranscriptUserDocumentNode::Activity {
                placeholder, style, ..
            } => {
                let placeholder = sanitize_terminal_text(placeholder);
                tokens.push(UserDocumentToken {
                    width: UnicodeWidthStr::width(placeholder.as_str()),
                    text: placeholder,
                    style: match style {
                        crate::TranscriptUserActivityStyle::Resource => Style::default()
                            .fg(agena_tui_components::theme::info_color())
                            .add_modifier(Modifier::BOLD),
                        crate::TranscriptUserActivityStyle::Skill => Style::default()
                            .fg(agena_tui_components::theme::accent_color())
                            .add_modifier(Modifier::BOLD),
                        crate::TranscriptUserActivityStyle::TextArtifact => Style::default()
                            .fg(agena_tui_components::theme::warning_color())
                            .add_modifier(Modifier::BOLD),
                        crate::TranscriptUserActivityStyle::Other => Style::default()
                            .fg(agena_tui_components::theme::accent_color())
                            .add_modifier(Modifier::BOLD),
                    },
                    newline: false,
                });
            }
        }
    }

    let copy_text = tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<String>();
    let line_width = usize::from(width).saturating_sub(2).max(1);
    let mut line_tokens = Vec::new();
    let mut used_width = 0_usize;
    let mut last_was_newline = false;
    let start_len = out.len();
    for token in tokens {
        if token.newline {
            push_user_document_line(out, std::mem::take(&mut line_tokens));
            used_width = 0;
            last_was_newline = true;
            continue;
        }
        if !line_tokens.is_empty() && used_width.saturating_add(token.width) > line_width {
            push_user_document_line(out, std::mem::take(&mut line_tokens));
            used_width = 0;
        }
        used_width = used_width.saturating_add(token.width);
        line_tokens.push(token);
        last_was_newline = false;
    }
    if !line_tokens.is_empty() || last_was_newline || out.len() == start_len {
        push_user_document_line(out, line_tokens);
    }
    copy_text
}

fn push_user_document_line(out: &mut Vec<RenderedLine>, tokens: Vec<UserDocumentToken>) {
    let copy_text = tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<String>();
    let mut spans = vec![Span::raw("  ")];
    for token in tokens {
        if let Some(last) = spans.last_mut()
            && last.style == token.style
        {
            last.content.to_mut().push_str(token.text.as_str());
        } else {
            spans.push(Span::styled(token.text, token.style));
        }
    }
    out.push(RenderedLine::rich(Line::from(spans)).with_copy_projection(copy_text, 2));
}

pub(crate) fn thinking_collapsed_summary(
    status: PartExecutionStatusResource,
    text: &str,
) -> String {
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
