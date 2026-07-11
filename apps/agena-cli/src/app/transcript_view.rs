use agena::message::{ExecutionStatus, FileChangeKind, MessageStatus, OperationBlock, RequestPart};
use agena_tui_components::{line_plain_text, trim_empty_line_edges};
use textwrap::{Options as WrapOptions, WordSplitter, wrap};
use tui_markdown::from_str as markdown_to_text;
use unicode_width::UnicodeWidthStr;

use crate::app::TranscriptNodeKind;

mod transcript_aux;
mod transcript_diff;
mod transcript_markdown;
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

const COLLAPSED_ACTIVITY_VISIBLE_TAIL: usize = 5;

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
            start_line: header_start,
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
            // permission: runtime.get"), and remains the single selectable
            // transcript item for that call.
            if interactive_request_is_embedded_in_operation(parts, part_index) {
                part_index += 1;
                continue;
            }
            if let Some(run_end) =
                collapsed_activity_run_end(message, parts, part_index, defaults, expansions)
            {
                let run = &parts[part_index..run_end];
                if run.len() > COLLAPSED_ACTIVITY_VISIBLE_TAIL {
                    let key = TranscriptNodeKey::ActivitySummary {
                        message_id: message.id,
                        first_part_id: run[0].id,
                        last_part_id: run.last().expect("non-empty activity run").id,
                    };
                    let expanded = expansions.get(&key).copied().unwrap_or(false);
                    let hidden_count = run.len().saturating_sub(COLLAPSED_ACTIVITY_VISIBLE_TAIL);
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
                        &crate::fl_args!("count" => (if expanded { run.len() } else { hidden_count }) as i64),
                    );
                    push_single_line(
                        &mut lines,
                        "  ",
                        summary.as_str(),
                        Style::default().fg(Color::DarkGray),
                        width,
                    );
                    nodes.push(RenderedTranscriptNode {
                        key,
                        kind: TranscriptNodeKind::Activity,
                        start_line,
                        end_line: lines.len(),
                        copy_text: run
                            .iter()
                            .take(if expanded { run.len() } else { hidden_count })
                            .filter_map(|part| activity_part_copy_text(part, i18n))
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                        toggleable: true,
                        expanded,
                    });
                    let first_visible =
                        collapsed_activity_visible_start(part_index, run_end, expanded);
                    for part in &parts[first_visible..run_end] {
                        append_rendered_part_node(
                            message, part, width, &mut lines, &mut nodes, i18n, defaults,
                            expansions,
                        );
                    }
                } else {
                    for part in run {
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
                    render_markdown_block(&mut lines, "  ", &block, width);
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
mod tests {
    use super::{
        ExecutionStatus, MessagePart, OperationPart, PartContent, RequestPart, ToolInvocation,
        TranscriptNodeKind, UnicodeWidthStr, activity_status_icon,
        collapsed_activity_visible_start, interactive_request_is_embedded_in_operation,
        markdown_blocks, render_markdown_block, should_suppress_markdown_block, spinner_frame,
        thinking_collapsed_summary, tool_execution_compact_summary, tool_invocation_label,
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
    fn collapsed_activity_runs_only_keep_the_latest_few_blocks_visible() {
        assert_eq!(collapsed_activity_visible_start(4, 12, false), 7);
        assert_eq!(collapsed_activity_visible_start(4, 12, true), 4);
        assert_eq!(collapsed_activity_visible_start(4, 6, false), 4);
    }

    #[test]
    fn activity_status_icons_use_a_single_width_spinner_and_stable_terminal_symbols() {
        assert_eq!(spinner_frame(0), "⠋");
        assert_eq!(spinner_frame(100), "⠙");
        assert_eq!(spinner_frame(900), "⠏");
        assert_eq!(activity_status_icon(ExecutionStatus::Pending), "○");
        assert_eq!(activity_status_icon(ExecutionStatus::Completed), "●");
        assert_eq!(activity_status_icon(ExecutionStatus::Failed), "×");
        assert_eq!(activity_status_icon(ExecutionStatus::Cancelled), "–");
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
                "Awaiting permission: runtime.get",
                Default::default(),
            )),
        );
        operation.operation_id = Some(operation_id.clone());

        let request = PermissionRequest {
            request_id: "host-permission:7:0:1".to_string(),
            session_id: Some(7),
            action: PermissionAction::Tool {
                tool_name: "agena.runtime.get".to_string(),
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

        let parts = vec![operation, permission];
        assert!(!interactive_request_is_embedded_in_operation(
            parts.as_slice(),
            0
        ));
        assert!(interactive_request_is_embedded_in_operation(
            parts.as_slice(),
            1
        ));
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
}
use self::{
    activity_status_icon, collapsed_activity_visible_start,
    interactive_request_is_embedded_in_operation, render_markdown_block,
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
