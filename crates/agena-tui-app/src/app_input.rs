impl App {
    pub(crate) fn handle_key_event(&mut self, key: KeyEvent) {
        if matches!(key.kind, KeyEventKind::Release) {
            return;
        }

        self.refresh_input_derived_state();

        if self.focus != Focus::Transcript
            || !self.current_route_is_main()
            || self.overlay.is_some()
            || self.context_help.is_some()
            || resolve_tui_key(KeyContext::Transcript, key) != Some(KeyAction::Copy)
        {
            self.transcript_yank_pending = false;
        }

        let global_action = resolve_tui_key(KeyContext::Global, key);
        if prompt_history_preempts_global_interrupt(self.prompt_history_search.is_some(), key) {
            self.handle_prompt_history_search_key(key);
            return;
        }

        if global_action == Some(KeyAction::Interrupt) {
            let now = Instant::now();
            let double = self
                .last_ctrl_c_at
                .map(|previous| now.duration_since(previous) <= self.double_esc_window)
                .unwrap_or(false);
            if double {
                // A stuck plugin-host callback cannot observe cancellation
                // until its waiter is released. Never let that prevent a
                // user from leaving the TUI entirely.
                self.should_quit = true;
                return;
            }
            if let Some(session_id) = self.active_run_session_id() {
                self.request_cancel_run(session_id);
                self.last_ctrl_c_at = Some(now);
                self.flash_warning(ui_text::t(&self.i18n, "flash-run-cancelling-quit"));
                return;
            }
            self.last_ctrl_c_at = Some(now);
            self.flash_warning(ui_text::t(&self.i18n, "flash-quit-confirm"));
            return;
        }

        if global_action == Some(KeyAction::Help) {
            self.toggle_context_help();
            return;
        }

        self.last_ctrl_c_at = None;

        if self.handle_context_help_key(key) {
            return;
        }

        if self.handle_overlay_key(key) {
            return;
        }

        if self.handle_route_key(key) {
            return;
        }

        self.maybe_capture_transcript_motion_prefix(key);

        if !self.current_route_is_main() {
            return;
        }

        let main_action = resolve_tui_key(KeyContext::Main, key);
        let composer_child_owns_tab = self.focus == Focus::Composer
            && (self.prompt_history_search.is_some()
                || ((self.file_mention_suggestions.is_some()
                    || self.slash_command_suggestions.is_some())
                    && resolve_tui_key(KeyContext::Suggestion, key).is_some())
                || (self.composer_item_selection.is_active()
                    && resolve_tui_key(KeyContext::ComposerItem, key).is_some()));
        if !composer_child_owns_tab
            && let Some(action @ (KeyAction::NextTab | KeyAction::PreviousTab)) = main_action
        {
            self.focus = self
                .focus
                .move_pane(if action == KeyAction::NextTab { 1 } else { -1 });
            self.sync_composer_suggestions();
            return;
        }

        if self.focus != Focus::Composer
            && let Some(action) = main_action
        {
            match action {
                KeyAction::SearchForward | KeyAction::SearchBackward => {
                    self.focus = Focus::Transcript;
                    self.open_transcript_search_overlay(action == KeyAction::SearchForward);
                    return;
                }
                KeyAction::SearchPrevious
                    if self.focus == Focus::Transcript
                        && !self.transcript.search_query.trim().is_empty() =>
                {
                    self.jump_search_match(!self.transcript_search_forward);
                    return;
                }
                KeyAction::SearchNext
                    if self.focus == Focus::Transcript
                        && !self.transcript.search_query.trim().is_empty() =>
                {
                    self.jump_search_match(self.transcript_search_forward);
                    return;
                }
                KeyAction::New => {
                    self.create_session(None);
                    return;
                }
                KeyAction::Continue => {
                    self.continue_current_session();
                    return;
                }
                KeyAction::OpenUsage => {
                    self.open_usage_dashboard("");
                    return;
                }
                _ => {}
            }
        }

        match self.focus {
            Focus::Sessions => self.handle_sessions_key(key),
            Focus::Transcript => self.handle_transcript_key(key),
            Focus::Composer => self.handle_composer_key(key),
        }
        self.maybe_auto_open_pending_interactive_overlay();
    }

    pub(crate) fn maybe_capture_transcript_motion_prefix(&mut self, key: KeyEvent) {
        if self.focus != Focus::Transcript
            || !self.current_route_is_main()
            || self.overlay.is_some()
        {
            self.transcript_motion_prefix = None;
            return;
        }
        if !key.modifiers.is_empty() {
            self.transcript_motion_prefix = None;
            return;
        }
        match resolve_tui_key(KeyContext::Transcript, key) {
            Some(KeyAction::CountDigit(digit @ 1..=9)) => {
                self.transcript_motion_prefix
                    .get_or_insert_with(String::new)
                    .push(char::from(b'0' + digit));
            }
            Some(KeyAction::CountDigit(0)) if self.transcript_motion_prefix.is_some() => {
                if let Some(prefix) = self.transcript_motion_prefix.as_mut() {
                    prefix.push('0');
                }
            }
            Some(
                KeyAction::MoveUp
                | KeyAction::MoveDown
                | KeyAction::MoveLeft
                | KeyAction::MoveRight,
            ) => {}
            _ => {
                self.transcript_motion_prefix = None;
            }
        }
    }

    pub(crate) fn transcript_motion_count(&mut self) -> usize {
        self.transcript_motion_prefix
            .take()
            .and_then(|prefix| prefix.parse::<usize>().ok())
            .filter(|count| *count > 0)
            .unwrap_or(1)
    }

    pub(crate) fn handle_overlay_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut overlay) = self.overlay.take() else {
            return false;
        };

        let close = match &mut overlay {
            Overlay::TranscriptSearch(dialog) => {
                self.handle_line_overlay_key(key, dialog, OverlayCommit::TranscriptSearch)
            }
            Overlay::SessionRename(dialog) => self.handle_session_rename_overlay_key(key, dialog),
            Overlay::AgentCreate(dialog) => self.handle_agent_create_overlay_key(key, dialog),
            Overlay::SettingsValueEdit(dialog) => {
                self.handle_settings_value_edit_overlay_key(key, dialog)
            }
            Overlay::Choice(dialog) => self.handle_choice_overlay_key(key, dialog),
            Overlay::FileAttach(dialog) => self.handle_file_attach_overlay_key(key, dialog),
            Overlay::PathBrowser(dialog) => self.handle_path_browser_overlay_key(key, dialog),
            Overlay::Permission(dialog) => self.handle_permission_overlay_key(key, dialog),
            Overlay::UserInputReply(dialog) => self.handle_user_input_overlay_key(key, dialog),
            Overlay::Confirm(dialog) => self.handle_confirm_overlay_key(key, dialog),
            Overlay::SessionSearch(dialog) => self.handle_session_search_overlay_key(key, dialog),
            Overlay::Timeline(dialog) => self.handle_timeline_overlay_key(key, dialog),
            Overlay::ProviderStudio(dialog) => self.handle_provider_studio_overlay_key(key, dialog),
            Overlay::ModelCatalogStudio(dialog) => {
                self.handle_model_catalog_studio_overlay_key(key, dialog)
            }
        };

        if !close {
            if self.overlay.is_none() {
                self.overlay = Some(overlay);
            }
        } else if self.overlay.is_none() && !self.current_route_is_main() {
            // Some modal actions promote their workflow to a full-screen
            // route. Do not restore an overlay-stack parent above that route;
            // the route owns any suspended modal it needs to restore later.
        } else if self.overlay.is_none()
            && let Some(parent) = self.overlay_stack.pop()
        {
            self.overlay = Some(self.refresh_restored_overlay(parent));
        } else if self.overlay.is_none() {
            self.maybe_auto_open_pending_interactive_overlay();
        }

        true
    }

    pub(crate) fn handle_route_key(&mut self, key: KeyEvent) -> bool {
        let route = std::mem::replace(&mut self.current_route, Route::Main);
        let mut route = match route {
            Route::Main => return false,
            route => route,
        };

        let close = match &mut route {
            Route::Main => false,
            Route::Usage(dialog) => self.handle_usage_dashboard_key(key, dialog),
            Route::SettingsStudio(dialog) => self.handle_settings_studio_overlay_key(key, dialog),
            Route::AgentStudio(dialog) => self.handle_agent_studio_overlay_key(key, dialog),
            Route::PermissionStudio(dialog) => {
                self.handle_permission_studio_overlay_key(key, dialog)
            }
            Route::PermissionRuleStudio(dialog) => {
                self.handle_permission_rule_studio_overlay_key(key, dialog)
            }
            Route::SessionSearch(dialog) => self.handle_session_search_overlay_key(key, dialog),
            Route::CommandPalette(dialog) => self.handle_command_palette_key(key, dialog),
            Route::SessionNavigation(dialog) => self.handle_session_navigation_key(key, dialog),
            Route::SelectionPicker(dialog) => self.handle_selection_picker_key(key, dialog),
            Route::SessionModelChooser(dialog) => {
                self.handle_session_model_chooser_overlay_key(key, dialog)
            }
            Route::Timeline(dialog) => self.handle_timeline_overlay_key(key, dialog),
            Route::PluginWorkbench(dialog) => self.handle_plugin_workbench_key(key, dialog),
            Route::ProviderStudio(dialog) => self.handle_provider_studio_overlay_key(key, dialog),
            Route::ModelCatalogStudio(dialog) => {
                self.handle_model_catalog_studio_overlay_key(key, dialog)
            }
        };

        if !close {
            if self.current_route_is_main() {
                self.current_route = route;
            }
        } else if self.current_route_is_main() {
            let return_permission = match &route {
                Route::PermissionRuleStudio(dialog) => dialog.return_permission.clone(),
                _ => None,
            };
            if let Some(parent) = self.route_stack.pop() {
                self.current_route = self.refresh_restored_route(parent);
            } else {
                self.current_route = Route::Main;
            }
            if let Some(permission) = return_permission {
                self.overlay = Some(Overlay::Permission(permission));
            } else {
                self.maybe_auto_open_pending_interactive_overlay();
            }
        }

        true
    }
}

fn prompt_history_preempts_global_interrupt(history_open: bool, key: KeyEvent) -> bool {
    history_open
        && resolve_tui_key(KeyContext::Global, key) == Some(KeyAction::Interrupt)
        && resolve_tui_key(KeyContext::PromptHistory, key) == Some(KeyAction::Close)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::prompt_history_preempts_global_interrupt;

    #[test]
    fn prompt_history_ctrl_c_closes_before_the_global_interrupt_handler() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert!(prompt_history_preempts_global_interrupt(true, ctrl_c));
        assert!(!prompt_history_preempts_global_interrupt(false, ctrl_c));
        assert!(!prompt_history_preempts_global_interrupt(
            true,
            KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ),
        ));
    }
}
use crate::{App, Instant, KeyEvent, KeyEventKind, Overlay, OverlayCommit, Route, ui_text};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;
