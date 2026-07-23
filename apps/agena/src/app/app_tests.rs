use super::{
    ConfigJsonSources, I18n, JsonValue, PermissionAction, PermissionConfig, PermissionMode,
    PermissionOverlayChoice, PermissionPromptDecision, PermissionPromptPage, PermissionReplyKind,
    PermissionRequest, PermissionRuleSubjectKind, PermissionScope, PermissionStudioCatalogKind,
    PermissionStudioModeTarget, RenderedTranscriptNode, SettingsPickerAction, ToolPermissionRules,
    TranscriptMoveDirection, TranscriptNodeKey, TranscriptNodeKind, Utc,
    apply_permission_studio_entries_mode, apply_permission_studio_mode_input,
    initial_search_match_index, path_rule_modes, permission_overlay_choice,
    permission_rule_draft_from_request, settings_studio_permission_items,
    transcript_message_navigation_target, transcript_node_highlight_range,
    transcript_selection_scroll_position,
};

macro_rules! api_message_part {
    ($id:expr, $message_id:expr, $created_at:expr, $status:expr, PartContent::text($text:expr $(,)?) $(,)?) => {
        crate::app::TranscriptFixture::text_part($id, $message_id, $created_at, $status, $text)
    };
    ($id:expr, $message_id:expr, $created_at:expr, $status:expr, PartContent::Reasoning($reasoning:expr) $(,)?) => {
        crate::app::TranscriptFixture::reasoning_part(
            $id,
            $message_id,
            $created_at,
            $status,
            $reasoning,
        )
    };
    ($id:expr, $message_id:expr, $created_at:expr, $status:expr, PartContent::Text($text_part:expr) $(,)?) => {{
        let text_part = $text_part;
        crate::app::TranscriptFixture::text_part_with_flags(
            $id,
            $message_id,
            $created_at,
            $status,
            text_part.text,
            text_part.synthetic,
            text_part.ignored,
        )
    }};
}

#[cfg(test)]
mod prompt_history_tests {
    use super::super::{App, Editor, PromptHistory, PromptHistorySearchState, SearchPickerConfig};

    #[test]
    fn history_search_indexes_the_bounded_catalog_newest_first() {
        let history = PromptHistory {
            items: (0..55).map(|index| format!("prompt {index:02}")).collect(),
        };
        let mut search = PromptHistorySearchState::new(
            "History".to_string(),
            String::new(),
            String::new(),
            "No matches".to_string(),
            Editor::default(),
            SearchPickerConfig::searchable(),
            None,
            (),
        );

        App::refresh_prompt_history_search(&history, &mut search);
        assert_eq!(search.items.len(), 55);
        assert_eq!(search.items[0].text, "prompt 54");
        assert_eq!(search.items.last().expect("history item").text, "prompt 00");
        assert_eq!(search.result_count(), 55);

        search.input.set_text("prompt 54".to_string());
        search.refresh_results();
        assert!(search.result_count() < 55);
        assert_eq!(
            search.selected_item().expect("best ranked match").text,
            "prompt 54"
        );
    }
}

#[cfg(test)]
mod pending_message_tests {
    use super::super::{
        ExecutionStatus, MessageResource, MessageRole, MessageStatus, PaginatedResponse,
        PendingUserMessage, TranscriptFixture, TranscriptState, Utc,
    };

    #[test]
    fn confirmed_optimistic_message_is_atomically_replaced_by_its_persisted_message() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 42,
            text: "send this now".to_string(),
            confirmed: false,
            persisted_message_id: None,
        });

        let pending_lines = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>();
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("send this now"))
        );
        assert!(
            pending_lines
                .first()
                .is_some_and(|line| line.starts_with("user "))
        );

        transcript.confirm_pending_user_message(42);
        assert!(
            transcript
                .rendered(80)
                .lines
                .iter()
                .any(|line| line.text.contains("send this now"))
        );
        assert_eq!(transcript.rendered(80).lines[0].text, "user");

        let now = Utc::now();
        let parts = vec![TranscriptFixture::text_part(
            51,
            50,
            now,
            ExecutionStatus::Completed,
            "send this now",
        )];
        transcript.merge_latest_messages(
            PaginatedResponse {
                items: vec![MessageResource {
                    id: 50,
                    session_id: 7,
                    role: MessageRole::User,
                    state: MessageStatus::Completed,
                    created_at: now,
                    updated_at: now,
                    metadata: Default::default(),
                    usage: None,
                    part_count: parts.len() as u64,
                    parts: Some(parts),
                }],
                page: Default::default(),
            },
            80,
            20,
        );

        assert!(transcript.pending_user_messages.is_empty());
        assert_eq!(transcript.messages.len(), 1);
        assert_eq!(
            transcript
                .rendered(80)
                .lines
                .iter()
                .filter(|line| line.text.contains("send this now"))
                .count(),
            1
        );
    }
}

#[cfg(test)]
mod transcript_mouse_scroll_tests {
    use agena_domain::ExecutionStatus;

    use super::super::{
        MessageResource, MessageRole, MessageStatus, PendingUserMessage, TranscriptMoveDirection,
        TranscriptNodeKey, TranscriptState, TranscriptTextPosition, TranscriptTextSelection, Utc,
    };

