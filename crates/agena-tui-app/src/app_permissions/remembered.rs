use crate::{
    App, ChoiceItem, ChoiceOverlayAction, PermissionMode, PermissionRuleDraft,
    PermissionRuleSubjectKind, PermissionStudioOverlay, PermissionStudioSource, Route,
    permission_rule_draft_from_resource, permission_rule_draft_label, permission_rule_label,
};
use agena_api::{
    queries::{GetSessionParams, ListPermissionRulesParams, Query, QueryResult},
    resource::PermissionRuleResource,
};

const NEW_RULE: &str = "__new_permission_rule__";

fn rule_matches_source(
    rule: &PermissionRuleResource,
    source: &PermissionStudioSource,
    workspace_id: Option<i64>,
) -> bool {
    if rule.revoked_at.is_some() {
        return false;
    }
    let global = rule.scope == "global";
    let workspace =
        rule.scope == "workspace" && workspace_id.is_some() && rule.workspace_id == workspace_id;
    match source {
        PermissionStudioSource::GlobalConfig => global,
        PermissionStudioSource::WorkspaceConfig => workspace,
        PermissionStudioSource::Session { session_id } => {
            rule.scope == "session" && rule.session_id == Some(*session_id)
        }
        PermissionStudioSource::EffectiveSession { session_id } => {
            global || workspace || (rule.scope == "session" && rule.session_id == Some(*session_id))
        }
    }
}

fn rules_editable(source: &PermissionStudioSource) -> bool {
    matches!(
        source,
        PermissionStudioSource::GlobalConfig | PermissionStudioSource::WorkspaceConfig
    )
}

impl App {
    pub(crate) fn open_remembered_permission_rules(&mut self, dialog: &PermissionStudioOverlay) {
        if crate::permission_studio_has_draft(dialog) {
            self.flash_warning(self.i18n.text("permission-studio-save-or-cancel"));
            return;
        }
        let source = dialog.source.clone();
        let requested_source = source.clone();
        let origin_workspace_id = self.application.workspace_id();
        let request = uuid::Uuid::new_v4();
        self.permission_studio_load_request = Some(request);
        self.dispatch_backend_operation(
            move |application| async move {
                let workspace_id = match &requested_source {
                    PermissionStudioSource::Session { session_id }
                    | PermissionStudioSource::EffectiveSession { session_id } => {
                        let QueryResult::Session(session) = application
                            .client()
                            .query_revalidated(Query::GetSession(GetSessionParams {
                                session_id: *session_id,
                            }))
                            .await?
                        else {
                            anyhow::bail!("expected a session for the permission rule scope");
                        };
                        session.workspace_id
                    }
                    _ => Some(application.workspace_id()),
                };
                let mut rules = Vec::new();
                let mut cursor = None;
                loop {
                    let QueryResult::PermissionRules(page) = application
                        .client()
                        .query_revalidated(Query::ListPermissionRules(ListPermissionRulesParams {
                            cursor: cursor.clone(),
                            limit: Some(200),
                            search: None,
                        }))
                        .await?
                    else {
                        anyhow::bail!("expected a permission rule page");
                    };
                    rules.extend(
                        page.items.into_iter().filter(|rule| {
                            rule_matches_source(rule, &requested_source, workspace_id)
                        }),
                    );
                    if !page.page.has_more {
                        break;
                    }
                    let next = page.page.next_cursor;
                    if next.is_none() || next == cursor {
                        anyhow::bail!("permission rule pagination did not advance");
                    }
                    cursor = next;
                }
                Ok((workspace_id, rules))
            },
            move |app, result| {
                if app.permission_studio_load_request != Some(request) {
                    return;
                }
                app.permission_studio_load_request = None;
                if app.application.workspace_id() != origin_workspace_id
                    || app.overlay.is_some()
                    || !matches!(&app.current_route, Route::PermissionStudio(current)
                        if current.source == source && !crate::permission_studio_has_draft(current))
                {
                    return;
                }
                match result {
                    Ok((workspace_id, rules)) => {
                        app.show_remembered_permission_rules(source, workspace_id, rules)
                    }
                    Err(error) => app.flash_error(error),
                }
            },
        );
    }

