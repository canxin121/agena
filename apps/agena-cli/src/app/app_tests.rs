use super::{
    ConfigJsonSources, I18n, JsonValue, PermissionAction, PermissionConfig, PermissionMode,
    PermissionOverlayChoice, PermissionOverlayDecision, PermissionOverlayPage, PermissionReplyKind,
    PermissionRequest, PermissionRiskLevel, PermissionRuleSubjectKind, PermissionScope,
    PermissionStudioModeTarget, RenderedTranscriptNode, SettingsPickerAction, ToolPermissionRules,
    TranscriptBlockCursor, TranscriptBlockSelectionMode, TranscriptMoveDirection,
    TranscriptNodeKey, TranscriptNodeKind, TranscriptVerticalNavigationStep, Utc,
    apply_permission_studio_mode_input, initial_search_match_index, path_rule_modes,
    permission_overlay_choice, permission_overlay_choices, permission_rule_draft_from_request,
    settings_studio_permission_items, transcript_message_navigation_target,
    transcript_node_highlight_range, transcript_selection_scroll_position,
    transcript_should_fall_back_to_message_navigation, transcript_should_follow_tail,
    transcript_vertical_line_navigation_step, transcript_vertical_navigation_step,
};

#[cfg(test)]
mod prompt_history_tests {
    use super::super::{
        App, ComposerDraft, Editor, PROMPT_HISTORY_PAGE_SIZE, PromptHistory,
        PromptHistorySearchMeta, PromptHistorySearchState,
    };

    #[test]
    fn history_search_is_newest_first_and_loads_bounded_pages() {
        let history = PromptHistory {
            items: (0..55).map(|index| format!("prompt {index:02}")).collect(),
        };
        let mut search = PromptHistorySearchState::new(
            Editor::default(),
            0,
            PromptHistorySearchMeta {
                original: ComposerDraft::default(),
                loaded_count: PROMPT_HISTORY_PAGE_SIZE,
                total_matches: 0,
                has_more: false,
            },
        );

        App::refresh_prompt_history_search(&history, &mut search);
        assert_eq!(search.items.len(), PROMPT_HISTORY_PAGE_SIZE);
        assert_eq!(search.items[0].text, "prompt 54");
        assert_eq!(search.items.last().expect("page item").text, "prompt 35");
        assert_eq!(search.meta.total_matches, 55);
        assert!(search.meta.has_more);

        search.meta.loaded_count += PROMPT_HISTORY_PAGE_SIZE;
        App::refresh_prompt_history_search(&history, &mut search);
        assert_eq!(search.items.len(), PROMPT_HISTORY_PAGE_SIZE * 2);
        assert_eq!(
            search.items.last().expect("second page item").text,
            "prompt 15"
        );
        assert!(search.meta.has_more);
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
        PermissionStudioModeTarget, SettingsPickerAction, ToolPermissionRules,
        apply_permission_studio_mode_input, path_rule_modes, settings_studio_permission_items,
    };

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
        RenderedTranscriptNode, TranscriptBlockCursor, TranscriptBlockSelectionMode,
        TranscriptMoveDirection, TranscriptNodeKey, TranscriptNodeKind,
        TranscriptVerticalNavigationStep, initial_search_match_index,
        transcript_message_navigation_target, transcript_node_highlight_range,
        transcript_selection_scroll_position, transcript_should_fall_back_to_message_navigation,
        transcript_should_follow_tail, transcript_vertical_line_navigation_step,
        transcript_vertical_navigation_step,
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
    fn transcript_only_follows_new_output_when_the_cursor_is_at_the_tail() {
        assert!(transcript_should_follow_tail(9, 10, true));
        assert!(!transcript_should_follow_tail(8, 10, true));
        assert!(!transcript_should_follow_tail(9, 10, false));
        assert!(transcript_should_follow_tail(0, 0, false));
    }

    fn cursor(
        key: TranscriptNodeKey,
        direction: TranscriptMoveDirection,
        mode: TranscriptBlockSelectionMode,
    ) -> TranscriptBlockCursor {
        TranscriptBlockCursor {
            key,
            direction,
            mode,
        }
    }

