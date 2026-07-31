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

mod ui;

macro_rules! api_message_part {
    ($id:expr, $message_id:expr, $created_at:expr, $status:expr, PartContent::text($text:expr $(,)?) $(,)?) => {
        crate::TranscriptFixture::text_part($id, $message_id, $created_at, $status, $text)
    };
    ($id:expr, $message_id:expr, $created_at:expr, $status:expr, PartContent::Reasoning($reasoning:expr) $(,)?) => {
        crate::TranscriptFixture::reasoning_part($id, $message_id, $created_at, $status, $reasoning)
    };
    ($id:expr, $message_id:expr, $created_at:expr, $status:expr, PartContent::Text($text_part:expr) $(,)?) => {{
        let text_part = $text_part;
        crate::TranscriptFixture::text_part_with_flags(
            $id,
            $message_id,
            $created_at,
            $status,
            text_part.text,
            text_part.synthetic,
        )
    }};
}

#[cfg(test)]
mod transcript_character_cursor_tests {
    use super::super::{
        ExecutionStatus, MessageResource, MessageRole, MessageStatus, TranscriptFixture,
        TranscriptMoveDirection, TranscriptState, TranscriptTextPosition, Utc,
    };
    use unicode_width::UnicodeWidthStr;

    fn message(id: i64, text: &str) -> MessageResource {
        message_with_role(id, MessageRole::Assistant, text)
    }

    fn message_with_role(id: i64, role: MessageRole, text: &str) -> MessageResource {
        let now = Utc::now();
        MessageResource {
            id,
            session_id: 7,
            role,
            state: MessageStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: 1,
            parts: Some(vec![TranscriptFixture::text_part(
                id * 10,
                id,
                now,
                ExecutionStatus::Completed,
                text,
            )]),
        }
    }

    fn snapshot_turn(
        sequence: i64,
        input: &str,
        response: &str,
        status: agena_domain::ResponseStatus,
    ) -> agena_domain::TurnSnapshot {
        let turn_id = agena_domain::TurnId::new();
        agena_domain::TurnSnapshot {
            id: turn_id,
            session_id: 7,
            sequence,
            input: agena_domain::ContentDocument::new(vec![agena_domain::ContentNode::text(input)]),
            response: agena_domain::ResponseSnapshot {
                id: agena_domain::ResponseId::new(),
                turn_id,
                execution_id: agena_domain::ExecutionId::new(),
                status,
                content: if response.is_empty() {
                    agena_domain::ContentDocument::default()
                } else {
                    agena_domain::ContentDocument::new(vec![agena_domain::ContentNode::text(
                        response,
                    )])
                },
                revision_seq: 1,
                created_at_ms: sequence * 10,
                finished_at_ms: status.is_terminal().then_some(sequence * 10 + 1),
            },
            created_at_ms: sequence * 10,
        }
    }

    fn display_column_before(text: &str, marker: &str) -> usize {
        let byte_index = text.find(marker).expect("test marker must be rendered");
        UnicodeWidthStr::width(&text[..byte_index])
    }