    pub(crate) fn show_remembered_permission_rules(
        &mut self,
        source: PermissionStudioSource,
        workspace_id: Option<i64>,
        rules: Vec<PermissionRuleResource>,
    ) {
        let editable = rules_editable(&source);
        let mut items = Vec::new();
        if editable {
            let label = self.i18n.text("permission-studio-remembered-create");
            items.push(ChoiceItem {
                search_text: label.clone(),
                label,
                detail: self.i18n.text("permission-studio-remembered-detail"),
                value: NEW_RULE.to_string(),
                current: false,
            });
        }
        items.extend(
            rules
                .iter()
                .filter(|rule| rule_matches_source(rule, &source, workspace_id))
                .map(|rule| {
                    let label = permission_rule_label(&self.i18n, rule);
                    let draft = permission_rule_draft_from_resource(rule);
                    let detail = format!(
                        "{} · {}",
                        crate::permission_mode_label(&self.i18n, draft.mode),
                        rule.scope
                    );
                    ChoiceItem {
                        search_text: format!("{label} {detail}"),
                        label,
                        detail,
                        value: rule.id.to_string(),
                        current: false,
                    }
                }),
        );
        let mut picker = self.build_choice_overlay(
            self.i18n.text("permission-studio-remembered-rules"),
            self.i18n.text(if editable {
                "permission-studio-remembered-detail"
            } else {
                "permission-studio-remembered-read-only"
            }),
            None,
            items,
            ChoiceOverlayAction::RememberedPermissionRules {
                source,
                workspace_id,
                rules,
            },
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        );
        picker.presentation.empty_message = self.i18n.text("permission-studio-remembered-empty");
        self.open_choice_overlay(picker);
    }

