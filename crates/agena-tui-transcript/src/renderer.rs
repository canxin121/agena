use agena_api::resource::{MessageRole, MessageStatus, SessionExecutionResource};
use agena_tui::i18n::I18n;
use agena_tui_components::trim_empty_line_edges;
use chrono::{DateTime, Local};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use crate::ui_text;
use crate::{
    RenderedCopySegment, RenderedLine, RenderedTranscriptNode, ToolOutputPreview,
    TranscriptDetailDefaults, TranscriptEntry, TranscriptNodeKey, TranscriptNodeKind,
    TranscriptPartContent, TranscriptPointerSelection, transcript_spinner_placeholder,
};

mod transcript_ast;
mod transcript_aux;
mod transcript_diff;
mod transcript_render;
mod transcript_text;
mod transcript_tool_summary;

pub(super) use self::transcript_aux::*;
pub(super) use self::transcript_diff::*;
pub use self::transcript_render::*;
pub use self::transcript_render::{
    render_entry_export, render_transcript_snapshot_export_markdown, rewind_message_preview,
};
pub use self::transcript_text::*;
pub use self::transcript_tool_summary::*;
pub(super) use crate::{
    TableColumnAlignment, fit_table_column_widths, is_markdown_table_header,
    markdown_fence_delimiter, push_markdown, push_multiline, push_table_border, push_wrapped_line,
    push_wrapped_rich_line, sanitize_terminal_text, strip_terminal_ansi_sequences,
    truncate_display_width, wrap_rich_line,
};

pub(crate) const TOOL_CARD_PREVIEW_LINES: usize = 8;
pub(crate) const TOOL_CARD_PREVIEW_CHARS: usize = 2_500;