    #[test]
    fn nonempty_transcript_materializes_a_visible_primary_cursor_before_rendering() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 1,
            text: (0..40)
                .map(|line| format!("line {line}"))
                .collect::<Vec<_>>()
                .join("\n\n"),
            confirmed: false,
            persisted_message_id: None,
        });
        assert_eq!(transcript.navigation_cursor_line(), None);

        transcript.ensure_visual_focus(40, 10);

        let focused_line = transcript
            .navigation_cursor_line()
            .expect("nonempty transcript should materialize a line focus");
        assert!(focused_line >= transcript.viewport.top);
        assert!(focused_line < transcript.viewport.top + 10);
        assert_eq!(focused_line, transcript.rendered(40).lines.len() - 1);
        assert!(transcript.viewport.follow_tail);
    }

    #[test]
    fn wheel_moves_the_cursor_and_preserves_its_screen_row() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 1,
            text: (0..80)
                .map(|line| format!("line {line}"))
                .collect::<Vec<_>>()
                .join("\n\n"),
            confirmed: false,
            persisted_message_id: None,
        });

        transcript.scroll_to_bottom(40, 10);
        let bottom = transcript.viewport.top;
        assert!(bottom > 3);
        assert!(transcript.viewport.follow_tail);
        let bottom_cursor = transcript.navigation_cursor_line().expect("tail cursor");
        let bottom_screen_row = bottom_cursor - bottom;

        transcript.move_cursor_by_wheel(40, 10, -3);
        assert_eq!(transcript.viewport.top, bottom - 3);
        assert!(!transcript.viewport.follow_tail);
        assert_eq!(transcript.navigation_cursor_line(), Some(bottom_cursor - 3));
        assert_eq!(
            bottom_cursor - 3 - transcript.viewport.top,
            bottom_screen_row
        );
        assert_eq!(transcript.highlighted_block_key(), None);

        transcript.move_cursor_by_wheel(40, 10, 3);
        assert_eq!(transcript.viewport.top, bottom);
        assert!(transcript.viewport.follow_tail);
        assert_eq!(transcript.navigation_cursor_line(), Some(bottom_cursor));
    }

    #[test]
    fn scrollbar_and_half_page_motion_use_directional_edge_placement() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 1,
            text: (0..80)
                .map(|line| format!("line {line}"))
                .collect::<Vec<_>>()
                .join("\n\n"),
            confirmed: false,
            persisted_message_id: None,
        });

        transcript.scroll_to_top(40, 10);
        transcript.relocate_cursor_from_scrollbar(40, 10, 5);
        assert_eq!(transcript.navigation_cursor_line(), Some(6));
        assert_eq!(transcript.viewport.top, 5);

        transcript.relocate_cursor_from_scrollbar(40, 10, 30);
        assert_eq!(transcript.navigation_cursor_line(), Some(31));
        assert_eq!(transcript.viewport.top, 30);

        transcript.move_cursor_by_half_page(40, 10, false);
        assert_eq!(transcript.navigation_cursor_line(), Some(26));
        assert_eq!(transcript.viewport.top, 18);
        assert_eq!(26 - transcript.viewport.top, 8);
    }

    #[test]
    fn scrollbar_relocation_collapses_a_block_and_selects_the_directional_edge() {
        let now = Utc::now();
        let message = |id: i64, text: String| MessageResource {
            id,
            session_id: 7,
            role: MessageRole::Assistant,
            state: MessageStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: 1,
            parts: Some(vec![api_message_part!(
                id * 10,
                id,
                now,
                ExecutionStatus::Completed,
                PartContent::text(text),
            )]),
        };
        let long_text = |prefix: &str| {
            (0..30)
                .map(|line| format!("{prefix} {line}"))
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![
                message(1, long_text("first")),
                message(2, long_text("second")),
            ],
            ..TranscriptState::default()
        };
        let second_message = transcript
            .rendered(40)
            .nodes
            .iter()
            .position(|node| node.key == TranscriptNodeKey::Message { message_id: 2 })
            .expect("second message parent");
        transcript.set_block_cursor(40, 10, second_message, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::Message { message_id: 2 })
        );

        transcript.relocate_cursor_from_scrollbar(40, 10, 0);
        assert_eq!(transcript.highlighted_block_key(), None);
        let focused_line = transcript
            .navigation_cursor_line()
            .expect("scrollbar relocation should leave a visible line target");
        assert!(focused_line < 10);
        assert!(focused_line >= 7);

        transcript.move_by_blocks(40, 10, TranscriptMoveDirection::Down, 1);
        assert_eq!(transcript.viewport.top, 0);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::Message { message_id: 1 })
        );
    }

    #[test]
    fn pointer_can_select_a_markdown_line_or_its_whole_block_and_resume_text_selection() {
        let now = Utc::now();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![MessageResource {
                id: 1,
                session_id: 7,
                role: MessageRole::Assistant,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    10,
                    1,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text(
                        "first paragraph contains enough words to wrap across several rendered lines while remaining one markdown block\n\nsecond paragraph",
                    ),
                )]),
            }],
            ..TranscriptState::default()
        };
        let (clickable_line, block_key, block_range) = {
            let node = transcript
                .rendered(40)
                .nodes
                .iter()
                .find(|node| {
                    matches!(&node.key, TranscriptNodeKey::MarkdownBlock { .. })
                        && node.end_line.saturating_sub(node.start_line) > 1
                })
                .expect("multiline markdown block");
            (
                node.start_line + 1,
                node.key.clone(),
                node.start_line..node.end_line,
            )
        };
        let position = TranscriptTextPosition {
            line: clickable_line,
            column: 3,
        };

        transcript.select_pointer_line(40, 10, position);
        assert_eq!(transcript.highlighted_block_key(), None);
        assert_eq!(transcript.navigation_cursor_line(), Some(clickable_line));
        assert!(
            transcript
                .current_selected_line_text(40)
                .is_some_and(|line| !line.is_empty())
        );

        transcript.select_pointer_block(40, 10, position);
        assert_eq!(transcript.highlighted_block_key(), Some(block_key));
        assert_eq!(
            transcript.highlighted_block_range(40),
            Some(block_range.clone())
        );
        assert_eq!(transcript.navigation_cursor_line(), Some(clickable_line));
        assert_eq!(transcript.current_selected_line_text(40), None);

        transcript.move_cursor_one_line(40, 10, TranscriptMoveDirection::Down);
        assert_eq!(transcript.highlighted_block_key(), None);
        assert_eq!(
            transcript.navigation_cursor_line(),
            Some(block_range.start),
            "continuing from a whole-block selection enters at its directional edge"
        );

        let head_line = clickable_line.saturating_add(1);
        let cursor_before_drag = transcript.navigation_cursor_line();
        let selection = transcript.set_text_selection(
            40,
            TranscriptTextSelection {
                anchor: TranscriptTextPosition {
                    line: clickable_line,
                    column: 1,
                },
                head: TranscriptTextPosition {
                    line: head_line,
                    column: 4,
                },
            },
        );
        assert_eq!(transcript.text_selection(), Some(selection));
        assert_eq!(
            transcript.navigation_cursor_line(),
            cursor_before_drag,
            "committing a pointer range must not move the navigation cursor"
        );

        transcript.cancel_text_selection(40, 10);
        assert_eq!(transcript.navigation_cursor_line(), cursor_before_drag);
        assert_eq!(transcript.text_selection(), None);

        transcript.set_text_selection(
            40,
            TranscriptTextSelection {
                anchor: TranscriptTextPosition {
                    line: clickable_line,
                    column: 1,
                },
                head: TranscriptTextPosition {
                    line: head_line,
                    column: 4,
                },
            },
        );
        transcript.select_pointer_line(
            40,
            10,
            TranscriptTextPosition {
                line: head_line,
                column: 2,
            },
        );
        assert_eq!(transcript.navigation_cursor_line(), Some(head_line));
        assert_eq!(
            transcript.text_selection(),
            None,
            "a later click moves the navigation cursor and clears the old drag range"
        );
    }

    #[test]
    fn cursor_reflow_keeps_the_same_semantic_markdown_block() {
        let now = Utc::now();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![MessageResource {
                id: 1,
                session_id: 7,
                role: MessageRole::Assistant,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    10,
                    1,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text(
                        "a deliberately long markdown paragraph whose wrapped rows change when the terminal width changes but whose semantic identity must remain stable",
                    ),
                )]),
            }],
            ..TranscriptState::default()
        };
        transcript.viewport.follow_tail = false;
        let (line, key) = {
            let node = transcript
                .rendered(24)
                .nodes
                .iter()
                .find(|node| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
                .expect("markdown block");
            (node.start_line.saturating_add(2), node.key.clone())
        };
        transcript.select_pointer_line(24, 10, TranscriptTextPosition { line, column: 2 });

        transcript.ensure_visual_focus(70, 10);

        assert_eq!(
            transcript
                .current_cursor_node_cloned(70)
                .map(|node| node.key),
            Some(key)
        );
        let cursor = transcript
            .navigation_cursor_line()
            .expect("cursor after reflow");
        assert!(cursor >= transcript.viewport.top);
        assert!(cursor < transcript.viewport.top + 10);
    }
}

#[cfg(test)]
mod transcript_expansion_tests {
    use agena_domain::ExecutionStatus;
    use agena_domain::ReasoningPart;

    use super::super::{
        MessageResource, MessageRole, MessageStatus, TranscriptMoveDirection, TranscriptNodeKey,
        TranscriptNodeKind, TranscriptState, TranscriptTextPosition, TranscriptTextSelection, Utc,
        transcript_text_selection_text,
    };

