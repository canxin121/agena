use super::*;

#[cfg(test)]
mod run_activity_tests {
    use crate::app_types::{RunActivityTarget, RunActivityTracker, RunOperation};

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
        assert_eq!(tool.mode, PermissionMode::Auto);

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
    fn whole_message_highlight_includes_the_role_header() {
        let message = TranscriptNodeKey::Entry {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(7),
        };
        let child = TranscriptNodeKey::Content {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(7),
            content_id: Some(agena_tui_transcript::TranscriptContentId::StoredPart(11)),
        };
        let nodes = vec![node(child.clone(), 4, 8), node(message.clone(), 3, 8)];

        assert_eq!(
            transcript_node_highlight_range(nodes.as_slice(), &message),
            Some(3..8),
            "the complete message should include its selectable role header"
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
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10),
            content_id: agena_tui_transcript::TranscriptContentId::StoredPart(1),
            block_index: 0,
        };
        let first_message = TranscriptNodeKey::Entry {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10),
        };
        let second_message = TranscriptNodeKey::Entry {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(11),
        };
        let nodes = vec![
            node(first_child.clone(), 1, 4),
            node(first_message.clone(), 0, 4),
            node(
                TranscriptNodeKey::Content {
                    entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(11),
                    content_id: Some(agena_tui_transcript::TranscriptContentId::StoredPart(2)),
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
