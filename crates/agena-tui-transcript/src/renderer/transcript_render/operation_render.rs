use super::super::transcript_ast::render_attachment_image;
use super::super::{
    I18n, Modifier, RenderedLine, Style, apply_patch_details, compact_tool_identity, diff_stats,
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

pub(crate) fn render_tool_execution(
    part: &TranscriptEntryPart,
    tool: &OperationPartResource,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) {
    if part.status == PartExecutionStatusResource::Completed && is_interaction_notification(tool) {
        render_interaction_notification(tool, out, width, expanded);
        return;
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
        return;
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
    }

    // The human view is a single Markdown document the tool produced; it takes
    // precedence over the flat model preview. The model preview stays available
    // below only when there is no richer human presentation.
    let human_markdown = tool.result.human.markdown.as_str();
    if !human_markdown.trim().is_empty() && failure_text.is_none() {
        push_section_heading(
            out,
            "    › Output",
            Style::default()
                .fg(agena_tui_components::theme::special_color())
                .add_modifier(Modifier::BOLD),
            width,
        );
        if expanded {
            push_expanded_markdown(out, "      ", human_markdown, width);
        } else {
            push_collapsible_text(out, "      ", human_markdown, Style::default(), width, i18n);
        }
    } else if should_render_tool_model_output(tool, failure_text) {
        push_section_heading(
            out,
            "    › Output",
            Style::default()
                .fg(agena_tui_components::theme::special_color())
                .add_modifier(Modifier::BOLD),
            width,
        );
        if expanded {
            render_expanded_tool_text_block(out, "      ", tool.model_output.text.as_str(), width);
        } else {
            push_collapsible_text(
                out,
                "      ",
                tool.model_output.text.as_str(),
                Style::default(),
                width,
                i18n,
            );
        }
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
        expanded,
        failure_text,
        apply_patch
            .as_ref()
            .is_some_and(|payload| !payload.changes.is_empty()),
    );
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
            OperationBlockResource::Json { .. }
            | OperationBlockResource::Table { .. }
            | OperationBlockResource::Log { .. }
            | OperationBlockResource::Custom { .. } => {}
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