    #[test]
    fn collapsing_from_inside_an_activity_keeps_the_cursor_on_that_activity() {
        let now = Utc::now();
        let message_id = 17;
        let part_id = 23;
        let key = TranscriptNodeKey::ActivityPart {
            message_id,
            part_id,
        };
        let part = api_message_part!(
            part_id,
            message_id,
            now,
            ExecutionStatus::Completed,
            PartContent::Reasoning(ReasoningPart {
                summary: vec!["first line\nsecond line\nthird line".to_string()],
                raw_content: Vec::new(),
                encrypted_content: None,
            }),
        );
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![MessageResource {
                id: message_id,
                session_id: 7,
                role: MessageRole::Assistant,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![part]),
            }],
            ..TranscriptState::default()
        };
        transcript.node_expansions.insert(key.clone(), true);

        let expanded = transcript
            .rendered(80)
            .nodes
            .iter()
            .find(|node| node.key == key)
            .cloned()
            .expect("expanded reasoning node");
        assert!(expanded.end_line.saturating_sub(expanded.start_line) > 2);
        transcript.set_cursor_line(80, 10, expanded.end_line - 1);
        assert_eq!(
            transcript
                .current_cursor_node_cloned(80)
                .map(|node| node.key),
            Some(key.clone())
        );

        let (_, expanded) = transcript
            .toggle_cursor_node_expansion(80, 10)
            .expect("reasoning should be toggleable");

        assert!(!expanded);
        let collapsed = transcript
            .current_cursor_node_cloned(80)
            .expect("cursor should remain on collapsed reasoning");
        assert_eq!(collapsed.key, key);
        assert_eq!(
            transcript.navigation_cursor_line(),
            Some(collapsed.start_line)
        );
        assert_eq!(collapsed.end_line, collapsed.start_line + 1);
    }

    #[test]
    fn expanding_the_final_activity_scrolls_its_new_lines_into_view() {
        let now = Utc::now();
        let activity_key = TranscriptNodeKey::ActivityPart {
            message_id: 18,
            part_id: 24,
        };
        let preceding_part = api_message_part!(
            22,
            17,
            now,
            ExecutionStatus::Completed,
            PartContent::text("one\n\ntwo\n\nthree\n\nfour\n\nfive"),
        );
        let activity_part = api_message_part!(
            24,
            18,
            now,
            ExecutionStatus::Completed,
            PartContent::Reasoning(ReasoningPart {
                summary: vec!["first line\nsecond line\nthird line".to_string()],
                raw_content: Vec::new(),
                encrypted_content: None,
            }),
        );
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![
                MessageResource {
                    id: 17,
                    session_id: 7,
                    role: MessageRole::User,
                    state: MessageStatus::Completed,
                    created_at: now,
                    updated_at: now,
                    metadata: Default::default(),
                    usage: None,
                    part_count: 1,
                    parts: Some(vec![preceding_part]),
                },
                MessageResource {
                    id: 18,
                    session_id: 7,
                    role: MessageRole::Assistant,
                    state: MessageStatus::Completed,
                    created_at: now,
                    updated_at: now,
                    metadata: Default::default(),
                    usage: None,
                    part_count: 1,
                    parts: Some(vec![activity_part]),
                },
            ],
            ..TranscriptState::default()
        };
        let node_index = transcript
            .rendered(80)
            .nodes
            .iter()
            .position(|node| node.key == activity_key)
            .expect("collapsed final activity");
        transcript.set_block_cursor(80, 5, node_index, TranscriptMoveDirection::Down);
        let collapsed_scroll = transcript.viewport.top;

        let (_, expanded) = transcript
            .toggle_cursor_node_expansion(80, 5)
            .expect("reasoning should expand");

        assert!(expanded);
        let expanded_node = transcript
            .current_cursor_node_cloned(80)
            .expect("cursor remains on expanded final activity");
        assert_eq!(expanded_node.key, activity_key);
        assert!(transcript.viewport.top > collapsed_scroll);
        assert!(
            expanded_node.end_line <= transcript.viewport.top.saturating_add(5),
            "the complete expanded activity should fit in the viewport"
        );
        let max_scroll = transcript.max_scroll(80, 5);
        assert_eq!(transcript.viewport.top, max_scroll);
    }

    #[test]
    fn vertical_navigation_stops_on_messages_and_blocks_before_entering_text() {
        let now = Utc::now();
        let message = |id: i64, role: MessageRole, text: &str| MessageResource {
            id,
            session_id: 7,
            role,
            state: MessageStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: 1,
            parts: Some(vec![api_message_part!(
                id * 10,
                id,
                now,
                ExecutionStatus::Completed,
                PartContent::text(text),
            )]),
        };
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![
                message(9, MessageRole::User, "before"),
                message(
                    10,
                    MessageRole::Assistant,
                    "a wrapped answer with several rendered rows that can be navigated independently",
                ),
                message(11, MessageRole::User, "after"),
            ],
            ..TranscriptState::default()
        };
        transcript.viewport.follow_tail = false;
        let first_message_index = transcript
            .rendered(24)
            .nodes
            .iter()
            .position(|node| node.key == TranscriptNodeKey::Message { message_id: 9 })
            .expect("first message parent");
        transcript.set_block_cursor(24, 20, first_message_index, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::Message { message_id: 10 }),
            "crossing a boundary first selects the complete destination message"
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert!(matches!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::MarkdownBlock { message_id: 10, .. })
        ));
        let block_start = transcript
            .current_cursor_node_cloned(24)
            .expect("selected Markdown block")
            .start_line;
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(transcript.navigation_cursor_line(), Some(block_start));
        assert_eq!(transcript.highlighted_block_key(), None);

        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert!(
            transcript
                .navigation_cursor_line()
                .is_some_and(|line| line > block_start)
        );
        assert_eq!(transcript.highlighted_block_key(), None);

        let third_message_line = transcript
            .rendered(24)
            .nodes
            .iter()
            .find(|node| !node.key.is_message_container() && node.key.message_id() == 11)
            .expect("third message child")
            .start_line;
        transcript.select_pointer_line(
            24,
            20,
            TranscriptTextPosition {
                line: third_message_line,
                column: 2,
            },
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Up);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::Message { message_id: 10 }),
            "Up crossing a boundary first selects the complete previous message"
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Up);
        assert!(matches!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::MarkdownBlock { message_id: 10, .. })
        ));
    }

    #[test]
    fn code_blocks_expose_clean_semantic_source_lines_without_stopping_on_card_chrome() {
        let now = Utc::now();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![MessageResource {
                id: 10,
                session_id: 7,
                role: MessageRole::Assistant,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    20,
                    10,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text(
                        "before\n\n```rust\nlet first = \"abcdefghijklmnopqrstuvwxyz\";\nlet second = 2;\n```\n\nafter",
                    ),
                )]),
            }],
            ..TranscriptState::default()
        };
        let blocks = transcript
            .rendered(40)
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
            .map(|(index, node)| (index, node.key.clone(), node.atomic))
            .collect::<Vec<_>>();
        assert_eq!(blocks.len(), 3);
        assert!(!blocks[0].2);
        assert!(!blocks[1].2, "a code block has meaningful source lines");
        assert!(!blocks[2].2);

        let (code_range, first_source_range) = {
            let rendered = transcript.rendered(24);
            let node = rendered.nodes.get(blocks[1].0).expect("code node");
            let first_source_line = (node.start_line..node.end_line)
                .find(|line| {
                    rendered.lines[*line].navigation_copy_text
                        == "let first = \"abcdefghijklmnopqrstuvwxyz\";"
                })
                .expect("first code source line");
            let unit = rendered.lines[first_source_line]
                .navigation_unit
                .expect("code source navigation unit");
            let start = (node.start_line..=first_source_line)
                .rev()
                .take_while(|line| rendered.lines[*line].navigation_unit == Some(unit))
                .last()
                .expect("source range start");
            let end = (first_source_line..node.end_line)
                .take_while(|line| rendered.lines[*line].navigation_unit == Some(unit))
                .last()
                .expect("source range end")
                .saturating_add(1);
            (node.start_line..node.end_line, start..end)
        };
        assert!(
            first_source_range.len() > 1,
            "fixture must wrap one source line"
        );

        transcript.select_pointer_line(
            24,
            20,
            TranscriptTextPosition {
                line: first_source_range.start.saturating_add(1),
                column: 5,
            },
        );
        assert_eq!(transcript.highlighted_block_key(), None);
        assert_eq!(
            transcript.highlighted_line_range(24),
            Some(first_source_range.clone())
        );
        assert_eq!(
            transcript.current_selected_line_text(24).as_deref(),
            Some("let first = \"abcdefghijklmnopqrstuvwxyz\";")
        );

        let drag_line = first_source_range.start.saturating_add(1);
        let cursor_before_drag = transcript.navigation_cursor_line();
        let selection = transcript.set_text_selection(
            24,
            TranscriptTextSelection {
                anchor: TranscriptTextPosition {
                    line: drag_line,
                    column: 4,
                },
                head: TranscriptTextPosition {
                    line: drag_line,
                    column: 6,
                },
            },
        );
        assert_eq!(selection.anchor.line, drag_line);
        assert_eq!(selection.anchor.column, 4);
        assert_eq!(selection.head.line, drag_line);
        assert_eq!(selection.head.column, 6);
        assert_eq!(transcript.navigation_cursor_line(), cursor_before_drag);
        transcript.cancel_text_selection(24, 20);

        transcript.set_block_cursor(24, 20, blocks[0].0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[1].1.clone())
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(transcript.highlighted_block_key(), None);
        assert_eq!(
            transcript.current_selected_line_text(24).as_deref(),
            Some("let first = \"abcdefghijklmnopqrstuvwxyz\";")
        );
        assert!(
            transcript
                .navigation_cursor_line()
                .is_some_and(|line| line > code_range.start && line + 1 < code_range.end),
            "the code-card borders must not become navigation stops"
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.current_selected_line_text(24).as_deref(),
            Some("let second = 2;")
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[2].1.clone()),
            "leaving the last source line skips the bottom card border"
        );
    }

    #[test]
    fn tables_expose_one_clean_semantic_unit_per_table_row() {
        let now = Utc::now();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![MessageResource {
                id: 10,
                session_id: 7,
                role: MessageRole::Assistant,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    20,
                    10,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text(
                        "before\n\n| name | value |\n| --- | ---: |\n| answer | 42 |\n\nafter",
                    ),
                )]),
            }],
            ..TranscriptState::default()
        };
        let blocks = transcript
            .rendered(40)
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
            .map(|(index, node)| (index, node.key.clone(), node.atomic))
            .collect::<Vec<_>>();
        assert_eq!(blocks.len(), 3);
        assert!(!blocks[1].2, "table rows are semantically divisible");

        transcript.set_block_cursor(40, 20, blocks[0].0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(40, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[1].1.clone())
        );

        transcript.move_cursor_one_line(40, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.current_selected_line_text(40).as_deref(),
            Some("name\tvalue")
        );
        let header_line = transcript
            .navigation_cursor_line()
            .expect("table header row");
        assert!(
            !transcript.rendered(40).lines[header_line]
                .text
                .contains('┌')
        );

        transcript.move_cursor_one_line(40, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.current_selected_line_text(40).as_deref(),
            Some("answer\t42")
        );
        let (answer_line, answer_column) = {
            let rendered = transcript.rendered(40);
            rendered
                .lines
                .iter()
                .enumerate()
                .find_map(|(line, rendered_line)| {
                    rendered_line
                        .copy_segments
                        .iter()
                        .find(|segment| segment.text == "answer")
                        .map(|segment| (line, segment.display_column))
                })
                .expect("answer table cell copy segment")
        };
        let cursor_before_drag = transcript.navigation_cursor_line();
        let selection = transcript.set_text_selection(
            40,
            TranscriptTextSelection {
                anchor: TranscriptTextPosition {
                    line: answer_line,
                    column: answer_column.saturating_add(1),
                },
                head: TranscriptTextPosition {
                    line: answer_line,
                    column: answer_column.saturating_add(3),
                },
            },
        );
        assert_eq!(selection.anchor.line, answer_line);
        assert_eq!(selection.head.line, answer_line);
        assert_eq!(transcript.navigation_cursor_line(), cursor_before_drag);
        let copied = {
            let rendered = transcript.rendered(40);
            transcript_text_selection_text(
                rendered.lines.as_slice(),
                rendered.nodes.as_slice(),
                rendered.line_nodes.as_slice(),
                selection,
                "",
            )
        };
        assert_eq!(copied, "nsw");
        transcript.cancel_text_selection(40, 20);
        transcript.move_cursor_one_line(40, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[2].1.clone()),
            "separator and bottom-border rows must not become stops"
        );
    }

    #[test]
    fn single_formulas_remain_atomic_while_inline_formula_canvases_form_one_semantic_line() {
        let now = Utc::now();
        let message = |id, text: &str| MessageResource {
            id,
            session_id: 7,
            role: MessageRole::Assistant,
            state: MessageStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: 1,
            parts: Some(vec![api_message_part!(
                id * 10,
                id,
                now,
                ExecutionStatus::Completed,
                PartContent::text(text),
            )]),
        };
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![
                message(10, "before\n\n$$\n\\frac{a}{b}\n$$\n\nafter"),
                message(
                    11,
                    "inline before $\\begin{bmatrix}a\\\\b\\\\c\\end{bmatrix}$ inline after",
                ),
            ],
            ..TranscriptState::default()
        };
        let (before, formula, after) = {
            let blocks = transcript
                .rendered(80)
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| {
                    matches!(
                        node.key,
                        TranscriptNodeKey::MarkdownBlock { message_id: 10, .. }
                    )
                })
                .map(|(index, node)| (index, node.key.clone(), node.atomic))
                .collect::<Vec<_>>();
            assert_eq!(blocks.len(), 3);
            assert!(blocks[1].2, "a display formula is one semantic object");
            (blocks[0].clone(), blocks[1].clone(), blocks[2].clone())
        };
        transcript.set_block_cursor(80, 20, before.0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(80, 20, TranscriptMoveDirection::Down);
        assert_eq!(transcript.highlighted_block_key(), Some(formula.1.clone()));
        transcript.move_cursor_one_line(80, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(after.1),
            "formula canvas rows are crossed in one keypress"
        );

        let inline = transcript
            .rendered(80)
            .nodes
            .iter()
            .enumerate()
            .find(|(_, node)| {
                matches!(
                    node.key,
                    TranscriptNodeKey::MarkdownBlock { message_id: 11, .. }
                )
            })
            .map(|(index, node)| (index, node.clone()))
            .expect("inline-formula paragraph");
        assert!(
            !inline.1.atomic,
            "the paragraph itself must remain enterable"
        );
        let units = transcript.rendered(80).lines[inline.1.start_line..inline.1.end_line]
            .iter()
            .filter_map(|line| line.navigation_unit)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            units.len(),
            1,
            "all formula canvas rows form one logical line"
        );
        transcript.set_block_cursor(80, 20, inline.0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(80, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.current_selected_line_text(80).as_deref(),
            Some("inline before \\begin{bmatrix}a\\\\b\\\\c\\end{bmatrix} inline after")
        );
    }

    #[test]
    fn aligned_formula_blocks_enter_and_navigate_by_top_level_equation_row() {
        let now = Utc::now();
        let aligned = concat!(
            "before\n\n$$\n",
            "\\begin{aligned}\n",
            "y &= \\ln u, \\quad u = \\sin v, \\quad v = e^{2x}, \\quad w = 2x \\\\[4pt]\n",
            "\\frac{dy}{dx} &= \\frac{dy}{du} \\cdot \\frac{du}{dv} \\cdot \\frac{dv}{dw} \\cdot \\frac{dw}{dx} \\\\[4pt]\n",
            "&= \\frac{1}{u} \\cdot \\cos v \\cdot e^{w} \\cdot 2 \\\\[4pt]\n",
            "&= \\frac{1}{\\sin(e^{2x})} \\cdot \\cos(e^{2x}) \\cdot e^{2x} \\cdot 2 \\\\[4pt]\n",
            "&= \\frac{2e^{2x} \\cos(e^{2x})}{\\sin(e^{2x})} = 2e^{2x} \\cot(e^{2x})\n",
            "\\end{aligned}\n",
            "$$\n\nafter",
        );
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![MessageResource {
                id: 12,
                session_id: 7,
                role: MessageRole::Assistant,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    120,
                    12,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text(aligned),
                )]),
            }],
            ..TranscriptState::default()
        };
        let blocks = transcript
            .rendered(100)
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
            .map(|(index, node)| (index, node.clone()))
            .collect::<Vec<_>>();
        assert_eq!(blocks.len(), 3);
        assert!(
            !blocks[1].1.atomic,
            "a structurally multi-row formula block must be enterable"
        );

        transcript.set_block_cursor(100, 30, blocks[0].0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(100, 30, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[1].1.key.clone()),
            "the whole formula block remains the first hierarchy stop"
        );

        let expected = [
            r"y &= \ln u, \quad u = \sin v, \quad v = e^{2x}, \quad w = 2x",
            r"\frac{dy}{dx} &= \frac{dy}{du} \cdot \frac{du}{dv} \cdot \frac{dv}{dw} \cdot \frac{dw}{dx}",
            r"&= \frac{1}{u} \cdot \cos v \cdot e^{w} \cdot 2",
            r"&= \frac{1}{\sin(e^{2x})} \cdot \cos(e^{2x}) \cdot e^{2x} \cdot 2",
            r"&= \frac{2e^{2x} \cos(e^{2x})}{\sin(e^{2x})} = 2e^{2x} \cot(e^{2x})",
        ];
        for source in expected {
            transcript.move_cursor_one_line(100, 30, TranscriptMoveDirection::Down);
            assert_eq!(transcript.highlighted_block_key(), None);
            assert_eq!(
                transcript.current_selected_line_text(100).as_deref(),
                Some(source)
            );
        }
        transcript.move_cursor_one_line(100, 30, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[2].1.key.clone()),
            "leaving the last equation row selects the next Markdown block"
        );
    }

    #[test]
    fn extended_aligned_formula_is_enterable_from_both_directions_end_to_end() {
        let now = Utc::now();
        let source = concat!(
            "before\n\n$$\n",
            "\\begin{aligned}\n",
            "\\lim_{x \\to 0} \\frac{e^x - 1 - x}{x^2}\n",
            "&\\xlongequal{\\text{L'Hôpital}} \\lim_{x \\to 0} \\frac{e^x - 1}{2x} \\\\\n",
            "&\\xlongequal{\\text{L'Hôpital}} \\lim_{x \\to 0} \\frac{e^x}{2}\n",
            "= \\frac{1}{2}\n",
            "\\end{aligned}\n",
            "$$\n\nafter",
        );
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![MessageResource {
                id: 13,
                session_id: 7,
                role: MessageRole::Assistant,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    130,
                    13,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text(source),
                )]),
            }],
            ..TranscriptState::default()
        };
        let blocks = transcript
            .rendered(80)
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
            .map(|(index, node)| (index, node.clone()))
            .collect::<Vec<_>>();
        assert_eq!(blocks.len(), 3);
        assert!(!blocks[1].1.atomic, "extended formula must not be atomic");
        let equation_lines = transcript.rendered(80).lines
            [blocks[1].1.start_line..blocks[1].1.end_line]
            .iter()
            .filter_map(|line| {
                line.navigation_unit
                    .map(|unit| (unit, line.navigation_copy_text.clone()))
            })
            .fold(Vec::<(usize, String)>::new(), |mut rows, row| {
                if rows.last().is_none_or(|previous| previous.0 != row.0) {
                    rows.push(row);
                }
                rows
            });
        assert_eq!(equation_lines.len(), 2);

        transcript.set_block_cursor(80, 30, blocks[0].0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[1].1.key.clone())
        );
        transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.current_selected_line_text(80),
            Some(equation_lines[0].1.clone())
        );

        transcript.set_block_cursor(80, 30, blocks[2].0, TranscriptMoveDirection::Up);
        transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Up);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[1].1.key.clone())
        );
        transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Up);
        assert_eq!(
            transcript.current_selected_line_text(80),
            Some(equation_lines[1].1.clone())
        );

        let config = crate::math_render::MathLayoutConfig {
            native_graphics: true,
            cell_width: 10,
            cell_height: 20,
            ..crate::math_render::MathLayoutConfig::default()
        };
        let context = crate::math_render::test_math_render_context(config);
        let mut native_transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![MessageResource {
                id: 14,
                session_id: 7,
                role: MessageRole::Assistant,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    140,
                    14,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text(source),
                )]),
            }],
            ..TranscriptState::default()
        };
        crate::math_render::with_math_render_context(&context, || {
            let native_blocks = native_transcript
                .rendered(80)
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
                .map(|(index, node)| (index, node.clone()))
                .collect::<Vec<_>>();
            assert_eq!(native_blocks.len(), 3);
            assert!(
                !native_blocks[1].1.atomic,
                "native end-to-end formula must not be atomic"
            );
            native_transcript.set_block_cursor(
                80,
                30,
                native_blocks[0].0,
                TranscriptMoveDirection::Down,
            );
            native_transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
            native_transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
            assert_eq!(
                native_transcript.current_selected_line_text(80),
                Some(equation_lines[0].1.clone())
            );
        });

        let mut formula_only = TranscriptState {
            session_id: Some(7),
            messages: vec![MessageResource {
                id: 15,
                session_id: 7,
                role: MessageRole::Assistant,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    150,
                    15,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text(concat!(
                        "$$\n",
                        "\\begin{aligned}\n",
                        "\\lim_{x \\to 0} \\frac{e^x - 1 - x}{x^2}\n",
                        "&\\xlongequal{\\text{L'Hôpital}} \\lim_{x \\to 0} \\frac{e^x - 1}{2x} \\\\\n",
                        "&\\xlongequal{\\text{L'Hôpital}} \\lim_{x \\to 0} \\frac{e^x}{2}\n",
                        "= \\frac{1}{2}\n",
                        "\\end{aligned}\n",
                        "$$",
                    )),
                )]),
            }],
            ..TranscriptState::default()
        };
        let (message_index, formula_index, formula_key, formula_start) = {
            let rendered = formula_only.rendered(80);
            let message_index = rendered
                .nodes
                .iter()
                .position(|node| matches!(node.key, TranscriptNodeKey::Message { .. }))
                .expect("message node");
            let (formula_index, formula) = rendered
                .nodes
                .iter()
                .enumerate()
                .find(|(_, node)| node.kind == TranscriptNodeKind::MarkdownMath)
                .expect("formula node");
            assert!(!formula.atomic);
            (
                message_index,
                formula_index,
                formula.key.clone(),
                formula.start_line,
            )
        };
        formula_only.set_block_cursor(80, 30, message_index, TranscriptMoveDirection::Down);
        formula_only.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
        assert_eq!(
            formula_only.highlighted_block_key(),
            None,
            "a message's only child must not create a duplicate whole-formula stop: {formula_key:?}"
        );
        assert_eq!(
            formula_only.current_selected_line_text(80),
            Some(equation_lines[0].1.clone())
        );

        formula_only.set_block_cursor(80, 30, message_index, TranscriptMoveDirection::Up);
        formula_only.move_cursor_one_line(80, 30, TranscriptMoveDirection::Up);
        assert_eq!(formula_only.highlighted_block_key(), None);
        assert_eq!(
            formula_only.current_selected_line_text(80),
            Some(equation_lines[1].1.clone())
        );

        formula_only.select_pointer_block(
            80,
            30,
            TranscriptTextPosition {
                line: formula_start,
                column: 2,
            },
        );
        assert_eq!(
            formula_only.highlighted_block_key(),
            Some(formula_key.clone())
        );
        formula_only.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
        assert_eq!(formula_only.highlighted_block_key(), None);
        assert_eq!(
            formula_only.current_selected_line_text(80),
            Some(equation_lines[0].1.clone())
        );
        formula_only.select_pointer_block(
            80,
            30,
            TranscriptTextPosition {
                line: formula_start,
                column: 2,
            },
        );
        formula_only.move_cursor_one_line(80, 30, TranscriptMoveDirection::Up);
        assert_eq!(formula_only.highlighted_block_key(), None);
        assert_eq!(
            formula_only.current_selected_line_text(80),
            Some(equation_lines[1].1.clone())
        );
        assert_eq!(
            formula_only
                .rendered(80)
                .nodes
                .get(formula_index)
                .map(|node| node.key.clone()),
            Some(formula_key)
        );

        let second_equation_line = formula_only
            .rendered(80)
            .lines
            .iter()
            .position(|line| line.navigation_copy_text == equation_lines[1].1)
            .expect("second equation display row");
        formula_only.select_pointer_line(
            80,
            30,
            TranscriptTextPosition {
                line: second_equation_line,
                column: 2,
            },
        );
        assert_eq!(formula_only.highlighted_block_key(), None);
        assert_eq!(
            formula_only.current_selected_line_text(80),
            Some(equation_lines[1].1.clone())
        );
    }
}

