use super::{
    ConfigJsonSources, I18n, JsonValue, PermissionAction, PermissionConfig, PermissionMode,
    PermissionOverlayChoice, PermissionOverlayDecision, PermissionOverlayPage, PermissionReplyKind,
    PermissionRequest, PermissionRiskLevel, PermissionRuleSubjectKind, PermissionScope,
    PermissionStudioCatalogKind, PermissionStudioModeTarget, RenderedTranscriptNode,
    SettingsPickerAction, ToolPermissionRules, TranscriptMoveDirection, TranscriptNodeKey,
    TranscriptNodeKind, Utc, apply_permission_studio_entries_mode,
    apply_permission_studio_mode_input, initial_search_match_index, path_rule_modes,
    permission_overlay_choice, permission_overlay_choices, permission_rule_draft_from_request,
    settings_studio_permission_items, transcript_message_navigation_target,
    transcript_node_highlight_range, transcript_selection_scroll_position,
};

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
        ExecutionStatus, MessagePart, MessageResource, MessageRole, MessageStatus,
        PaginatedResponse, PartContent, PendingUserMessage, TranscriptState, Utc,
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
        let parts = vec![MessagePart::from_content(
            51,
            50,
            now,
            ExecutionStatus::Completed,
            PartContent::text("send this now"),
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
    use agena::message::{ExecutionStatus, MessagePart, PartContent};

    use super::super::{
        MessageResource, MessageRole, MessageStatus, PendingUserMessage, TranscriptMoveDirection,
        TranscriptNodeKey, TranscriptState, TranscriptTextPosition, Utc,
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
            parts: Some(vec![MessagePart::from_content(
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
                parts: Some(vec![MessagePart::from_content(
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
        assert_eq!(transcript.highlighted_block_range(40), Some(block_range));
        assert_eq!(transcript.navigation_cursor_line(), Some(clickable_line));
        assert_eq!(transcript.current_selected_line_text(40), None);

        transcript.move_cursor_one_line(40, 10, TranscriptMoveDirection::Down);
        assert_eq!(transcript.highlighted_block_key(), None);
        assert!(
            transcript
                .navigation_cursor_line()
                .is_some_and(|line| line > clickable_line)
        );

        let head_line = clickable_line.saturating_add(1);
        transcript.begin_text_selection(
            40,
            10,
            TranscriptTextPosition {
                line: clickable_line,
                column: 1,
            },
        );
        let selection = transcript
            .finish_text_selection(
                40,
                10,
                TranscriptTextPosition {
                    line: head_line,
                    column: 4,
                },
            )
            .expect("non-empty text selection");
        assert_eq!(transcript.text_selection(), Some(selection));
        assert!(!transcript.text_selection_is_dragging());
        assert_eq!(transcript.navigation_cursor_line(), Some(head_line));

        transcript.cancel_text_selection(40, 10);
        assert_eq!(transcript.navigation_cursor_line(), Some(head_line));
        assert_eq!(transcript.text_selection(), None);
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
                parts: Some(vec![MessagePart::from_content(
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
    use agena::message::{ExecutionStatus, MessagePart, PartContent, ReasoningPart};

    use super::super::{
        MessageResource, MessageRole, MessageStatus, TranscriptMoveDirection, TranscriptNodeKey,
        TranscriptState, Utc,
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
        let part = MessagePart::from_content(
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
        let preceding_part = MessagePart::from_content(
            22,
            17,
            now,
            ExecutionStatus::Completed,
            PartContent::text("one\n\ntwo\n\nthree\n\nfour\n\nfive"),
        );
        let activity_part = MessagePart::from_content(
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
    fn rendered_line_navigation_moves_only_the_primary_cursor() {
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
            parts: Some(vec![MessagePart::from_content(
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
        let start = transcript
            .navigation_cursor_line()
            .expect("block primary line");
        let expected = {
            let rendered = transcript.rendered(24);
            (start + 1..rendered.lines.len())
                .find(|line| {
                    rendered.line_nodes[*line].is_some()
                        || !rendered.lines[*line].text.trim().is_empty()
                })
                .expect("next focusable row")
        };

        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);

        assert_eq!(transcript.navigation_cursor_line(), Some(expected));
        assert_eq!(transcript.highlighted_block_key(), None);
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Up);
        assert_eq!(transcript.navigation_cursor_line(), Some(start));
        assert_eq!(transcript.highlighted_block_key(), None);
    }
}

#[cfg(test)]
mod rewind_message_tests {
    use agena::message::{ExecutionStatus, MessagePart, PartContent, TextPart};

    use super::super::{
        MessageResource, MessageRole, MessageStatus, Utc, rewind_message_composer_text,
    };

    #[test]
    fn composer_text_restores_only_visible_user_text() {
        let now = Utc::now();
        let parts = vec![
            MessagePart::from_content(
                1,
                42,
                now,
                ExecutionStatus::Completed,
                PartContent::text("first"),
            ),
            MessagePart::from_content(
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
            MessagePart::from_content(
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
            MessagePart::from_content(
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
    use agena::{
        event::{
            DomainEvent, EventKind, EventMeta, MessagePartCheckpointedEvent, MessagePartDeltaEvent,
            PartDeltaField,
        },
        message::{ExecutionStatus, MessagePart, MessageStatus, PartContent},
        role::Role,
    };
    use uuid::Uuid;

    use super::super::{MessageRole, TranscriptState, Utc};

    fn event(kind: EventKind, seq: i64) -> DomainEvent {
        DomainEvent {
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
            kind,
        }
    }

    fn part_checkpointed(message_id: i64, part_id: i64, seq: i64) -> DomainEvent {
        let now = Utc::now();
        event(
            EventKind::MessagePartCheckpointed(MessagePartCheckpointedEvent {
                session_id: 7,
                execution_id: None,
                run_id: None,
                message_id,
                message_role: Role::Assistant,
                message_state: MessageStatus::InProgress,
                message_created_at: now,
                part: MessagePart::from_content(
                    part_id,
                    message_id,
                    now,
                    ExecutionStatus::Pending,
                    PartContent::text(String::new()),
                ),
                ts_ms: now.timestamp_millis(),
            }),
            seq,
        )
    }

    fn text_delta(message_id: i64, part_id: i64, text: &str, seq: i64) -> DomainEvent {
        event(
            EventKind::MessagePartDelta(MessagePartDeltaEvent {
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

        assert!(!transcript.apply_live_event(&part_checkpointed(10, 101, 1), 80, 20));
        assert!(!transcript.apply_live_event(&text_delta(10, 101, "I'm Grok", 2), 80, 20));

        assert_eq!(transcript.messages.len(), 1);
        assert_eq!(transcript.messages[0].state, MessageStatus::InProgress);
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
    fn consecutive_assistant_passes_keep_independent_live_messages() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };

        transcript.apply_live_event(&part_checkpointed(10, 101, 1), 80, 20);
        transcript.apply_live_event(&text_delta(10, 101, "first", 2), 80, 20);
        transcript.apply_live_event(&part_checkpointed(11, 102, 3), 80, 20);
        transcript.apply_live_event(&text_delta(11, 102, "second", 4), 80, 20);

        assert_eq!(transcript.messages.len(), 2);
        assert_eq!(transcript.messages[0].id, 10);
        assert_eq!(transcript.messages[0].role, MessageRole::Assistant);
        assert_eq!(transcript.messages[1].id, 11);
        assert_eq!(transcript.messages[1].role, MessageRole::Assistant);
        let first_parts = transcript.messages[0]
            .parts
            .as_ref()
            .expect("first live parts");
        let second_parts = transcript.messages[1]
            .parts
            .as_ref()
            .expect("second live parts");
        assert_eq!(first_parts.len(), 1);
        assert_eq!(second_parts.len(), 1);
        assert_eq!(first_parts[0].message_id, 10);
        assert_eq!(second_parts[0].message_id, 11);
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
        I18n, PermissionAction, PermissionMode, PermissionOverlayChoice, PermissionOverlayDecision,
        PermissionOverlayPage, PermissionReplyKind, PermissionRequest, PermissionRiskLevel,
        PermissionRuleSubjectKind, PermissionScope, Utc, permission_overlay_choice,
        permission_overlay_choices, permission_rule_draft_from_request,
    };

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
        let i18n = I18n::default();
        let choices = permission_overlay_choices(&i18n, PermissionOverlayPage::Action);

        assert_eq!(choices.len(), 4);
        assert_eq!(
            permission_overlay_choice(PermissionOverlayPage::Action, 0),
            PermissionOverlayChoice::OpenScope(PermissionOverlayDecision::Allow)
        );
        assert_eq!(
            permission_overlay_choice(PermissionOverlayPage::Action, 1),
            PermissionOverlayChoice::OpenScope(PermissionOverlayDecision::Deny)
        );
        assert_eq!(
            permission_overlay_choice(PermissionOverlayPage::Action, 2),
            PermissionOverlayChoice::EditRule
        );
        assert_eq!(
            permission_overlay_choice(PermissionOverlayPage::Action, 3),
            PermissionOverlayChoice::Details
        );
    }

    #[test]
    fn scope_menu_maps_allow_and_deny_to_the_correct_reply_kinds() {
        assert_eq!(
            permission_overlay_choice(
                PermissionOverlayPage::Scope(PermissionOverlayDecision::Allow),
                0,
            ),
            PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::AllowOnce,
                scope: None,
            }
        );
        assert_eq!(
            permission_overlay_choice(
                PermissionOverlayPage::Scope(PermissionOverlayDecision::Allow),
                2,
            ),
            PermissionOverlayChoice::Reply {
                kind: PermissionReplyKind::AllowAlways,
                scope: Some(PermissionScope::Workspace),
            }
        );
        assert_eq!(
            permission_overlay_choice(
                PermissionOverlayPage::Scope(PermissionOverlayDecision::Deny),
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
