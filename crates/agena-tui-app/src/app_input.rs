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
        {
            self.clear_transcript_pending_command();
        }

        let global_action = resolve_tui_key(KeyContext::Global, key);
        if prompt_history_preempts_global_interrupt(self.prompt_history_search.is_some(), key) {
            self.handle_prompt_history_search_key(key);
            return;
        }

        // Ctrl+C has a Composer-local meaning while the editor is actively
        // accepting a non-empty draft. This must run before the global
        // interrupt handler: otherwise clearing a draft would start the
        // double-Ctrl+C quit sequence (or cancel an active run) instead.
        if global_action == Some(KeyAction::Interrupt) && self.composer_owns_ctrl_c_to_clear() {
            self.reset_prompt_history_recall();
            self.clear_composer_state();
            self.last_ctrl_c_at = None;
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

        // Ctrl+H is the global contextual-help toggle: it opens help for the
        // current interface, and closes it again while help is open.
        if global_action == Some(KeyAction::Help) {
            self.toggle_context_help();
            return;
        }

        // Copy an active non-transcript surface selection (session header
        // rows, composer status row, or composer editor) from any main-surface
        // focus with Ctrl+Y.
        if self.mouse_capture_active()
            && key.code == KeyCode::Char('y')
            && key.modifiers == KeyModifiers::CONTROL
            && self.copy_active_surface_selection()
        {
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
                // `r` is a Vim replace command. Transcript is intentionally
                // read-only, but it must not unexpectedly continue a session
                // while the user is navigating that Vim surface.
                KeyAction::Continue if self.focus != Focus::Transcript => {
                    self.continue_current_session();
                    return;
                }
                // Preserve Vim's `U` namespace while browsing a read-only
                // transcript; the dashboard remains available from the other
                // main-surface panes and its explicit route.
                KeyAction::OpenUsage if self.focus != Focus::Transcript => {
                    self.open_usage_dashboard();
                    return;
                }
                KeyAction::OpenPlan if self.focus != Focus::Composer => {
                    self.open_plan_viewer();
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
        if self.focus == Focus::Transcript {
            self.request_older_transcript_parts_if_needed();
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
            return;
        }
        match resolve_tui_key(KeyContext::Transcript, key) {
            Some(KeyAction::CountDigit(digit @ 1..=9)) => {
                self.transcript_motion_prefix
                    .get_or_insert_with(String::new)
                    .push(char::from(b'0' + digit));
            }
            Some(KeyAction::LineStart)
                if matches!(key.code, crossterm::event::KeyCode::Char('0'))
                    && self.transcript_motion_prefix.is_some() =>
            {
                if let Some(prefix) = self.transcript_motion_prefix.as_mut() {
                    prefix.push('0');
                }
            }
            _ => {}
        }
    }

    pub(crate) fn transcript_motion_count(&mut self) -> usize {
        self.transcript_motion_prefix
            .take()
            .and_then(|prefix| prefix.parse::<usize>().ok())
            .filter(|count| *count > 0)
            .unwrap_or(1)
    }

    pub(crate) fn transcript_motion_count_if_present(&mut self) -> Option<usize> {
        self.transcript_motion_prefix
            .take()
            .and_then(|prefix| prefix.parse::<usize>().ok())
            .filter(|count| *count > 0)
    }

    pub(crate) fn clear_transcript_pending_command(&mut self) {
        self.transcript_motion_prefix = None;
        self.transcript_yank_pending = false;
        self.transcript_yank_origin = None;
        self.transcript_goto_pending = false;
        self.transcript_viewport_pending = false;
        self.transcript_find_pending = None;
        self.transcript_text_object_pending = None;
    }

    /// In INSERT mode, Ctrl+C clears an actual Composer draft. A blank
    /// Composer (and every non-editor surface) deliberately falls through to
    /// the global interrupt / double-Ctrl+C quit flow.
    fn composer_owns_ctrl_c_to_clear(&self) -> bool {
        composer_ctrl_c_clears_input(
            self.focus,
            self.current_route_is_main(),
            self.overlay.is_some(),
            self.context_help.is_some(),
            self.composer_item_selection.is_active(),
            !self.composer.text().is_empty(),
            !self.composer_items.is_empty(),
        )
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
            Overlay::SettingsValueEdit(dialog) => {
                self.handle_settings_value_edit_overlay_key(key, dialog)
            }
            Overlay::Choice(dialog) => self.handle_choice_overlay_key(key, dialog),
            Overlay::PathBrowser(dialog) => self.handle_path_browser_overlay_key(key, dialog),
            Overlay::Permission(dialog) => self.handle_permission_overlay_key(key, dialog),
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
            Route::Activities(dialog) => self.handle_activities_key(key, dialog),
            Route::PlanViewer(dialog) => self.handle_plan_viewer_key(key, dialog),
            Route::SettingsStudio(dialog) => self.handle_settings_studio_overlay_key(key, dialog),
            Route::ClientVersionsStudio(dialog) => {
                self.handle_client_versions_studio_overlay_key(key, dialog)
            }
            Route::PermissionStudio(dialog) => {
                self.handle_permission_studio_overlay_key(key, dialog)
            }
            Route::PermissionRuleStudio(dialog) => {
                self.handle_permission_rule_studio_overlay_key(key, dialog)
            }
            Route::SessionSearch(dialog) => self.handle_session_search_overlay_key(key, dialog),
            Route::Hub(state) => self.handle_hub_key(key, state),
            Route::CommandPalette(dialog) => self.handle_command_palette_key(key, dialog),
            Route::SkillPicker(dialog) => self.handle_skill_picker_key(key, dialog),
            Route::SkillStudio(dialog) => self.handle_skill_studio_key(key, dialog),
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

/// Keeps the Ctrl+C precedence rule testable without requiring a live backend
/// to construct an [`App`]. A staged attachment, paste, or Skill makes the
/// Composer non-empty even when its ordinary text buffer has no characters.
fn composer_ctrl_c_clears_input(
    focus: Focus,
    main_route: bool,
    overlay_open: bool,
    context_help_open: bool,
    composer_item_selection_active: bool,
    composer_has_text: bool,
    composer_has_items: bool,
) -> bool {
    focus == Focus::Composer
        && main_route
        && !overlay_open
        && !context_help_open
        && !composer_item_selection_active
        && (composer_has_text || composer_has_items)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{composer_ctrl_c_clears_input, prompt_history_preempts_global_interrupt};
    use agena_tui::main_focus::Focus;

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

    #[test]
    fn ctrl_c_clears_only_a_nonempty_composer_in_insert_mode() {
        assert!(composer_ctrl_c_clears_input(
            Focus::Composer,
            true,
            false,
            false,
            false,
            true,
            false,
        ));
        // Inline attachments, Skills, and large pastes are all visible draft
        // content even if the text editor itself is empty.
        assert!(composer_ctrl_c_clears_input(
            Focus::Composer,
            true,
            false,
            false,
            false,
            false,
            true,
        ));

        for (focus, main_route, overlay_open, context_help_open, item_selection, text, items) in [
            (Focus::Composer, true, false, false, false, false, false),
            (Focus::Transcript, true, false, false, false, true, false),
            (Focus::Composer, false, false, false, false, true, false),
            (Focus::Composer, true, true, false, false, true, false),
            (Focus::Composer, true, false, true, false, true, false),
            (Focus::Composer, true, false, false, true, true, false),
        ] {
            assert!(!composer_ctrl_c_clears_input(
                focus,
                main_route,
                overlay_open,
                context_help_open,
                item_selection,
                text,
                items,
            ));
        }
    }
}
use crate::{App, Instant, KeyEvent, KeyEventKind, Overlay, OverlayCommit, Route, ui_text};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;
use crossterm::event::{KeyCode, KeyModifiers};