#[cfg(test)]
mod rewind_message_tests {
    use agena_domain::ExecutionStatus;
    use agena_domain::TextPart;

    use super::super::{
        MessageResource, MessageRole, MessageStatus, Utc, rewind_message_composer_text,
    };

    #[test]
    fn composer_text_restores_only_visible_user_text() {
        let now = Utc::now();
        let parts = vec![
            api_message_part!(
                1,
                42,
                now,
                ExecutionStatus::Completed,
                PartContent::text("first"),
            ),
            api_message_part!(
                2,
                42,
                now,
                ExecutionStatus::Completed,
                PartContent::Text(TextPart {
                    text: "generated".to_string(),
                    synthetic: true,
                    ignored: false,
                }),
            ),
            api_message_part!(
                3,
                42,
                now,
                ExecutionStatus::Completed,
                PartContent::Text(TextPart {
                    text: "hidden".to_string(),
                    synthetic: false,
                    ignored: true,
                }),
            ),
            api_message_part!(
                4,
                42,
                now,
                ExecutionStatus::Completed,
                PartContent::text("second"),
            ),
        ];
        let message = MessageResource {
            id: 42,
            session_id: 7,
            role: MessageRole::User,
            state: MessageStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: parts.len() as u64,
            parts: Some(parts),
        };

        assert_eq!(rewind_message_composer_text(&message), "first\n\nsecond");
    }
}