pub(crate) fn format_timestamp(timestamp: DateTime<chrono::Utc>) -> String {
    DateTime::<Local>::from(timestamp)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

pub fn style_for_role(role: MessageRole) -> Style {
    match role {
        agena_api::resource::MessageRole::User => {
            Style::default().fg(agena_tui_components::theme::success_color())
        }
        agena_api::resource::MessageRole::Assistant => {
            Style::default().fg(agena_tui_components::theme::accent_color())
        }
        agena_api::resource::MessageRole::System => {
            Style::default().fg(agena_tui_components::theme::special_color())
        }
        agena_api::resource::MessageRole::Tool => {
            Style::default().fg(agena_tui_components::theme::warning_color())
        }
    }
}

pub fn render_entry_detailed(
    message: &TranscriptEntry,
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
    let cancelled_response =
        message.role == MessageRole::Assistant && message.state == MessageStatus::Cancelled;
    if parts.is_empty() {
        let body_start = lines.len();
        lines.push(RenderedLine::dim(format!(
            "  {}",
            ui_text::t(
                i18n,
                if cancelled_response {
                    "message-activity-response-cancelled"
                } else {
                    "message-empty"
                }
            )
        )));
        nodes.push(RenderedTranscriptNode {
            key: TranscriptNodeKey::Content {
                entry_id: message.id,
                content_id: None,
            },
            kind: TranscriptNodeKind::Message,
            start_line: body_start,
            end_line: lines.len(),
            copy_text: String::new(),
            atomic: true,
            toggleable: false,
            expanded: true,
        });
    } else {
        let mut part_index = 0_usize;
        while part_index < parts.len() {
            // A permission/user-input request is lifecycle state for the
            // operation carrying the same operation id. Rendering both
            // records creates the misleading impression that one tool call
            // was parsed as two independent calls. The operation already
            // carries the current title and status (for example, "Awaiting
            // permission: session.get"), and remains the single selectable
            // transcript item for that call.
            if interactive_request_is_embedded_in_operation(parts, part_index) {
                part_index += 1;
                continue;
            }
            if message.role != MessageRole::User
                && let Some(run_end) = collapsed_activity_run_end(parts, part_index)
            {
                let activities = parts[part_index..run_end]
                    .iter()
                    .filter(|part| is_activity_node(part))
                    .collect::<Vec<_>>();
                let hidden_count = activities
                    .len()
                    .saturating_sub(COLLAPSED_ACTIVITY_VISIBLE_COUNT);
                if hidden_count > 0 {
                    let key = TranscriptNodeKey::ActivitySummary {
                        entry_id: message.id,
                        first_content_id: activities[0].id,
                        last_content_id: activities.last().expect("non-empty activity run").id,
                    };
                    let expanded = expansions.get(&key).copied().unwrap_or(false);
                    // Message headers belong exclusively to the message-level
                    // parent selection. An activity summary must never make
                    // the adjacent `assistant` header look selected.
                    let start_line = lines.len();
                    let summary = i18n.text_args(
                        if expanded {
                            "message-activity-run-expanded"
                        } else {
                            "message-activity-run-collapsed"
                        },
                        &agena_tui::fl_args!("count" => hidden_count as i64),
                    );
                    push_single_line(
                        &mut lines,
                        "  ",
                        summary.as_str(),
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                        width,
                    );
                    nodes.push(RenderedTranscriptNode {
                        key,
                        kind: TranscriptNodeKind::Activity,
                        start_line,
                        end_line: lines.len(),
                        copy_text: activities
                            .iter()
                            .take(hidden_count)
                            .filter_map(|part| activity_copy_text(part, i18n))
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                        atomic: true,
                        toggleable: true,
                        expanded,
                    });
                    let first_visible = if expanded { 0 } else { hidden_count };
                    for part in activities.into_iter().skip(first_visible) {
                        append_rendered_part_node(
                            message, part, width, &mut lines, &mut nodes, i18n, defaults,
                            expansions,
                        );
                    }
                } else {
                    for part in activities {
                        append_rendered_part_node(
                            message, part, width, &mut lines, &mut nodes, i18n, defaults,
                            expansions,
                        );
                    }
                }
                part_index = run_end;
                continue;
            }

            let part = &parts[part_index];
            if let TranscriptPartContent::Text(text) = transcript_part_content(part) {
                let blocks = markdown_blocks(text.text.as_str());
                for (block_index, block) in blocks.iter().enumerate() {
                    if should_suppress_markdown_block(blocks.as_slice(), block_index) {
                        continue;
                    }
                    if block.leading_blank_line && lines.len() > header_start.saturating_add(1) {
                        lines.push(
                            RenderedLine::plain("  ".to_string(), Style::default())
                                .with_copy_projection(String::new(), 2),
                        );
                    }

                    // Keep the message header outside Markdown selections.  A selected code
                    // block or list should be exactly that block, both visually and on copy.
                    let start_line = lines.len();
                    render_markdown_block(&mut lines, "  ", block, width);
                    if lines.len() > start_line {
                        let rendered_block = &lines[start_line..];
                        let semantic_unit_count = rendered_block
                            .iter()
                            .filter_map(|line| line.navigation_unit)
                            .fold((None, 0_usize), |(previous, count), unit| {
                                (
                                    Some(unit),
                                    count.saturating_add(usize::from(previous != Some(unit))),
                                )
                            })
                            .1;
                        let atomic = if block.kind == TranscriptNodeKind::MarkdownMath {
                            semantic_unit_count <= 1
                        } else {
                            block.kind.uses_atomic_navigation()
                        };
                        nodes.push(RenderedTranscriptNode {
                            key: TranscriptNodeKey::MarkdownBlock {
                                entry_id: message.id,
                                content_id: part.id,
                                block_index,
                            },
                            kind: block.kind,
                            start_line,
                            end_line: lines.len(),
                            copy_text: block.copy_text.clone(),
                            atomic,
                            toggleable: false,
                            expanded: true,
                        });
                    }
                }
                part_index += 1;
                continue;
            }

            append_rendered_part_node(
                message, part, width, &mut lines, &mut nodes, i18n, defaults, expansions,
            );
            part_index += 1;
        }
        if cancelled_response {
            let start_line = lines.len();
            let text = ui_text::t(i18n, "message-activity-response-cancelled");
            lines.push(RenderedLine::dim(format!("  {text}")));
            nodes.push(RenderedTranscriptNode {
                key: TranscriptNodeKey::Content {
                    entry_id: message.id,
                    content_id: None,
                },
                kind: TranscriptNodeKind::Message,
                start_line,
                end_line: lines.len(),
                copy_text: text,
                atomic: true,
                toggleable: false,
                expanded: true,
            });
        }
    }

    RenderedMessageBlock { lines, nodes }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        I18n, Line, MessageStatus, TRANSCRIPT_EXPORT_WIDTH, TranscriptDetailDefaults,
        TranscriptEntry, TranscriptNodeKey, TranscriptNodeKind, UnicodeWidthStr,
        activity_status_icon, collapsed_activity_run_end,
        interactive_request_is_embedded_in_operation, markdown_blocks, refresh_spinner_line,
        render_entry_detailed, render_entry_export, render_markdown_block, render_tool_execution,
        render_transcript_entries_export_markdown, should_suppress_markdown_block, spinner_frame,
        thinking_collapsed_summary, tool_execution_compact_summary, tool_invocation_label,
        transcript_spinner_placeholder,
    };
    use crate::{
        OperationPartResource, PartExecutionStatusResource, ToolInvocationResource,
        TranscriptContentId, TranscriptEntryId, TranscriptEntryPart, TranscriptFixture,
    };
    use agena_domain::ExecutionStatus;
    use chrono::{DateTime, Utc};

    fn operation_resource(part: &crate::TranscriptEntryPart) -> &crate::OperationPartResource {
        match &part.content {
            crate::TranscriptPartContent::Operation(operation) => operation,
            _ => panic!("fixture must contain an operation"),
        }
    }

    fn entry(
        id: i64,
        role: agena_api::resource::MessageRole,
        state: MessageStatus,
        created_at: DateTime<Utc>,
        parts: Vec<TranscriptEntryPart>,
    ) -> TranscriptEntry {
        TranscriptEntry {
            id: TranscriptEntryId::StoredMessage(id),
            role,
            state,
            created_at,
            parts,
        }
    }

    fn fixture_operation(call_id: i64, name: &str, title: &str) -> OperationPartResource {
        OperationPartResource {
            call_id,
            invocation: ToolInvocationResource {
                name: name.to_owned(),
                ..Default::default()
            },
            title: title.to_owned(),
            ..Default::default()
        }
    }

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
    fn paragraphs_with_inline_rich_graphics_are_not_atomic_navigation_blocks() {
        let blocks = markdown_blocks("before $\\frac{a}{b}$ after");

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, TranscriptNodeKind::MarkdownParagraph);
        assert!(!blocks[0].kind.uses_atomic_navigation());
    }

    #[test]
    fn collapsed_thinking_is_a_single_line_preview() {
        let preview = thinking_collapsed_summary(
            PartExecutionStatusResource::Completed,
            "\nfirst thought\nsecond thought\n",
        );

        assert_eq!(preview, "● thinking · first thought …");
        assert!(!preview.contains('\n'));
    }

    #[test]
    fn expanded_thinking_renders_every_reasoning_line() {
        let now = Utc::now();
        let reasoning = (0..64)
            .map(|index| format!("reasoning line {index:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let parts = vec![TranscriptFixture::reasoning_part(
            21,
            7,
            now,
            ExecutionStatus::Completed,
            agena_domain::ReasoningPart {
                summary: vec![reasoning],
                raw_content: Vec::new(),
                encrypted_content: None,
            },
        )];
        let message = entry(
            7,
            agena_api::resource::MessageRole::Assistant,
            MessageStatus::Completed,
            now,
            parts,
        );

        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            TranscriptDetailDefaults {
                activity_expanded: true,
            },
            &Default::default(),
        );
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("reasoning line 63"), "{text}");
        assert!(!text.contains("more lines"), "{text}");
    }

    #[test]
    fn failed_activity_is_persistent_visible_content() {
        let now = Utc::now();
        let activity = crate::TranscriptActivityPresentation {
            title: "Response failed".to_owned(),
            summary: "provider unavailable".to_owned(),
            problem: Some(
                agena_failure::Failure::new(
                    agena_failure::FailureCode::new("provider.unavailable"),
                    agena_failure::FailureCategory::DependencyUnavailable,
                    agena_failure::FailureResponsibility::Dependency,
                    agena_failure::RetryDirective::Backoff,
                    agena_failure::RecoveryDirective::Retry,
                    agena_failure::FailureImpact::OperationFailed,
                    agena_failure::UserPresentation::new(
                        "provider-unavailable",
                        "The provider is temporarily unavailable.",
                    ),
                )
                .into(),
            ),
        };
        let parts = vec![TranscriptFixture::activity(
            21,
            7,
            now,
            ExecutionStatus::Failed,
            activity,
        )];
        let content_id = parts[0].id;
        let message = entry(
            7,
            agena_api::resource::MessageRole::System,
            MessageStatus::Failed,
            now,
            parts,
        );
        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            TranscriptDetailDefaults {
                activity_expanded: true,
            },
            &Default::default(),
        );
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Response failed"), "{text}");
        assert!(text.contains("provider unavailable"), "{text}");
        assert!(rendered.nodes.iter().any(|node| {
            matches!(
                &node.key,
                TranscriptNodeKey::Activity {
                    content_id: rendered_content_id,
                    ..
                } if rendered_content_id == &content_id
            )
        }));
    }

    #[test]
    fn consecutive_activities_collapse_old_items_and_keep_the_latest_five_visible() {
        let now = Utc::now();
        let parts = vec![
            TranscriptFixture::reasoning_part(
                21,
                7,
                now,
                ExecutionStatus::Completed,
                agena_domain::ReasoningPart {
                    summary: vec!["first thought".to_string()],
                    raw_content: Vec::new(),
                    encrypted_content: None,
                },
            ),
            TranscriptFixture::operation_part(
                22,
                7,
                now,
                ExecutionStatus::Completed,
                fixture_operation(1, "agena.fs.read", "Read file"),
            ),
            TranscriptFixture::reasoning_part(
                23,
                7,
                now,
                ExecutionStatus::Completed,
                agena_domain::ReasoningPart {
                    summary: vec!["third activity".to_string()],
                    raw_content: Vec::new(),
                    encrypted_content: None,
                },
            ),
            TranscriptFixture::operation_part(
                24,
                7,
                now,
                ExecutionStatus::Completed,
                fixture_operation(2, "agena.fs.write", "Write file"),
            ),
            TranscriptFixture::reasoning_part(
                25,
                7,
                now,
                ExecutionStatus::Completed,
                agena_domain::ReasoningPart {
                    summary: vec!["fifth activity".to_string()],
                    raw_content: Vec::new(),
                    encrypted_content: None,
                },
            ),
            TranscriptFixture::operation_part(
                26,
                7,
                now,
                ExecutionStatus::Completed,
                fixture_operation(3, "agena.fs.search", "Search files"),
            ),
            TranscriptFixture::reasoning_part(
                27,
                7,
                now,
                ExecutionStatus::Completed,
                agena_domain::ReasoningPart {
                    summary: vec!["latest activity".to_string()],
                    raw_content: Vec::new(),
                    encrypted_content: None,
                },
            ),
        ];
        let message = entry(
            7,
            agena_api::resource::MessageRole::Assistant,
            MessageStatus::Completed,
            now,
            parts,
        );

        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            TranscriptDetailDefaults {
                activity_expanded: true,
            },
            &Default::default(),
        );
        let activity_nodes = rendered
            .nodes
            .iter()
            .filter(|node| node.kind == TranscriptNodeKind::Activity)
            .collect::<Vec<_>>();
        assert_eq!(activity_nodes.len(), 6);
        assert!(activity_nodes[0].toggleable);
        assert!(!activity_nodes[0].expanded);
        assert!(rendered.lines.iter().any(|line| line.text.contains("2")));
        assert!(
            rendered
                .lines
                .iter()
                .all(|line| !line.text.contains("first thought"))
        );
        assert!(
            rendered
                .lines
                .iter()
                .all(|line| !line.text.contains("Read file"))
        );
        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line.text.contains("latest activity"))
        );

        let expansion = std::collections::BTreeMap::from([(
            TranscriptNodeKey::ActivitySummary {
                entry_id: TranscriptEntryId::StoredMessage(7),
                first_content_id: TranscriptContentId::StoredPart(21),
                last_content_id: TranscriptContentId::StoredPart(27),
            },
            true,
        )]);
        let expanded = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            TranscriptDetailDefaults {
                activity_expanded: true,
            },
            &expansion,
        );
        assert!(
            expanded
                .lines
                .iter()
                .any(|line| line.text.contains("first thought"))
        );
        assert!(expanded.nodes.iter().all(|node| {
            !matches!(node.key, TranscriptNodeKey::Activity { .. })
                || node.kind == TranscriptNodeKind::Activity
        }));
    }

    #[test]
    fn activity_status_icons_use_a_single_width_spinner_and_stable_terminal_symbols() {
        assert_eq!(spinner_frame(0), "⠋");
        assert_eq!(spinner_frame(100), "⠙");
        assert_eq!(spinner_frame(900), "⠏");
        assert_eq!(
            activity_status_icon(PartExecutionStatusResource::Pending),
            "○"
        );
        assert_eq!(
            activity_status_icon(PartExecutionStatusResource::InProgress),
            transcript_spinner_placeholder()
        );
        assert_eq!(
            activity_status_icon(PartExecutionStatusResource::Completed),
            "●"
        );
        assert_eq!(
            activity_status_icon(PartExecutionStatusResource::Failed),
            "×"
        );
        assert_eq!(
            activity_status_icon(PartExecutionStatusResource::Cancelled),
            "–"
        );

        let refreshed = refresh_spinner_line(
            Line::from(format!("assistant {}", transcript_spinner_placeholder())),
            "⠴",
        );
        assert_eq!(refreshed.to_string(), "assistant ⠴");
    }

    #[test]
    fn in_progress_message_header_uses_a_spinner_instead_of_state_text() {
        let now = Utc::now();
        let message = entry(
            1,
            agena_api::resource::MessageRole::Assistant,
            MessageStatus::InProgress,
            now,
            Vec::new(),
        );

        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        assert_eq!(
            rendered.lines[0].text,
            format!("assistant {}", transcript_spinner_placeholder())
        );
        assert_eq!(
            refresh_spinner_line(
                rendered.lines[0]
                    .rich_line
                    .clone()
                    .expect("header should keep its rich line"),
                "⠋",
            )
            .to_string(),
            "assistant ⠋"
        );
        assert!(!rendered.lines[0].text.contains("in_progress"));
        assert_eq!(
            rendered.nodes[0].start_line, 1,
            "the empty-message body node must start after the role header"
        );
    }

    #[test]
    fn tool_api_calls_show_the_model_action_and_execution_tool() {
        let input = agena_api::message_part::StructuredObjectResource {
            fields: vec![agena_api::message_part::StructuredFieldResource {
                name: "tool".to_owned(),
                value: agena_api::message_part::StructuredValueResource::Text {
                    value: "web.search".to_owned(),
                },
            }],
        };
        let invocation = crate::ToolInvocationResource {
            name: "tools_call".to_owned(),
            input,
            ..Default::default()
        };

        assert_eq!(
            tool_invocation_label(&invocation),
            "tools.call · web.search"
        );
    }

    #[test]
    fn interaction_notifications_render_as_markdown_cards() {
        let operation = OperationPartResource {
            call_id: 7,
            invocation: ToolInvocationResource {
                name: "agena.interaction.notify".to_owned(),
                ..Default::default()
            },
            title: "Production ready".to_owned(),
            model_output: agena_api::message_part::ModelVisibleOutputResource {
                text: "**Deployment finished**".to_owned(),
                ..Default::default()
            },
            result: agena_api::message_part::ToolResultEnvelopeResource {
                metadata: std::collections::BTreeMap::from([(
                    "agena.notification.level".to_owned(),
                    serde_json::Value::String("success".to_owned()),
                )]),
                ..Default::default()
            },
            ..Default::default()
        };
        let part = TranscriptFixture::operation_part(
            9,
            3,
            Utc::now(),
            ExecutionStatus::Completed,
            operation,
        );
        let mut rendered = Vec::new();
        render_tool_execution(
            &part,
            operation_resource(&part),
            &mut rendered,
            80,
            &I18n::english(),
            true,
        );
        let text = rendered
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("╭─ ● Production ready"));
        assert!(text.contains("Deployment finished"));
        assert!(text.contains("╰─ success"));
    }

    #[test]
    fn expanded_tool_output_has_no_secondary_preview_limit() {
        let output = (0..45)
            .map(|index| {
                let suffix = if index == 44 { " final sentinel" } else { "" };
                format!("output line {index:02} {}{suffix}", "x".repeat(400))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let operation = OperationPartResource {
            call_id: 7,
            invocation: ToolInvocationResource {
                name: "agena.test".to_owned(),
                ..Default::default()
            },
            blocks: vec![agena_api::message_part::OperationBlockResource::Text { text: output }],
            ..Default::default()
        };
        let part = TranscriptFixture::operation_part(
            9,
            3,
            Utc::now(),
            ExecutionStatus::Completed,
            operation,
        );
        let mut rendered = Vec::new();

        render_tool_execution(
            &part,
            operation_resource(&part),
            &mut rendered,
            80,
            &I18n::english(),
            true,
        );
        let text = rendered
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("output line 44"), "{text}");
        assert!(text.contains("final sentinel"), "{text}");
        assert!(!text.contains("more lines"), "{text}");
    }

    #[test]
    fn tool_image_attachments_render_once_through_the_rich_content_pipeline() {
        let png = concat!(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk",
            "+A8AAQUBAScY42YAAAAASUVORK5CYII="
        )
        .to_owned();
        let attachment = agena_api::resource::MessageAttachment {
            kind: agena_api::resource::MessageAttachmentKind::Image,
            mime: "image/png".to_owned(),
            source: agena_api::resource::MessageAttachmentSource::Base64 { data: png.clone() },
            filename: Some("pixel.png".to_owned()),
            title: None,
            size_bytes: None,
            sha256: None,
            width: Some(1),
            height: Some(1),
            duration_ms: None,
            page_count: None,
        };
        let operation = OperationPartResource {
            call_id: 7,
            invocation: ToolInvocationResource {
                name: "agena.image".to_owned(),
                ..Default::default()
            },
            model_output: agena_api::message_part::ModelVisibleOutputResource {
                text: "created an image".to_owned(),
                ..Default::default()
            },
            blocks: vec![
                agena_api::message_part::OperationBlockResource::EmbeddedResource {
                    uri: "pixel.png".to_owned(),
                    mime: "image/png".to_owned(),
                    text: None,
                    base64: Some(png),
                },
            ],
            attachments: vec![attachment],
            ..Default::default()
        };
        let part = TranscriptFixture::operation_part(
            9,
            3,
            Utc::now(),
            ExecutionStatus::Completed,
            operation,
        );
        let mut rendered = Vec::new();
        render_tool_execution(
            &part,
            operation_resource(&part),
            &mut rendered,
            80,
            &I18n::english(),
            true,
        );
        let text = rendered
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("attachments"), "{text}");
        assert_eq!(text.matches("pixel.png").count(), 2, "{text}");
        assert!(text.contains("embedded image"));
    }

    #[test]
    fn tool_image_attachment_without_a_block_keeps_its_attachment_section() {
        let attachment = agena_api::resource::MessageAttachment {
            kind: agena_api::resource::MessageAttachmentKind::Image,
            mime: "image/png".to_owned(),
            source: agena_api::resource::MessageAttachmentSource::Url {
                url: "https://example.com/pixel.png".to_owned(),
            },
            filename: Some("pixel.png".to_owned()),
            title: None,
            size_bytes: None,
            sha256: None,
            width: Some(1),
            height: Some(1),
            duration_ms: None,
            page_count: None,
        };
        let operation = OperationPartResource {
            call_id: 7,
            invocation: ToolInvocationResource {
                name: "agena.image".to_owned(),
                ..Default::default()
            },
            model_output: agena_api::message_part::ModelVisibleOutputResource {
                text: "created an image".to_owned(),
                ..Default::default()
            },
            attachments: vec![attachment],
            ..Default::default()
        };
        let part = TranscriptFixture::operation_part(
            9,
            3,
            Utc::now(),
            ExecutionStatus::Completed,
            operation,
        );
        let mut rendered = Vec::new();
        render_tool_execution(
            &part,
            operation_resource(&part),
            &mut rendered,
            80,
            &I18n::english(),
            true,
        );
        let text = rendered
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("attachments"), "{text}");
        assert_eq!(text.matches("pixel.png").count(), 2, "{text}");
        assert!(text.contains("https://example.com/pixel.png"), "{text}");
    }

    #[test]
    fn folded_apply_patch_reports_paths_and_line_delta() {
        let input = agena_api::message_part::StructuredObjectResource {
            fields: vec![
                agena_api::message_part::StructuredFieldResource {
                    name: "tool".to_owned(),
                    value: agena_api::message_part::StructuredValueResource::Text {
                        value: "fs.apply_patch".to_owned(),
                    },
                },
                agena_api::message_part::StructuredFieldResource {
                    name: "input".to_owned(),
                    value: agena_api::message_part::StructuredValueResource::Object {
                        fields: vec![agena_api::message_part::StructuredFieldResource {
                            name: "patch".to_owned(),
                            value: agena_api::message_part::StructuredValueResource::Text {
                                value: "*** Begin Patch\n*** Update File: apps/agena-cli/src/app.rs\n@@\n-old\n+new\n*** End Patch".to_owned(),
                            },
                        }],
                    },
                },
            ],
        };
        let tool = OperationPartResource {
            call_id: 0,
            invocation: ToolInvocationResource {
                name: "tools_call".to_owned(),
                input,
                ..Default::default()
            },
            title: "Tool tools.call".to_owned(),
            ..Default::default()
        };

        assert_eq!(
            tool_execution_compact_summary(PartExecutionStatusResource::Completed, &tool,),
            "● fs.apply_patch · M apps/agena-cli/src/app.rs · +1 −1"
        );
    }

    #[test]
    fn folded_tool_unwraps_tool_api_calls_and_reports_result_count() {
        let input = agena_api::message_part::StructuredObjectResource {
            fields: vec![
                agena_api::message_part::StructuredFieldResource {
                    name: "tool".to_owned(),
                    value: agena_api::message_part::StructuredValueResource::Text {
                        value: "fs.grep".to_owned(),
                    },
                },
                agena_api::message_part::StructuredFieldResource {
                    name: "input".to_owned(),
                    value: agena_api::message_part::StructuredValueResource::Object {
                        fields: vec![
                            agena_api::message_part::StructuredFieldResource {
                                name: "pattern".to_owned(),
                                value: agena_api::message_part::StructuredValueResource::Text {
                                    value: "TODO".to_owned(),
                                },
                            },
                            agena_api::message_part::StructuredFieldResource {
                                name: "path".to_owned(),
                                value: agena_api::message_part::StructuredValueResource::Text {
                                    value: "crates".to_owned(),
                                },
                            },
                            agena_api::message_part::StructuredFieldResource {
                                name: "include".to_owned(),
                                value: agena_api::message_part::StructuredValueResource::Text {
                                    value: "*.rs".to_owned(),
                                },
                            },
                        ],
                    },
                },
            ],
        };
        let tool = OperationPartResource {
            call_id: 0,
            invocation: ToolInvocationResource {
                name: "tools_call".to_owned(),
                input,
                ..Default::default()
            },
            title: "Tool tools.call".to_owned(),
            details: agena_api::message_part::ToolOutputResource {
                payload: agena_api::message_part::StructuredObjectResource {
                    fields: vec![agena_api::message_part::StructuredFieldResource {
                        name: "matches".to_owned(),
                        value: agena_api::message_part::StructuredValueResource::Integer {
                            value: 36,
                        },
                    }],
                },
                ..Default::default()
            },
            ..Default::default()
        };

        assert_eq!(
            tool_execution_compact_summary(PartExecutionStatusResource::Completed, &tool,),
            "● fs.grep · /TODO/ in crates · *.rs · 36 matches"
        );
    }

    #[test]
    fn folded_tool_keeps_failure_reason_on_the_same_line() {
        let input = agena_api::message_part::StructuredObjectResource {
            fields: vec![agena_api::message_part::StructuredFieldResource {
                name: "file_path".to_owned(),
                value: agena_api::message_part::StructuredValueResource::Text {
                    value: "secrets.env".to_owned(),
                },
            }],
        };
        let tool = OperationPartResource {
            call_id: 0,
            invocation: ToolInvocationResource {
                name: "agena.fs.read".to_owned(),
                input,
                ..Default::default()
            },
            title: "Read file".to_owned(),
            error: Some(agena_api::message_part::OperationErrorResource {
                failure: agena_failure::Failure::new(
                    agena_failure::FailureCode::new("tool.permission_denied"),
                    agena_failure::FailureCategory::PermissionDenied,
                    agena_failure::FailureResponsibility::Policy,
                    agena_failure::RetryDirective::AfterUserAction,
                    agena_failure::RecoveryDirective::RequestPermission,
                    agena_failure::FailureImpact::OperationFailed,
                    agena_failure::UserPresentation::new(
                        "tool-permission-denied",
                        "permission denied by workspace policy",
                    ),
                )
                .into(),
            }),
            ..Default::default()
        };

        assert_eq!(
            tool_execution_compact_summary(PartExecutionStatusResource::Failed, &tool,),
            "× fs.read · secrets.env · permission denied by workspace policy"
        );
    }

    #[test]
    fn folded_tool_makes_pending_permission_actionable() {
        let tool = OperationPartResource {
            call_id: 0,
            invocation: ToolInvocationResource {
                name: "agena.lsp.servers".to_owned(),
                ..Default::default()
            },
            title: "Awaiting permission: tool 'agena.lsp.servers' requires confirmation".to_owned(),
            ..Default::default()
        };

        assert_eq!(
            tool_execution_compact_summary(PartExecutionStatusResource::Pending, &tool,),
            "○ lsp.servers · awaiting approval"
        );
    }

    #[test]
    fn permission_request_for_a_tool_is_rendered_as_part_of_that_tool_not_a_second_call() {
        let now = Utc::now();
        let operation_id = "call_outer".to_string();
        let mut operation = TranscriptFixture::operation_part(
            10,
            7,
            now,
            ExecutionStatus::Pending,
            OperationPartResource {
                call_id: 0,
                invocation: ToolInvocationResource {
                    name: "tools_call".to_owned(),
                    ..Default::default()
                },
                title: "Awaiting permission: session.get".to_owned(),
                ..Default::default()
            },
        );
        operation.operation_id = Some(operation_id.clone());

        let request = agena_api::resource::PermissionRequest {
            request_id: "host-permission:7:0:1".to_string(),
            session_id: Some(7),
            action: agena_api::resource::PermissionActionResource::Tool {
                tool_name: "agena.session.get".to_string(),
                qualifier: None,
            },
            related_actions: Vec::new(),
            requested_actions: Vec::new(),
            reason: "approval required".to_string(),
            explanation: String::new(),
            source: None,
            scope: None,
            operator: None,
            risk: agena_api::resource::PermissionRiskLevel::Medium,
            trace: Vec::new(),
            created_at: now,
        };
        let mut permission = TranscriptFixture::permission_request_part(
            11,
            7,
            now,
            ExecutionStatus::Pending,
            request,
        );
        permission.operation_id = Some(operation_id);

        let empty_text = TranscriptFixture::text_part(12, 7, now, ExecutionStatus::Completed, "");

        let trailing_activity = TranscriptFixture::reasoning_part(
            13,
            7,
            now,
            ExecutionStatus::Completed,
            agena_domain::ReasoningPart {
                summary: vec!["continue after approval".to_string()],
                raw_content: Vec::new(),
                encrypted_content: None,
            },
        );
        let parts = vec![operation, permission, empty_text, trailing_activity];
        assert!(!interactive_request_is_embedded_in_operation(
            parts.as_slice(),
            0
        ));
        assert!(interactive_request_is_embedded_in_operation(
            parts.as_slice(),
            1
        ));
        assert_eq!(collapsed_activity_run_end(parts.as_slice(), 0), Some(4));
    }

    #[test]
    fn code_blocks_render_as_bounded_numbered_cards_that_wrap_without_truncation() {
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
        assert!(lines.iter().all(|line| !line.text.contains('…')));
        assert!(
            lines.len() >= 4,
            "long code line should span multiple card rows"
        );
        assert!(lines.last().is_some_and(|line| line.text.ends_with('┘')));
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= 28)
        );
        let code_background = agena_tui_components::theme::active_palette().code_bg;
        assert!(lines.iter().all(|line| {
            line.rich_line.as_ref().is_some_and(|rich| {
                rich.spans
                    .iter()
                    .skip(1)
                    .all(|span| span.style.bg == Some(code_background))
            })
        }));
    }

    #[test]
    fn transcript_exports_never_materialize_unbounded_terminal_rules() {
        let now = Utc::now();
        let message = entry(
            42,
            agena_api::resource::MessageRole::Assistant,
            MessageStatus::Completed,
            now,
            vec![TranscriptFixture::text_part(
                1,
                42,
                now,
                ExecutionStatus::Completed,
                concat!(
                    "# Export fixture\n\n",
                    "---\n\n",
                    "```rust\n",
                    "let value = \"a deliberately long line that must stay bounded in exports\";\n",
                    "```\n\n",
                    "$$\n",
                    "\\operatorname{rank}(A)=\\sqrt[3]{8}\n",
                    "$$\n\n",
                    "$$\n",
                    "\\definitelyunsupported{x}\n",
                    "$$",
                ),
            )],
        );

        let rendered = render_entry_export(
            &message,
            &I18n::english(),
            TranscriptDetailDefaults {
                activity_expanded: true,
            },
        );
        assert!(rendered.iter().all(|line| {
            UnicodeWidthStr::width(line.text.as_str()) <= usize::from(TRANSCRIPT_EXPORT_WIDTH)
        }));

        let markdown = render_transcript_entries_export_markdown(
            &I18n::english(),
            Some(7),
            "Export fixture",
            None,
            std::slice::from_ref(&message),
        );
        assert!(markdown.len() < 32 * 1024, "export unexpectedly ballooned");
        assert!(
            markdown.contains('√') && markdown.contains("rank"),
            "supported extended formulas must render semantically: {markdown}"
        );
        assert!(
            markdown.contains(r"\definitelyunsupported{x}"),
            "unsupported formulas must remain readable as LaTeX source: {markdown}"
        );
        assert!(
            !markdown
                .chars()
                .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
            "transcript exports must never contain Braille raster cells: {markdown}"
        );
        assert!(
            markdown.lines().all(|line| {
                UnicodeWidthStr::width(line) <= usize::from(TRANSCRIPT_EXPORT_WIDTH)
            })
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
    fn markdown_tables_render_a_compact_full_grid() {
        let block =
            markdown_blocks("| key | value |\n| --- | ---: |\n| first | 1 |\n| second | 2 |")
                .pop()
                .expect("table block");
        let mut table = Vec::new();
        render_markdown_block(&mut table, "", &block, 40);

        let rendered = table
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            rendered,
            vec![
                "┌────────┬───────┐",
                "│ key    │ value │",
                "├────────┼───────┤",
                "│ first  │     1 │",
                "├────────┼───────┤",
                "│ second │     2 │",
                "└────────┴───────┘",
            ]
        );
    }

    #[test]
    fn markdown_tables_size_columns_from_their_rendered_link_text() {
        let block = markdown_blocks(concat!(
            "| item | documentation |\n",
            "| --- | --- |\n",
            "| Agena | [guide](https://example.com/a/long/documentation/path/that/needs/room) |",
        ))
        .pop()
        .expect("table block");
        let mut table = Vec::new();
        render_markdown_block(&mut table, "  ", &block, 72);

        assert_eq!(
            UnicodeWidthStr::width(table[0].text.as_str()),
            72,
            "a table whose rendered cells exceed their Markdown labels should use the available width"
        );
        assert!(
            table
                .iter()
                .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= 72)
        );
        assert!(
            table.len() < 10,
            "the link destination should not be wrapped inside an artificially narrow column: {table:#?}"
        );
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

    #[test]
    fn markdown_math_blocks_are_independently_navigable() {
        let blocks =
            markdown_blocks("Before\n\n$$\n\\frac{a}{b}\n$$\n\n```math\n\\sqrt{x}\n```\n\nAfter");

        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0].kind, TranscriptNodeKind::MarkdownParagraph);
        assert_eq!(blocks[1].kind, TranscriptNodeKind::MarkdownMath);
        assert_eq!(blocks[2].kind, TranscriptNodeKind::MarkdownMath);
        assert_eq!(blocks[3].kind, TranscriptNodeKind::MarkdownParagraph);
    }
}
use self::{
    activity_status_icon, interactive_request_is_embedded_in_operation,
    should_suppress_markdown_block, tool_invocation_label,
};
