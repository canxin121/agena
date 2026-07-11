impl App {
    pub(in crate::app) fn handle_permission_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionOverlay,
    ) -> bool {
        let action = resolve_tui_key(KeyContext::PermissionPrompt, key);
        if let PermissionOverlayPage::Details(return_to) = dialog.page {
            if action == Some(KeyAction::Back) {
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
        match action {
            Some(KeyAction::Back) => match dialog.page {
                PermissionOverlayPage::Action => true,
                PermissionOverlayPage::Scope(_) => {
                    dialog.page = PermissionOverlayPage::Action;
                    dialog.selection.selected = 0;
                    false
                }
                PermissionOverlayPage::Details(_) => unreachable!("details are handled above"),
            },
            Some(KeyAction::MoveUp) => {
                dialog.selection.move_by(choice_count, -1);
                false
            }
            Some(KeyAction::MoveDown) => {
                dialog.selection.move_by(choice_count, 1);
                false
            }
            Some(KeyAction::Activate) => self.activate_permission_overlay_choice(dialog),
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
            PermissionOverlayChoice::Details => {
                dialog.page = match dialog.page {
                    PermissionOverlayPage::Action => {
                        PermissionOverlayPage::Details(PermissionOverlayDetailsReturn::Action)
                    }
                    PermissionOverlayPage::Scope(decision) => PermissionOverlayPage::Details(
                        PermissionOverlayDetailsReturn::Scope(decision),
                    ),
                    PermissionOverlayPage::Details(_) => return false,
                };
                dialog.selection.selected = 0;
                false
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
        match resolve_tui_key(KeyContext::SettingsStudio, key) {
            Some(KeyAction::Close) => true,
            Some(KeyAction::NextTab) if dialog.state.focus() == SettingsStudioFocus::Navigation => {
                dialog.state.set_focus(SettingsStudioFocus::Items);
                false
            }
            Some(KeyAction::PreviousTab) if dialog.state.focus() == SettingsStudioFocus::Items => {
                dialog.state.set_focus(SettingsStudioFocus::Navigation);
                false
            }
            Some(KeyAction::PageUp) => {
                dialog.state.move_selection_page(-1, 10);
                false
            }
            Some(KeyAction::PageDown) => {
                dialog.state.move_selection_page(1, 10);
                false
            }
            Some(KeyAction::Home) => {
                dialog.state.move_selection_home();
                false
            }
            Some(KeyAction::End) => {
                dialog.state.move_selection_end();
                false
            }
            Some(KeyAction::MoveUp) => {
                dialog.state.move_selection(-1);
                false
            }
            Some(KeyAction::MoveDown) => {
                dialog.state.move_selection(1);
                false
            }
            Some(KeyAction::Activate) => self.activate_settings_studio_selection(dialog),
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

        match resolve_tui_key(KeyContext::AgentStudio, key) {
            Some(KeyAction::Close) => true,
            _ if dialog
                .workbench
                .list
                .handle_structural_navigation_key(key, 10) =>
            {
                false
            }
            Some(KeyAction::Activate) => self.activate_agent_studio_selection(dialog),
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

        match resolve_tui_key(KeyContext::PermissionStudio, key) {
            Some(KeyAction::Back) => match dialog.pane_focus {
                PermissionStudioPaneFocus::Navigation => true,
                PermissionStudioPaneFocus::Content => {
                    set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Navigation);
                    false
                }
                PermissionStudioPaneFocus::Actions => {
                    set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Content);
                    false
                }
            },
            Some(KeyAction::NextTab) => {
                let next = match dialog.pane_focus {
                    PermissionStudioPaneFocus::Navigation => PermissionStudioPaneFocus::Content,
                    PermissionStudioPaneFocus::Content
                        if permission_studio_action_count(dialog) > 0 =>
                    {
                        PermissionStudioPaneFocus::Actions
                    }
                    PermissionStudioPaneFocus::Content | PermissionStudioPaneFocus::Actions => {
                        PermissionStudioPaneFocus::Navigation
                    }
                };
                set_permission_studio_pane_focus(dialog, next);
                false
            }
            Some(KeyAction::PreviousTab) => {
                let next = match dialog.pane_focus {
                    PermissionStudioPaneFocus::Navigation
                        if permission_studio_action_count(dialog) > 0 =>
                    {
                        PermissionStudioPaneFocus::Actions
                    }
                    PermissionStudioPaneFocus::Navigation => PermissionStudioPaneFocus::Content,
                    PermissionStudioPaneFocus::Content => PermissionStudioPaneFocus::Navigation,
                    PermissionStudioPaneFocus::Actions => PermissionStudioPaneFocus::Content,
                };
                set_permission_studio_pane_focus(dialog, next);
                false
            }
            Some(KeyAction::MoveLeft)
                if dialog.pane_focus == PermissionStudioPaneFocus::Actions =>
            {
                dialog.selected_action = dialog.selected_action.saturating_sub(1);
                false
            }
            Some(KeyAction::MoveRight)
                if dialog.pane_focus == PermissionStudioPaneFocus::Actions =>
            {
                dialog.selected_action = (dialog.selected_action + 1)
                    .min(permission_studio_action_count(dialog).saturating_sub(1));
                false
            }
            Some(KeyAction::PageUp)
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                permission_studio_nav_move_page(&mut dialog.nav, -1, 10);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            Some(KeyAction::PageDown)
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                permission_studio_nav_move_page(&mut dialog.nav, 1, 10);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            Some(KeyAction::Home) if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                permission_studio_nav_move_home(&mut dialog.nav);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            Some(KeyAction::End) if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                permission_studio_nav_move_end(&mut dialog.nav);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            Some(KeyAction::MoveUp)
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                permission_studio_nav_move_step(&mut dialog.nav, -1);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            Some(KeyAction::MoveDown)
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                permission_studio_nav_move_step(&mut dialog.nav, 1);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            Some(KeyAction::PageUp) => {
                dialog.state.move_selection_page(-1, 10);
                false
            }
            Some(KeyAction::PageDown) => {
                dialog.state.move_selection_page(1, 10);
                false
            }
            Some(KeyAction::Home) => {
                dialog.state.move_selection_home();
                false
            }
            Some(KeyAction::End) => {
                dialog.state.move_selection_end();
                false
            }
            Some(KeyAction::MoveUp) => {
                dialog.state.move_selection(-1);
                false
            }
            Some(KeyAction::MoveDown) => {
                dialog.state.move_selection(1);
                false
            }
            Some(KeyAction::Activate)
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Content);
                false
            }
            Some(KeyAction::Activate)
                if dialog.pane_focus == PermissionStudioPaneFocus::Actions =>
            {
                self.activate_permission_studio_action(dialog)
            }
            Some(KeyAction::Activate) => self.activate_permission_studio_selection(dialog),
            _ => false,
        }
    }

    fn activate_permission_studio_action(&mut self, dialog: &mut PermissionStudioOverlay) -> bool {
        match dialog.selected_action {
            0 => self.open_permission_studio_add_current(dialog),
            1 => return self.activate_permission_studio_selection(dialog),
            2 => self.open_permission_studio_rename_current(dialog),
            3 => self.open_permission_studio_duplicate_current(dialog),
            4 => self.open_permission_studio_delete_current(dialog),
            _ => {}
        }
        false
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

        match resolve_tui_key(KeyContext::PermissionRuleStudio, key) {
            Some(KeyAction::Close) => true,
            _ if dialog
                .workbench
                .list
                .handle_structural_navigation_key(key, 8) =>
            {
                false
            }
            Some(KeyAction::Activate) => self.activate_permission_rule_studio_selection(dialog),
            _ => false,
        }
    }
}

fn permission_studio_action_count(dialog: &PermissionStudioOverlay) -> usize {
    if !dialog.editable {
        return 0;
    }
    match dialog.state.selected_section().map(|section| section.id) {
        Some(
            crate::app::PermissionStudioSectionId::PathRules
            | crate::app::PermissionStudioSectionId::NetworkRules
            | crate::app::PermissionStudioSectionId::ToolTags
            | crate::app::PermissionStudioSectionId::ToolNames
            | crate::app::PermissionStudioSectionId::ToolCommandRules,
        ) => 5,
        _ => 0,
    }
}
use crate::app::{
    AgentStudioOverlay, App, EditorDialogKeyResult, InputDialogKeyResult, KeyEvent,
    LineInputOverlay, Overlay, PermissionOverlay, PermissionOverlayChoice,
    PermissionOverlayDetailsReturn, PermissionOverlayPage, PermissionRuleStudioOverlay,
    PermissionStudioOverlay, PermissionStudioPaneFocus, Route, SettingsStudioFocus,
    SettingsStudioOverlay, drive_editor_dialog_key, drive_input_dialog_key,
    permission_overlay_choice, permission_overlay_choices, permission_overlay_reply_label,
    permission_rule_draft_from_request, permission_studio_nav_move_end,
    permission_studio_nav_move_home, permission_studio_nav_move_page,
    permission_studio_nav_move_step, set_permission_studio_pane_focus, ui_text,
};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