#[cfg(test)]
mod live_transcript_tests {
    use agena_api::resource::MessageStatus as WireMessageStatus;
    use agena_domain::{
        EventMeta, ExecutionStatus as DomainExecutionStatus, MessagePartDeltaEvent, PartDeltaField,
        PartKind, Role,
    };
    use agena_runtime::{
        RuntimeMessageMetadata, RuntimeMessagePartCheckpoint, RuntimePresentationEvent,
        RuntimePresentationEventKind, SessionProjectedMessagePart, SessionProjectedPartDetail,
    };
    use uuid::Uuid;

    use super::super::{MessageRole, TranscriptState, Utc};

    fn event(kind: RuntimePresentationEventKind, seq: i64) -> RuntimePresentationEvent {
        RuntimePresentationEvent {
            meta: EventMeta {
                id: Uuid::new_v4(),
                seq_global: seq,
                seq_session: Some(seq),
                session_id: Some(7),
                workspace_id: None,
                created_at: Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: 1,
            },
            invalidates_ancestor_projection: false,
            kind,
        }
    }

    fn part_checkpointed(message_id: i64, part_id: i64, seq: i64) -> RuntimePresentationEvent {
        part_checkpointed_in_turn(message_id, part_id, None, seq)
    }

    fn part_checkpointed_in_turn(
        message_id: i64,
        part_id: i64,
        turn_id: Option<i64>,
        seq: i64,
    ) -> RuntimePresentationEvent {
        part_checkpointed_in_turn_and_model(message_id, part_id, turn_id, "provider", "model", seq)
    }

