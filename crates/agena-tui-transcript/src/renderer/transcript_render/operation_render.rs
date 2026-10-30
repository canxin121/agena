use super::super::{
    I18n, Modifier, RenderedLine, Style, compact_json_cell, compact_tool_identity,
    json_value_to_markdown, operation_block_copy_text, push_activity_headline,
    push_collapsible_text, push_expanded_diff_text, push_expanded_markdown,
    push_expanded_tool_text, push_label_value, push_multiline, push_section_heading,
    push_single_line, render_expanded_tool_text_block, tool_display_label_localized,
};
use super::request_render::render_file_changes;
use crate::layout::{BODY_PREFIX, DETAIL_PREFIX, NESTED_PREFIX};
use crate::part_headline::{attachment_label, part_headline};
use crate::ui_text;
use crate::{PartExecutionStatusResource, ToolCallView, TranscriptEntryPart};
use agena_domain::ViewBlock;
use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) struct ToolExecutionSectionRender {
    pub start_line: usize,
    pub end_line: usize,
    pub copy_text: String,
    pub expanded: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ToolExecutionRender {
    pub headline_end: usize,
    pub details: Option<ToolExecutionSectionRender>,
    pub render_data: Option<ToolExecutionSectionRender>,
    pub metadata: Option<ToolExecutionSectionRender>,
    pub input: Option<ToolExecutionSectionRender>,
    pub output: Option<ToolExecutionSectionRender>,
    pub presentation: Option<ToolExecutionSectionRender>,
    pub visible_copy_text: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_tool_execution_with_sections(
    part: &TranscriptEntryPart,
    metadata_expanded: bool,
    input_expanded: bool,
    output_expanded: bool,
    details_expanded: bool,
    render_data_expanded: bool,
    tool: &ToolCallView,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) -> ToolExecutionRender {
    if part.status == PartExecutionStatusResource::Completed && is_interaction_notification(tool) {
        render_interaction_notification(part, tool, out, width, expanded, i18n);
        let headline_end = out.len();
        let details = render_tool_detail_sections_with_sections(
            part,
            metadata_expanded,
            input_expanded,
            output_expanded,
            details_expanded,
            render_data_expanded,
            tool,
            out,
            width,
            i18n,
            expanded,
        );
        let visible_copy_text = [
            tool_display_label_localized(tool, i18n),
            details.visible_copy_text,
        ]
        .into_iter()
        .filter(|section| !section.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
        return ToolExecutionRender {
            headline_end,
            visible_copy_text,
            ..details
        };
    }
    let headline = part_headline(part, i18n);
    let label = headline.title;
    if !expanded {
        push_activity_headline(
            out,
            part.status,
            false,
            true,
            label.as_str(),
            &headline.summary,
            width,
            i18n,
        );
        return ToolExecutionRender {
            headline_end: out.len(),
            details: None,
            render_data: None,
            metadata: None,
            input: None,
            output: None,
            presentation: None,
            visible_copy_text: String::new(),
        };
    }
    push_activity_headline(
        out,
        part.status,
        true,
        true,
        label.as_str(),
        &headline.summary,
        width,
        i18n,
    );
    let headline_end = out.len();
    let details = render_tool_detail_sections_with_sections(
        part,
        metadata_expanded,
        input_expanded,
        output_expanded,
        details_expanded,
        render_data_expanded,
        tool,
        out,
        width,
        i18n,
        expanded,
    );
    let visible_copy_text = [label, details.visible_copy_text]
        .into_iter()
        .filter(|section| !section.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    ToolExecutionRender {
        headline_end,
        visible_copy_text,
        ..details
    }
}

/// Render the independently expandable tool detail sections without drawing
/// the operation headline. Interaction operations use this after their
/// interaction body has already rendered; ordinary operations call it from
/// [`render_tool_execution_with_sections`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_tool_detail_sections_with_sections(
    part: &TranscriptEntryPart,
    metadata_expanded: bool,
    input_expanded: bool,
    output_expanded: bool,
    details_expanded: bool,
    render_data_expanded: bool,
    tool: &ToolCallView,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) -> ToolExecutionRender {
    let headline_end = out.len();
    if !expanded {
        return ToolExecutionRender {
            headline_end,
            details: None,
            render_data: None,
            metadata: None,
            input: None,
            output: None,
            presentation: None,
            visible_copy_text: String::new(),
        };
    }

    let mut visible_copy_sections = Vec::new();

    let recorded_failure_text = tool
        .error_message()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let missing_failure_reason = (part.status == PartExecutionStatusResource::Failed
        && recorded_failure_text.is_none())
    .then(|| ui_text::t(i18n, "tool-error-unavailable"));
    let failure_text = if part.status == PartExecutionStatusResource::Failed {
        recorded_failure_text.or(missing_failure_reason.as_deref())
    } else {
        None
    };

    if let Some(error_message) = failure_text {
        push_section_heading(
            out,
            format!("  › {}", ui_text::t(i18n, "activity-section-error")).as_str(),
            Style::default()
                .fg(agena_tui_components::theme::danger_color())
                .add_modifier(Modifier::BOLD),
            width,
        );
        let error_start = out.len();
        render_expanded_tool_text_block(out, DETAIL_PREFIX, error_message, width, i18n);
        patch_rendered_lines_style(
            &mut out[error_start..],
            Style::default().fg(agena_tui_components::theme::danger_color()),
        );
        visible_copy_sections.push(format!(
            "{}\n{error_message}",
            ui_text::t(i18n, "activity-section-error")
        ));
    }

    // Human output is the primary view. Only the technical sections live
    // behind the first disclosure; opening it never loads their payloads.
    let presentation_start = out.len();
    render_tool_presentation_body(tool, out, width, i18n, failure_text);
    let presentation_copy_text = tool_presentation_copy_text(tool, i18n, failure_text);
    let presentation = ToolExecutionSectionRender {
        start_line: presentation_start,
        end_line: out.len(),
        copy_text: presentation_copy_text.clone(),
        expanded: true,
    };
    visible_copy_sections.push(presentation_copy_text);
    // Failure facts are independent of raw output: a rejected tool may have
    // no output at all. Keep its public diagnostic available immediately in
    // Technical details without loading any of the optional payload sections.
    // Never serialize the model-feedback channel into a human view or copy.
    let failure_json = tool.operation.error.as_ref().map(|error| {
        technical_json_text(
            &serde_json::to_value(agena_failure::UserProblem::from(&error.failure))
                .expect("public tool failure is JSON serializable"),
        )
    });
    let failure_body = match (failure_json, missing_failure_reason.as_ref()) {
        (Some(json), Some(reason)) => Some(format!("{reason}\n\n{json}")),
        (json, None) => json,
        (None, Some(reason)) => Some(reason.clone()),
    };
    let failure_copy_text = failure_body
        .as_ref()
        .map(|text| format!("{}\n{text}", ui_text::t(i18n, "activity-section-error")))
        .unwrap_or_default();
    let details = render_detail_section_with_body(
        out,
        &ui_text::t(i18n, "tool-technical-details"),
        details_expanded,
        width,
        |body| {
            if let Some(text) = &failure_body {
                push_section_heading(
                    body,
                    &format!("    › {}", ui_text::t(i18n, "activity-section-error")),
                    Style::default()
                        .fg(agena_tui_components::theme::danger_color())
                        .add_modifier(Modifier::BOLD),
                    width,
                );
                push_expanded_tool_text(body, NESTED_PREFIX, text, Style::default(), width);
            }
        },
        failure_copy_text.clone(),
    );
    if !details_expanded {
        return ToolExecutionRender {
            headline_end,
            details: Some(details),
            render_data: None,
            metadata: None,
            input: None,
            output: None,
            presentation: Some(presentation),
            visible_copy_text: visible_copy_sections.join("\n\n"),
        };
    }
    if !failure_copy_text.is_empty() {
        visible_copy_sections.push(failure_copy_text);
    }
    let nested_start = out.len();
    let width = width.saturating_sub(2);

    // Tool arguments, presented as a nested Markdown bullet list instead of the
    // raw JSON dump. `compact_tool_identity` also unwraps a `tools.call` wrapper
    // to the inner tool + its real input.
    let tool_input = compact_tool_identity(&tool.operation.invocation).1;
    let input_markdown = if tool.input_loaded {
        json_value_to_markdown(&tool_input)
    } else {
        ui_text::t(i18n, "content-loading")
    };
    let mut input = render_markdown_detail_section(
        out,
        &ui_text::t(i18n, "tool-detail-input"),
        &input_markdown,
        input_expanded,
        width,
        i18n,
    );
    if !tool.input_loaded {
        input.copy_text.clear();
    }
    if input_expanded && tool.input_loaded {
        visible_copy_sections.push(format!(
            "{}\n{input_markdown}",
            ui_text::t(i18n, "activity-section-input")
        ));
    }

    let output_json = if tool.output_loaded {
        technical_json_text(
            &serde_json::to_value(&tool.operation.output)
                .expect("raw tool output is JSON serializable"),
        )
    } else {
        ui_text::t(i18n, "content-loading")
    };
    let output_copy_text = if tool.output_loaded {
        format!(
            "{}\n{output_json}",
            ui_text::t(i18n, "activity-section-output")
        )
    } else {
        String::new()
    };
    let output = render_detail_section_with_body(
        out,
        &ui_text::t(i18n, "tool-detail-output"),
        output_expanded,
        width,
        |body| push_expanded_tool_text(body, DETAIL_PREFIX, &output_json, Style::default(), width),
        output_copy_text.clone(),
    );
    if output_expanded && tool.output_loaded {
        visible_copy_sections.push(output_copy_text);
    }

    let metadata_value = serde_json::to_value(&tool.operation.metadata)
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let metadata_copy = json_detail_copy_text("Metadata", &metadata_value);
    let metadata = if tool.metadata_loaded {
        render_json_detail_section(
            out,
            &ui_text::t(i18n, "tool-detail-metadata"),
            &metadata_value,
            metadata_expanded,
            width,
        )
    } else {
        render_markdown_detail_section(
            out,
            &ui_text::t(i18n, "tool-detail-metadata"),
            &ui_text::t(i18n, "content-loading"),
            metadata_expanded,
            width,
            i18n,
        )
    };
    let mut metadata = metadata;
    if !tool.metadata_loaded {
        metadata.copy_text.clear();
    }
    if metadata_expanded && tool.metadata_loaded {
        visible_copy_sections.push(metadata_copy);
    }

    let raw_presentation =
        serde_json::to_value(&tool.presentation).expect("presentation is serializable");
    let render_data = render_json_detail_section(
        out,
        &ui_text::t(i18n, "tool-presentation-data"),
        &raw_presentation,
        render_data_expanded,
        width,
    );
    if render_data_expanded {
        visible_copy_sections.push(render_data.copy_text.clone());
    }

    // Indent both disclosure rows and their bodies as children of Details.
    // Preserve the copy projection so indentation never enters copied data.
    for line in &mut out[nested_start..] {
        line.text.insert_str(0, "  ");
        if let Some(rich) = &mut line.rich_line {
            rich.spans.insert(0, ratatui::text::Span::raw("  "));
        }
        line.copy_column += 2;
        for placement in &mut line.math {
            placement.column = placement.column.saturating_add(2);
        }
        for segment in &mut line.copy_segments {
            segment.display_column += 2;
        }
    }
    ToolExecutionRender {
        headline_end,
        details: Some(details),
        render_data: Some(render_data),
        metadata: Some(metadata),
        input: Some(input),
        output: Some(output),
        presentation: Some(presentation),
        visible_copy_text: visible_copy_sections
            .into_iter()
            .filter(|section| !section.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

fn render_detail_section_with_body(
    out: &mut Vec<RenderedLine>,
    title: &str,
    expanded: bool,
    width: u16,
    render_body: impl FnOnce(&mut Vec<RenderedLine>),
    copy_text: String,
) -> ToolExecutionSectionRender {
    let section_start = out.len();
    push_section_heading(
        out,
        &format!("  {} {title}", if expanded { "▾" } else { "▸" }),
        Style::default()
            .fg(agena_tui_components::theme::special_color())
            .add_modifier(Modifier::BOLD),
        width,
    );
    if expanded {
        render_body(out);
    }
    ToolExecutionSectionRender {
        start_line: section_start,
        end_line: out.len(),
        copy_text,
        expanded,
    }
}

fn render_json_detail_section(
    out: &mut Vec<RenderedLine>,
    title: &str,
    value: &Value,
    expanded: bool,
    width: u16,
) -> ToolExecutionSectionRender {
    let body_text = technical_json_text(value);
    let text = format!("{title}\n{body_text}");
    render_detail_section_with_body(
        out,
        title,
        expanded,
        width,
        |body| push_expanded_tool_text(body, DETAIL_PREFIX, &body_text, Style::default(), width),
        text,
    )
}

fn render_markdown_detail_section(
    out: &mut Vec<RenderedLine>,
    title: &str,
    markdown: &str,
    expanded: bool,
    width: u16,
    i18n: &I18n,
) -> ToolExecutionSectionRender {
    let copy_text = format!("{title}\n{markdown}");
    render_detail_section_with_body(
        out,
        title,
        expanded,
        width,
        |body| push_expanded_markdown(body, DETAIL_PREFIX, markdown, width, i18n),
        copy_text,
    )
}

fn json_detail_copy_text(title: &str, value: &Value) -> String {
    let rendered = technical_json_text(value);
    format!("{title}\n{rendered}")
}

fn technical_json_text(value: &Value) -> String {
    fn summarize_inline_data(value: &mut Value) {
        match value {
            Value::String(text) if text.starts_with("data:") => {
                if let Some((header, body)) = text.split_once(',') {
                    let mime: String = header["data:".len()..]
                        .split(';')
                        .next()
                        .unwrap_or("application/octet-stream")
                        .chars()
                        .take(128)
                        .collect();
                    *text = format!(
                        "[embedded {mime} data: {} encoded bytes omitted]",
                        body.len()
                    );
                }
            }
            Value::Array(values) => {
                for value in values {
                    summarize_inline_data(value);
                }
            }
            Value::Object(object) => {
                for value in object.values_mut() {
                    summarize_inline_data(value);
                }
            }
            _ => {}
        }
    }
    // Pixel bytes stay in the bounded preview cache. Technical rows and their
    // copy projection retain metadata without materializing megabytes of text.
    let mut value = value.clone();
    summarize_inline_data(&mut value);
    serde_json::to_string_pretty(&value).expect("JSON is serializable")
}

fn render_tool_presentation_body(
    tool: &ToolCallView,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    failure_text: Option<&str>,
) {
    if tool.presentation.blocks.is_empty() && !tool.presentation.summary.trim().is_empty() {
        push_expanded_markdown(
            out,
            BODY_PREFIX,
            tool.presentation.summary.as_str(),
            width,
            i18n,
        );
    }
    render_operation_blocks(
        human_tool_blocks(tool),
        out,
        width,
        i18n,
        true,
        failure_text,
        &tool_diff_paths(tool),
        ContentObservation {
            frames: &tool.contents,
            errors: &tool.content_errors,
        },
    );
    for attachment in tool.attachments() {
        render_output_attachment(attachment, out, width, i18n);
    }
}

fn output_attachment(item: &agena_domain::AttachmentItem) -> agena_api::resource::PartAttachment {
    use agena_api::resource::{PartAttachment, PartAttachmentKind, PartAttachmentSource};
    use agena_domain::AttachmentKind;

    PartAttachment {
        kind: match item.kind {
            AttachmentKind::Image => PartAttachmentKind::Image,
            AttachmentKind::Audio => PartAttachmentKind::Audio,
            AttachmentKind::Video => PartAttachmentKind::Video,
            AttachmentKind::Pdf => PartAttachmentKind::Pdf,
            AttachmentKind::File => PartAttachmentKind::File,
        },
        mime: item.mime.clone(),
        source: PartAttachmentSource::from(&item.source),
        filename: item.filename.clone(),
        title: item.title.clone(),
        size_bytes: item.size_bytes,
        sha256: item.sha256.clone(),
        width: item.width,
        height: item.height,
        duration_ms: item.duration_ms,
        page_count: item.page_count,
    }
}

fn render_output_attachment(
    item: &agena_domain::AttachmentItem,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    let item = output_attachment(item);
    if !super::super::transcript_ast::render_attachment_image(out, BODY_PREFIX, &item, width, i18n)
    {
        push_label_value(
            out,
            "  - ",
            &attachment_label(&item, i18n),
            Style::default().fg(agena_tui_components::theme::muted_color()),
            width,
        );
    }
}

fn artifact_matches_attachment(
    artifact: &agena_domain::ArtifactRef,
    attachment: &agena_domain::AttachmentItem,
) -> bool {
    if artifact
        .sha256
        .as_ref()
        .filter(|hash| !hash.is_empty())
        .is_some_and(|hash| attachment.sha256.as_ref() == Some(hash))
    {
        return true;
    }
    match &attachment.source {
        agena_domain::AttachmentSource::Url { url }
        | agena_domain::AttachmentSource::DataUrl { url } => artifact.uri == *url,
        agena_domain::AttachmentSource::LocalPath { path } => {
            artifact
                .uri
                .strip_prefix("file://")
                .unwrap_or(&artifact.uri)
                == path
        }
        _ => false,
    }
}

fn tool_diff_paths(tool: &ToolCallView) -> std::collections::BTreeSet<String> {
    tool.presentation
        .blocks
        .iter()
        .filter_map(|block| match block {
            ViewBlock::Diff { diff, .. } => Some(super::super::transcript_diff_files(diff)),
            _ => None,
        })
        .flatten()
        .flat_map(|file| [file.path, file.old_path])
        .filter(|path| !path.is_empty())
        .collect()
}

fn human_tool_blocks(tool: &ToolCallView) -> impl Iterator<Item = &ViewBlock> {
    tool.presentation.blocks.iter().filter(|block| {
        let is_attachment_copy = matches!(block, ViewBlock::Media { artifact, .. }
            if tool.attachments().iter().any(|attachment| artifact_matches_attachment(artifact, attachment)));
        !is_attachment_copy
            && (tool.operation.user_input.requests.is_empty()
            || !matches!(
                block.block_id(),
                Some("answers" | "interaction-answers" | "interaction-meta")
            ))
    })
}

fn tool_presentation_copy_text(
    tool: &ToolCallView,
    i18n: &I18n,
    failure_text: Option<&str>,
) -> String {
    let mut sections = Vec::new();
    let diff_paths = tool_diff_paths(tool);
    if tool.presentation.blocks.is_empty() && !tool.presentation.summary.trim().is_empty() {
        sections.push(tool.presentation.summary.clone());
    }
    sections.extend(
        human_tool_blocks(tool)
            .filter(|block| {
                !failure_text.is_some_and(|failure| {
                    block
                        .text_value()
                        .is_some_and(|text| text.trim() == failure)
                })
            })
            .map(|block| match block {
                ViewBlock::Content { resource, .. } => {
                    let frame = tool.contents.get(&resource.resource_id);
                    let mut text = frame
                        .map(|frame| {
                            let mut text = String::new();
                            if let Some(document) = &frame.document {
                                text.push_str(
                                    &document
                                        .blocks
                                        .iter()
                                        .map(|block| operation_block_copy_text(block, i18n))
                                        .filter(|text| !text.is_empty())
                                        .collect::<Vec<_>>()
                                        .join("\n\n"),
                                );
                                if !text.is_empty()
                                    && (!frame.lines.is_empty() || !frame.text.is_empty())
                                {
                                    text.push_str("\n\n");
                                }
                            }
                            if frame.resource.kind == agena_domain::ContentKind::Text {
                                text.push_str(&frame.text);
                            }
                            for (index, line) in frame.lines.iter().enumerate() {
                                if index > 0
                                    && !frame.wrapped.get(index - 1).copied().unwrap_or(false)
                                {
                                    text.push('\n');
                                }
                                for span in &line.spans {
                                    text.push_str(&span.content);
                                }
                            }
                            if frame.gap {
                                text.insert_str(0, &format!("[{}]\n", i18n.text("content-gap")));
                            }
                            if frame.windowed {
                                text.insert_str(0, &format!("[{}]\n", i18n.text("content-window")));
                            }
                            text
                        })
                        .unwrap_or_default();
                    let error = tool
                        .content_errors
                        .get(&resource.resource_id)
                        .map(String::as_str)
                        .or_else(|| {
                            frame.and_then(|frame| frame.resource.capture_error.as_deref())
                        });
                    if let Some(error) = error {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&crate::content::content_status_text(error));
                    }
                    // Text resources retain their original bytes. Their
                    // screen rendering already strips terminal controls; the
                    // copy projection must not reintroduce those controls.
                    crate::sanitize_terminal_text(&text)
                }
                ViewBlock::FileChanges { changes, .. } => operation_block_copy_text(
                    &ViewBlock::FileChanges {
                        id: None,
                        changes: changes
                            .iter()
                            .filter(|change| !diff_paths.contains(&change.path))
                            .cloned()
                            .collect(),
                    },
                    i18n,
                ),
                _ => operation_block_copy_text(block, i18n),
            })
            .filter(|text| !text.trim().is_empty()),
    );
    sections.extend(
        tool.attachments()
            .iter()
            .map(|attachment| attachment_label(&output_attachment(attachment), i18n)),
    );
    sections.join("\n\n")
}

fn patch_rendered_lines_style(lines: &mut [RenderedLine], style: Style) {
    for line in lines {
        line.style = line.style.patch(style);
        if let Some(rich_line) = line.rich_line.take() {
            line.rich_line = Some(rich_line.patch_style(style));
        }
    }
}

fn is_interaction_notification(tool: &ToolCallView) -> bool {
    tool.metadata_value("agena.effect")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|effect| effect == "notification")
        || matches!(
            compact_tool_identity(&tool.operation.invocation).0.as_str(),
            "interaction.notify" | "agena_interaction_notify" | "interaction_notify"
        )
}

fn render_interaction_notification(
    part: &TranscriptEntryPart,
    tool: &ToolCallView,
    out: &mut Vec<RenderedLine>,
    width: u16,
    expanded: bool,
    i18n: &I18n,
) {
    let headline = part_headline(part, i18n);
    push_activity_headline(
        out,
        part.status,
        expanded,
        true,
        &headline.title,
        &headline.summary,
        width,
        i18n,
    );
    // Notification severity is independent of successful delivery. Retain its
    // warning/error icon while sharing disclosure and summary geometry.
    if let Some((icon, color)) = match tool
        .metadata_value("agena.notification.level")
        .and_then(serde_json::Value::as_str)
    {
        Some("warning") => Some(("▲", agena_tui_components::theme::warning_color())),
        Some("error") => Some(("◆", agena_tui_components::theme::danger_color())),
        Some("success") => Some(("●", agena_tui_components::theme::success_color())),
        _ => None,
    } && let Some(line) = out.last_mut()
        && let Some(rich) = &mut line.rich_line
        && let Some(span) = rich.spans.get_mut(2)
    {
        span.content = icon.into();
        span.style = Style::default().fg(color);
        line.text = rich
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
    }
    if expanded && tool.presentation.blocks.is_empty() {
        let model_text = tool.model_text();
        if !model_text.trim().is_empty() {
            push_expanded_markdown(out, BODY_PREFIX, &model_text, width, i18n);
        }
    }
}

pub(crate) struct ContentObservation<'a> {
    pub frames: &'a std::collections::BTreeMap<
        agena_domain::ContentId,
        std::sync::Arc<crate::content::ContentFrame>,
    >,
    pub errors: &'a std::collections::BTreeMap<agena_domain::ContentId, String>,
}

pub(crate) fn render_operation_blocks<'a>(
    blocks: impl IntoIterator<Item = &'a ViewBlock>,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
    skipped_text: Option<&str>,
    diff_paths: &std::collections::BTreeSet<String>,
    observation: ContentObservation<'_>,
) {
    let ContentObservation {
        frames: contents,
        errors: content_errors,
    } = observation;
    for block in blocks {
        match block {
            ViewBlock::Progress {
                completed, total, ..
            } => {
                let label = operation_block_copy_text(block, i18n);
                push_single_line(
                    out,
                    BODY_PREFIX,
                    &label,
                    Style::default().fg(agena_tui_components::theme::accent_color()),
                    width,
                );
                if let Some(total) = total.filter(|total| *total > 0) {
                    let columns = usize::from(width)
                        .saturating_sub(BODY_PREFIX.len() + 2)
                        .min(36);
                    let filled = (u128::from(*completed).saturating_mul(columns as u128)
                        / u128::from(total)) as usize;
                    push_single_line(
                        out,
                        BODY_PREFIX,
                        &format!(
                            "[{}{}]",
                            "━".repeat(filled.min(columns)),
                            "─".repeat(columns.saturating_sub(filled))
                        ),
                        Style::default().fg(agena_tui_components::theme::accent_color()),
                        width,
                    );
                }
            }
            ViewBlock::Content {
                resource, format, ..
            } => crate::content::render_content(
                contents
                    .get(&resource.resource_id)
                    .map(std::sync::Arc::as_ref),
                out,
                width,
                i18n,
                content_errors
                    .get(&resource.resource_id)
                    .map(String::as_str),
                *format,
            ),
            ViewBlock::Text { text, .. } => {
                if skipped_text.is_some_and(|candidate| text.trim() == candidate) {
                    continue;
                }
                if expanded {
                    render_expanded_tool_text_block(out, BODY_PREFIX, text, width, i18n);
                } else {
                    push_collapsible_text(out, BODY_PREFIX, text, Style::default(), width, i18n);
                }
            }
            ViewBlock::Markdown { text, .. } => {
                if skipped_text.is_some_and(|candidate| text.trim() == candidate) {
                    continue;
                }
                if expanded {
                    push_expanded_markdown(out, BODY_PREFIX, text, width, i18n);
                } else {
                    push_collapsible_text(out, BODY_PREFIX, text, Style::default(), width, i18n);
                }
            }
            ViewBlock::Command {
                command,
                exit_code,
                stdout,
                stderr,
                ..
            } => {
                let longest_fence = command
                    .split(|ch| ch != '`')
                    .map(str::len)
                    .max()
                    .unwrap_or(0);
                let fence = "`".repeat((longest_fence + 1).max(3));
                let separator = if command.ends_with('\n') { "" } else { "\n" };
                push_expanded_markdown(
                    out,
                    BODY_PREFIX,
                    &format!("{fence}sh\n{command}{separator}{fence}\n"),
                    width,
                    i18n,
                );
                if !stdout.trim().is_empty() {
                    if expanded {
                        push_expanded_tool_text(
                            out,
                            DETAIL_PREFIX,
                            stdout,
                            Style::default(),
                            width,
                        );
                    } else {
                        push_collapsible_text(
                            out,
                            DETAIL_PREFIX,
                            stdout,
                            Style::default(),
                            width,
                            i18n,
                        );
                    }
                }
                if !stderr.trim().is_empty() {
                    if expanded {
                        push_expanded_tool_text(
                            out,
                            DETAIL_PREFIX,
                            stderr,
                            Style::default().fg(agena_tui_components::theme::danger_color()),
                            width,
                        );
                    } else {
                        push_collapsible_text(
                            out,
                            DETAIL_PREFIX,
                            stderr,
                            Style::default().fg(agena_tui_components::theme::danger_color()),
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
                        DETAIL_PREFIX,
                        &ui_text::operation_command_exit_line(i18n, *exit_code),
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                    );
                }
            }
            ViewBlock::Diff { diff, .. } => {
                if expanded {
                    push_expanded_diff_text(out, BODY_PREFIX, diff, width, i18n);
                } else {
                    push_collapsible_text(
                        out,
                        BODY_PREFIX,
                        diff,
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                        i18n,
                    );
                }
            }
            ViewBlock::FileChanges { changes, .. } => {
                let remaining = changes
                    .iter()
                    .filter(|change| !diff_paths.contains(&change.path))
                    .cloned()
                    .collect::<Vec<_>>();
                render_file_changes(&remaining, out, width, i18n)
            }
            ViewBlock::SearchResults { items, .. } => {
                let heading = ui_text::operation_search_heading(i18n, None);
                push_section_heading(
                    out,
                    &format!("  {heading}"),
                    Style::default()
                        .fg(agena_tui_components::theme::accent_color())
                        .add_modifier(Modifier::BOLD),
                    width,
                );
                for result in items {
                    push_label_value(
                        out,
                        "    - ",
                        result.title.as_str(),
                        Style::default(),
                        width,
                    );
                    push_multiline(
                        out,
                        NESTED_PREFIX,
                        result.url.as_str(),
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                    );
                    if let Some(snippet) = &result.snippet
                        && !snippet.trim().is_empty()
                    {
                        push_multiline(out, NESTED_PREFIX, snippet, Style::default(), width);
                    }
                }
            }
            ViewBlock::Media { artifact, .. } => {
                render_output_attachment(&artifact.clone().into(), out, width, i18n);
            }
            ViewBlock::Json { value, .. } => {
                let text = json_value_to_markdown(value);
                if expanded {
                    push_expanded_markdown(out, BODY_PREFIX, &text, width, i18n);
                } else {
                    push_collapsible_text(
                        out,
                        BODY_PREFIX,
                        text.as_str(),
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                        i18n,
                    );
                }
            }
            ViewBlock::Table { columns, rows, .. } => {
                let headings = columns.iter().map(String::as_str).collect::<Vec<_>>();
                let mut table = String::new();
                table.push_str(&format!("| {} |\n", headings.join(" | ")));
                table.push_str(&format!(
                    "| {} |\n",
                    headings
                        .iter()
                        .map(|_| "---")
                        .collect::<Vec<_>>()
                        .join(" | ")
                ));
                for row in rows {
                    let cells = row
                        .iter()
                        .map(compact_json_cell)
                        .collect::<Vec<_>>()
                        .join(" | ");
                    table.push_str(&format!("| {cells} |\n"));
                }
                if expanded {
                    push_expanded_markdown(out, BODY_PREFIX, table.as_str(), width, i18n);
                } else {
                    push_collapsible_text(
                        out,
                        BODY_PREFIX,
                        table.as_str(),
                        Style::default(),
                        width,
                        i18n,
                    );
                }
            }
            ViewBlock::Log { stream, text, .. } => {
                let is_stderr = matches!(stream, agena_domain::CommandOutputStream::Stderr);
                let style = if is_stderr {
                    Style::default().fg(agena_tui_components::theme::danger_color())
                } else {
                    Style::default().fg(agena_tui_components::theme::muted_color())
                };
                let stream_name = match stream {
                    agena_domain::CommandOutputStream::Stdout => "stdout",
                    agena_domain::CommandOutputStream::Stderr => "stderr",
                };
                push_label_value(
                    out,
                    BODY_PREFIX,
                    &format!("[{stream_name}]"),
                    Style::default().fg(agena_tui_components::theme::accent_color()),
                    width,
                );
                if expanded {
                    push_expanded_tool_text(out, DETAIL_PREFIX, text, style, width);
                } else {
                    push_collapsible_text(out, DETAIL_PREFIX, text, style, width, i18n);
                }
            }
            ViewBlock::Custom { presentation, .. } => {
                let text = operation_block_copy_text(block, i18n);
                if expanded {
                    if presentation.is_empty() {
                        // Unknown custom schemas are data, not executable UI.
                        // Keep their kind and fields visible when the plugin
                        // did not supply a textual human presentation.
                        push_expanded_tool_text(out, BODY_PREFIX, &text, Style::default(), width);
                    } else {
                        push_expanded_markdown(out, BODY_PREFIX, text.as_str(), width, i18n);
                    }
                } else {
                    push_collapsible_text(
                        out,
                        BODY_PREFIX,
                        text.as_str(),
                        Style::default(),
                        width,
                        i18n,
                    );
                }
            }
        }
    }
}