    #[test]
    fn normal_mode_cursor_moves_one_grapheme_and_restores_its_column_after_short_rows() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![message(1, "a界🙂z\n\nq\n\nabcdefgh")],
            ..TranscriptState::default()
        };
        let rows = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.text.contains("a界🙂z"))
            .map(|(index, line)| (index, display_column_before(line.text.as_str(), "a界🙂z")))
            .collect::<Vec<_>>();
        let (first_line, first_column) = *rows.first().expect("first source row");
        let short_line = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(index, line)| {
                line.text
                    .contains("q")
                    .then(|| (index, display_column_before(line.text.as_str(), "q")))
            })
            .expect("short source row");
        let long_line = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(index, line)| {
                line.text
                    .contains("abcdefgh")
                    .then(|| (index, display_column_before(line.text.as_str(), "abcdefgh")))
            })
            .expect("later source row");

        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first_line,
                column: 0,
            },
        );
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((first_line, first_column..first_column + 1)),
            "the character cursor must clamp past layout-only indentation"
        );

        let header_line = transcript
            .rendered(80)
            .lines
            .iter()
            .position(|line| line.text.starts_with("assistant"))
            .expect("assistant role header");
        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: header_line,
                column: 0,
            },
        );
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((header_line, 0..1)),
            "role labels are selectable transcript text"
        );

        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first_line,
                column: first_column,
            },
        );
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((first_line, first_column..first_column + 1))
        );

        transcript.move_cursor_horizontally(80, 20, true, 1);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((first_line, first_column + 1..first_column + 3))
        );
        transcript.move_cursor_horizontally(80, 20, true, 1);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((first_line, first_column + 3..first_column + 5))
        );
        transcript.move_cursor_horizontally(80, 20, false, 1);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((first_line, first_column + 1..first_column + 3))
        );

        // Reset onto the long row, then verify Vim-style desired-column
        // preservation through the one-character middle row.
        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first_line,
                column: first_column + 6,
            },
        );
        transcript.move_cursor_by_visual_lines(80, 20, TranscriptMoveDirection::Down, 1);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((short_line.0, short_line.1..short_line.1 + 1))
        );
        transcript.move_cursor_by_visual_lines(80, 20, TranscriptMoveDirection::Down, 1);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((long_line.0, long_line.1 + 6..long_line.1 + 7))
        );
    }

    #[test]
    fn visual_character_and_line_modes_extend_the_cursor_range_like_vim() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![message(1, "a界🙂z\n\nq\n\nabcdefgh")],
            ..TranscriptState::default()
        };
        let first = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(line, rendered)| {
                rendered.text.contains("a界🙂z").then(|| {
                    (
                        line,
                        display_column_before(rendered.text.as_str(), "a界🙂z"),
                    )
                })
            })
            .expect("first source row");
        let second = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(line, rendered)| {
                rendered
                    .text
                    .contains("q")
                    .then(|| (line, display_column_before(rendered.text.as_str(), "q")))
            })
            .expect("second source row");

        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first.0,
                column: first.1,
            },
        );
        transcript.toggle_visual_selection(
            80,
            20,
            super::super::TranscriptVisualSelectionMode::Character,
        );
        assert_eq!(
            transcript
                .text_selection()
                .and_then(|selection| selection.cell_range_for_line(first.0)),
            Some(first.1..first.1 + 1)
        );
        transcript.move_cursor_horizontally(80, 20, true, 1);
        assert_eq!(
            transcript
                .text_selection()
                .and_then(|selection| selection.cell_range_for_line(first.0)),
            Some(first.1..first.1 + 3),
            "v plus l selects the next complete wide grapheme"
        );

        transcript.toggle_visual_selection(
            80,
            20,
            super::super::TranscriptVisualSelectionMode::Line,
        );
        assert_eq!(
            transcript
                .text_selection()
                .and_then(|selection| selection.cell_range_for_line(first.0)),
            Some(0..usize::MAX)
        );
        transcript.move_cursor_by_visual_lines(80, 20, TranscriptMoveDirection::Down, 1);
        assert_eq!(
            transcript
                .text_selection()
                .and_then(|selection| selection.cell_range_for_line(first.0)),
            Some(0..usize::MAX)
        );
        assert_eq!(
            transcript
                .text_selection()
                .and_then(|selection| selection.cell_range_for_line(second.0)),
            Some(0..usize::MAX),
            "V plus j selects every complete rendered row between anchor and cursor"
        );

        transcript.cancel_text_selection(80, 20);
        assert!(!transcript.has_visual_selection());
        assert_eq!(transcript.text_selection(), None);

        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first.0,
                column: first.1,
            },
        );
        transcript.toggle_visual_selection(
            80,
            20,
            super::super::TranscriptVisualSelectionMode::Block,
        );
        transcript.move_cursor_by_visual_lines(80, 20, TranscriptMoveDirection::Down, 1);
        let block_ranges = transcript.selection_cell_ranges(80);
        assert_eq!(block_ranges[first.0], Some(first.1..first.1 + 1));
        assert_eq!(block_ranges[second.0], Some(first.1..first.1 + 1));
        assert_eq!(
            transcript.selected_text(80, ""),
            Some("a\n\nq".to_string()),
            "Ctrl+V copies the selected terminal-cell rectangle without card chrome"
        );
    }

    #[test]
    fn ctrl_message_motion_skips_to_the_adjacent_message_without_block_highlighting() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![message(1, "first message"), message(2, "second message")],
            ..TranscriptState::default()
        };
        let first = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(line, rendered)| rendered.text.contains("first message").then_some(line))
            .expect("first message line");
        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first,
                column: 0,
            },
        );

        transcript.move_cursor_by_messages(80, 20, TranscriptMoveDirection::Down, 1);

        let (line, _) = transcript.cursor_cell_range(80).expect("character cursor");
        assert!(
            transcript.rendered(80).lines[line]
                .text
                .contains("assistant")
        );
        assert_eq!(transcript.highlighted_block_key(), None);
    }

    #[test]
    fn vim_word_line_find_and_reselect_motions_keep_character_cursor_semantics() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![message(1, "  one,two three")],
            ..TranscriptState::default()
        };
        let (line, column) = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(line, rendered)| {
                rendered.text.contains("one,two three").then(|| {
                    (
                        line,
                        display_column_before(rendered.text.as_str(), "one,two three"),
                    )
                })
            })
            .expect("rendered text line");
        transcript.select_pointer_line(80, 20, TranscriptTextPosition { line, column });

        transcript.move_cursor_to_line_start(80, 20, true);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((line, column..column + 1))
        );

        transcript.move_cursor_by_words(80, 20, true, false, false, 1);
        let comma_column = display_column_before(
            transcript.rendered(80).lines[line].text.as_str(),
            ",two three",
        );
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((line, comma_column..comma_column + 1)),
            "w stops at punctuation, matching Vim's keyword-word split"
        );

        transcript.move_cursor_to_find(80, 20, true, false, 't', 2);
        let second_t =
            display_column_before(transcript.rendered(80).lines[line].text.as_str(), "three");
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((line, second_t..second_t + 1))
        );

        transcript.toggle_visual_selection(
            80,
            20,
            super::super::TranscriptVisualSelectionMode::Character,
        );
        transcript.move_cursor_to_line_end(80, 20);
        let selected = transcript.selected_text(80, "").expect("Visual text");
        assert_eq!(selected, "three");
        transcript.cancel_text_selection(80, 20);
        transcript.reselect_last_visual_selection(80, 20);
        assert_eq!(transcript.selected_text(80, ""), Some("three".to_string()));

        let two_column = display_column_before(
            transcript.rendered(80).lines[line].text.as_str(),
            "two three",
        );
        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line,
                column: two_column,
            },
        );
        assert!(transcript.select_current_word_text_object(80, 20, false));
        assert_eq!(transcript.selected_text(80, ""), Some("two".to_string()));
        transcript.cancel_text_selection(80, 20);
        assert!(transcript.select_current_word_text_object(80, 20, true));
        assert_eq!(transcript.selected_text(80, ""), Some("two ".to_string()));
    }

    #[test]
    fn markdown_and_message_text_objects_select_semantic_ranges() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![message(1, "first paragraph\n\nsecond paragraph")],
            ..TranscriptState::default()
        };
        let first_line = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(line, rendered)| rendered.text.contains("first paragraph").then_some(line))
            .expect("first markdown block");
        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first_line,
                column: 0,
            },
        );
        assert!(transcript.select_current_text_object(80, 20, false));
        assert_eq!(
            transcript.selected_text(80, ""),
            Some("first paragraph".to_string())
        );

        transcript.cancel_text_selection(80, 20);
        assert!(transcript.select_current_text_object(80, 20, true));
        let message = transcript
            .selected_text(80, "")
            .expect("message Visual range");
        assert!(message.contains("first paragraph"));
        assert!(message.contains("second paragraph"));
    }

    #[test]
    fn cancelled_response_activity_is_rendered_after_its_user_turn_as_an_assistant_outcome() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            snapshot: agena_domain::TranscriptSnapshot {
                session_id: 7,
                seq_session: 3,
                turns: vec![snapshot_turn(
                    1,
                    "please answer",
                    "partial assistant response",
                    agena_domain::ResponseStatus::Cancelled,
                )],
                session_activities: Vec::new(),
            },
            ..TranscriptState::default()
        };
        let lines = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>();
        let user = lines
            .iter()
            .position(|line| line.contains("please answer"))
            .expect("user message");
        let response = lines
            .iter()
            .position(|line| line.contains("partial assistant response"))
            .expect("assistant response");
        let cancelled = lines
            .iter()
            .position(|line| line.contains("Response cancelled"))
            .expect("cancelled outcome");

        assert!(user < response && response < cancelled, "{lines:#?}");
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("Response cancelled"))
                .count(),
            1
        );
        assert!(lines.iter().any(|line| line.starts_with("assistant –")));
        assert!(lines.iter().all(|line| !line.starts_with("system –")));
        assert_eq!(
            lines[cancelled], "  ▸ Response cancelled",
            "a response outcome must use the visible Activity headline contract"
        );
        assert!(transcript.rendered(80).nodes.iter().any(|node| {
            matches!(
                node.key,
                agena_tui_transcript::TranscriptNodeKey::Activity {
                    content_id: agena_tui_transcript::TranscriptContentId::ResponseLifecycle(_),
                    ..
                }
            ) && node.kind == agena_tui_transcript::TranscriptNodeKind::Activity
        }));
    }

    #[test]
    fn cancelled_response_activity_never_moves_across_a_later_user_turn() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            snapshot: agena_domain::TranscriptSnapshot {
                session_id: 7,
                seq_session: 4,
                turns: vec![
                    snapshot_turn(
                        1,
                        "cancel this turn",
                        "",
                        agena_domain::ResponseStatus::Cancelled,
                    ),
                    snapshot_turn(
                        2,
                        "the next turn",
                        "next answer",
                        agena_domain::ResponseStatus::Completed,
                    ),
                ],
                session_activities: Vec::new(),
            },
            ..TranscriptState::default()
        };
        let lines = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>();
        let first_user = lines
            .iter()
            .position(|line| line.contains("cancel this turn"))
            .expect("first user");
        let cancelled = lines
            .iter()
            .position(|line| line.contains("Response cancelled"))
            .expect("cancelled outcome");
        let second_user = lines
            .iter()
            .position(|line| line.contains("the next turn"))
            .expect("later user");

        assert!(
            first_user < cancelled && cancelled < second_user,
            "{lines:#?}"
        );
        assert!(lines[cancelled.saturating_sub(1)].starts_with("assistant –"));
    }

    #[test]
    fn delayed_cancellations_stay_with_their_original_user_turns() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            snapshot: agena_domain::TranscriptSnapshot {
                session_id: 7,
                seq_session: 8,
                turns: vec![
                    snapshot_turn(1, "first turn", "", agena_domain::ResponseStatus::Cancelled),
                    snapshot_turn(
                        2,
                        "second turn",
                        "",
                        agena_domain::ResponseStatus::Cancelled,
                    ),
                ],
                session_activities: Vec::new(),
            },
            ..TranscriptState::default()
        };

        let lines = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>();
        let first_user = lines
            .iter()
            .position(|line| line.contains("first turn"))
            .expect("first user message");
        let second_user = lines
            .iter()
            .position(|line| line.contains("second turn"))
            .expect("second user message");
        let cancellations = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| line.contains("Response cancelled").then_some(index))
            .collect::<Vec<_>>();

        assert_eq!(cancellations.len(), 2, "{lines:#?}");
        assert!(
            first_user < cancellations[0]
                && cancellations[0] < second_user
                && second_user < cancellations[1],
            "{lines:#?}"
        );
    }
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
    use super::super::{PendingUserMessage, TranscriptState};
    use agena_domain::{
        ActivityId, ActivityPayload, ActivityProvenance, ComposerActivity, ComposerDocument,
        ComposerNode, ContentDocument, ContentNode, ExecutionId, ResponseId, ResponseSnapshot,
        ResponseStatus, SkillReferenceActivity, TranscriptSnapshot, TurnId, TurnSnapshot,
    };

    #[test]
    fn optimistic_user_document_uses_the_same_activity_and_placeholder_projection() {
        let activity_id = ActivityId::new();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 9,
            document: ComposerDocument(vec![
                ComposerNode::Text {
                    text: "use ".to_owned(),
                },
                ComposerNode::Activity {
                    activity: Box::new(ComposerActivity {
                        id: activity_id,
                        payload: ActivityPayload::SkillReference(SkillReferenceActivity {
                            name: "batch".to_owned(),
                            description: "Run independent work".to_owned(),
                            instructions: "Use isolated tasks".to_owned(),
                            content_hash: "sha256:test".to_owned(),
                            source: "test".to_owned(),
                            aliases: Vec::new(),
                        }),
                        provenance: ActivityProvenance::default(),
                    }),
                },
                ComposerNode::Text {
                    text: " now".to_owned(),
                },
            ]),
            confirmed: false,
        });

        let rendered = transcript.rendered(100);
        let activity_line = rendered
            .lines
            .iter()
            .position(|line| line.text.contains("Skill: batch"))
            .expect("full optimistic Skill Activity");
        let document_line = rendered
            .lines
            .iter()
            .position(|line| line.text.contains("use [Skill: batch] now"))
            .expect("inline optimistic placeholder");
        assert!(activity_line < document_line);
        assert!(rendered.nodes.iter().any(|node| {
            node.key
                == agena_tui_transcript::TranscriptNodeKey::Activity {
                    entry_id: agena_tui_transcript::TranscriptEntryId::PendingTurn(9),
                    content_id: agena_tui_transcript::TranscriptContentId::Activity(activity_id),
                }
        }));
    }

    #[test]
    fn confirmed_optimistic_message_is_atomically_replaced_by_its_persisted_message() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 42,
            document: ComposerDocument(vec![ComposerNode::Text {
                text: "send this now".to_string(),
            }]),
            confirmed: false,
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

        let turn_id = TurnId::new();
        transcript.merge_snapshot(TranscriptSnapshot {
            session_id: 7,
            seq_session: 1,
            turns: vec![TurnSnapshot {
                id: turn_id,
                session_id: 7,
                sequence: 1,
                input: ContentDocument::new(vec![ContentNode::text("send this now")]),
                response: ResponseSnapshot {
                    id: ResponseId::new(),
                    turn_id,
                    execution_id: ExecutionId::new(),
                    status: ResponseStatus::InProgress,
                    content: ContentDocument::default(),
                    revision_seq: 1,
                    created_at_ms: 1,
                    finished_at_ms: None,
                },
                created_at_ms: 1,
            }],
            session_activities: Vec::new(),
        });

        assert!(transcript.pending_user_messages.is_empty());
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
    use agena_domain::{ComposerDocument, ComposerNode, ExecutionStatus};

    fn pending_document(text: String) -> ComposerDocument {
        ComposerDocument(vec![ComposerNode::Text { text }])
    }

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
            document: pending_document(
                (0..40)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            confirmed: false,
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
            document: pending_document(
                (0..80)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            confirmed: false,
        });

        transcript.scroll_to_bottom(40, 10);
        let bottom = transcript.viewport.top;
        assert!(bottom > 3);
        assert!(transcript.viewport.follow_tail);
        let bottom_cursor = transcript.navigation_cursor_line().expect("tail cursor");
        let bottom_screen_row = bottom_cursor - bottom;

        transcript.move_cursor_by_wheel(40, 10, -3);
        assert!(transcript.viewport.top < bottom);
        assert!(!transcript.viewport.follow_tail);
        let upward_cursor = transcript
            .navigation_cursor_line()
            .expect("wheel keeps a semantic cursor");
        assert!(upward_cursor < bottom_cursor);
        assert_eq!(upward_cursor - transcript.viewport.top, bottom_screen_row);
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
            document: pending_document(
                (0..80)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            confirmed: false,
        });

        transcript.scroll_to_top(40, 10);
        transcript.relocate_cursor_from_scrollbar(40, 10, 5);
        assert_eq!(transcript.viewport.top, 5);
        assert!(matches!(transcript.navigation_cursor_line(), Some(5..=7)));

        transcript.relocate_cursor_from_scrollbar(40, 10, 30);
        assert_eq!(transcript.viewport.top, 30);
        assert!(matches!(transcript.navigation_cursor_line(), Some(30..=32)));

        transcript.move_cursor_by_half_page(40, 10, false);
        let cursor = transcript
            .navigation_cursor_line()
            .expect("half-page motion keeps a semantic cursor");
        assert!(transcript.viewport.top < 30);
        assert!((7..=9).contains(&(cursor - transcript.viewport.top)));
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
            .position(|node| {
                node.key
                    == TranscriptNodeKey::Entry {
                        entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(2),
                    }
            })
            .expect("second message parent");
        transcript.set_block_cursor(40, 10, second_message, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::Entry {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(2)
            })
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
            Some(TranscriptNodeKey::Entry {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(1)
            })
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
    use agena_domain::{
        ActivityActor, ActivityId, ActivityLifecycle, ActivityNode, ActivityOwner, ActivityPayload,
        ActivityProvenance, ActivityState, ContentDocument, ContentNode, ContentPosition,
        ExecutionId, ExecutionStatus, OperationActivity, ReasoningPart, ResponseId,
        ResponseSnapshot, ResponseStatus, StructuredObject, ToolCallId, ToolInvocation, ToolOutput,
        TranscriptSnapshot, TurnId, TurnSnapshot,
    };

    use super::super::{
        MessageResource, MessageRole, MessageStatus, TranscriptMoveDirection, TranscriptNodeKey,
        TranscriptNodeKind, TranscriptState, TranscriptTextPosition, TranscriptTextSelection, Utc,
        transcript_text_selection_text,
    };

    #[test]
    fn canonical_tools_list_activity_toggles_open_through_transcript_state() {
        let turn_id = TurnId::new();
        let response_id = ResponseId::new();
        let activity_id = ActivityId::new();
        let details = ToolOutput::from_json_payload(Some(&serde_json::json!({
            "tools": [{"name": "repo.status"}, {"name": "fs.read"}]
        })))
        .expect("tools_list output");
        let operation = ActivityNode {
            id: activity_id,
            owner: ActivityOwner::Response { response_id },
            actor: ActivityActor::Tool,
            state: ActivityState::Completed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Operation(OperationActivity {
                call_id: ToolCallId::new("call-tools-list"),
                invocation: ToolInvocation::new("tools_list", StructuredObject::default()),
                title: "tools_list".to_owned(),
                summary: String::new(),
                model_output_text: String::new(),
                details,
                resource_activity_ids: Vec::new(),
                error: None,
            }),
            provenance: ActivityProvenance::default(),
        };
        let mut transcript = TranscriptState {
            session_id: Some(7),
            snapshot: TranscriptSnapshot {
                session_id: 7,
                seq_session: 1,
                turns: vec![TurnSnapshot {
                    id: turn_id,
                    session_id: 7,
                    sequence: 1,
                    input: ContentDocument::new(vec![ContentNode::text("list tools")]),
                    response: ResponseSnapshot {
                        id: response_id,
                        turn_id,
                        execution_id: ExecutionId::new(),
                        status: ResponseStatus::Completed,
                        content: ContentDocument::new(vec![ContentNode::activity(operation)]),
                        revision_seq: 1,
                        created_at_ms: 1,
                        finished_at_ms: Some(2),
                    },
                    created_at_ms: 1,
                }],
                session_activities: Vec::new(),
            },
            detail_expanded_by_default: agena_tui_transcript::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            ..TranscriptState::default()
        };
        let key = TranscriptNodeKey::Activity {
            entry_id: agena_tui_transcript::TranscriptEntryId::Response(response_id),
            content_id: agena_tui_transcript::TranscriptContentId::Activity(activity_id),
        };
        let collapsed = transcript
            .rendered(100)
            .nodes
            .iter()
            .find(|node| node.key == key)
            .cloned()
            .expect("collapsed canonical tools_list Activity");
        assert!(collapsed.toggleable);
        assert!(!collapsed.expanded);
        assert!(
            transcript
                .rendered(100)
                .lines
                .iter()
                .all(|line| !line.text.contains("repo.status"))
        );

        transcript.set_cursor_line(100, 20, collapsed.start_line);
        assert_eq!(
            transcript.toggle_cursor_node_expansion(100, 20),
            Some((TranscriptNodeKind::Activity, true))
        );
        let expanded = transcript
            .rendered(100)
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("expanded canonical tools_list Activity");
        assert!(expanded.expanded);
        assert!(
            transcript
                .rendered(100)
                .lines
                .iter()
                .any(|line| line.text.contains("repo.status"))
        );
    }

    #[test]
    fn collapsing_from_inside_an_activity_keeps_the_cursor_on_that_activity() {
        let now = Utc::now();
        let message_id = 17;
        let part_id = 23;
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
        let key = TranscriptNodeKey::Activity {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(message_id),
            content_id: agena_tui_transcript::TranscriptContentId::Activity(
                part.activity_id.expect("reasoning activity identity"),
            ),
        };
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
        let preceding_part = api_message_part!(
            22,
            17,
            now,
            ExecutionStatus::Completed,
            PartContent::text("one\n\ntwo\n\nthree\n\nfour\n\nfive"),
        );
        let activity = api_message_part!(
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
        let activity_key = TranscriptNodeKey::Activity {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(18),
            content_id: agena_tui_transcript::TranscriptContentId::Activity(
                activity.activity_id.expect("reasoning activity identity"),
            ),
        };
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
                    parts: Some(vec![activity]),
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
            .position(|node| {
                node.key
                    == TranscriptNodeKey::Entry {
                        entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(9),
                    }
            })
            .expect("first message parent");
        transcript.set_block_cursor(24, 20, first_message_index, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::Entry {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10)
            }),
            "crossing a boundary first selects the complete destination message"
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert!(matches!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::MarkdownBlock {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10),
                ..
            })
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
            .find(|node| {
                !node.key.is_entry_container()
                    && node.key.entry_id()
                        == agena_tui_transcript::TranscriptEntryId::StoredMessage(11)
            })
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
            Some(TranscriptNodeKey::Entry {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10)
            }),
            "Up crossing a boundary first selects the complete previous message"
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Up);
        assert!(matches!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::MarkdownBlock {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10),
                ..
            })
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
                        TranscriptNodeKey::MarkdownBlock {
                            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10),
                            ..
                        }
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
                    TranscriptNodeKey::MarkdownBlock {
                        entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(11),
                        ..
                    }
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

        let config = agena_tui_media::MathLayoutConfig {
            native_graphics: true,
            cell_width: 10,
            cell_height: 20,
            ..agena_tui_media::MathLayoutConfig::default()
        };
        let context = agena_tui_media::test_support::test_math_render_context(config);
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
        agena_tui_media::with_math_render_context(&context, || {
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
                .position(|node| matches!(node.key, TranscriptNodeKey::Entry { .. }))
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
                }),
            ),
            api_message_part!(
                3,
                42,
                now,
                ExecutionStatus::Completed,
                PartContent::Text(TextPart {
                    text: "hidden".to_string(),
                    synthetic: true,
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
    use agena_domain::{
        ActivityOwner, ContentDocument, ContentNode, EventMeta, ExecutionId, ResponseId,
        ResponseSegmentId, ResponseSnapshot, ResponseStatus, TranscriptPatch, TranscriptSnapshot,
        TurnId, TurnSnapshot,
    };
    use agena_runtime::{RuntimePresentationEvent, RuntimePresentationEventKind};
    use uuid::Uuid;

    use super::super::{TranscriptState, Utc};

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

    fn text_patch(
        response_id: ResponseId,
        segment_id: ResponseSegmentId,
        text: &str,
        seq: i64,
    ) -> RuntimePresentationEvent {
        event(
            RuntimePresentationEventKind::TranscriptPatch(TranscriptPatch::ContentUpserted {
                seq_session: seq,
                owner: ActivityOwner::Response { response_id },
                node: ContentNode::text_at(segment_id, text, 0, seq),
            }),
            seq,
        )
    }

    fn turn(sequence: i64) -> TurnSnapshot {
        let turn_id = TurnId::new();
        TurnSnapshot {
            id: turn_id,
            session_id: 7,
            sequence,
            input: ContentDocument::new(vec![ContentNode::text("question")]),
            response: ResponseSnapshot {
                id: ResponseId::new(),
                turn_id,
                execution_id: ExecutionId::new(),
                status: ResponseStatus::InProgress,
                content: ContentDocument::default(),
                revision_seq: 0,
                created_at_ms: sequence,
                finished_at_ms: None,
            },
            created_at_ms: sequence,
        }
    }

    #[test]
    fn live_text_patch_is_rendered_without_waiting_for_a_refresh() {
        let turn = turn(1);
        let response_id = turn.response.id;
        let segment_id = ResponseSegmentId::new();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            snapshot: TranscriptSnapshot {
                session_id: 7,
                turns: vec![turn],
                ..Default::default()
            },
            ..TranscriptState::default()
        };

        assert!(!transcript.apply_presentation_event(
            &text_patch(response_id, segment_id, "I'm Grok", 2),
            80,
            20,
        ));

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
    fn repeated_live_upserts_replace_one_stable_segment() {
        let turn = turn(1);
        let response_id = turn.response.id;
        let segment_id = ResponseSegmentId::new();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            snapshot: TranscriptSnapshot {
                session_id: 7,
                turns: vec![turn],
                ..Default::default()
            },
            ..TranscriptState::default()
        };

        transcript.apply_presentation_event(
            &text_patch(response_id, segment_id, "first", 2),
            80,
            20,
        );
        transcript.apply_presentation_event(
            &text_patch(response_id, segment_id, "first second", 3),
            80,
            20,
        );

        assert_eq!(
            transcript.snapshot.turns[0].response.content.nodes().len(),
            1
        );
        let rendered = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("first second"));
        assert_eq!(rendered.matches("first second").count(), 1);
    }
}