    fn part_checkpointed_in_turn_and_model(
        message_id: i64,
        part_id: i64,
        turn_id: Option<i64>,
        provider_id: &str,
        model_id: &str,
        seq: i64,
    ) -> RuntimePresentationEvent {
        let now = Utc::now();
        let message_metadata = RuntimeMessageMetadata {
            source: agena_domain::MessageSource::Assistant,
            turn_id,
            model_provider_id: provider_id.to_owned(),
            model_id: model_id.to_owned(),
            parent_message_id: None,
            generated_by_call_id: None,
            model_adapter_id: None,
            model_thinking_mode: None,
            model_speed_mode: None,
        };
        event(
            RuntimePresentationEventKind::MessagePartCheckpointed(Box::new(
                RuntimeMessagePartCheckpoint {
                    session_id: 7,
                    execution_id: None,
                    run_id: None,
                    message_id,
                    message_role: Role::Assistant,
                    message_state: DomainExecutionStatus::InProgress,
                    message_created_at: now,
                    message_metadata,
                    part: SessionProjectedMessagePart {
                        id: part_id,
                        message_id,
                        part_index: 0,
                        status: DomainExecutionStatus::Pending,
                        kind: PartKind::Text,
                        name: None,
                        summary: None,
                        has_detail: true,
                        operation_id: None,
                        created_at: now,
                        detail: Some(SessionProjectedPartDetail::Text {
                            text: String::new(),
                            synthetic: false,
                            ignored: false,
                        }),
                        content: None,
                    },
                    ts_ms: now.timestamp_millis(),
                },
            )),
            seq,
        )
    }

    fn text_delta(message_id: i64, part_id: i64, text: &str, seq: i64) -> RuntimePresentationEvent {
        event(
            RuntimePresentationEventKind::MessagePartDelta(MessagePartDeltaEvent {
                session_id: 7,
                execution_id: None,
                run_id: None,
                message_id,
                part_id,
                call_id: None,
                field: PartDeltaField::Text,
                delta: text.to_string(),
                seq: seq as u64,
                ts_ms: Utc::now().timestamp_millis(),
            }),
            seq,
        )
    }

    #[test]
    fn ephemeral_provider_delta_is_rendered_without_waiting_for_a_refresh() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };

        assert!(!transcript.apply_presentation_event(&part_checkpointed(10, 101, 1), 80, 20));
        assert!(!transcript.apply_presentation_event(&text_delta(10, 101, "I'm Grok", 2), 80, 20));

        assert_eq!(transcript.messages.len(), 1);
        assert_eq!(transcript.messages[0].state, WireMessageStatus::InProgress);
        let rendered = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("Grok"),
            "rendered transcript: {rendered:?}"
        );
    }

    #[test]
    fn assistant_passes_in_one_turn_share_one_live_message() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };

        transcript.apply_presentation_event(
            &part_checkpointed_in_turn(10, 101, Some(7), 1),
            80,
            20,
        );
        transcript.apply_presentation_event(&text_delta(10, 101, "first", 2), 80, 20);
        transcript.apply_presentation_event(
            &part_checkpointed_in_turn(11, 102, Some(7), 3),
            80,
            20,
        );
        transcript.apply_presentation_event(&text_delta(11, 102, "second", 4), 80, 20);

        assert_eq!(transcript.messages.len(), 1);
        assert_eq!(transcript.messages[0].id, 10);
        assert_eq!(transcript.messages[0].role, MessageRole::Assistant);
        assert_eq!(transcript.messages[0].metadata.turn_id, Some(7));
        let parts = transcript.messages[0]
            .parts
            .as_ref()
            .expect("aggregated live parts");
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|part| part.message_id == 10));
        let rendered = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("first"));
        assert!(rendered.contains("second"));
    }

    #[test]
    fn changing_model_route_keeps_a_separate_live_assistant_message() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };

        transcript.apply_presentation_event(
            &part_checkpointed_in_turn_and_model(10, 101, Some(7), "openai", "model-a", 1),
            80,
            20,
        );
        transcript.apply_presentation_event(
            &part_checkpointed_in_turn_and_model(11, 102, Some(7), "openai", "model-b", 2),
            80,
            20,
        );

        assert_eq!(transcript.messages.len(), 2);
        assert_eq!(transcript.messages[0].id, 10);
        assert_eq!(transcript.messages[1].id, 11);
    }
}

