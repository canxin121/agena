//! Transcript renderer driving the terminal frame.

use agena_api::resource::{RunRole, RunStatus, SessionExecutionResource};
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
pub use self::transcript_render::{render_entry_export, rewind_message_preview};
pub use self::transcript_text::*;
pub use self::transcript_tool_summary::*;
pub(super) use crate::{
    TableColumnAlignment, fit_table_column_widths, is_markdown_table_header,
    markdown_fence_delimiter, push_multiline, push_table_border, push_wrapped_line,
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

/// Compact wall-clock time for one transcript event row, e.g. `14:32:05`.
pub(crate) fn format_occurred_time(occurred_at_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(occurred_at_ms)
        .map(|time| DateTime::<Local>::from(time).format("%H:%M:%S").to_string())
        .unwrap_or_default()
}

pub fn style_for_role(role: RunRole) -> Style {
    match role {
        agena_api::resource::RunRole::User => {
            Style::default().fg(agena_tui_components::theme::success_color())
        }
        agena_api::resource::RunRole::Assistant => {
            Style::default().fg(agena_tui_components::theme::accent_color())
        }
        agena_api::resource::RunRole::System => {
            Style::default().fg(agena_tui_components::theme::special_color())
        }
        agena_api::resource::RunRole::Tool => {
            Style::default().fg(agena_tui_components::theme::warning_color())
        }
    }
}

pub fn render_entry_detailed(
    message: &TranscriptEntry,
    width: u16,
    i18n: &I18n,
    defaults: &TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
) -> RenderedMessageBlock {
    render_entry_detailed_with_interactions(
        message,
        width,
        i18n,
        defaults,
        expansions,
        &std::collections::BTreeMap::new(),
    )
}

/// Like [`render_entry_detailed`], but with inline interaction views for
/// pending user-input parts. The map is keyed by `request_id`; when an
/// expanded pending interaction part has an entry, the renderer draws the
/// native plan + decision rows from the wire request and the live selection
/// snapshot instead of the plain awaiting summary.
#[allow(clippy::too_many_arguments)]
pub fn render_entry_detailed_with_interactions(
    message: &TranscriptEntry,
    width: u16,
    i18n: &I18n,
    defaults: &TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
    interactions: &std::collections::BTreeMap<
        String,
        crate::interaction_view::PendingInteractionView,
    >,
) -> RenderedMessageBlock {
    render_entry_detailed_with_progressive_expansion(
        message,
        width,
        i18n,
        defaults,
        expansions,
        &std::collections::BTreeMap::new(),
        interactions,
    )
}

/// Like [`render_entry_detailed_with_interactions`], but allows the caller to
/// reveal a bounded number of older activities from each folded run. A `true`
/// entry in `expansions` means "show all" for the ordinary per-part expansion
/// map.
#[allow(clippy::too_many_arguments)]
pub fn render_entry_detailed_with_progressive_expansion(
    message: &TranscriptEntry,
    width: u16,
    i18n: &I18n,
    defaults: &TranscriptDetailDefaults,
    expansions: &std::collections::BTreeMap<TranscriptNodeKey, bool>,
    summary_visible_counts: &std::collections::BTreeMap<TranscriptNodeKey, usize>,
    interactions: &std::collections::BTreeMap<
        String,
        crate::interaction_view::PendingInteractionView,
    >,
) -> RenderedMessageBlock {
    let mut lines = Vec::new();
    let mut nodes = Vec::new();
    let header_start = lines.len();
    if message.role.is_some() {
        push_message_header(&mut lines, message, width, i18n);
    }

    let parts = transcript_message_parts(message);
    if parts.is_empty() {
        let body_start = lines.len();
        lines.push(RenderedLine::dim(format!(
            "  {}",
            ui_text::t(i18n, "message-empty")
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
            if message.role != Some(RunRole::User)
                && let Some(run_end) = collapsed_activity_run_end(parts, part_index)
            {
                let activities = parts[part_index..run_end]
                    .iter()
                    .filter(|part| is_activity_node(part))
                    .collect::<Vec<_>>();
                // Every activity folds uniformly, exactly like a consecutive
                // tool-call block: when the run exceeds the visible budget the
                // oldest activities collapse into one marker row and the newest
                // `COLLAPSED_ACTIVITY_VISIBLE_COUNT` stay visible. Session
                // notices injected mid-reply (hook runs, background notices)
                // are no exception — a long run of hook rows would otherwise
                // pile up without ever folding. The run still groups them with
                // the surrounding tool calls, so the whole block folds as one.
                let key = TranscriptNodeKey::ActivitySummary {
                    entry_id: message.id,
                    first_content_id: activities[0].id,
                };
                let foldable_count = activities.len();
                let show_all = expansions.get(&key).copied().unwrap_or(false);
                let visible_count = if show_all {
                    foldable_count
                } else {
                    summary_visible_counts
                        .get(&key)
                        .copied()
                        .unwrap_or(COLLAPSED_ACTIVITY_VISIBLE_COUNT)
                };
                let collapsed_prefix_len = foldable_count.saturating_sub(visible_count);
                let hidden_count = collapsed_prefix_len;
                // Run folding is purely positional. Individual Activity
                // expansion controls that Activity's body only and must not
                // exempt an old Activity from the collapsed prefix.
                let hidden_when_collapsed = activities
                    .iter()
                    .enumerate()
                    .map(|(foldable_index, _part)| foldable_index < collapsed_prefix_len)
                    .collect::<Vec<_>>();
                if hidden_count > 0 {
                    // Message headers belong exclusively to the message-level
                    // parent selection. An activity summary must never make
                    // the adjacent `assistant` header look selected.
                    let start_line = lines.len();
                    let summary = i18n.text_args(
                        "message-activity-run-collapsed",
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
                        // The folded run is a UI marker, never real content:
                        // copying the collapsed block must copy the visible
                        // marker line, never the hidden activities' full text.
                        copy_text: summary.clone(),
                        atomic: true,
                        toggleable: true,
                        expanded: false,
                    });
                    for (part, hidden) in activities.into_iter().zip(hidden_when_collapsed) {
                        if hidden {
                            continue;
                        }
                        append_rendered_part_node(
                            message,
                            part,
                            width,
                            &mut lines,
                            &mut nodes,
                            i18n,
                            defaults,
                            expansions,
                            interactions,
                        );
                    }
                } else {
                    for part in activities {
                        append_rendered_part_node(
                            message,
                            part,
                            width,
                            &mut lines,
                            &mut nodes,
                            i18n,
                            defaults,
                            expansions,
                            interactions,
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
                message,
                part,
                width,
                &mut lines,
                &mut nodes,
                i18n,
                defaults,
                expansions,
                interactions,
            );
            part_index += 1;
        }
    }

    RenderedMessageBlock { lines, nodes }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        I18n, Line, RunStatus, TRANSCRIPT_EXPORT_WIDTH, TranscriptDetailDefaults, TranscriptEntry,
        TranscriptNodeKey, TranscriptNodeKind, UnicodeWidthStr, activity_status_icon,
        bounded_title_summary, markdown_blocks, refresh_spinner_line, render_entry_detailed,
        render_entry_export, render_markdown_block, render_tool_execution,
        render_transcript_entries_export_markdown, should_suppress_markdown_block, spinner_frame,
        thinking_collapsed_summary, tool_execution_compact_summary, tool_invocation_label,
        transcript_spinner_placeholder,
    };
    use crate::{
        PartExecutionStatusResource, ToolCallView, TranscriptActivityContent, TranscriptContentId,
        TranscriptEntryId, TranscriptEntryPart, TranscriptFixture, TranscriptPartContent,
    };
    use agena_api::live::ToolHumanPresentationResource;
    use agena_domain::{
        AttachmentItem, AttachmentKind, AttachmentSource, ExecutionStatus, OperationError,
        RawOutput, StructuredObject, TimeRange, ToolInvocation, ToolResultState, ViewBlock,
    };
    use agena_runtime_contracts::part::OperationPart;
    use chrono::{DateTime, Utc};

    fn tool_view<'a>(part: &'a crate::TranscriptEntryPart<'a>) -> &'a crate::ToolCallView {
        match &part.content {
            crate::TranscriptPartContent::Activity(
                crate::TranscriptActivityContent::Operation(operation),
            ) => operation,
            _ => panic!("fixture must contain an operation"),
        }
    }

    fn entry(
        id: i64,
        role: agena_api::resource::RunRole,
        state: RunStatus,
        created_at: DateTime<Utc>,
        parts: Vec<TranscriptEntryPart>,
    ) -> TranscriptEntry {
        TranscriptEntry {
            id: TranscriptEntryId::StoredMessage(id),
            role: Some(role),
            state,
            created_at,
            parts,
        }
    }

    fn fixture_operation(call_id: i64, name: &str, title: &str) -> ToolCallView {
        ToolCallView::from_operation(
            OperationPart {
                call_id,
                invocation: ToolInvocation::new(name, StructuredObject::default()),
                authorization: Default::default(),
                user_input: Default::default(),
                output: None,
                state: ToolResultState::Completed,
                error: None,
                metadata: Default::default(),
                lifecycle: TimeRange::default(),
            },
            Some(ToolHumanPresentationResource {
                title: title.to_owned(),
                summary: String::new(),
                blocks: Vec::new(),
            }),
        )
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
            agena_api::resource::RunRole::Assistant,
            RunStatus::Completed,
            now,
            parts,
        );

        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            &TranscriptDetailDefaults {
                activity_default_expanded: true,
                kind_defaults: std::collections::BTreeMap::new(),
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
    fn answer_body_renders_independently_selectable_markdown_blocks() {
        // Regression: the final assistant answer used to own its whole body as
        // one opaque child section, so block/line selection and the `vim` /
        // `yam` / `gq` text objects could only grab the entire part. The body
        // must project as one MarkdownBlock node per block — the exact shape
        // plain message text uses — so it operates like body text.
        let now = Utc::now();
        let part = TranscriptEntryPart {
            id: TranscriptContentId::StoredPart(9),
            status: PartExecutionStatusResource::Completed,
            content: TranscriptPartContent::Activity(TranscriptActivityContent::Answer(Box::new(
                agena_domain::TextSegmentActivity {
                    text: "Introduction.\n\n```rust\nlet answer = 42;\n```\n\n- first\n- second"
                        .to_owned(),
                },
            ))),
        };
        let message = entry(
            3,
            agena_api::resource::RunRole::Assistant,
            RunStatus::Completed,
            now,
            vec![part],
        );

        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            &TranscriptDetailDefaults {
                activity_default_expanded: true,
                kind_defaults: std::collections::BTreeMap::new(),
            },
            &Default::default(),
        );

        // The answer keeps its toggleable headline node; the body is NOT one
        // opaque ActivitySection child.
        let activity = rendered
            .nodes
            .iter()
            .find(|node| matches!(&node.key, TranscriptNodeKey::Activity { .. }))
            .expect("answer headline node");
        assert_eq!(activity.kind, TranscriptNodeKind::Activity);
        assert!(activity.toggleable);
        assert!(activity.expanded);
        assert!(
            !rendered
                .nodes
                .iter()
                .any(|node| matches!(&node.key, TranscriptNodeKey::ActivitySection { .. })),
            "answer body must not be a single opaque section"
        );

        // One MarkdownBlock node per body block, with per-block copy text and
        // all of it living inside the expanded answer.
        let block_nodes = rendered
            .nodes
            .iter()
            .filter(|node| matches!(&node.key, TranscriptNodeKey::MarkdownBlock { .. }))
            .collect::<Vec<_>>();
        assert_eq!(block_nodes.len(), 3);
        assert_eq!(block_nodes[0].kind, TranscriptNodeKind::MarkdownParagraph);
        assert_eq!(block_nodes[1].kind, TranscriptNodeKind::MarkdownCode);
        assert_eq!(block_nodes[1].copy_text, "let answer = 42;");
        assert_eq!(block_nodes[2].kind, TranscriptNodeKind::MarkdownList);
        assert!(
            block_nodes
                .iter()
                .all(|node| node.start_line >= activity.end_line
                    && node.end_line <= rendered.lines.len())
        );
    }

    #[test]
    fn failed_activity_is_persistent_visible_content() {
        let now = Utc::now();
        let error_payload = agena_domain::ActivityPayload::Error(agena_domain::ErrorActivity {
            problem: agena_failure::Failure::new(
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
        });
        let parts = vec![TranscriptFixture::canonical_activity(
            21,
            7,
            now,
            ExecutionStatus::Failed,
            &error_payload,
        )];
        let content_id = parts[0].id;
        let message = entry(
            7,
            agena_api::resource::RunRole::System,
            RunStatus::Failed,
            now,
            parts,
        );
        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            &TranscriptDetailDefaults {
                activity_default_expanded: true,
                kind_defaults: std::collections::BTreeMap::new(),
            },
            &Default::default(),
        );
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Error"), "{text}");
        assert!(
            text.contains("The provider is temporarily unavailable."),
            "{text}"
        );
        assert!(text.contains("▾ × Error"), "{text}");
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
            agena_api::resource::RunRole::Assistant,
            RunStatus::Completed,
            now,
            parts,
        );

        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            &TranscriptDetailDefaults {
                activity_default_expanded: false,
                kind_defaults: std::collections::BTreeMap::new(),
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
            },
            true,
        )]);
        let expanded = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            &TranscriptDetailDefaults {
                activity_default_expanded: false,
                kind_defaults: std::collections::BTreeMap::new(),
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
    fn session_notice_folds_as_one_block_with_the_tool_run() {
        let now = Utc::now();
        let mut parts = (0..7)
            .map(|index| {
                TranscriptFixture::operation_part(
                    100 + index,
                    7,
                    now,
                    ExecutionStatus::Completed,
                    fixture_operation(index, "agena.fs.read", &format!("Read file {index}")),
                )
            })
            .collect::<Vec<_>>();
        // A session Notice injected mid-reply (hook run, background notice)
        // belongs to the same foldable run as the surrounding tool calls and
        // folds like any other older activity block.
        let notice_payload = agena_domain::ActivityPayload::Notice(agena_domain::NoticeActivity {
            kind: "hook".to_owned(),
            summary: "mid-reply hook fired".to_owned(),
            detail: None,
            occurred_at_ms: Some(1_700_000_000_000),
            title: None,
        });
        parts.push(TranscriptFixture::canonical_activity(
            200,
            7,
            now,
            ExecutionStatus::Completed,
            &notice_payload,
        ));
        parts.extend((0..2).map(|index| {
            TranscriptFixture::operation_part(
                210 + index,
                7,
                now,
                ExecutionStatus::Completed,
                fixture_operation(10 + index, "agena.fs.write", "Write file"),
            )
        }));
        let message = entry(
            7,
            agena_api::resource::RunRole::Assistant,
            RunStatus::Completed,
            now,
            parts,
        );

        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            &TranscriptDetailDefaults {
                activity_default_expanded: false,
                kind_defaults: std::collections::BTreeMap::new(),
            },
            &Default::default(),
        );
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        // Ten activities fold as one block to the newest five: the oldest
        // five are hidden, the hook (index 7) falls inside the newest five so
        // it renders at its chronological position, and the newest tool
        // calls remain on screen. No activity kind is exempt from the fold.
        assert!(text.starts_with("assistant"), "{text}");
        assert!(!text.starts_with("system"), "{text}");
        assert!(text.contains("Hook · mid-reply hook fired"), "{text}");
        assert!(text.contains("5 older parts hidden"), "{text}");
        let notice_line = rendered
            .lines
            .iter()
            .find(|line| line.text.contains("mid-reply hook fired"))
            .map(|line| line.text.as_str())
            .expect("notice line");
        assert!(notice_line.contains(':'), "{notice_line}");
        assert!(!text.contains("Read file 1"), "{text}");
        assert!(!text.contains("Read file 4"), "{text}");
        assert!(text.contains("Read file 5"), "{text}");
        assert_eq!(
            rendered
                .nodes
                .iter()
                .filter(|node| matches!(node.key, TranscriptNodeKey::ActivitySummary { .. }))
                .count(),
            1,
            "the tool calls must fold as a single block"
        );
        assert!(
            rendered.nodes.iter().any(|node| node.key
                == TranscriptNodeKey::Activity {
                    entry_id: TranscriptEntryId::StoredMessage(7),
                    content_id: TranscriptContentId::StoredPart(200),
                }),
            "injected session notice must render as its own Activity node"
        );
    }

    #[test]
    fn many_hook_notices_fold_keeping_the_newest_five_visible() {
        let now = Utc::now();
        let payloads = (0..9)
            .map(|index| {
                agena_domain::ActivityPayload::Notice(agena_domain::NoticeActivity {
                    kind: "hook".to_owned(),
                    summary: format!("hook run {index}"),
                    detail: Some(format!("detail {index}")),
                    occurred_at_ms: Some(1_700_000_000_000 + index),
                    title: None,
                })
            })
            .collect::<Vec<_>>();
        let parts = payloads
            .iter()
            .enumerate()
            .map(|(index, payload)| {
                TranscriptFixture::canonical_activity(
                    300 + index as i64,
                    7,
                    now,
                    ExecutionStatus::Completed,
                    payload,
                )
            })
            .collect::<Vec<_>>();
        let message = entry(
            7,
            agena_api::resource::RunRole::Assistant,
            RunStatus::Completed,
            now,
            parts,
        );

        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            &TranscriptDetailDefaults {
                activity_default_expanded: false,
                kind_defaults: std::collections::BTreeMap::new(),
            },
            &Default::default(),
        );
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        // A long run of session hook rows must fold exactly like tool calls:
        // the oldest four collapse into one marker row and the newest five
        // stay visible instead of piling up.
        assert!(text.contains("4 older parts hidden"), "{text}");
        assert!(!text.contains("hook run 0"), "{text}");
        assert!(!text.contains("hook run 1"), "{text}");
        assert!(!text.contains("hook run 2"), "{text}");
        assert!(!text.contains("hook run 3"), "{text}");
        assert!(text.contains("hook run 4"), "{text}");
        assert!(text.contains("hook run 8"), "{text}");
        assert_eq!(
            rendered
                .nodes
                .iter()
                .filter(|node| matches!(node.key, TranscriptNodeKey::ActivitySummary { .. }))
                .count(),
            1,
            "the hook rows must fold as a single block"
        );
    }

    #[test]
    fn reply_content_notice_folds_with_older_activity_blocks() {
        let now = Utc::now();
        let operation = |index: i64| {
            TranscriptFixture::operation_part(
                100 + index,
                7,
                now,
                ExecutionStatus::Completed,
                fixture_operation(index, "agena.fs.read", &format!("Read file {index}")),
            )
        };
        // A reply-content maintenance notice (prompt compaction completed,
        // recorded by the runtime with `kind == "compaction"`) is durable
        // reply content, not a mid-reply session notice. Inside a long tool
        // run it must fold with the older activity blocks instead of staying
        // pinned above the recent work.
        let compaction_payload =
            agena_domain::ActivityPayload::Notice(agena_domain::NoticeActivity {
                kind: "compaction".to_owned(),
                summary: "Prompt compaction completed".to_owned(),
                detail: Some("compaction of execution 42".to_owned()),
                occurred_at_ms: None,
                title: None,
            });
        let mut parts = vec![operation(0), operation(1)];
        parts.push(TranscriptFixture::canonical_activity(
            200,
            7,
            now,
            ExecutionStatus::Completed,
            &compaction_payload,
        ));
        parts.extend((2..11).map(operation));
        let message = entry(
            7,
            agena_api::resource::RunRole::Assistant,
            RunStatus::Completed,
            now,
            parts,
        );

        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            &TranscriptDetailDefaults {
                activity_default_expanded: false,
                kind_defaults: std::collections::BTreeMap::new(),
            },
            &Default::default(),
        );
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        // Twelve activities (the compaction notice included) fold as one block
        // to the newest five. The notice sits in the older prefix, so it is
        // hidden with the other stale blocks and never displayed above the
        // recent tool calls.
        assert!(
            !text.contains("Prompt compaction completed"),
            "stale reply-content notice must fold away, got: {text}"
        );
        assert!(text.contains("7 older parts hidden"), "{text}");
        assert!(!text.contains("Read file 5"), "{text}");
        assert!(text.contains("Read file 6"), "{text}");
        assert_eq!(
            rendered
                .nodes
                .iter()
                .filter(|node| matches!(node.key, TranscriptNodeKey::ActivitySummary { .. }))
                .count(),
            1,
            "the tool calls must fold as a single block"
        );

        // Expanding the fold reveals the notice at its chronological position
        // inside the older run.
        let summary_key = TranscriptNodeKey::ActivitySummary {
            entry_id: TranscriptEntryId::StoredMessage(7),
            first_content_id: TranscriptContentId::StoredPart(100),
        };
        let expanded = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            &TranscriptDetailDefaults {
                activity_default_expanded: false,
                kind_defaults: std::collections::BTreeMap::new(),
            },
            &std::collections::BTreeMap::from([(summary_key, true)]),
        );
        let expanded_text = expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            expanded_text.contains("Prompt compaction completed"),
            "{expanded_text}"
        );
    }

    #[test]
    fn show_all_activity_run_stays_visible_when_the_assistant_appends_an_activity() {
        let now = Utc::now();
        let activity = |part_id: i64| {
            TranscriptFixture::reasoning_part(
                part_id,
                17,
                now,
                ExecutionStatus::Completed,
                agena_domain::ReasoningPart {
                    summary: vec![format!("activity {part_id}")],
                    raw_content: Vec::new(),
                    encrypted_content: None,
                },
            )
        };
        let mut message = entry(
            17,
            agena_api::resource::RunRole::Assistant,
            RunStatus::InProgress,
            now,
            (31..37).map(activity).collect(),
        );
        let summary_key = TranscriptNodeKey::ActivitySummary {
            entry_id: TranscriptEntryId::StoredMessage(17),
            first_content_id: TranscriptContentId::StoredPart(31),
        };
        let expansions = std::collections::BTreeMap::from([(summary_key.clone(), true)]);
        let defaults = TranscriptDetailDefaults {
            activity_default_expanded: false,
            kind_defaults: std::collections::BTreeMap::new(),
        };

        let before = render_entry_detailed(&message, 80, &I18n::english(), &defaults, &expansions);
        assert!(
            before.nodes.iter().all(|node| node.key != summary_key),
            "the visibility marker disappears after show all"
        );
        assert!(before.nodes.iter().any(|node| {
            node.key
                == (TranscriptNodeKey::Activity {
                    entry_id: TranscriptEntryId::StoredMessage(17),
                    content_id: TranscriptContentId::StoredPart(31),
                })
        }));

        message.parts.push(activity(37));
        let after = render_entry_detailed(&message, 80, &I18n::english(), &defaults, &expansions);
        assert!(
            after.nodes.iter().all(|node| node.key != summary_key),
            "appending an Activity must not restore a hidden-parts marker after show all"
        );
        assert!(after.nodes.iter().any(|node| {
            node.key
                == (TranscriptNodeKey::Activity {
                    entry_id: TranscriptEntryId::StoredMessage(17),
                    content_id: TranscriptContentId::StoredPart(31),
                })
        }));
    }

    #[test]
    fn individually_expanded_activity_does_not_escape_the_count_based_fold() {
        let now = Utc::now();
        let activity = |part_id: i64| {
            TranscriptFixture::reasoning_part(
                part_id,
                18,
                now,
                ExecutionStatus::Completed,
                agena_domain::ReasoningPart {
                    summary: vec![format!("activity {part_id}")],
                    raw_content: Vec::new(),
                    encrypted_content: None,
                },
            )
        };
        let mut message = entry(
            18,
            agena_api::resource::RunRole::Assistant,
            RunStatus::InProgress,
            now,
            (41..46).map(activity).collect(),
        );
        let activity_key = TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::StoredMessage(18),
            content_id: TranscriptContentId::StoredPart(41),
        };
        let mut expansions = std::collections::BTreeMap::from([(activity_key.clone(), true)]);
        let defaults = TranscriptDetailDefaults {
            activity_default_expanded: false,
            kind_defaults: std::collections::BTreeMap::new(),
        };
        assert!(
            render_entry_detailed(&message, 80, &I18n::english(), &defaults, &expansions,)
                .nodes
                .iter()
                .find(|node| node.key == activity_key)
                .is_some_and(|node| node.expanded)
        );

        message.parts.extend([activity(46), activity(47)]);
        let after = render_entry_detailed(&message, 80, &I18n::english(), &defaults, &expansions);
        assert!(
            after.nodes.iter().all(|node| node.key != activity_key),
            "an old Activity must stay behind the count-based fold even when its own body was open"
        );
        let summary_key = after
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.key,
                    TranscriptNodeKey::ActivitySummary {
                        entry_id: TranscriptEntryId::StoredMessage(18),
                        ..
                    }
                ) && !node.expanded
            })
            .map(|node| node.key.clone())
            .expect("count-based fold summary");

        expansions.insert(summary_key, true);
        let revealed =
            render_entry_detailed(&message, 80, &I18n::english(), &defaults, &expansions);
        assert!(
            revealed
                .nodes
                .iter()
                .find(|node| node.key == activity_key)
                .is_some_and(|node| node.expanded),
            "opening the run fold reveals the Activity with its own expansion state preserved"
        );
    }

    #[test]
    fn folded_activity_run_copies_only_the_marker_not_the_hidden_activities() {
        let now = Utc::now();
        let activity = |part_id: i64| {
            TranscriptFixture::reasoning_part(
                part_id,
                19,
                now,
                ExecutionStatus::Completed,
                agena_domain::ReasoningPart {
                    summary: vec![format!("deep thought {part_id}")],
                    raw_content: Vec::new(),
                    encrypted_content: None,
                },
            )
        };
        let message = entry(
            19,
            agena_api::resource::RunRole::Assistant,
            RunStatus::Completed,
            now,
            (51..59).map(activity).collect(),
        );

        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            &TranscriptDetailDefaults {
                activity_default_expanded: false,
                kind_defaults: std::collections::BTreeMap::new(),
            },
            &Default::default(),
        );
        let summary = rendered
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.key,
                    TranscriptNodeKey::ActivitySummary {
                        entry_id: TranscriptEntryId::StoredMessage(19),
                        ..
                    }
                )
            })
            .expect("folded run summary");
        assert!(!summary.expanded);
        assert!(
            summary.copy_text.contains("older parts hidden"),
            "the folded marker text belongs in copy: {}",
            summary.copy_text
        );
        assert!(
            !summary.copy_text.contains("deep thought"),
            "a collapsed fold must never copy the hidden activities' full text: {}",
            summary.copy_text
        );
        assert!(
            !summary.contributes_to_aggregate_copy(),
            "the fold marker must never contribute to an aggregate copy"
        );
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
            activity_status_icon(PartExecutionStatusResource::PolicyDenied),
            "⊘"
        );
        assert_eq!(
            activity_status_icon(PartExecutionStatusResource::UserDeclined),
            "–"
        );
        assert_eq!(
            activity_status_icon(PartExecutionStatusResource::CapabilityUnavailable),
            "◇"
        );
        assert_eq!(
            activity_status_icon(PartExecutionStatusResource::ToolUnavailable),
            "◇"
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
            agena_api::resource::RunRole::Assistant,
            RunStatus::InProgress,
            now,
            Vec::new(),
        );

        let rendered = render_entry_detailed(
            &message,
            80,
            &I18n::english(),
            &TranscriptDetailDefaults {
                activity_default_expanded: false,
                kind_defaults: std::collections::BTreeMap::new(),
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
        let invocation = ToolInvocation::new(
            "tools_call",
            StructuredObject::try_from(serde_json::json!({
                "tool": "web.search",
            }))
            .expect("structured tool input"),
        );

        assert_eq!(
            tool_invocation_label(&invocation),
            "tools.call · web.search"
        );
    }

    #[test]
    fn interaction_notifications_render_as_markdown_cards() {
        let operation = ToolCallView::from_operation(
            OperationPart {
                call_id: 7,
                invocation: ToolInvocation::new(
                    "agena.interaction.notify",
                    StructuredObject::default(),
                ),
                authorization: Default::default(),
                user_input: Default::default(),
                output: Some(RawOutput {
                    text: "**Deployment finished**".to_owned(),
                    ..Default::default()
                }),
                state: ToolResultState::Completed,
                error: None,
                metadata: std::collections::BTreeMap::from([(
                    "agena.notification.level".to_owned(),
                    serde_json::Value::String("success".to_owned()),
                )]),
                lifecycle: TimeRange::default(),
            },
            Some(ToolHumanPresentationResource {
                title: "Production ready".to_owned(),
                summary: String::new(),
                blocks: Vec::new(),
            }),
        );
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
            tool_view(&part),
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
        let operation = ToolCallView::from_operation(
            OperationPart {
                call_id: 7,
                invocation: ToolInvocation::new("agena.test", StructuredObject::default()),
                authorization: Default::default(),
                user_input: Default::default(),
                output: None,
                state: ToolResultState::Completed,
                error: None,
                metadata: Default::default(),
                lifecycle: TimeRange::default(),
            },
            Some(ToolHumanPresentationResource {
                title: "agena.test".to_owned(),
                summary: String::new(),
                blocks: vec![ViewBlock::Text {
                    id: None,
                    text: output,
                }],
            }),
        );
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
            tool_view(&part),
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
    fn expanded_tool_input_renders_as_nested_markdown_bullets() {
        let operation = ToolCallView::from_operation(
            OperationPart {
                call_id: 7,
                invocation: ToolInvocation::new(
                    "agena.fs.read",
                    StructuredObject::try_from(serde_json::json!({
                        "file_path": "README.md",
                        "line_count": 5,
                        "options": { "follow_symlinks": true },
                    }))
                    .expect("structured tool input"),
                ),
                authorization: Default::default(),
                user_input: Default::default(),
                output: None,
                state: ToolResultState::Completed,
                error: None,
                metadata: Default::default(),
                lifecycle: TimeRange::default(),
            },
            Some(ToolHumanPresentationResource {
                title: "fs.read · Read README.md".to_owned(),
                summary: String::new(),
                blocks: Vec::new(),
            }),
        );
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
            tool_view(&part),
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
        assert!(text.contains("▾ Input"), "{text}");
        // Markdown bullets render as terminal list markers; `**` emphasis and
        // backticks are styling, not literal text.
        assert!(text.contains("• file_path: README.md"), "{text}");
        assert!(text.contains("• line_count: 5"), "{text}");
        assert!(text.contains("• options:"), "{text}");
        assert!(text.contains("◦ follow_symlinks: true"), "{text}");
    }

    #[test]
    fn exact_tool_default_overrides_the_operation_activity_kind() {
        let now = Utc::now();
        let mut operation = fixture_operation(7, "fs.read", "fs.read · README.md");
        operation.operation.invocation.plugin_name = Some("agena.fs".to_owned());
        operation.presentation.summary = "Read README.md".to_owned();
        let message = entry(
            3,
            agena_api::resource::RunRole::Assistant,
            RunStatus::Completed,
            now,
            vec![TranscriptFixture::operation_part(
                9,
                3,
                now,
                ExecutionStatus::Completed,
                operation,
            )],
        );
        let key = TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::StoredMessage(3),
            content_id: TranscriptContentId::StoredPart(9),
        };
        let defaults = TranscriptDetailDefaults {
            activity_default_expanded: true,
            kind_defaults: std::collections::BTreeMap::from([
                (agena_domain::ACTIVITY_KIND_OPERATION.to_owned(), true),
                ("tool:agena.fs.read".to_owned(), false),
            ]),
        };

        let rendered = render_entry_detailed(
            &message,
            100,
            &I18n::english(),
            &defaults,
            &Default::default(),
        );
        assert!(
            rendered
                .nodes
                .iter()
                .find(|node| node.key == key)
                .is_some_and(|node| !node.expanded),
            "the exact tool setting must win over the broad operation default"
        );
    }

    #[test]
    fn canonical_tool_call_sections_keep_presentation_open_by_default() {
        let now = Utc::now();
        let operation = ToolCallView::from_operation(
            OperationPart {
                call_id: 7,
                invocation: ToolInvocation::new(
                    "agena.shell.run",
                    StructuredObject::try_from(serde_json::json!({
                        "script": "private input sentinel",
                    }))
                    .expect("structured tool input"),
                ),
                authorization: Default::default(),
                user_input: Default::default(),
                output: Some(RawOutput::text("model output sentinel")),
                state: ToolResultState::Completed,
                error: None,
                metadata: Default::default(),
                lifecycle: TimeRange::default(),
            },
            Some(ToolHumanPresentationResource {
                title: "shell.run · Execute command".to_owned(),
                summary: String::new(),
                blocks: vec![ViewBlock::Command {
                    id: None,
                    command: "printf output".to_owned(),
                    cwd: None,
                    exit_code: Some(0),
                    stdout: "stdout sentinel".to_owned(),
                    stderr: "stderr sentinel".to_owned(),
                }],
            }),
        );
        let part =
            TranscriptFixture::operation_part(9, 3, now, ExecutionStatus::Completed, operation);
        let message = entry(
            3,
            agena_api::resource::RunRole::Assistant,
            RunStatus::Completed,
            now,
            vec![part],
        );
        let activity_key = TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::StoredMessage(3),
            content_id: TranscriptContentId::StoredPart(9),
        };
        let input_key = TranscriptNodeKey::ActivitySection {
            entry_id: TranscriptEntryId::StoredMessage(3),
            content_id: TranscriptContentId::StoredPart(9),
            section: crate::TranscriptActivitySection::Input,
        };
        let output_key = TranscriptNodeKey::ActivitySection {
            entry_id: TranscriptEntryId::StoredMessage(3),
            content_id: TranscriptContentId::StoredPart(9),
            section: crate::TranscriptActivitySection::Output,
        };
        let output_metadata_key = TranscriptNodeKey::ActivitySection {
            entry_id: TranscriptEntryId::StoredMessage(3),
            content_id: TranscriptContentId::StoredPart(9),
            section: crate::TranscriptActivitySection::OutputMetadata,
        };
        let presentation_key = TranscriptNodeKey::ActivitySection {
            entry_id: TranscriptEntryId::StoredMessage(3),
            content_id: TranscriptContentId::StoredPart(9),
            section: crate::TranscriptActivitySection::Presentation,
        };
        let defaults = TranscriptDetailDefaults {
            activity_default_expanded: true,
            kind_defaults: std::collections::BTreeMap::new(),
        };

        let folded = render_entry_detailed(
            &message,
            100,
            &I18n::english(),
            &defaults,
            &Default::default(),
        );
        let folded_text = folded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(folded_text.contains("▸ Input"), "{folded_text}");
        assert!(folded_text.contains("▸ Output"), "{folded_text}");
        assert!(folded_text.contains("▸ Output metadata"), "{folded_text}");
        assert!(folded_text.contains("▾ Presentation"), "{folded_text}");
        assert!(
            !folded_text.contains("private input sentinel"),
            "{folded_text}"
        );
        assert!(
            !folded_text.contains("model output sentinel"),
            "{folded_text}"
        );
        assert!(folded_text.contains("stdout sentinel"), "{folded_text}");
        for key in [&input_key, &output_key, &output_metadata_key] {
            let node = folded
                .nodes
                .iter()
                .find(|node| &node.key == key)
                .expect("folded operation section node");
            assert!(node.toggleable);
            assert!(!node.expanded);
        }
        let presentation = folded
            .nodes
            .iter()
            .find(|node| node.key == presentation_key)
            .expect("presentation section node");
        assert!(presentation.toggleable);
        assert!(presentation.expanded);
        let section_order = folded
            .nodes
            .iter()
            .filter_map(|node| match &node.key {
                TranscriptNodeKey::ActivitySection { section, .. }
                    if matches!(
                        section,
                        crate::TranscriptActivitySection::Metadata
                            | crate::TranscriptActivitySection::Input
                            | crate::TranscriptActivitySection::Output
                            | crate::TranscriptActivitySection::OutputMetadata
                            | crate::TranscriptActivitySection::Presentation
                    ) =>
                {
                    Some(*section)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            section_order,
            vec![
                crate::TranscriptActivitySection::Metadata,
                crate::TranscriptActivitySection::Input,
                crate::TranscriptActivitySection::Output,
                crate::TranscriptActivitySection::OutputMetadata,
                crate::TranscriptActivitySection::Presentation,
            ]
        );
        let parent = folded
            .nodes
            .iter()
            .find(|node| node.key == activity_key)
            .expect("operation Activity node");
        assert!(!parent.copy_text.contains("private input sentinel"));
        assert!(parent.copy_text.contains("stdout sentinel"));

        let input_expanded = render_entry_detailed(
            &message,
            100,
            &I18n::english(),
            &defaults,
            &std::collections::BTreeMap::from([(input_key.clone(), true)]),
        );
        let input_text = input_expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(input_text.contains("▾ Input"), "{input_text}");
        assert!(
            input_text.contains("private input sentinel"),
            "{input_text}"
        );
        assert!(input_text.contains("▸ Output"), "{input_text}");
        assert!(input_text.contains("▾ Presentation"), "{input_text}");
        assert!(
            !input_text.contains("model output sentinel"),
            "{input_text}"
        );
        assert!(input_text.contains("stdout sentinel"), "{input_text}");

        let output_expanded = render_entry_detailed(
            &message,
            100,
            &I18n::english(),
            &defaults,
            &std::collections::BTreeMap::from([(output_key.clone(), true)]),
        );
        let output_text = output_expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(output_text.contains("▸ Input"), "{output_text}");
        assert!(
            !output_text.contains("private input sentinel"),
            "{output_text}"
        );
        assert!(output_text.contains("▾ Output"), "{output_text}");
        assert!(
            output_text.contains("model output sentinel"),
            "{output_text}"
        );
        assert!(output_text.contains("stdout sentinel"), "{output_text}");
        assert!(output_text.contains("stderr sentinel"), "{output_text}");
        let parent = output_expanded
            .nodes
            .iter()
            .find(|node| node.key == activity_key)
            .expect("operation Activity node");
        assert!(parent.copy_text.contains("stdout sentinel"));
        assert!(!parent.copy_text.contains("private input sentinel"));

        let export_text = render_entry_export(&message, &I18n::english(), &defaults)
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            export_text.contains("private input sentinel"),
            "{export_text}"
        );
        assert!(
            export_text.contains("model output sentinel"),
            "{export_text}"
        );
        assert!(export_text.contains("stdout sentinel"), "{export_text}");
    }

    #[test]
    fn tools_call_input_unwraps_to_the_inner_tool_arguments() {
        let operation = ToolCallView::from_operation(
            OperationPart {
                call_id: 7,
                invocation: ToolInvocation::new(
                    "tools_call",
                    StructuredObject::try_from(serde_json::json!({
                        "tool": "web.search",
                        "input": { "query": "agena docs" },
                    }))
                    .expect("structured tool input"),
                ),
                authorization: Default::default(),
                user_input: Default::default(),
                output: None,
                state: ToolResultState::Completed,
                error: None,
                metadata: Default::default(),
                lifecycle: TimeRange::default(),
            },
            Some(ToolHumanPresentationResource {
                title: "tools.call · web.search".to_owned(),
                summary: String::new(),
                blocks: Vec::new(),
            }),
        );
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
            tool_view(&part),
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
        // The wrapper's `tool`/`input` fields must not leak; only the inner
        // tool's real arguments render.
        assert!(text.contains("• query: agena docs"), "{text}");
        assert!(!text.contains("• tool:"), "{text}");
        assert!(!text.contains("tool:"), "{text}");
    }

    #[test]
    fn json_table_log_and_custom_blocks_render_richly() {
        let operation = ToolCallView::from_operation(
            OperationPart {
                call_id: 7,
                invocation: ToolInvocation::new("agena.test", StructuredObject::default()),
                authorization: Default::default(),
                user_input: Default::default(),
                output: None,
                state: ToolResultState::Completed,
                error: None,
                metadata: Default::default(),
                lifecycle: TimeRange::default(),
            },
            Some(ToolHumanPresentationResource {
                title: "agena.test".to_owned(),
                summary: String::new(),
                blocks: vec![
                    ViewBlock::Json {
                        id: None,
                        value: serde_json::json!({ "key": "value", "n": 42 }),
                    },
                    ViewBlock::Table {
                        id: None,
                        columns: vec!["name".to_owned(), "Score".to_owned()],
                        rows: vec![
                            vec![serde_json::json!("alice"), serde_json::json!(9.5)],
                            vec![serde_json::json!("bob"), serde_json::json!(8)],
                        ],
                    },
                    ViewBlock::Log {
                        id: None,
                        stream: agena_domain::CommandOutputStream::Stderr,
                        text: "warning: something odd".to_owned(),
                    },
                    ViewBlock::Custom {
                        id: None,
                        kind: "chips".to_owned(),
                        schema: serde_json::Value::Null,
                        presentation: std::collections::BTreeMap::from([(
                            "title".to_owned(),
                            "Chips".to_owned(),
                        )]),
                    },
                ],
            }),
        );
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
            tool_view(&part),
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
        // Json block: pretty-printed JSON inside a fenced code box.
        assert!(text.contains("┌─ json"), "{text}");
        assert!(text.contains("\"key\": \"value\""), "{text}");
        // Table block: a box-drawn table carrying the column labels and rows.
        assert!(text.contains("│ name"), "{text}");
        assert!(text.contains("│ Score"), "{text}");
        assert!(text.contains("alice"), "{text}");
        assert!(text.contains("9.5"), "{text}");
        // Log block: stream label + text body.
        assert!(text.contains("[stderr]"), "{text}");
        assert!(text.contains("warning: something odd"), "{text}");
        // Custom object payload: nested bullets from the presentation map.
        assert!(text.contains("• presentation:"), "{text}");
        assert!(text.contains("◦ title: Chips"), "{text}");
    }

    #[test]
    fn tool_image_attachments_render_once_through_the_rich_content_pipeline() {
        let png = concat!(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk",
            "+A8AAQUBAScY42YAAAAASUVORK5CYII="
        )
        .to_owned();
        let attachment = AttachmentItem {
            kind: AttachmentKind::Image,
            mime: "image/png".to_owned(),
            source: AttachmentSource::Base64 { data: png.clone() },
            filename: Some("pixel.png".to_owned()),
            title: None,
            size_bytes: None,
            sha256: None,
            width: Some(1),
            height: Some(1),
            duration_ms: None,
            page_count: None,
        };
        let operation = ToolCallView::from_operation(
            OperationPart {
                call_id: 7,
                invocation: ToolInvocation::new("agena.image", StructuredObject::default()),
                authorization: Default::default(),
                user_input: Default::default(),
                output: Some(RawOutput {
                    text: "created an image".to_owned(),
                    attachments: vec![attachment],
                    ..Default::default()
                }),
                state: ToolResultState::Completed,
                error: None,
                metadata: Default::default(),
                lifecycle: TimeRange::default(),
            },
            None,
        );
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
            tool_view(&part),
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
        assert_eq!(text.matches("pixel.png").count(), 1, "{text}");
        assert!(text.contains("embedded image"));
    }

    #[test]
    fn tool_image_attachment_without_a_block_keeps_its_attachment_section() {
        let attachment = AttachmentItem {
            kind: AttachmentKind::Image,
            mime: "image/png".to_owned(),
            source: AttachmentSource::Url {
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
        let operation = ToolCallView::from_operation(
            OperationPart {
                call_id: 7,
                invocation: ToolInvocation::new("agena.image", StructuredObject::default()),
                authorization: Default::default(),
                user_input: Default::default(),
                output: Some(RawOutput {
                    text: "created an image".to_owned(),
                    attachments: vec![attachment],
                    ..Default::default()
                }),
                state: ToolResultState::Completed,
                error: None,
                metadata: Default::default(),
                lifecycle: TimeRange::default(),
            },
            None,
        );
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
            tool_view(&part),
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
    fn folded_operation_headline_uses_the_composed_operation_title() {
        let tool = ToolCallView::from_operation(
            OperationPart {
                call_id: 0,
                invocation: ToolInvocation::new(
                    "fs.apply_patch",
                    StructuredObject::try_from(serde_json::json!({
                        "tool": "fs.apply_patch",
                        "input": {
                            "patch": "*** Begin Patch\n*** Update File: apps/agena-cli/src/app.rs\n@@\n-old\n+new\n*** End Patch",
                        },
                    }))
                    .expect("structured tool input"),
                ),
                authorization: Default::default(),
                user_input: Default::default(),
                output: None,
                state: ToolResultState::Completed,
                error: None,
                metadata: Default::default(),
                lifecycle: TimeRange::default(),
            },
            Some(ToolHumanPresentationResource {
                title: "Apply patch".to_owned(),
                summary: "1 file changed · +1 −1".to_owned(),
                blocks: Vec::new(),
            }),
        );

        assert_eq!(
            tool_execution_compact_summary(PartExecutionStatusResource::Completed, &tool, 80),
            "● Apply patch · 1 file changed · +1 −1"
        );
    }

    #[test]
    fn folded_headline_preserves_title_and_spends_only_remaining_width_on_summary() {
        assert_eq!(
            bounded_title_summary("Read README.md", "86 lines", 40),
            "Read README.md · 86 lines"
        );

        let bounded = bounded_title_summary(
            "Process run Create a tiny test PNG",
            "Exit 0 · generated pixel.png with a transparent background",
            52,
        );
        assert!(bounded.starts_with("Process run Create a tiny test PNG · "));
        assert!(bounded.ends_with('…'));
        assert!(UnicodeWidthStr::width(bounded.as_str()) <= 52);

        let long_title = "Inspect a genuinely long tool subject that consumes the whole viewport";
        let bounded =
            bounded_title_summary(long_title, "This summary must not displace the title", 32);
        assert!(bounded.ends_with('…'));
        assert!(!bounded.contains("summary"));
        assert!(UnicodeWidthStr::width(bounded.as_str()) <= 32);
    }

    #[test]
    fn folded_tool_headline_shows_the_composed_operation_title_and_reports_result_count() {
        let tool = ToolCallView::from_operation(
            OperationPart {
                call_id: 0,
                invocation: ToolInvocation::new(
                    "fs.grep",
                    StructuredObject::try_from(serde_json::json!({
                        "tool": "fs.grep",
                        "input": {
                            "pattern": "TODO",
                            "path": "crates",
                            "include": "*.rs",
                        },
                    }))
                    .expect("structured tool input"),
                ),
                authorization: Default::default(),
                user_input: Default::default(),
                output: Some(RawOutput {
                    payload: Some(serde_json::json!({ "matches": 36 })),
                    ..Default::default()
                }),
                state: ToolResultState::Completed,
                error: None,
                metadata: Default::default(),
                lifecycle: TimeRange::default(),
            },
            Some(ToolHumanPresentationResource {
                title: "Grep TODO".to_owned(),
                summary: "36 matches in crates".to_owned(),
                blocks: Vec::new(),
            }),
        );

        assert_eq!(
            tool_execution_compact_summary(PartExecutionStatusResource::Completed, &tool, 80),
            "● Grep TODO · 36 matches in crates"
        );
    }

    #[test]
    fn folded_tool_keeps_failure_reason_on_the_same_line() {
        let tool = ToolCallView::from_operation(
            OperationPart {
                call_id: 0,
                invocation: ToolInvocation::new(
                    "agena.fs.read",
                    StructuredObject::try_from(serde_json::json!({
                        "file_path": "secrets.env",
                    }))
                    .expect("structured tool input"),
                ),
                authorization: Default::default(),
                user_input: Default::default(),
                output: None,
                state: ToolResultState::Failed,
                error: Some(OperationError {
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
                    ),
                }),
                metadata: Default::default(),
                lifecycle: TimeRange::default(),
            },
            Some(ToolHumanPresentationResource {
                title: "Read secrets.env".to_owned(),
                summary: "permission denied by workspace policy".to_owned(),
                blocks: Vec::new(),
            }),
        );

        assert_eq!(
            tool_execution_compact_summary(PartExecutionStatusResource::Failed, &tool, 80),
            "× Read secrets.env · permission denied by workspace policy"
        );
    }

    #[test]
    fn folded_tool_makes_pending_permission_actionable() {
        let mut tool = fixture_operation(0, "agena.lsp.servers", "Language servers");
        tool.presentation.summary = "Awaiting approval".to_owned();

        assert_eq!(
            tool_execution_compact_summary(PartExecutionStatusResource::Pending, &tool, 80),
            "○ Language servers · Awaiting approval"
        );
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
    fn unlabeled_code_fences_infer_the_language_from_the_first_line() {
        let block = markdown_blocks("```\n#!/usr/bin/env bash\necho hello\n```")
            .pop()
            .expect("code block");
        let mut lines = Vec::new();
        render_markdown_block(&mut lines, "", &block, 48);

        let label = lines
            .first()
            .map(|line| line.text.as_str())
            .unwrap_or_default()
            .to_string();
        assert!(
            label.contains("┌─ sh") || label.contains("┌─ bash"),
            "unlabeled shell fence should infer its language: {label}"
        );
    }

    #[test]
    fn prose_fences_without_a_language_hint_stay_plain() {
        let block = markdown_blocks("```\nJust some prose that is not code.\n```")
            .pop()
            .expect("code block");
        let mut lines = Vec::new();
        render_markdown_block(&mut lines, "", &block, 48);

        let label = lines
            .first()
            .map(|line| line.text.as_str())
            .unwrap_or_default()
            .to_string();
        assert!(
            label.contains("┌─ text"),
            "uninferable fences keep the plain-text label: {label}"
        );
    }

    #[test]
    fn transcript_exports_never_materialize_unbounded_terminal_rules() {
        let now = Utc::now();
        let message = entry(
            42,
            agena_api::resource::RunRole::Assistant,
            RunStatus::Completed,
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
            &TranscriptDetailDefaults {
                activity_default_expanded: true,
                kind_defaults: std::collections::BTreeMap::new(),
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
    fn wide_markdown_tables_render_horizontally_instead_of_stacking_cells() {
        let cells = (0..20).map(|index| format!("c{index}")).collect::<Vec<_>>();
        let header = format!("| {} |", cells.join(" | "));
        let separator = format!("| {} |", vec!["---"; 20].join(" | "));
        let row = format!(
            "| {} |",
            (0..20)
                .map(|index| format!("v{index}"))
                .collect::<Vec<_>>()
                .join(" | ")
        );
        let block = markdown_blocks(&format!("{header}\n{separator}\n{row}"))
            .pop()
            .expect("table block");
        let mut table = Vec::new();
        render_markdown_block(&mut table, "", &block, 60);

        let rendered = table
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("c0") && line.contains("c1") && line.contains("c2")),
            "cells should be joined on one horizontal row: {rendered:#?}"
        );
        assert!(
            !rendered.iter().any(|line| line.starts_with("├ ")),
            "cells must not stack vertically: {rendered:#?}"
        );
        let header_line = table
            .iter()
            .find(|line| line.text.contains("c0"))
            .expect("header row");
        assert_eq!(
            header_line.navigation_copy_text,
            cells.join("\t"),
            "copying a fallback row should yield tab-separated cells"
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
use self::{should_suppress_markdown_block, tool_invocation_label};
