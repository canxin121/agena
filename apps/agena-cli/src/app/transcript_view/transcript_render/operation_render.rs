use super::super::transcript_ast::render_attachment_image;
use super::super::{
    ExecutionStatus, I18n, MessagePart, Modifier, OperationBlock, OperationPart, RenderedLine,
    Style, apply_patch_details, compact_tool_identity, diff_stats, media_artifact_label,
    push_collapsible_text, push_expanded_diff_text, push_expanded_markdown,
    push_expanded_tool_text, push_label_value, push_multiline, push_section_heading,
    push_single_line, render_expanded_tool_text_block, should_render_tool_model_output,
    tool_display_label, tool_execution_collapsed_summary, tool_execution_status_summary,
    tool_status_color, ui_text,
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
    if let Some(activity) = compaction_activity(tool) {
        render_compaction_activity(&activity, out, width, i18n, expanded);
        return;
    }
    if part.status == ExecutionStatus::Completed && is_interaction_notification(tool) {
        render_interaction_notification(tool, out, width, expanded);
        return;
    }
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
            render_expanded_tool_text_block(out, "    ", tool.model_output.text.as_str(), width);
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

fn compaction_activity(tool: &OperationPart) -> Option<agena::session::PromptCompactionActivity> {
    if !tool.is_ui_only()
        || tool
            .metadata
            .get("agena.activity.kind")
            .and_then(serde_json::Value::as_str)
            != Some("compaction")
    {
        return None;
    }
    serde_json::from_value(tool.structured.clone()?).ok()
}

fn render_compaction_activity(
    activity: &agena::session::PromptCompactionActivity,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
    expanded: bool,
) {
    let title_key = match activity.trigger {
        agena::session::PromptCompactionTrigger::Manual => "message-compaction-title-manual",
        agena::session::PromptCompactionTrigger::Auto => "message-compaction-title-auto",
        agena::session::PromptCompactionTrigger::Reactive => "message-compaction-title-reactive",
    };
    let strategy_key = match activity.strategy {
        agena::session::PromptCompactionStrategy::LocalSummary => {
            "message-compaction-strategy-local"
        }
        agena::session::PromptCompactionStrategy::OpenAiResponses => {
            "message-compaction-strategy-native"
        }
    };
    let title = ui_text::t(i18n, title_key);
    let summary = i18n.text_args(
        "message-compaction-token-summary",
        &crate::fl_args!(
            "before" => activity.before_tokens as i64,
            "after" => activity.after_tokens as i64,
            "percent" => format!("{:.1}", activity.reduction_percent()),
        ),
    );
    let strategy = ui_text::t(i18n, strategy_key);
    let color = agena_tui_components::theme::success_color();

    if !expanded {
        push_single_line(
            out,
            "  ",
            format!("● {title} · {summary}").as_str(),
            Style::default().fg(color),
            width,
        );
        return;
    }

    push_single_line(
        out,
        "  ╭─ ",
        format!("● {title}").as_str(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
        width,
    );
    push_single_line(out, "  │  ", summary.as_str(), Style::default(), width);
    push_single_line(
        out,
        "  │  ",
        i18n.text_args(
            "message-compaction-strategy-line",
            &crate::fl_args!("strategy" => strategy),
        )
        .as_str(),
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
    push_single_line(
        out,
        "  ╰─ ",
        i18n.text_args(
            "message-compaction-generation-line",
            &crate::fl_args!("generation" => activity.generation as i64),
        )
        .as_str(),
        Style::default().fg(agena_tui_components::theme::muted_color()),
        width,
    );
}

fn render_operation_attachments(
    tool: &OperationPart,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    let mut seen: Vec<&agena::message::AttachmentItem> = Vec::new();
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
        if tool
            .blocks
            .iter()
            .filter_map(OperationBlock::to_attachment_item)
            .any(|block_item| same_attachment_resource(&block_item, item))
        {
            continue;
        }
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
                item.summary_label().as_str(),
                Style::default(),
                width,
            );
        }
    }
}

fn same_attachment_resource(
    left: &agena::message::AttachmentItem,
    right: &agena::message::AttachmentItem,
) -> bool {
    left.kind == right.kind
        && (left.source == right.source
            || left.sha256.as_deref().is_some_and(|digest| {
                !digest.is_empty() && right.sha256.as_deref() == Some(digest)
            }))
}

fn is_interaction_notification(tool: &OperationPart) -> bool {
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
    tool: &OperationPart,
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
        if let Some(item) = block.to_attachment_item()
            && render_attachment_image(out, "    ", &item, width)
        {
            continue;
        }
        match block {
            OperationBlock::Text { text } => {
                if skipped_text.is_some_and(|candidate| text.trim() == candidate) {
                    continue;
                }
                if expanded {
                    render_expanded_tool_text_block(out, "    ", text, width);
                } else {
                    push_collapsible_text(out, "    ", text, Style::default(), width, i18n);
                }
            }
            OperationBlock::Markdown { text } => {
                if skipped_text.is_some_and(|candidate| text.trim() == candidate) {
                    continue;
                }
                if expanded {
                    push_expanded_markdown(out, "    ", text, width);
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
            OperationBlock::Diff { diff, .. } => {
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
                        push_expanded_tool_text(out, "      ", text, Style::default(), width);
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

#[cfg(test)]
mod tests {
    use super::{compaction_activity, render_tool_execution};
    use agena::message::{
        ExecutionStatus, MessagePart, OperationPart, PartContent, StructuredObject, TimeRange,
        ToolInvocation, ToolOutput,
    };
    use chrono::Utc;

    #[test]
    fn structured_compaction_activity_is_recognized() {
        let expected = agena::session::PromptCompactionActivity {
            checkpoint_id: "checkpoint".to_owned(),
            generation: 3,
            compacted_through_message_id: 99,
            trigger: agena::session::PromptCompactionTrigger::Auto,
            strategy: agena::session::PromptCompactionStrategy::OpenAiResponses,
            before_tokens: 20_000,
            after_tokens: 4_000,
        };
        let mut operation = OperationPart::completed(
            0,
            ToolInvocation::new("session.compact", StructuredObject::default()),
            "compacted",
            Vec::new(),
            Vec::new(),
            ToolOutput::default(),
            TimeRange::default(),
        );
        operation.set_ui_only(true);
        operation.structured = Some(serde_json::to_value(&expected).expect("serialize activity"));
        operation.metadata.insert(
            "agena.activity.kind".to_owned(),
            serde_json::Value::String("compaction".to_owned()),
        );

        assert_eq!(compaction_activity(&operation), Some(expected));

        let part = MessagePart::from_content(
            1,
            1,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::Operation(operation.clone()),
        );
        let mut rendered = Vec::new();
        render_tool_execution(
            &part,
            &operation,
            &mut rendered,
            100,
            &crate::i18n::I18n::english(),
            false,
        );
        let text = rendered
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Context compacted automatically"), "{text}");
        assert!(text.contains("20000 → 4000 tokens"), "{text}");
        assert!(text.contains("80.0% reduction"), "{text}");
    }
}