#[cfg(test)]
mod run_activity_tests {
    use super::super::{RunActivityTarget, RunActivityTracker, RunOperation};

    #[test]
    fn overlapping_run_requests_share_one_counted_activity_source() {
        let mut activity = RunActivityTracker::default();
        let session = RunActivityTarget::Session(7);

        activity.begin(session, RunOperation::SubmitMessage);
        activity.begin(session, RunOperation::PermissionReply);
        assert!(activity.is_active(session));

        activity.finish(session, RunOperation::PermissionReply);
        assert!(
            activity.is_active(session),
            "finishing an overlapping permission reply must not make the active submission idle"
        );
        assert!(activity.has_operation(session, RunOperation::SubmitMessage));

        activity.finish(session, RunOperation::SubmitMessage);
        assert!(!activity.is_active(session));
    }
}

#[cfg(test)]
mod permission_studio_tests {
    use super::{
        ConfigJsonSources, I18n, JsonValue, PermissionConfig, PermissionMode,
        PermissionStudioCatalogKind, PermissionStudioModeTarget, SettingsPickerAction,
        ToolPermissionRules, apply_permission_studio_entries_mode,
        apply_permission_studio_mode_input, path_rule_modes, settings_studio_permission_items,
    };

    #[test]
    fn selected_tool_entries_use_the_explicitly_chosen_mode() {
        let mut permission = PermissionConfig::default();

        apply_permission_studio_entries_mode(
            &mut permission,
            PermissionStudioCatalogKind::ToolTags,
            vec!["filesystem".to_owned(), "network".to_owned()],
            PermissionMode::Deny,
        );
        apply_permission_studio_entries_mode(
            &mut permission,
            PermissionStudioCatalogKind::ToolNames,
            vec!["agena.shell.run".to_owned()],
            PermissionMode::Allow,
        );

        let tools = permission
            .tools
            .expect("tool permissions should be created");
        assert_eq!(tools.tags.get("filesystem"), Some(&PermissionMode::Deny));
        assert_eq!(tools.tags.get("network"), Some(&PermissionMode::Deny));
        assert_eq!(
            tools.names.get("agena.shell.run"),
            Some(&PermissionMode::Allow)
        );
    }

    #[test]
    fn rule_rows_update_their_permission_modes_in_place() {
        let i18n = I18n::default();
        let mut permission = PermissionConfig::default();

        apply_permission_studio_mode_input(
            &i18n,
            &mut permission,
            &PermissionStudioModeTarget::NetworkRule {
                target: "api.github.com:443".to_string(),
            },
            "allow",
        )
        .expect("set network rule mode");
        assert_eq!(
            permission
                .network
                .as_ref()
                .and_then(|network| network.rules.get("api.github.com:443"))
                .copied(),
            Some(PermissionMode::Allow)
        );

        apply_permission_studio_mode_input(
            &i18n,
            &mut permission,
            &PermissionStudioModeTarget::PathRuleWrite {
                pattern: "<workspace>/generated/**".to_string(),
            },
            "deny",
        )
        .expect("set path write mode");
        let modes = permission
            .path
            .as_ref()
            .and_then(|path| path.rules.get("<workspace>/generated/**"))
            .and_then(|rule| path_rule_modes(Some(rule)))
            .expect("path rule modes");
        assert_eq!(modes.write, Some(PermissionMode::Deny));
    }

    #[test]
    fn command_pattern_mode_converts_a_tool_rule_to_an_ordered_rule_set() {
        let i18n = I18n::default();
        let mut permission = PermissionConfig::default();
        permission
            .tools
            .get_or_insert_with(Default::default)
            .rules
            .insert(
                "shell".to_string(),
                ToolPermissionRules::Mode(PermissionMode::Ask),
            );

        apply_permission_studio_mode_input(
            &i18n,
            &mut permission,
            &PermissionStudioModeTarget::ToolCommandPattern {
                tool_name: "shell".to_string(),
                pattern: "git status".to_string(),
            },
            "allow",
        )
        .expect("set shell command pattern mode");

        let Some(ToolPermissionRules::Ordered(entries)) = permission
            .tools
            .as_ref()
            .and_then(|tools| tools.rules.get("shell"))
        else {
            panic!("tool rule should become an ordered command rule set");
        };
        assert_eq!(entries.get("*"), Some(&PermissionMode::Ask));
        assert_eq!(entries.get("git status"), Some(&PermissionMode::Allow));
    }

    #[test]
    fn settings_list_global_and_workspace_permissions_as_distinct_edit_targets() {
        let i18n = I18n::default();
        let sources = ConfigJsonSources {
            config_path: "/home/test/.agena/agena.json".into(),
            config_found: true,
            project_config_path: "/workspace/.agena/agena.json".into(),
            project_config_found: true,
            applied_layers: vec!["global".to_string(), "workspace".to_string()],
            file: JsonValue::Null,
            project_file: JsonValue::Null,
            effective: JsonValue::Null,
        };
        let permission = PermissionConfig::default();
        let items = settings_studio_permission_items(
            &i18n,
            &sources,
            &permission,
            &permission,
            &permission,
            None,
        );

        assert_eq!(items.len(), 2);
        assert!(matches!(
            items.first().map(|item| &item.action),
            Some(SettingsPickerAction::OpenGlobalPermissionWorkbench)
        ));
        assert!(matches!(
            items.get(1).map(|item| &item.action),
            Some(SettingsPickerAction::OpenWorkspacePermissionWorkbench)
        ));
        assert!(
            items[1]
                .source_rows
                .first()
                .is_some_and(|row| row.value.contains("/workspace/.agena/agena.json"))
        );
    }
}

#[cfg(test)]
mod permission_overlay_tests {
    use super::{
        PermissionAction, PermissionMode, PermissionOverlayChoice, PermissionPromptDecision,
        PermissionPromptPage, PermissionReplyKind, PermissionRequest, PermissionRuleSubjectKind,
        PermissionScope, Utc, permission_overlay_choice, permission_rule_draft_from_request,
    };
    use agena_domain::PermissionRiskLevel;

