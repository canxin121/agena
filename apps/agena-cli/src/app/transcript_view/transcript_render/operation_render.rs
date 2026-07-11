use super::super::{
    ExecutionStatus, I18n, MessagePart, Modifier, OperationBlock, OperationPart, RenderedLine,
    Style, apply_patch_details, diff_stats, media_artifact_label, push_collapsible_text,
    push_label_value, push_limited_diff_text, push_limited_markdown, push_limited_tool_text,
    push_multiline, push_section_heading, push_single_line, render_limited_tool_text_block,
    should_render_tool_model_output, tool_display_label, tool_execution_collapsed_summary,
    tool_execution_status_summary, tool_status_color, ui_text,
};
use super::request_render::{render_checklist, render_file_changes};

pub(in crate::app) fn render_tool_execution(
    part: &MessagePart,
    tool: &OperationPart,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) {
    let label = tool_display_label(tool);
    let color = tool_status_color(part.status);
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
        tool_execution_status_summary(part, label.as_str()).as_str(),
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
            Style::default().fg(agena_tui_components::theme::danger_color()),
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
            Style::default().fg(agena_tui_components::theme::muted_color()),
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

pub(in crate::app) fn render_operation_blocks(
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
                    Style::default().fg(agena_tui_components::theme::special_color()),
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
                            Style::default().fg(agena_tui_components::theme::danger_color()),
                            width,
                            i18n,
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
            OperationBlock::Diff { diff, .. } => {
                if expanded {
                    push_limited_diff_text(out, "    ", diff, width, i18n);
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
            OperationBlock::ResourceLink { uri, title, .. }
            | OperationBlock::Citation { uri, title, .. } => {
                push_label_value(
                    out,
                    "    - ",
                    title.as_deref().unwrap_or(uri.as_str()),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
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
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    width,
                );
            }
            OperationBlock::EmbeddedResource { uri, text, .. } => {
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
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    width,
                );
            }
            OperationBlock::Progress { message, .. } => {
                if expanded {
                    push_limited_tool_text(
                        out,
                        "    ",
                        message,
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                        i18n,
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
                    Style::default().fg(agena_tui_components::theme::muted_color()),
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