    pub(crate) fn open_remembered_permission_rule_selection(
        &mut self,
        source: PermissionStudioSource,
        workspace_id: Option<i64>,
        rules: Vec<PermissionRuleResource>,
        selected: &str,
    ) {
        let Route::PermissionStudio(parent) = &self.current_route else {
            return;
        };
        if parent.source != source || crate::permission_studio_has_draft(parent) {
            return;
        }
        let editable = rules_editable(&source);
        let (rule_id, draft) = if selected == NEW_RULE && editable {
            (
                None,
                PermissionRuleDraft {
                    subject_kind: PermissionRuleSubjectKind::Tool,
                    tool_name: String::new(),
                    qualifier: String::new(),
                    path_access_kind: "read".to_string(),
                    workspace_root: self.application.workspace_root().display().to_string(),
                    target_path: String::new(),
                    network_target: String::new(),
                    network_host: String::new(),
                    network_port: String::new(),
                    scope: if source == PermissionStudioSource::GlobalConfig {
                        "global"
                    } else {
                        "workspace"
                    }
                    .to_string(),
                    session_id: String::new(),
                    mode: PermissionMode::Ask,
                },
            )
        } else {
            let Some(rule) = rules.iter().find(|rule| {
                rule.id.to_string() == selected && rule_matches_source(rule, &source, workspace_id)
            }) else {
                return;
            };
            (Some(rule.id), permission_rule_draft_from_resource(rule))
        };
        let title = format!(
            "{} · {}",
            self.i18n.text("permission-studio-remembered-rules"),
            permission_rule_draft_label(&self.i18n, &draft)
        );
        let mut editor = self.build_permission_rule_studio_overlay(rule_id, title, draft, None);
        editor.editable = editable;
        editor.scope_locked = true;
        if !editable {
            editor.presentation.footer = self
                .i18n
                .text("permission-studio-remembered-read-only-footer");
        }
        let parent =
            std::mem::replace(&mut self.current_route, Route::PermissionRuleStudio(editor));
        self.route_stack.push(parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn rule(
        id: i64,
        scope: &str,
        workspace_id: Option<i64>,
        session_id: Option<i64>,
    ) -> PermissionRuleResource {
        serde_json::from_value(serde_json::json!({
            "id": id, "action_key": format!("tool:test-{id}"), "subject_kind": "tool",
            "tool_name": format!("test-{id}"), "mode": "ask", "scope": scope,
            "workspace_id": workspace_id, "session_id": session_id, "source": "user",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    fn app() -> (tempfile::TempDir, App) {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app = App::new_with_backend(
            backend,
            crate::LaunchOptions::default(),
            crate::I18n::english(),
        );
        app.draft_store_path = temp.path().join("drafts.json");
        app.prompt_history_path = temp.path().join("history.jsonl");
        let mut permission: agena_domain::PermissionConfig =
            serde_json::from_value(serde_json::json!({
                "path": {"workspace": "readonly"}, "network": {"default": "deny"},
                "tools": {"default": "ask", "names": {"test": "deny"}}
            }))
            .unwrap();
        permission.approval_model = Some(agena_domain::ApprovalModelSelection::from_model_ref(
            &agena_domain::ModelRef::new("test", "approval"),
        ));
        let mut parent = PermissionStudioOverlay {
            title: String::new(),
            footer: String::new(),
            source: PermissionStudioSource::WorkspaceConfig,
            title_context: String::new(),
            editable: true,
            permission,
            inherited_permission: agena_domain::PermissionConfig::global_default(),
            network_rule: None,
            save_request: None,
            page: crate::PermissionStudioPage::Access,
            state: agena_tui_components::SectionedListState::new(
                Vec::new(),
                0,
                0,
                crate::PermissionStudioFocus::Items,
            ),
            editor: None,
        };
        crate::refresh_permission_studio_dialog(
            &app.i18n,
            &mut parent,
            Some(crate::PermissionStudioSectionId::RememberedRules),
            None,
            None,
        );
        app.current_route = Route::PermissionStudio(parent);
        (temp, app)
    }

    #[test]
    fn permission_remembered_scope_filters_include_effective_and_bot_sources() {
        let rules = vec![
            rule(1, "global", None, None),
            rule(2, "workspace", Some(7), None),
            rule(3, "workspace", Some(8), None),
            rule(4, "session", Some(7), Some(9)),
            rule(5, "session", Some(7), Some(10)),
        ];
        for (source, workspace_id, expected) in [
            (PermissionStudioSource::GlobalConfig, Some(7), vec![1]),
            (PermissionStudioSource::WorkspaceConfig, Some(7), vec![2]),
            (
                PermissionStudioSource::Session { session_id: 9 },
                Some(7),
                vec![4],
            ),
            (
                PermissionStudioSource::EffectiveSession { session_id: 9 },
                Some(7),
                vec![1, 2, 4],
            ),
            (
                PermissionStudioSource::EffectiveSession { session_id: 9 },
                None,
                vec![1, 4],
            ),
        ] {
            assert_eq!(
                rules
                    .iter()
                    .filter(|rule| rule_matches_source(rule, &source, workspace_id))
                    .map(|rule| rule.id)
                    .collect::<Vec<_>>(),
                expected
            );
        }
        let mut revoked = rules[0].clone();
        revoked.revoked_at = Some(revoked.updated_at);
        assert!(!rule_matches_source(
            &revoked,
            &PermissionStudioSource::GlobalConfig,
            None
        ));
    }

    #[test]
    fn permission_remembered_editor_returns_to_exact_parent_and_locks_scope() {
        let (_temp, mut app) = app();
        let Route::PermissionStudio(parent) = &app.current_route else {
            unreachable!()
        };
        let before = parent.clone();
        let workspace_id = Some(app.application.workspace_id());
        app.open_remembered_permission_rule_selection(
            PermissionStudioSource::WorkspaceConfig,
            workspace_id,
            vec![rule(1, "workspace", workspace_id, None)],
            "1",
        );
        let Route::PermissionRuleStudio(editor) = &app.current_route else {
            panic!("expected remembered rule editor")
        };
        assert!(editor.editable && editor.scope_locked);
        assert!(editor.return_permission.is_none());
        let mut edited = editor.clone();
        edited.draft.scope = "global".to_string();
        assert!(app.commit_permission_rule_studio_save(&mut edited).is_err());
        assert!(edited.save_request.is_none());
        app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let Route::PermissionStudio(after) = &app.current_route else {
            panic!("expected access permissions parent")
        };
        assert_eq!(after.permission, before.permission);
        assert_eq!(after.source, before.source);
        assert_eq!(
            after.state.selected_section_index(),
            before.state.selected_section_index()
        );
        assert_eq!(
            after.state.selected_item_index(),
            before.state.selected_item_index()
        );
    }

    #[test]
    fn permission_remembered_session_and_effective_editors_are_read_only() {
        for source in [
            PermissionStudioSource::Session { session_id: 9 },
            PermissionStudioSource::EffectiveSession { session_id: 9 },
        ] {
            let (_temp, mut app) = app();
            let Route::PermissionStudio(parent) = &mut app.current_route else {
                unreachable!()
            };
            parent.source = source.clone();
            let workspace_id = Some(app.application.workspace_id());
            app.show_remembered_permission_rules(
                source.clone(),
                workspace_id,
                vec![rule(1, "session", workspace_id, Some(9))],
            );
            let Some(crate::Overlay::Choice(picker)) = &app.overlay else {
                unreachable!()
            };
            assert!(
                !picker
                    .presentation
                    .items
                    .iter()
                    .any(|item| item.value == NEW_RULE)
            );
            app.overlay = None;
            app.open_remembered_permission_rule_selection(
                source,
                workspace_id,
                vec![rule(1, "session", workspace_id, Some(9))],
                "1",
            );
            let Route::PermissionRuleStudio(editor) = &app.current_route else {
                unreachable!()
            };
            assert!(!editor.editable);
            let before = editor.draft.clone();
            for key in [
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            ] {
                app.handle_key_event(key);
            }
            let Route::PermissionRuleStudio(editor) = &app.current_route else {
                unreachable!()
            };
            assert_eq!(editor.draft, before);
            assert!(editor.editor.is_none() && editor.save_request.is_none());
            assert!(app.overlay.is_none());
        }
    }

    #[test]
    fn permission_remembered_nested_hub_navigation_preserves_parent_draft() {
        let (_temp, mut app) = app();
        let workspace_id = Some(app.application.workspace_id());
        app.open_remembered_permission_rule_selection(
            PermissionStudioSource::WorkspaceConfig,
            workspace_id,
            vec![rule(1, "workspace", workspace_id, None)],
            "1",
        );
        let Route::PermissionStudio(parent) = app.route_stack.last_mut().unwrap() else {
            unreachable!()
        };
        parent.editor = Some(crate::PermissionStudioEditor::new(
            String::new(),
            String::new(),
            String::new(),
            true,
            crate::Editor::from_text("pending parent".to_owned()),
            crate::PermissionStudioEditorAction::EditNetworkPolicy,
        ));
        app.handle_key_event(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
        assert!(matches!(app.current_route, Route::PermissionRuleStudio(_)));
        app.open_hub();
        assert!(matches!(app.current_route, Route::PermissionRuleStudio(_)));
        app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let Route::PermissionStudio(parent) = &app.current_route else {
            unreachable!()
        };
        assert_eq!(
            parent.editor.as_ref().unwrap().input.text(),
            "pending parent"
        );
    }
}
