impl App {
    pub(in crate::app) fn handle_permission_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionOverlay,
    ) -> bool {
        match agena_tui::permission_prompt::handle_key(&mut dialog.presentation, key) {
            PermissionPromptEffect::Close => true,
            PermissionPromptEffect::KeepOpen => false,
            PermissionPromptEffect::Activate { page, selected } => {
                self.activate_permission_overlay_choice(dialog, page, selected)
            }
        }
    }

    pub(in crate::app) fn activate_permission_overlay_choice(
        &mut self,
        dialog: &mut PermissionOverlay,
        page: PermissionPromptPage,
        selected: usize,
    ) -> bool {
        let choice = permission_overlay_choice(page, selected);
        match choice {
            PermissionOverlayChoice::OpenScope(decision) => {
                dialog.presentation.open_scope(decision);
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
                let _ = dialog.presentation.open_details();
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
        studio.return_permission = Some(dialog.clone());
        studio.presentation.footer =
            ui_text::t(&self.i18n, "overlay-permission-rule-studio-footer-return");
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
        match agena_tui::settings_studio::handle_key(&mut dialog.state, key) {
            agena_tui::settings_studio::SettingsStudioEffect::Close => true,
            agena_tui::settings_studio::SettingsStudioEffect::Refresh => {
                self.refresh_settings_studio_overlay(dialog);
                false
            }
            agena_tui::settings_studio::SettingsStudioEffect::Activate => {
                self.activate_settings_studio_selection(dialog)
            }
            agena_tui::settings_studio::SettingsStudioEffect::KeepOpen => false,
        }
    }

    pub(in crate::app) fn handle_agent_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut AgentStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => {}
                EditorDialogKeyResult::Close => {
                    dialog.editor = None;
                }
                EditorDialogKeyResult::Submit(action, input) => {
                    if let Err(error) = self.commit_agent_studio_editor(dialog, action, input) {
                        self.flash_error(error);
                    } else {
                        dialog.editor = None;
                    }
                }
            }
            return false;
        }

        match agena_tui::agent_studio::handle_key(&mut dialog.presentation, key) {
            agena_tui::agent_studio::AgentStudioEffect::Close => true,
            agena_tui::agent_studio::AgentStudioEffect::Activate => {
                self.activate_agent_studio_selection(dialog)
            }
            agena_tui::agent_studio::AgentStudioEffect::KeepOpen => false,
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
            },
            Some(KeyAction::NextTab | KeyAction::PreviousTab) => {
                let next = dialog.pane_focus.next();
                set_permission_studio_pane_focus(dialog, next);
                false
            }
            Some(KeyAction::MoveLeft) => {
                set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Navigation);
                false
            }
            Some(KeyAction::MoveRight) => {
                set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Content);
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
            Some(KeyAction::MoveUp) => {
                dialog.state.move_selection(-1);
                false
            }
            Some(KeyAction::MoveDown) => {
                dialog.state.move_selection(1);
                false
            }
            Some(KeyAction::Delete) if dialog.pane_focus == PermissionStudioPaneFocus::Content => {
                self.open_permission_studio_delete_current(dialog);
                false
            }
            Some(KeyAction::PermissionAdd) => {
                self.open_permission_studio_add_current(dialog);
                false
            }
            Some(KeyAction::PermissionRename)
                if dialog.pane_focus == PermissionStudioPaneFocus::Content =>
            {
                self.open_permission_studio_rename_current(dialog);
                false
            }
            Some(KeyAction::Activate)
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            Some(KeyAction::Activate) => self.activate_permission_studio_selection(dialog),
            _ => false,
        }
    }

    pub(in crate::app) fn handle_permission_rule_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionRuleStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => {}
                EditorDialogKeyResult::Close => {
                    dialog.editor = None;
                }
                EditorDialogKeyResult::Submit(action, input) => {
                    if let Err(error) =
                        self.commit_permission_rule_studio_editor(dialog, action, input)
                    {
                        self.flash_error(error);
                    } else {
                        dialog.editor = None;
                    }
                }
            }
            return false;
        }

        match agena_tui::permission_rule_studio::handle_key(
            &mut dialog.presentation,
            key,
            dialog.rule_id.is_some(),
        ) {
            agena_tui::permission_rule_studio::PermissionRuleStudioEffect::Close => true,
            agena_tui::permission_rule_studio::PermissionRuleStudioEffect::Browse => {
                self.browse_selected_permission_rule_path(dialog);
                false
            }
            agena_tui::permission_rule_studio::PermissionRuleStudioEffect::Save => {
                match self.commit_permission_rule_studio_save(dialog) {
                    Ok(()) if dialog.return_permission.is_some() => return true,
                    Ok(()) => {}
                    Err(error) => self.flash_error(error),
                }
                false
            }
            agena_tui::permission_rule_studio::PermissionRuleStudioEffect::Delete => {
                self.revoke_permission_rule_studio_rule(dialog)
            }
            agena_tui::permission_rule_studio::PermissionRuleStudioEffect::Activate => {
                self.activate_permission_rule_studio_selection(dialog)
            }
            agena_tui::permission_rule_studio::PermissionRuleStudioEffect::KeepOpen => false,
        }
    }
    fn browse_selected_permission_rule_path(&mut self, dialog: &mut PermissionRuleStudioOverlay) {
        let Some(item) = dialog.presentation.list.selected_item() else {
            return;
        };
        match item.action {
            PermissionRuleStudioAction::TargetPath => {
                self.open_permission_rule_path_browser(
                    dialog,
                    PermissionRuleStudioPathField::TargetPath,
                );
            }
            PermissionRuleStudioAction::WorkspaceRoot => {
                self.open_permission_rule_path_browser(
                    dialog,
                    PermissionRuleStudioPathField::WorkspaceRoot,
                );
            }
            _ => self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-permission-rule-browse-path-selection",
            )),
        }
    }
}

use crate::app::{
    AgentStudioOverlay, App, EditorDialogKeyResult, InputDialogKeyResult, KeyEvent,
    LineInputOverlay, PermissionOverlay, PermissionOverlayChoice, PermissionPromptEffect,
    PermissionPromptPage, PermissionRuleStudioAction, PermissionRuleStudioOverlay,
    PermissionRuleStudioPathField, PermissionStudioOverlay, PermissionStudioPaneFocus, Route,
    SettingsStudioOverlay, drive_editor_dialog_key, drive_input_dialog_key,
    permission_overlay_choice, permission_overlay_reply_label, permission_rule_draft_from_request,
    permission_studio_nav_move_step, set_permission_studio_pane_focus, ui_text,
};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};

#[cfg(test)]
mod tests {
    use super::PermissionStudioPaneFocus;

    #[test]
    fn permission_focus_ring_moves_in_both_directions() {
        use PermissionStudioPaneFocus::{Content, Navigation};

        assert_eq!(Navigation.next(), Content);
        assert_eq!(Content.next(), Navigation);
    }
}
