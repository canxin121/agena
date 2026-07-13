use agena::message::{ExecutionStatus, FileChangeKind, MessageStatus, OperationBlock, RequestPart};
use agena_tui_components::{line_plain_text, trim_empty_line_edges};
use textwrap::{Options as WrapOptions, WordSplitter, wrap};
use tui_markdown::from_str as markdown_to_text;
use unicode_width::UnicodeWidthStr;

use crate::app::TranscriptNodeKind;

mod transcript_ast;
mod transcript_aux;
mod transcript_diff;
mod transcript_markdown;
mod transcript_math;
mod transcript_render;
mod transcript_text;
mod transcript_tool_summary;

pub(super) use self::transcript_aux::*;
pub(super) use self::transcript_diff::*;
pub(super) use self::transcript_markdown::*;
pub(super) use self::transcript_render::*;
pub(in crate::app) use self::transcript_render::{
    render_message_export, render_transcript_export_markdown, rewind_message_preview,
    sanitize_terminal_text,
};
pub(super) use self::transcript_text::*;
pub(super) use self::transcript_tool_summary::*;

pub(in crate::app) fn render_message_detailed(
    message: &MessageResource,
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
    if parts.is_empty() {
        let body_start = lines.len();
        lines.push(RenderedLine::dim(format!(
            "  {}",
            ui_text::t(i18n, "message-empty")
        )));
        nodes.push(RenderedTranscriptNode {
            key: TranscriptNodeKey::MessagePart {
                message_id: message.id,
                part_id: None,
            },
            kind: TranscriptNodeKind::Message,
            start_line: body_start,
            end_line: lines.len(),
            copy_text: String::new(),
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
            if let Some(run_end) = collapsed_activity_run_end(parts, part_index) {
                let activity_parts = parts[part_index..run_end]
                    .iter()
                    .filter(|part| is_activity_part(part))
                    .collect::<Vec<_>>();
                let hidden_count = activity_parts
                    .len()
                    .saturating_sub(COLLAPSED_ACTIVITY_VISIBLE_COUNT);
                if hidden_count > 0 {
                    let key = TranscriptNodeKey::ActivitySummary {
                        message_id: message.id,
                        first_part_id: activity_parts[0].id,
                        last_part_id: activity_parts.last().expect("non-empty activity run").id,
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
                        &crate::fl_args!("count" => hidden_count as i64),
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
                        copy_text: activity_parts
                            .iter()
                            .take(hidden_count)
                            .filter_map(|part| activity_part_copy_text(part, i18n))
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                        toggleable: true,
                        expanded,
                    });
                    let first_visible = if expanded { 0 } else { hidden_count };
                    for part in activity_parts.into_iter().skip(first_visible) {
                        append_rendered_part_node(
                            message, part, width, &mut lines, &mut nodes, i18n, defaults,
                            expansions,
                        );
                    }
                } else {
                    for part in activity_parts {
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
            if let PartContent::Text(text) = transcript_part_content(part) {
                let blocks = markdown_blocks(text.text.as_str());
                for (block_index, block) in blocks.iter().enumerate() {
                    if should_suppress_markdown_block(blocks.as_slice(), block_index) {
                        continue;
                    }
                    if block.leading_blank_line && lines.len() > header_start.saturating_add(1) {
                        lines.push(RenderedLine::plain("  ".to_string(), Style::default()));
                    }

                    // Keep the message header outside Markdown selections.  A selected code
                    // block or list should be exactly that block, both visually and on copy.
                    let start_line = lines.len();
                    render_markdown_block(&mut lines, "  ", block, width);
                    if lines.len() > start_line {
                        nodes.push(RenderedTranscriptNode {
                            key: TranscriptNodeKey::MarkdownBlock {
                                message_id: message.id,
                                part_id: part.id,
                                block_index,
                            },
                            kind: block.kind,
                            start_line,
                            end_line: lines.len(),
                            copy_text: block.copy_text.clone(),
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
    }

    RenderedMessageBlock { lines, nodes }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        ExecutionStatus, I18n, Line, MessagePart, MessageResource, MessageStatus, OperationPart,
        PartContent, RequestPart, TRANSCRIPT_EXPORT_WIDTH, ToolInvocation,
        TranscriptDetailDefaults, TranscriptNodeKey, TranscriptNodeKind, UnicodeWidthStr,
        activity_status_icon, collapsed_activity_run_end,
        interactive_request_is_embedded_in_operation, markdown_blocks, refresh_spinner_line,
        render_markdown_block, render_message_detailed, render_message_export,
        render_tool_execution, render_transcript_export_markdown, should_suppress_markdown_block,
        spinner_frame, thinking_collapsed_summary, tool_execution_compact_summary,
        tool_invocation_label, transcript_spinner_placeholder,
    };
    use agena::permission::{PermissionAction, PermissionRequest, PermissionRiskLevel};
    use chrono::Utc;

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
    fn collapsed_thinking_is_a_single_line_preview() {
        let preview = thinking_collapsed_summary(
            ExecutionStatus::Completed,
            "\nfirst thought\nsecond thought\n",
        );

        assert_eq!(preview, "● thinking · first thought …");
        assert!(!preview.contains('\n'));
    }

    #[test]
    fn consecutive_activity_parts_collapse_old_items_and_keep_the_latest_five_visible() {
        let now = Utc::now();
        let parts = vec![
            MessagePart::from_content(
                21,
                7,
                now,
                ExecutionStatus::Completed,
                PartContent::Reasoning(agena::message::ReasoningPart {
                    summary: vec!["first thought".to_string()],
                    raw_content: Vec::new(),
                    encrypted_content: None,
                }),
            ),
            MessagePart::from_content(
                22,
                7,
                now,
                ExecutionStatus::Completed,
                PartContent::Operation(OperationPart::pending(
                    1,
                    ToolInvocation::new("agena.fs.read", Default::default()),
                    "Read file",
                    Default::default(),
                )),
            ),
            MessagePart::from_content(
                23,
                7,
                now,
                ExecutionStatus::Completed,
                PartContent::Reasoning(agena::message::ReasoningPart {
                    summary: vec!["third activity".to_string()],
                    raw_content: Vec::new(),
                    encrypted_content: None,
                }),
            ),
            MessagePart::from_content(
                24,
                7,
                now,
                ExecutionStatus::Completed,
                PartContent::Operation(OperationPart::pending(
                    2,
                    ToolInvocation::new("agena.fs.write", Default::default()),
                    "Write file",
                    Default::default(),
                )),
            ),
            MessagePart::from_content(
                25,
                7,
                now,
                ExecutionStatus::Completed,
                PartContent::Reasoning(agena::message::ReasoningPart {
                    summary: vec!["fifth activity".to_string()],
                    raw_content: Vec::new(),
                    encrypted_content: None,
                }),
            ),
            MessagePart::from_content(
                26,
                7,
                now,
                ExecutionStatus::Completed,
                PartContent::Operation(OperationPart::pending(
                    3,
                    ToolInvocation::new("agena.fs.search", Default::default()),
                    "Search files",
                    Default::default(),
                )),
            ),
            MessagePart::from_content(
                27,
                7,
                now,
                ExecutionStatus::Completed,
                PartContent::Reasoning(agena::message::ReasoningPart {
                    summary: vec!["latest activity".to_string()],
                    raw_content: Vec::new(),
                    encrypted_content: None,
                }),
            ),
        ];
        let message = MessageResource {
            id: 7,
            session_id: 3,
            role: agena_api::resource::MessageRole::Assistant,
            state: MessageStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: parts.len() as u64,
            parts: Some(parts),
        };

        let rendered = render_message_detailed(
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
                message_id: 7,
                first_part_id: 21,
                last_part_id: 27,
            },
            true,
        )]);
        let expanded = render_message_detailed(
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
            !matches!(node.key, TranscriptNodeKey::ActivityPart { .. })
                || node.kind == TranscriptNodeKind::Activity
        }));
    }

    #[test]
    fn activity_status_icons_use_a_single_width_spinner_and_stable_terminal_symbols() {
        assert_eq!(spinner_frame(0), "⠋");
        assert_eq!(spinner_frame(100), "⠙");
        assert_eq!(spinner_frame(900), "⠏");
        assert_eq!(activity_status_icon(ExecutionStatus::Pending), "○");
        assert_eq!(
            activity_status_icon(ExecutionStatus::InProgress),
            transcript_spinner_placeholder()
        );
        assert_eq!(activity_status_icon(ExecutionStatus::Completed), "●");
        assert_eq!(activity_status_icon(ExecutionStatus::Failed), "×");
        assert_eq!(activity_status_icon(ExecutionStatus::Cancelled), "–");

        let refreshed = refresh_spinner_line(
            Line::from(format!("assistant {}", transcript_spinner_placeholder())),
            "⠴",
        );
        assert_eq!(refreshed.to_string(), "assistant ⠴");
    }

    #[test]
    fn in_progress_message_header_uses_a_spinner_instead_of_state_text() {
        let now = Utc::now();
        let message = MessageResource {
            id: 1,
            session_id: 1,
            role: agena_api::resource::MessageRole::Assistant,
            state: MessageStatus::InProgress,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: 0,
            parts: Some(Vec::new()),
        };

        let rendered = render_message_detailed(
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
    fn gateway_tool_calls_show_the_model_action_and_catalog_target() {
        let input = serde_json::json!({
            "tool": "web.search",
            "input": { "query": "Agena" },
        })
        .try_into()
        .expect("structured input");
        let invocation = ToolInvocation::new("agena.tools.call", input);

        assert_eq!(
            tool_invocation_label(&invocation),
            "tools.call · web.search"
        );
    }

    #[test]
    fn interaction_notifications_render_as_markdown_cards() {
        let mut operation = OperationPart::completed(
            7,
            ToolInvocation::new("agena.interaction.notify", Default::default()),
            "**Deployment finished**",
            Vec::new(),
            Vec::new(),
            Default::default(),
            Default::default(),
        );
        operation.set_title("Production ready");
        operation.result.metadata.insert(
            "agena.notification.level".to_string(),
            serde_json::Value::String("success".to_string()),
        );
        let part = MessagePart::from_content(
            9,
            3,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::Operation(operation.clone()),
        );
        let mut rendered = Vec::new();
        render_tool_execution(&part, &operation, &mut rendered, 80, &I18n::english(), true);
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
    fn folded_apply_patch_reports_paths_and_line_delta() {
        let input = serde_json::json!({
            "tool": "fs.apply_patch",
            "input": {
                "patch": "*** Begin Patch\n*** Update File: apps/agena-cli/src/app.rs\n@@\n-old\n+new\n*** End Patch"
            }
        })
        .try_into()
        .expect("structured input");
        let tool = OperationPart::pending(
            0,
            ToolInvocation::new("agena.tools.call", input),
            "Tool tools.call",
            Default::default(),
        );

        assert_eq!(
            tool_execution_compact_summary(ExecutionStatus::Completed, &tool),
            "● fs.apply_patch · M apps/agena-cli/src/app.rs · +1 −1"
        );
    }

    #[test]
    fn folded_tool_unwraps_gateway_calls_and_reports_result_count() {
        let input = serde_json::json!({
            "tool": "fs.grep",
            "input": { "pattern": "TODO", "path": "crates", "include": "*.rs" }
        })
        .try_into()
        .expect("structured input");
        let mut tool = OperationPart::pending(
            0,
            ToolInvocation::new("agena.tools.call", input),
            "Tool tools.call",
            Default::default(),
        );
        tool.details = agena::message::ToolOutput::from_json_payload(Some(
            &serde_json::json!({ "matches": 36 }),
        ))
        .expect("tool output");

        assert_eq!(
            tool_execution_compact_summary(ExecutionStatus::Completed, &tool),
            "● fs.grep · /TODO/ in crates · *.rs · 36 matches"
        );
    }

    #[test]
    fn folded_tool_keeps_failure_reason_on_the_same_line() {
        let input = serde_json::json!({ "file_path": "secrets.env" })
            .try_into()
            .expect("structured input");
        let mut tool = OperationPart::pending(
            0,
            ToolInvocation::new("agena.fs.read", input),
            "Read file",
            Default::default(),
        );
        tool.error = Some(agena::message::OperationError {
            message: "permission denied by workspace policy".to_string(),
            code: None,
        });

        assert_eq!(
            tool_execution_compact_summary(ExecutionStatus::Failed, &tool),
            "× fs.read · secrets.env · permission denied by workspace policy"
        );
    }

    #[test]
    fn folded_tool_makes_pending_permission_actionable() {
        let tool = OperationPart::pending(
            0,
            ToolInvocation::new("agena.lsp.servers", Default::default()),
            "Awaiting permission: tool 'agena.lsp.servers' requires confirmation",
            Default::default(),
        );

        assert_eq!(
            tool_execution_compact_summary(ExecutionStatus::Pending, &tool),
            "○ lsp.servers · awaiting approval"
        );
    }

    #[test]
    fn permission_request_for_a_tool_is_rendered_as_part_of_that_tool_not_a_second_call() {
        let now = Utc::now();
        let operation_id = "call_outer".to_string();
        let mut operation = MessagePart::from_content(
            10,
            7,
            now,
            ExecutionStatus::Pending,
            PartContent::Operation(OperationPart::pending(
                0,
                ToolInvocation::new("agena.tools.call", Default::default()),
                "Awaiting permission: session.get",
                Default::default(),
            )),
        );
        operation.operation_id = Some(operation_id.clone());

        let request = PermissionRequest {
            request_id: "host-permission:7:0:1".to_string(),
            session_id: Some(7),
            action: PermissionAction::Tool {
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
            risk: PermissionRiskLevel::Medium,
            trace: Vec::new(),
            created_at: now,
        };
        let mut permission = MessagePart::from_content(
            11,
            7,
            now,
            ExecutionStatus::Pending,
            PartContent::Request(RequestPart::Permission(
                agena::message::InteractiveRequestPart::pending(request),
            )),
        );
        permission.operation_id = Some(operation_id);

        let empty_text = MessagePart::from_content(
            12,
            7,
            now,
            ExecutionStatus::Completed,
            PartContent::text(""),
        );

        let trailing_activity = MessagePart::from_content(
            13,
            7,
            now,
            ExecutionStatus::Completed,
            PartContent::Reasoning(agena::message::ReasoningPart {
                summary: vec!["continue after approval".to_string()],
                raw_content: Vec::new(),
                encrypted_content: None,
            }),
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
        let message = MessageResource {
            id: 42,
            session_id: 7,
            role: agena_api::resource::MessageRole::Assistant,
            state: MessageStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: 1,
            parts: Some(vec![MessagePart::from_content(
                1,
                42,
                now,
                ExecutionStatus::Completed,
                PartContent::text(concat!(
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
                )),
            )]),
        };

        let rendered = render_message_export(
            &message,
            &I18n::english(),
            TranscriptDetailDefaults {
                activity_expanded: true,
            },
        );
        assert!(rendered.iter().all(|line| {
            UnicodeWidthStr::width(line.text.as_str()) <= usize::from(TRANSCRIPT_EXPORT_WIDTH)
        }));

        let markdown = render_transcript_export_markdown(
            &I18n::english(),
            Some(7),
            "Export fixture",
            None,
            std::slice::from_ref(&message),
            false,
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
use crate::app::{
    Color, I18n, MessagePart, MessageResource, OperationPart, PartContent, RenderedLine,
    RenderedTranscriptNode, Style, ToolInvocation, TranscriptDetailDefaults, TranscriptNodeKey,
    ui_text,
};
use crate::app::{
    Line, Local, Modifier, SessionExecutionResource, Span, TOOL_CARD_PREVIEW_CHARS,
    TOOL_CARD_PREVIEW_LINES, TOOL_EXPANDED_PREVIEW_CHARS, TOOL_EXPANDED_PREVIEW_LINES,
    ToolOutputPreview, format_timestamp, style_for_role, truncate_display_width,
};