    #[test]
    fn vertical_navigation_visits_messages_and_children_in_order() {
        let first_code = TranscriptNodeKey::MarkdownBlock {
            message_id: 10,
            part_id: 1,
            block_index: 0,
        };
        let first_list = TranscriptNodeKey::MarkdownBlock {
            message_id: 10,
            part_id: 1,
            block_index: 1,
        };
        let first_message = TranscriptNodeKey::Message { message_id: 10 };
        let second_message = TranscriptNodeKey::Message { message_id: 11 };
        let nodes = vec![
            node(first_code.clone(), 1, 3),
            node(first_list.clone(), 3, 5),
            node(first_message.clone(), 0, 5),
            node(
                TranscriptNodeKey::MessagePart {
                    message_id: 11,
                    part_id: Some(2),
                },
                6,
                7,
            ),
            node(second_message.clone(), 5, 7),
        ];

        assert_eq!(
            transcript_vertical_navigation_step(
                nodes.as_slice(),
                2,
                None,
                TranscriptMoveDirection::Down,
            ),
            Some(TranscriptVerticalNavigationStep::SelectNode {
                node_index: 2,
                mode: TranscriptBlockSelectionMode::Entering,
            }),
            "the first down-navigation press selects the complete message"
        );
        assert_eq!(
            transcript_vertical_navigation_step(
                nodes.as_slice(),
                0,
                Some(&cursor(
                    first_message.clone(),
                    TranscriptMoveDirection::Down,
                    TranscriptBlockSelectionMode::Entering,
                )),
                TranscriptMoveDirection::Down,
            ),
            Some(TranscriptVerticalNavigationStep::SelectNode {
                node_index: 0,
                mode: TranscriptBlockSelectionMode::Entering,
            }),
            "the next down-navigation press enters the first child"
        );
        assert_eq!(
            transcript_vertical_navigation_step(
                nodes.as_slice(),
                1,
                Some(&cursor(
                    first_code.clone(),
                    TranscriptMoveDirection::Down,
                    TranscriptBlockSelectionMode::Entering,
                )),
                TranscriptMoveDirection::Down,
            ),
            Some(TranscriptVerticalNavigationStep::MoveToLine(1)),
            "the next down-navigation press enters the selected child"
        );
        assert_eq!(
            transcript_vertical_line_navigation_step(
                nodes.as_slice(),
                1,
                TranscriptMoveDirection::Down,
            ),
            Some(TranscriptVerticalNavigationStep::MoveToLine(2)),
            "inside a child, down-navigation advances one rendered line"
        );
        assert_eq!(
            transcript_vertical_line_navigation_step(
                nodes.as_slice(),
                2,
                TranscriptMoveDirection::Down,
            ),
            Some(TranscriptVerticalNavigationStep::SelectNode {
                node_index: 1,
                mode: TranscriptBlockSelectionMode::Entering,
            }),
            "leaving the final line selects the following child first"
        );
        assert_eq!(
            transcript_vertical_navigation_step(
                nodes.as_slice(),
                3,
                Some(&cursor(
                    first_list.clone(),
                    TranscriptMoveDirection::Down,
                    TranscriptBlockSelectionMode::Entering,
                )),
                TranscriptMoveDirection::Down,
            ),
            Some(TranscriptVerticalNavigationStep::MoveToLine(3)),
            "a selected child enters at its first line"
        );
        assert_eq!(
            transcript_vertical_navigation_step(
                nodes.as_slice(),
                3,
                Some(&cursor(
                    first_list.clone(),
                    TranscriptMoveDirection::Up,
                    TranscriptBlockSelectionMode::Entering,
                )),
                TranscriptMoveDirection::Up,
            ),
            Some(TranscriptVerticalNavigationStep::MoveToLine(4)),
            "up-navigation enters a selected block at its final line"
        );
        assert_eq!(
            transcript_vertical_line_navigation_step(
                nodes.as_slice(),
                3,
                TranscriptMoveDirection::Up,
            ),
            Some(TranscriptVerticalNavigationStep::SelectNode {
                node_index: 0,
                mode: TranscriptBlockSelectionMode::Entering,
            }),
            "crossing upward selects the previous child before entering it"
        );
        assert_eq!(
            transcript_vertical_navigation_step(
                nodes.as_slice(),
                2,
                Some(&cursor(
                    first_code.clone(),
                    TranscriptMoveDirection::Up,
                    TranscriptBlockSelectionMode::Entering,
                )),
                TranscriptMoveDirection::Up,
            ),
            Some(TranscriptVerticalNavigationStep::MoveToLine(2)),
            "the next upward press enters the previous child at its last line"
        );
        assert_eq!(
            transcript_vertical_line_navigation_step(
                nodes.as_slice(),
                1,
                TranscriptMoveDirection::Up,
            ),
            Some(TranscriptVerticalNavigationStep::SelectNode {
                node_index: 2,
                mode: TranscriptBlockSelectionMode::Leaving,
            }),
            "only the first child exits upward to the enclosing message"
        );
        assert_eq!(
            transcript_vertical_navigation_step(
                nodes.as_slice(),
                0,
                Some(&cursor(
                    first_message,
                    TranscriptMoveDirection::Up,
                    TranscriptBlockSelectionMode::Entering,
                )),
                TranscriptMoveDirection::Up,
            ),
            Some(TranscriptVerticalNavigationStep::SelectNode {
                node_index: 1,
                mode: TranscriptBlockSelectionMode::Entering,
            }),
            "up-navigation selects the current message's final block before entering it"
        );
        assert_eq!(
            transcript_vertical_navigation_step(
                nodes.as_slice(),
                4,
                Some(&cursor(
                    first_list,
                    TranscriptMoveDirection::Up,
                    TranscriptBlockSelectionMode::Entering,
                )),
                TranscriptMoveDirection::Up,
            ),
            Some(TranscriptVerticalNavigationStep::MoveToLine(4)),
            "the next up-navigation press enters that block at its final line"
        );
        assert_eq!(
            transcript_vertical_navigation_step(
                nodes.as_slice(),
                0,
                Some(&cursor(
                    second_message,
                    TranscriptMoveDirection::Up,
                    TranscriptBlockSelectionMode::Entering,
                )),
                TranscriptMoveDirection::Up,
            ),
            Some(TranscriptVerticalNavigationStep::SelectNode {
                node_index: 3,
                mode: TranscriptBlockSelectionMode::Entering,
            }),
            "the same rule applies to every message"
        );
    }

    #[test]
    fn final_line_of_the_final_message_selects_its_block_without_wrapping() {
        let final_child = TranscriptNodeKey::MarkdownBlock {
            message_id: 10,
            part_id: 1,
            block_index: 0,
        };
        let final_message = TranscriptNodeKey::Message { message_id: 10 };
        let nodes = vec![node(final_child, 1, 4), node(final_message, 0, 4)];

        assert_eq!(
            transcript_vertical_line_navigation_step(
                nodes.as_slice(),
                3,
                TranscriptMoveDirection::Down,
            ),
            Some(TranscriptVerticalNavigationStep::SelectNode {
                node_index: 0,
                mode: TranscriptBlockSelectionMode::Leaving,
            }),
            "down from the final rendered line selects the complete block for copying"
        );
        assert!(
            !transcript_should_fall_back_to_message_navigation(nodes.as_slice(), 3),
            "the caller must not re-enter the enclosing message at its first line"
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
