impl App {
    pub(in crate::app) fn handle_permission_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionOverlay,
    ) -> bool {
        if matches!(key.code, KeyCode::Char('i')) {
            dialog.page = match dialog.page {
                PermissionOverlayPage::Action => {
                    PermissionOverlayPage::Details(PermissionOverlayDetailsReturn::Action)
                }
                PermissionOverlayPage::Scope(decision) => {
                    PermissionOverlayPage::Details(PermissionOverlayDetailsReturn::Scope(decision))
                }
                PermissionOverlayPage::Details(PermissionOverlayDetailsReturn::Action) => {
                    PermissionOverlayPage::Action
                }
                PermissionOverlayPage::Details(PermissionOverlayDetailsReturn::Scope(decision)) => {
                    PermissionOverlayPage::Scope(decision)
                }
            };
            dialog.selection.selected = 0;
            return false;
        }
        if let PermissionOverlayPage::Details(return_to) = dialog.page {
            if matches!(key.code, KeyCode::Esc | KeyCode::Left) {
                dialog.page = match return_to {
                    PermissionOverlayDetailsReturn::Action => PermissionOverlayPage::Action,
                    PermissionOverlayDetailsReturn::Scope(decision) => {
                        PermissionOverlayPage::Scope(decision)
                    }
                };
            }
            return false;
        }
        let choice_count = permission_overlay_choices(&self.i18n, dialog.page).len();
        match key.code {
            KeyCode::Esc | KeyCode::Left => match dialog.page {
                PermissionOverlayPage::Action => true,
                PermissionOverlayPage::Scope(_) => {
                    dialog.page = PermissionOverlayPage::Action;
                    dialog.selection.selected = 0;
                    false
                }
                PermissionOverlayPage::Details(_) => unreachable!("details are handled above"),
            },
            KeyCode::Up => {
                dialog.selection.move_by(choice_count, -1);
                false
            }
            KeyCode::Down => {
                dialog.selection.move_by(choice_count, 1);
                false
            }
            KeyCode::Enter | KeyCode::Right => self.activate_permission_overlay_choice(dialog),
            _ => false,
        }
    }

    pub(in crate::app) fn activate_permission_overlay_choice(
        &mut self,
        dialog: &mut PermissionOverlay,
    ) -> bool {
        let choice = permission_overlay_choice(dialog.page, dialog.selection.selected);
        match choice {
            PermissionOverlayChoice::OpenScope(decision) => {
                dialog.page = PermissionOverlayPage::Scope(decision);
                dialog.selection.selected = 0;
                false
            }
            PermissionOverlayChoice::Reply { kind, scope } => {
                self.submit_permission_reply(
                    dialog.session_id,
                    dialog.request.clone(),
                    kind,
                    scope,
                    permission_overlay_reply_label(&self.i18n, kind, scope),
                );
                true
            }
            PermissionOverlayChoice::EditRule => {
                self.open_permission_rule_studio_from_overlay(dialog);
                true
            }
        }
    }

    pub(in crate::app) fn open_permission_rule_studio_from_overlay(
        &mut self,
        dialog: &PermissionOverlay,
    ) {
        let mut studio = self.build_permission_rule_studio_overlay(
            None,
            ui_text::t(&self.i18n, "overlay-permission-rule-workbench-title"),
            permission_rule_draft_from_request(&dialog.request),
            None,
        );
        studio.return_to_permission = true;
        studio.workbench.footer =
            ui_text::t(&self.i18n, "overlay-permission-rule-studio-footer-return");
        self.overlay = Some(Overlay::Permission(dialog.clone()));
        self.route_stack.push(Route::Main);
        self.current_route = Route::PermissionRuleStudio(studio);
    }

    pub(in crate::app) fn handle_session_rename_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut LineInputOverlay,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(_, value) => self.submit_session_rename(value.as_str()),
            InputDialogKeyResult::Continue => false,
        }
    }

    pub(in crate::app) fn handle_agent_create_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut LineInputOverlay,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(_, value) => self.create_agent_from_list(value.as_str()),
            InputDialogKeyResult::Continue => false,
        }
    }

    pub(in crate::app) fn handle_settings_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SettingsStudioOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Char('r') => {
                self.refresh_settings_studio_overlay(dialog);
                false
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l')
                if dialog.state.focus() == SettingsStudioFocus::Navigation =>
            {
                dialog.state.set_focus(SettingsStudioFocus::Items);
                false
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h')
                if dialog.state.focus() == SettingsStudioFocus::Items =>
            {
                dialog.state.set_focus(SettingsStudioFocus::Navigation);
                false
            }
            KeyCode::PageUp => {
                dialog.state.move_selection_page(-1, 10);
                false
            }
            KeyCode::PageDown => {
                dialog.state.move_selection_page(1, 10);
                false
            }
            KeyCode::Home => {
                dialog.state.move_selection_home();
                false
            }
            KeyCode::End => {
                dialog.state.move_selection_end();
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.state.move_selection(-1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.state.move_selection(1);
                false
            }
            KeyCode::Enter => self.activate_settings_studio_selection(dialog),
            _ => false,
        }
    }

    pub(in crate::app) fn handle_agent_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut AgentStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.workbench.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => {}
                EditorDialogKeyResult::Close => {
                    dialog.workbench.editor = None;
                }
                EditorDialogKeyResult::Submit(action, input) => {
                    if let Err(error) = self.commit_agent_studio_editor(dialog, action, input) {
                        self.flash_error(error);
                    } else {
                        dialog.workbench.editor = None;
                    }
                }
            }
            return false;
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::Char('r') => {
                self.refresh_agent_studio_overlay(dialog);
                false
            }
            KeyCode::Char('o') => {
                self.open_agent_profile_source(&dialog.profile);
                false
            }
            KeyCode::Char('p') => {
                self.route_stack.push(Route::AgentStudio(dialog.clone()));
                self.open_agent_permission_studio(dialog.agent_name.as_str());
                false
            }
            _ if dialog.workbench.list.handle_navigation_key(key, 10) => false,
            KeyCode::Enter => self.activate_agent_studio_selection(dialog),
            _ => false,
        }
    }

    pub(in crate::app) fn handle_permission_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => {}
                EditorDialogKeyResult::Close => {
                    dialog.editor = None;
                }
                EditorDialogKeyResult::Submit(action, input) => {
                    if let Err(error) = self.commit_permission_studio_editor(dialog, action, input)
                    {
                        self.flash_error(error);
                    } else {
                        dialog.editor = None;
                    }
                }
            }
            return false;
        }

        match key.code {
            KeyCode::Esc => match dialog.pane_focus {
                PermissionStudioPaneFocus::Navigation => true,
                PermissionStudioPaneFocus::Content => {
                    set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Navigation);
                    false
                }
            },
            KeyCode::Char('r') => {
                self.refresh_permission_studio_overlay(dialog);
                false
            }
            KeyCode::Char('a') => {
                self.open_permission_studio_add_current(dialog);
                false
            }
            KeyCode::Char('e') => self.activate_permission_studio_selection(dialog),
            KeyCode::Char('n') => {
                self.open_permission_studio_rename_current(dialog);
                false
            }
            KeyCode::Char('y') => {
                self.open_permission_studio_duplicate_current(dialog);
                false
            }
            KeyCode::Char('d') => {
                self.open_permission_studio_delete_current(dialog);
                false
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l')
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Content);
                false
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h')
                if dialog.pane_focus == PermissionStudioPaneFocus::Content =>
            {
                set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Navigation);
                false
            }
            KeyCode::PageUp if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                permission_studio_nav_move_page(&mut dialog.nav, -1, 10);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::PageDown if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                permission_studio_nav_move_page(&mut dialog.nav, 1, 10);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::Home if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                permission_studio_nav_move_home(&mut dialog.nav);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::End if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                permission_studio_nav_move_end(&mut dialog.nav);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::Up | KeyCode::Char('k')
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                permission_studio_nav_move_step(&mut dialog.nav, -1);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::Down | KeyCode::Char('j')
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                permission_studio_nav_move_step(&mut dialog.nav, 1);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::PageUp => {
                dialog.state.move_selection_page(-1, 10);
                false
            }
            KeyCode::PageDown => {
                dialog.state.move_selection_page(1, 10);
                false
            }
            KeyCode::Home => {
                dialog.state.move_selection_home();
                false
            }
            KeyCode::End => {
                dialog.state.move_selection_end();
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.state.move_selection(-1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.state.move_selection(1);
                false
            }
            KeyCode::Enter if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Content);
                false
            }
            KeyCode::Enter => self.activate_permission_studio_selection(dialog),
            _ => false,
        }
    }

    pub(in crate::app) fn handle_permission_rule_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionRuleStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.workbench.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => {}
                EditorDialogKeyResult::Close => {
                    dialog.workbench.editor = None;
                }
                EditorDialogKeyResult::Submit(action, input) => {
                    if let Err(error) =
                        self.commit_permission_rule_studio_editor(dialog, action, input)
                    {
                        self.flash_error(error);
                    } else {
                        dialog.workbench.editor = None;
                    }
                }
            }
            return false;
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::Char('r') => {
                self.refresh_permission_rule_studio(dialog);
                false
            }
            KeyCode::Char('b') => {
                self.open_selected_permission_rule_path_browser(dialog);
                false
            }
            _ if dialog.workbench.list.handle_navigation_key(key, 8) => false,
            KeyCode::Enter => self.activate_permission_rule_studio_selection(dialog),
            _ => false,
        }
    }
}
use crate::app::{
    AgentStudioOverlay, App, EditorDialogKeyResult, InputDialogKeyResult, KeyCode, KeyEvent,
    LineInputOverlay, Overlay, PermissionOverlay, PermissionOverlayChoice,
    PermissionOverlayDetailsReturn, PermissionOverlayPage, PermissionRuleStudioOverlay,
    PermissionStudioOverlay, PermissionStudioPaneFocus, Route, SettingsStudioFocus,
    SettingsStudioOverlay, drive_editor_dialog_key, drive_input_dialog_key,
    permission_overlay_choice, permission_overlay_choices, permission_overlay_reply_label,
    permission_rule_draft_from_request, permission_studio_nav_move_end,
    permission_studio_nav_move_home, permission_studio_nav_move_page,
    permission_studio_nav_move_step, set_permission_studio_pane_focus, ui_text,
};
