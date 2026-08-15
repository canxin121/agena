use super::super::transcript_ast::render_attachment_image;
use super::super::{
    I18n, Modifier, RenderedLine, Style, apply_patch_details, compact_json_cell,
    compact_tool_identity, diff_stats, json_value_to_markdown, operation_block_copy_text,
    push_activity_headline, push_collapsible_text, push_expanded_diff_text, push_expanded_markdown,
    push_expanded_tool_text, push_label_value, push_multiline, push_section_heading,
    push_single_line, render_expanded_tool_text_block, should_render_tool_model_output,
    tool_display_label,
};
use super::request_render::{render_checklist, render_file_changes};
use crate::ui_text;
use crate::{
    OperationBlockResource, OperationPartResource, PartExecutionStatusResource, TranscriptEntryPart,
};

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
    pub input: Option<ToolExecutionSectionRender>,
    pub output: Option<ToolExecutionSectionRender>,
    pub visible_copy_text: String,
}

#[cfg(test)]
pub(crate) fn render_tool_execution(
    part: &TranscriptEntryPart,
    tool: &OperationPartResource,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) {
    let _ = render_tool_execution_with_sections(
        part, expanded, expanded, tool, out, width, i18n, expanded,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_tool_execution_with_sections(
    part: &TranscriptEntryPart,
    input_expanded: bool,
    output_expanded: bool,
    tool: &OperationPartResource,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) -> ToolExecutionRender {
    if part.status == PartExecutionStatusResource::Completed && is_interaction_notification(tool) {
        render_interaction_notification(tool, out, width, expanded);
        return ToolExecutionRender {
            headline_end: out.len(),
            input: None,
            output: None,
            visible_copy_text: tool_display_label(tool),
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
            tool.summary.as_str(),
            width,
        );
        return ToolExecutionRender {
            headline_end: out.len(),
            input: None,
            output: None,
            visible_copy_text: String::new(),
        };
    }
    push_activity_headline(
        out,
        part.status,
        true,
        true,
        label.as_str(),
        tool.summary.as_str(),
        width,
    );
    let headline_end = out.len();
    let mut visible_copy_sections = vec![label];

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

    // Tool arguments, presented as a nested Markdown bullet list instead of the
    // raw JSON dump. `compact_tool_identity` also unwraps a `tools.call` wrapper
    // to the inner tool + its real input.
    let tool_input = compact_tool_identity(&tool.invocation).1;
    let input = if !tool_input.is_null()
        && tool_input
            .as_object()
            .is_none_or(|fields| !fields.is_empty())
    {
        let input_markdown = json_value_to_markdown(&tool_input);
        let section_start = out.len();
        push_section_heading(
            out,
            if input_expanded {
                "    ▾ Input"
            } else {
                "    ▸ Input"
            },
            Style::default()
                .fg(agena_tui_components::theme::special_color())
                .add_modifier(Modifier::BOLD),
            width,
        );
        if input_expanded {
            push_expanded_markdown(out, "      ", input_markdown.as_str(), width);
            visible_copy_sections.push(format!("Input\n{input_markdown}"));
        }
        Some(ToolExecutionSectionRender {
            start_line: section_start,
            end_line: out.len(),
            copy_text: format!("Input\n{input_markdown}"),
            expanded: input_expanded,
        })
    } else {
        None
    };

    // All result-facing material belongs to one independently collapsible
    // Output section. This includes human/model output, stdout/stderr, rich
    // operation blocks, attachments, file changes, and diffs.
    let output_copy_text = tool_output_section_copy_text(tool, i18n, failure_text);
    let has_output = !output_copy_text.trim().is_empty()
        || tool
            .attachments
            .iter()
            .chain(tool.model_output.attachments.iter())
            .chain(tool.result.attachments.iter())
            .next()
            .is_some()
        || apply_patch_details(&tool.details)
            .is_some_and(|payload| !payload.changes.is_empty() || !payload.diff.trim().is_empty());
    let output = if !has_output {
        None
    } else {
        let section_start = out.len();
        push_section_heading(
            out,
            if output_expanded {
                "    ▾ Output"
            } else {
                "    ▸ Output"
            },
            Style::default()
                .fg(agena_tui_components::theme::special_color())
                .add_modifier(Modifier::BOLD),
            width,
        );
        if output_expanded {
            let mut output_body = Vec::new();
            render_tool_output_body(tool, &mut output_body, width, i18n, failure_text);
            out.append(&mut output_body);
            if !output_copy_text.trim().is_empty() {
                visible_copy_sections.push(format!("Output\n{output_copy_text}"));
            }
        }
        Some(ToolExecutionSectionRender {
            start_line: section_start,
            end_line: out.len(),
            copy_text: format!("Output\n{output_copy_text}"),
            expanded: output_expanded,
        })
    };

    ToolExecutionRender {
        headline_end,
        input,
        output,
        visible_copy_text: visible_copy_sections
            .into_iter()
            .filter(|section| !section.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

fn render_tool_output_body(
    tool: &OperationPartResource,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    failure_text: Option<&str>,
) {
    // The human view is a single Markdown document the tool produced; it takes
    // precedence over the flat model preview. The model preview stays available
    // below only when there is no richer human presentation.
    let human_markdown = tool.result.human.markdown.as_str();
    if !human_markdown.trim().is_empty() && failure_text.is_none() {
        push_expanded_markdown(out, "      ", human_markdown, width);
    } else if should_render_tool_model_output(tool, failure_text) {
        render_expanded_tool_text_block(out, "      ", tool.model_output.text.as_str(), width);
    }

    render_presentation_sections(tool, out, width);

    render_operation_attachments(tool, out, width, i18n);

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
            Style::default().fg(agena_tui_components::theme::muted_color()),
            width,
        );
        push_expanded_diff_text(out, "    ", diff, width);
    }

    render_operation_blocks(
        tool.blocks.as_slice(),
        out,
        width,
        i18n,
        true,
        failure_text,
        apply_patch
            .as_ref()
            .is_some_and(|payload| !payload.changes.is_empty()),
    );
}

fn tool_output_section_copy_text(
    tool: &OperationPartResource,
    i18n: &I18n,
    failure_text: Option<&str>,
) -> String {
    let human_markdown = tool.result.human.markdown.trim();
    let model_output = tool.model_output.text.trim();
    let mut sections = Vec::new();
    if !human_markdown.is_empty() && failure_text.is_none() {
        sections.push(human_markdown.to_owned());
    } else if should_render_tool_model_output(tool, failure_text) && !model_output.is_empty() {
        sections.push(model_output.to_owned());
    }

    let summary = tool.summary.trim();
    for section in &tool.result.display.sections {
        let title = section.title.trim();
        let text = section.text.trim();
        if title.is_empty()
            || text.is_empty()
            || text == model_output
            || text == summary
            || sections.iter().any(|existing| existing == text)
        {
            continue;
        }
        sections.push(format!("{title}\n{text}"));
    }

    if let Some(diff) = apply_patch_details(&tool.details)
        .map(|payload| payload.diff)
        .filter(|diff| !diff.trim().is_empty())
    {
        sections.push(diff.trim().to_owned());
    }
    sections.extend(
        tool.blocks
            .iter()
            .map(|block| operation_block_copy_text(block, i18n))
            .filter(|text| {
                !text.trim().is_empty()
                    && failure_text.is_none_or(|failure| text.trim() != failure.trim())
            }),
    );
    sections
        .into_iter()
        .filter(|section| !section.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn patch_rendered_lines_style(lines: &mut [RenderedLine], style: Style) {
    for line in lines {
        line.style = line.style.patch(style);
        if let Some(rich_line) = line.rich_line.take() {
            line.rich_line = Some(rich_line.patch_style(style));
        }
    }
}

fn render_presentation_sections(
    tool: &OperationPartResource,
    out: &mut Vec<RenderedLine>,
    width: u16,
) {
    let primary = tool.model_output.text.trim();
    let summary = tool.summary.trim();
    let mut rendered = Vec::new();
    for section in &tool.result.display.sections {
        let title = section.title.trim();
        let text = section.text.trim();
        if title.is_empty() || text.is_empty() || text == primary || text == summary {
            continue;
        }
        let key = (title, text);
        if rendered.contains(&key) {
            continue;
        }
        rendered.push(key);
        push_section_heading(
            out,
            format!("    › {title}").as_str(),
            Style::default()
                .fg(agena_tui_components::theme::special_color())
                .add_modifier(Modifier::BOLD),
            width,
        );
        push_expanded_markdown(out, "      ", text, width);
    }
}

fn render_operation_attachments(
    tool: &OperationPartResource,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    let mut seen: Vec<&agena_api::resource::PartAttachment> = Vec::new();
    let mut attachments = Vec::new();
    for item in tool
        .attachments
        .iter()
        .chain(tool.model_output.attachments.iter())
        .chain(tool.result.attachments.iter())
    {
        if seen
            .iter()
            .any(|existing| same_attachment_resource(existing, item))
        {
            continue;
        }
        seen.push(item);
        attachments.push(item);
    }
    if attachments.is_empty() {
        return;
    }
    push_section_heading(
        out,
        &format!("    {}", ui_text::t(i18n, "message-attachments")),
        Style::default()
            .fg(agena_tui_components::theme::special_color())
            .add_modifier(Modifier::BOLD),
        width,
    );
    for item in attachments {
        if !render_attachment_image(out, "      ", item, width) {
            push_label_value(
                out,
                "      - ",
                attachment_label(item).as_str(),
                Style::default(),
                width,
            );
        }
    }
}

fn same_attachment_resource(
    left: &agena_api::resource::PartAttachment,
    right: &agena_api::resource::PartAttachment,
) -> bool {
    left.kind == right.kind
        && (left.source == right.source
            || left.sha256.as_deref().is_some_and(|digest| {
                !digest.is_empty() && right.sha256.as_deref() == Some(digest)
            }))
}

fn attachment_label(item: &agena_api::resource::PartAttachment) -> String {
    item.title
        .as_ref()
        .or(item.filename.as_ref())
        .cloned()
        .unwrap_or_else(|| item.mime.clone())
}

fn is_interaction_notification(tool: &OperationPartResource) -> bool {
    tool.result
        .metadata
        .get("agena.effect")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|effect| effect == "notification")
        || matches!(
            compact_tool_identity(&tool.invocation).0.as_str(),
            "interaction.notify" | "agena_interaction_notify" | "interaction_notify"
        )
}

fn render_interaction_notification(
    tool: &OperationPartResource,
    out: &mut Vec<RenderedLine>,
    width: u16,
    expanded: bool,
) {
    let level = tool
        .result
        .metadata
        .get("agena.notification.level")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("info");
    let (icon, color) = match level {
        "success" => ("●", agena_tui_components::theme::success_color()),
        "warning" => ("▲", agena_tui_components::theme::warning_color()),
        "error" => ("◆", agena_tui_components::theme::danger_color()),
        _ => ("●", agena_tui_components::theme::info_color()),
    };
    let title = tool_display_label(tool);
    if !expanded {
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
    push_expanded_markdown(out, "  │  ", tool.model_output.text.as_str(), width);
    push_single_line(
        out,
        "  ╰─ ",
        level,
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
}

pub(crate) fn render_operation_blocks(
    blocks: &[OperationBlockResource],
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
    skipped_text: Option<&str>,
    skip_file_changes: bool,
) {
    for block in blocks {
        match block {
            OperationBlockResource::Text { text } => {
                if skipped_text.is_some_and(|candidate| text.trim() == candidate) {
                    continue;
                }
                if expanded {
                    render_expanded_tool_text_block(out, "    ", text, width);
                } else {
                    push_collapsible_text(out, "    ", text, Style::default(), width, i18n);
                }
            }
            OperationBlockResource::Markdown { text } => {
                if skipped_text.is_some_and(|candidate| text.trim() == candidate) {
                    continue;
                }
                if expanded {
                    push_expanded_markdown(out, "    ", text, width);
                } else {
                    push_collapsible_text(out, "    ", text, Style::default(), width, i18n);
                }
            }
            OperationBlockResource::Command {
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
                    Style::default().fg(agena_tui_components::theme::special_color()),
                    width,
                );
                if let Some(stdout) = stdout
                    && !stdout.trim().is_empty()
                {
                    if expanded {
                        push_expanded_tool_text(out, "      ", stdout, Style::default(), width);
                    } else {
                        push_collapsible_text(out, "      ", stdout, Style::default(), width, i18n);
                    }
                }
                if let Some(stderr) = stderr
                    && !stderr.trim().is_empty()
                {
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
            OperationBlockResource::Diff { diff, .. } => {
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
            OperationBlockResource::FileChanges { changes } => {
                if !skip_file_changes {
                    render_file_changes(changes, out, width, i18n)
                }
            }
            OperationBlockResource::Checklist { items } => {
                render_checklist(items, out, width, i18n)
            }
            OperationBlockResource::SearchResults { query, results } => {
                let heading = ui_text::operation_search_heading(i18n, query.as_deref());
                push_section_heading(
                    out,
                    &format!("    {heading}"),
                    Style::default()
                        .fg(agena_tui_components::theme::accent_color())
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
            OperationBlockResource::ResourceLink { uri, title, .. }
            | OperationBlockResource::Citation { uri, title, .. } => {
                push_label_value(
                    out,
                    "    - ",
                    title.as_deref().unwrap_or(uri.as_str()),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    width,
                );
            }
            OperationBlockResource::Image { url, .. }
            | OperationBlockResource::Audio { url, .. }
            | OperationBlockResource::File { url, .. } => {
                push_label_value(
                    out,
                    "    - ",
                    url.as_str(),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    width,
                );
            }
            OperationBlockResource::EmbeddedResource { uri, text, .. } => {
                push_label_value(
                    out,
                    "    - ",
                    uri.as_str(),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    width,
                );
                if let Some(text) = text
                    && !text.trim().is_empty()
                {
                    if expanded {
                        push_expanded_tool_text(out, "      ", text, Style::default(), width);
                    } else {
                        push_collapsible_text(out, "      ", text, Style::default(), width, i18n);
                    }
                }
            }
            OperationBlockResource::Media { artifact, .. } => {
                push_label_value(
                    out,
                    "    - ",
                    artifact.name.as_deref().unwrap_or(artifact.uri.as_str()),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    width,
                );
            }
            OperationBlockResource::Progress { message, .. } => {
                if expanded {
                    push_expanded_tool_text(
                        out,
                        "    ",
                        message,
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                    );
                } else {
                    push_collapsible_text(
                        out,
                        "    ",
                        message,
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                        i18n,
                    );
                }
            }
            OperationBlockResource::NestedTask {
                task_id,
                title,
                status,
            } => {
                let title = title.as_deref().unwrap_or(task_id.as_str());
                push_label_value(
                    out,
                    "    - ",
                    &format!("{} {title}", nested_task_status_icon(*status)),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    width,
                );
            }
            OperationBlockResource::Json { value } => {
                let text =
                    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
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
            OperationBlockResource::Table { columns, rows } => {
                let headings = columns
                    .iter()
                    .map(|column| column.label.as_deref().unwrap_or(column.key.as_str()))
                    .collect::<Vec<_>>();
                let mut table = String::new();
                table.push_str(&format!(
                    "| {} |\n",
                    headings.iter().copied().collect::<Vec<_>>().join(" | ")
                ));
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
            OperationBlockResource::Log { stream, text } => {
                let is_stderr = stream.as_deref().is_some_and(|stream| {
                    stream.eq_ignore_ascii_case("stderr") || stream.eq_ignore_ascii_case("err")
                });
                let style = if is_stderr {
                    Style::default().fg(agena_tui_components::theme::danger_color())
                } else {
                    Style::default().fg(agena_tui_components::theme::muted_color())
                };
                if let Some(stream) = stream
                    && !stream.trim().is_empty()
                {
                    push_label_value(
                        out,
                        "    ",
                        &format!("[{stream}]"),
                        Style::default().fg(agena_tui_components::theme::accent_color()),
                        width,
                    );
                }
                if expanded {
                    push_expanded_tool_text(out, "      ", text, style, width);
                } else {
                    push_collapsible_text(out, "      ", text, style, width, i18n);
                }
            }
            OperationBlockResource::Custom { schema: _, value } => {
                // Object payloads (e.g. a plugin's `presentation` map) read
                // best as nested bullets; any other shape falls back to
                // pretty-printed JSON.
                if value.is_object() {
                    let text = json_value_to_markdown(value);
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
                        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
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

fn nested_task_status_icon(status: PartExecutionStatusResource) -> &'static str {
    match status {
        PartExecutionStatusResource::Pending => "○",
        PartExecutionStatusResource::InProgress => "…",
        PartExecutionStatusResource::Completed => "●",
        PartExecutionStatusResource::PolicyDenied => "⊘",
        PartExecutionStatusResource::UserDeclined => "–",
        PartExecutionStatusResource::CapabilityUnavailable
        | PartExecutionStatusResource::ToolUnavailable => "◇",
        PartExecutionStatusResource::Failed => "×",
        PartExecutionStatusResource::Cancelled => "–",
    }
}