    fn request_for(action: PermissionAction, session_id: Option<i64>) -> PermissionRequest {
        PermissionRequest {
            request_id: "request-1".to_string(),
            session_id,
            action,
            related_actions: Vec::new(),
            requested_actions: Vec::new(),
            reason: "test request".to_string(),
            explanation: String::new(),
            source: None,
            scope: None,
            operator: None,
            risk: PermissionRiskLevel::Medium,
            trace: Vec::new(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn root_menu_groups_decisions_before_asking_for_scope() {
        assert_eq!(PermissionPromptPage::Action.choice_count(), 4);
        assert_eq!(
            permission_overlay_choice(PermissionPromptPage::Action, 0),
            PermissionOverlayChoice::OpenScope(PermissionPromptDecision::Allow)
        );
        assert_eq!(
            permission_overlay_choice(PermissionPromptPage::Action, 1),
            PermissionOverlayChoice::OpenScope(PermissionPromptDecision::Deny)
        );
        assert_eq!(
            permission_overlay_choice(PermissionPromptPage::Action, 2),
            PermissionOverlayChoice::EditRule
        );
        assert_eq!(
            permission_overlay_choice(PermissionPromptPage::Action, 3),
            PermissionOverlayChoice::Details
        );
    }

    #[test]
    fn scope_menu_maps_allow_and_deny_to_the_correct_reply_kinds() {
        assert_eq!(
            permission_overlay_choice(
                PermissionPromptPage::Scope(PermissionPromptDecision::Allow),
                0,
            ),
            PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::AllowOnce,
                scope: None,
            }
        );
        assert_eq!(
            permission_overlay_choice(
                PermissionPromptPage::Scope(PermissionPromptDecision::Allow),
                2,
            ),
            PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::AllowAlways,
                scope: Some(PermissionScope::Workspace),
            }
        );
        assert_eq!(
            permission_overlay_choice(
                PermissionPromptPage::Scope(PermissionPromptDecision::Deny),
                3,
            ),
            PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::DenyAlways,
                scope: Some(PermissionScope::Global),
            }
        );
    }

    #[test]
    fn custom_rule_studio_draft_preserves_the_pending_request_details() {
        let tool = permission_rule_draft_from_request(&request_for(
            PermissionAction::Tool {
                tool_name: "shell".to_string(),
                qualifier: Some("git push origin main".to_string()),
            },
            Some(42),
        ));
        assert_eq!(tool.subject_kind, PermissionRuleSubjectKind::Tool);
        assert_eq!(tool.tool_name, "shell");
        assert_eq!(tool.qualifier, "git push origin main");
        assert_eq!(tool.scope, "session");
        assert_eq!(tool.session_id, "42");
        assert_eq!(tool.mode, PermissionMode::Allow);

        let path = permission_rule_draft_from_request(&request_for(
            PermissionAction::PathAccess {
                access_kind: "write".to_string(),
                workspace_root: "/workspace".to_string(),
                target_path: "/workspace/.agena/agena.json".to_string(),
            },
            None,
        ));
        assert_eq!(path.subject_kind, PermissionRuleSubjectKind::PathAccess);
        assert_eq!(path.path_access_kind, "write");
        assert_eq!(path.workspace_root, "/workspace");
        assert_eq!(path.target_path, "/workspace/.agena/agena.json");
        assert_eq!(path.scope, "workspace");

        let network = permission_rule_draft_from_request(&request_for(
            PermissionAction::NetworkAccess {
                target: "https://api.github.com/repos".to_string(),
                host: "api.github.com".to_string(),
                port: Some(443),
            },
            None,
        ));
        assert_eq!(
            network.subject_kind,
            PermissionRuleSubjectKind::NetworkAccess
        );
        assert_eq!(network.network_target, "https://api.github.com/repos");
        assert_eq!(network.scope, "workspace");
    }
}

#[cfg(test)]
mod transcript_navigation_tests {
    use super::{
        RenderedTranscriptNode, TranscriptMoveDirection, TranscriptNodeKey, TranscriptNodeKind,
        initial_search_match_index, transcript_message_navigation_target,
        transcript_node_highlight_range, transcript_selection_scroll_position,
    };

    fn node(key: TranscriptNodeKey, start_line: usize, end_line: usize) -> RenderedTranscriptNode {
        RenderedTranscriptNode {
            kind: TranscriptNodeKind::Message,
            key,
            start_line,
            end_line,
            copy_text: String::new(),
            atomic: false,
            toggleable: false,
            expanded: true,
        }
    }

    #[test]
    fn transcript_search_starts_after_or_before_the_cursor_and_wraps() {
        let matches = [2, 5, 9];

        assert_eq!(initial_search_match_index(&matches, 6, true), 2);
        assert_eq!(initial_search_match_index(&matches, 6, false), 1);
        assert_eq!(initial_search_match_index(&matches, 10, true), 0);
        assert_eq!(initial_search_match_index(&matches, 1, false), 2);
    }

    #[test]
    fn whole_message_highlight_excludes_the_role_header() {
        let message = TranscriptNodeKey::Message { message_id: 7 };
        let child = TranscriptNodeKey::MessagePart {
            message_id: 7,
            part_id: Some(11),
        };
        let nodes = vec![node(child.clone(), 4, 8), node(message.clone(), 3, 8)];

        assert_eq!(
            transcript_node_highlight_range(nodes.as_slice(), &message),
            Some(4..8),
            "the message body should be highlighted without its role header"
        );
        assert_eq!(
            transcript_node_highlight_range(nodes.as_slice(), &child),
            Some(4..8),
            "leaf selections should retain their exact rendered range"
        );
    }

    #[test]
    fn horizontal_navigation_only_visits_complete_messages() {
        let first_child = TranscriptNodeKey::MarkdownBlock {
            message_id: 10,
            part_id: 1,
            block_index: 0,
        };
        let first_message = TranscriptNodeKey::Message { message_id: 10 };
        let second_message = TranscriptNodeKey::Message { message_id: 11 };
        let nodes = vec![
            node(first_child.clone(), 1, 4),
            node(first_message.clone(), 0, 4),
            node(
                TranscriptNodeKey::MessagePart {
                    message_id: 11,
                    part_id: Some(2),
                },
                5,
                6,
            ),
            node(second_message.clone(), 4, 6),
        ];

        assert_eq!(
            transcript_message_navigation_target(
                nodes.as_slice(),
                2,
                None,
                TranscriptMoveDirection::Down,
            ),
            Some(1)
        );
        assert_eq!(
            transcript_message_navigation_target(
                nodes.as_slice(),
                0,
                Some(&first_message),
                TranscriptMoveDirection::Down,
            ),
            Some(3)
        );
        assert_eq!(
            transcript_message_navigation_target(
                nodes.as_slice(),
                2,
                Some(&first_child),
                TranscriptMoveDirection::Down,
            ),
            Some(3),
            "horizontal navigation skips children"
        );
        assert_eq!(
            transcript_message_navigation_target(
                nodes.as_slice(),
                4,
                Some(&second_message),
                TranscriptMoveDirection::Up,
            ),
            Some(1)
        );
    }

    #[test]
    fn message_selection_scrolls_to_keep_first_and_last_messages_fully_visible() {
        assert_eq!(
            transcript_selection_scroll_position(30, 0, 6, 10, 0, TranscriptMoveDirection::Down,),
            0
        );
        assert_eq!(
            transcript_selection_scroll_position(30, 24, 30, 10, 0, TranscriptMoveDirection::Down,),
            20,
            "the final message shifts upward instead of clipping below the viewport"
        );
        assert_eq!(
            transcript_selection_scroll_position(30, 5, 22, 10, 5, TranscriptMoveDirection::Up,),
            12,
            "a selection taller than the viewport aligns its end when moving upward"
        );
    }

    #[test]
    fn block_selection_keeps_the_viewport_stable_until_it_would_clip() {
        assert_eq!(
            transcript_selection_scroll_position(
                100,
                15,
                17,
                10,
                10,
                TranscriptMoveDirection::Down,
            ),
            10,
            "a complete selection already in view must not jump to the top"
        );
        assert_eq!(
            transcript_selection_scroll_position(
                100,
                20,
                23,
                10,
                10,
                TranscriptMoveDirection::Down,
            ),
            13,
            "scroll only enough to keep the selection's final line visible"
        );
    }
}
