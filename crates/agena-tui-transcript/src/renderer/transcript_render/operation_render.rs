use super::super::{
    I18n, Modifier, RenderedLine, Style, compact_json_cell, compact_tool_identity,
    json_value_to_markdown, operation_block_copy_text, push_activity_headline,
    push_collapsible_text, push_expanded_diff_text, push_expanded_markdown,
    push_expanded_tool_text, push_label_value, push_multiline, push_section_heading,
    push_single_line, render_expanded_tool_text_block, tool_display_label,
};
use super::request_render::render_file_changes;
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

#[cfg(test)]
pub(crate) fn render_tool_execution(
    part: &TranscriptEntryPart,
    tool: &ToolCallView,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) {
    let _ = render_tool_execution_with_sections(
        part, expanded, expanded, expanded, expanded, expanded, tool, out, width, i18n, expanded,
    );
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
        render_interaction_notification(tool, out, width, expanded);
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
        let visible_copy_text = [tool_display_label(tool), details.visible_copy_text]
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
    let label = tool_display_label(tool);
    if !expanded {
        push_activity_headline(
            out,
            part.status,
            false,
            true,
            label.as_str(),
            tool.summary(),
            width,
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
        tool.summary(),
        width,
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

    let failure_text = if part.status == PartExecutionStatusResource::Failed {
        tool.error_message()
            .map(str::trim)
            .filter(|text| !text.is_empty())
    } else {
        None
    };

    if let Some(error_message) = failure_text {
        push_section_heading(
            out,
            "    › Error",
            Style::default()
                .fg(agena_tui_components::theme::danger_color())
                .add_modifier(Modifier::BOLD),
            width,
        );
        let error_start = out.len();
        render_expanded_tool_text_block(out, "      ", error_message, width);
        patch_rendered_lines_style(
            &mut out[error_start..],
            Style::default().fg(agena_tui_components::theme::danger_color()),
        );
        visible_copy_sections.push(format!("Error\n{error_message}"));
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
    let details = render_detail_section_with_body(
        out,
        &ui_text::t(i18n, "tool-technical-details"),
        details_expanded,
        width,
        |_| {},
        String::new(),
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
    let nested_start = out.len();
    let width = width.saturating_sub(2);

    // Tool arguments, presented as a nested Markdown bullet list instead of the
    // raw JSON dump. `compact_tool_identity` also unwraps a `tools.call` wrapper
    // to the inner tool + its real input.
    let tool_input = compact_tool_identity(&tool.operation.invocation).1;
    let input_markdown = json_value_to_markdown(&tool_input);
    let input = render_markdown_detail_section(
        out,
        &ui_text::t(i18n, "tool-detail-input"),
        &input_markdown,
        input_expanded,
        width,
    );
    if input_expanded {
        visible_copy_sections.push(format!("Input\n{input_markdown}"));
    }

    let output_json = serde_json::to_string_pretty(&tool.operation.output)
        .expect("raw tool output is JSON serializable");
    let output_copy_text = format!("Output\n{output_json}");
    let output = render_detail_section_with_body(
        out,
        &ui_text::t(i18n, "tool-detail-output"),
        output_expanded,
        width,
        |body| push_expanded_tool_text(body, "      ", &output_json, Style::default(), width),
        output_copy_text.clone(),
    );
    if output_expanded {
        visible_copy_sections.push(output_copy_text);
    }

    let metadata_value = serde_json::to_value(&tool.operation.metadata)
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let metadata_copy = json_detail_copy_text("Metadata", &metadata_value);
    let metadata = render_json_detail_section(
        out,
        &ui_text::t(i18n, "tool-detail-metadata"),
        &metadata_value,
        metadata_expanded,
        width,
    );
    if metadata_expanded {
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
        &format!("    {} {title}", if expanded { "▾" } else { "▸" }),
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
    let text = json_detail_copy_text(title, value);
    let body_text = serde_json::to_string_pretty(value).expect("JSON is serializable");
    render_detail_section_with_body(
        out,
        title,
        expanded,
        width,
        |body| push_expanded_tool_text(body, "      ", &body_text, Style::default(), width),
        text,
    )
}

fn render_markdown_detail_section(
    out: &mut Vec<RenderedLine>,
    title: &str,
    markdown: &str,
    expanded: bool,
    width: u16,
) -> ToolExecutionSectionRender {
    let copy_text = format!("{title}\n{markdown}");
    render_detail_section_with_body(
        out,
        title,
        expanded,
        width,
        |body| push_expanded_markdown(body, "      ", markdown, width),
        copy_text,
    )
}

fn json_detail_copy_text(title: &str, value: &Value) -> String {
    let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    format!("{title}\n{rendered}")
}

fn render_tool_presentation_body(
    tool: &ToolCallView,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    failure_text: Option<&str>,
) {
    if tool.presentation.blocks.is_empty() && !tool.presentation.summary.trim().is_empty() {
        push_expanded_markdown(out, "    ", tool.presentation.summary.as_str(), width);
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
        tool.operation.user_input.requests.is_empty()
            || !matches!(
                block.block_id(),
                Some("answers" | "interaction-answers" | "interaction-meta")
            )
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
                ViewBlock::Content { resource, .. } => tool
                    .contents
                    .get(&resource.resource_id)
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
                            if index > 0 && !frame.wrapped.get(index - 1).copied().unwrap_or(false)
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
                    .unwrap_or_default(),
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
    tool: &ToolCallView,
    out: &mut Vec<RenderedLine>,
    width: u16,
    expanded: bool,
) {
    let level = tool
        .metadata_value("agena.notification.level")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("info");
    let (icon, color) = match level {
        "success" => ("●", agena_tui_components::theme::success_color()),
        "warning" => ("▲", agena_tui_components::theme::warning_color()),
        "error" => ("◆", agena_tui_components::theme::danger_color()),
        _ => ("●", agena_tui_components::theme::info_color()),
    };
    let title = tool_display_label(tool);
    if !expanded || !tool.presentation.blocks.is_empty() {
        push_single_line(
            out,
            "  ",
            format!("{icon} {title}").as_str(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
            width,
        );
        return;
    }
    push_single_line(
        out,
        "  ╭─ ",
        format!("{icon} {title}").as_str(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
        width,
    );
    let model_text = tool.model_text();
    push_expanded_markdown(out, "  │  ", model_text.as_str(), width);
    push_single_line(
        out,
        "  ╰─ ",
        level,
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
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
                    "    ",
                    &label,
                    Style::default().fg(agena_tui_components::theme::accent_color()),
                    width,
                );
                if let Some(total) = total.filter(|total| *total > 0) {
                    let columns = usize::from(width.saturating_sub(8)).min(36);
                    let filled = (u128::from(*completed).saturating_mul(columns as u128)
                        / u128::from(total)) as usize;
                    push_single_line(
                        out,
                        "    ",
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
                    render_expanded_tool_text_block(out, "    ", text, width);
                } else {
                    push_collapsible_text(out, "    ", text, Style::default(), width, i18n);
                }
            }
            ViewBlock::Markdown { text, .. } => {
                if skipped_text.is_some_and(|candidate| text.trim() == candidate) {
                    continue;
                }
                if expanded {
                    push_expanded_markdown(out, "    ", text, width);
                } else {
                    push_collapsible_text(out, "    ", text, Style::default(), width, i18n);
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
                    "    ",
                    &format!("{fence}sh\n{command}{separator}{fence}\n"),
                    width,
                );
                if !stdout.trim().is_empty() {
                    if expanded {
                        push_expanded_tool_text(out, "      ", stdout, Style::default(), width);
                    } else {
                        push_collapsible_text(out, "      ", stdout, Style::default(), width, i18n);
                    }
                }
                if !stderr.trim().is_empty() {
                    if expanded {
                        push_expanded_tool_text(
                            out,
                            "      ",
                            stderr,
                            Style::default().fg(agena_tui_components::theme::danger_color()),
                            width,
                        );
                    } else {
                        push_collapsible_text(
                            out,
                            "      ",
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
                        "      ",
                        &ui_text::operation_command_exit_line(i18n, *exit_code),
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                    );
                }
            }
            ViewBlock::Diff { diff, .. } => {
                if expanded {
                    push_expanded_diff_text(out, "    ", diff, width);
                } else {
                    push_collapsible_text(
                        out,
                        "    ",
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
                    &format!("    {heading}"),
                    Style::default()
                        .fg(agena_tui_components::theme::accent_color())
                        .add_modifier(Modifier::BOLD),
                    width,
                );
                for result in items {
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
                        result.url.as_str(),
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                    );
                    if let Some(snippet) = &result.snippet
                        && !snippet.trim().is_empty()
                    {
                        push_multiline(out, "        ", snippet, Style::default(), width);
                    }
                }
            }
            ViewBlock::Media { artifact, .. } => {
                push_label_value(
                    out,
                    "    - ",
                    artifact.name.as_deref().unwrap_or(artifact.uri.as_str()),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    width,
                );
            }
            ViewBlock::Json { value, .. } => {
                let text = json_value_to_markdown(value);
                if expanded {
                    push_expanded_markdown(out, "    ", &text, width);
                } else {
                    push_collapsible_text(
                        out,
                        "    ",
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
                    push_expanded_markdown(out, "    ", table.as_str(), width);
                } else {
                    push_collapsible_text(
                        out,
                        "    ",
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
                    "    ",
                    &format!("[{stream_name}]"),
                    Style::default().fg(agena_tui_components::theme::accent_color()),
                    width,
                );
                if expanded {
                    push_expanded_tool_text(out, "      ", text, style, width);
                } else {
                    push_collapsible_text(out, "      ", text, style, width, i18n);
                }
            }
            ViewBlock::Custom { presentation, .. } => {
                // Object payloads (e.g. a plugin's `presentation` map) read
                // best as nested bullets; any other shape falls back to
                // pretty-printed JSON.
                let value = serde_json::json!(presentation);
                if value.is_object() {
                    let text = json_value_to_markdown(&value);
                    if expanded {
                        push_expanded_markdown(out, "    ", text.as_str(), width);
                    } else {
                        push_collapsible_text(
                            out,
                            "    ",
                            text.as_str(),
                            Style::default(),
                            width,
                            i18n,
                        );
                    }
                } else {
                    let text =
                        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                    if expanded {
                        push_expanded_markdown(
                            out,
                            "    ",
                            format!("```json\n{text}\n```").as_str(),
                            width,
                        );
                    } else {
                        push_collapsible_text(
                            out,
                            "    ",
                            text.as_str(),
                            Style::default().fg(agena_tui_components::theme::muted_color()),
                            width,
                            i18n,
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{
        FileChangeKind, FileChangeRecord, RawOutput, StructuredObject, TimeRange, ToolInvocation,
    };
    use agena_runtime_contracts::part::OperationPart;

    #[test]
    fn a_partial_diff_keeps_uncovered_file_changes_in_the_view_and_copy() {
        let tool = ToolCallView::from_operation(
            OperationPart::completed(
                7,
                ToolInvocation::new("fs.apply_patch", StructuredObject::default()),
                RawOutput::default(),
                TimeRange::default(),
            ),
            Some(agena_domain::PartDocument {
                title: "Applied patch".into(),
                summary: String::new(),
                blocks: vec![
                    ViewBlock::FileChanges {
                        id: None,
                        changes: ["a.rs", "binary.png"]
                            .into_iter()
                            .map(|path| FileChangeRecord {
                                path: path.into(),
                                kind: FileChangeKind::Updated,
                                from_path: None,
                            })
                            .collect(),
                    },
                    ViewBlock::Diff {
                        id: None,
                        language: Some("diff".into()),
                        diff: "--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n".into(),
                    },
                ],
            }),
        );
        let mut lines = Vec::new();
        render_tool_presentation_body(&tool, &mut lines, 80, &I18n::english(), None);
        let visible = lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(visible.contains("binary.png"), "{visible}");
        assert_eq!(visible.matches("a.rs").count(), 1, "{visible}");
        let copied = tool_presentation_copy_text(&tool, &I18n::english(), None);
        assert!(copied.contains("binary.png"), "{copied}");
        assert!(!copied.contains("M a.rs"), "{copied}");
        assert!(copied.contains("-old\n+new"), "{copied}");
    }
}
