impl App {
    pub(crate) fn handle_key_event(&mut self, key: KeyEvent) {
        if matches!(key.kind, KeyEventKind::Release) {
            return;
        }

        if !self.is_session_mark_key(key) {
            self.pointer_targets = Default::default();
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
        // Ctrl+C is a client-level exit choice, not implicit run cancellation.
        // Offer the same choice from the Hub, focused editors, and overlays.
        if global_action == Some(KeyAction::Interrupt) {
            self.open_exit_menu();
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

        if self.handle_context_help_key(key) {
            return;
        }

        if self.handle_overlay_key(key) {
            return;
        }

        if self.handle_route_key(key) {
            return;
        }

        // Fold hints stay visible while focus is in the Composer. Route their
        // shortcuts back to the selected transcript reply before Composer
        // input handling, while preserving Enter queueing and Ctrl+Enter
        // submission whenever a draft is waiting.
        if self.current_route_is_main()
            && self.focus == Focus::Composer
            && self.overlay.is_none()
            && self.context_help.is_none()
            && self.interaction_editing.is_none()
            && !self.composer_item_selection.is_active()
            && self.handle_composer_transcript_fold_shortcut(key)
        {
            return;
        }

        if self.current_route_is_main()
            && resolve_tui_key(KeyContext::Main, key) == Some(KeyAction::ToggleSessionWork)
            && self.interaction_editing.is_none()
            && self.prompt_history_search.is_none()
        {
            self.open_session_work_panel(crate::app_session_work::Tab::Files);
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
                    self.open_plan_viewer_fullscreen();
                    return;
                }
                _ => {}
            }
        }

        match self.focus {
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

    pub(crate) fn handle_overlay_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut overlay) = self.overlay.take() else {
            return false;
        };

        let exit_menu = matches!(&overlay, Overlay::Choice(dialog) if matches!(dialog.action, crate::ChoiceOverlayAction::Exit));
        let close = match &mut overlay {
            Overlay::InlineUi(dialog) => self.handle_inline_ui_key(key, dialog),
            Overlay::TranscriptSearch(dialog) => {
                self.handle_line_overlay_key(key, dialog, OverlayCommit::TranscriptSearch)
            }
            Overlay::TranscriptFoldLoad(dialog) => {
                self.handle_transcript_fold_load_key(key, dialog)
            }
            Overlay::SessionRename(dialog) => self.handle_session_rename_overlay_key(key, dialog),
            Overlay::HubProject(dialog) => self.handle_hub_project_key(key, dialog),
            Overlay::SettingsValueEdit(dialog) => {
                self.handle_settings_value_edit_overlay_key(key, dialog)
            }
            Overlay::SessionPromptEdit(dialog) => {
                self.handle_session_prompt_edit_overlay_key(key, dialog)
            }
            Overlay::QueueEditor(dialog) => self.handle_queue_editor_key(key, dialog),
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

        if close && exit_menu {
            // Unlike other choice pickers, Ctrl+C may suspend an overlay on
            // any route (including Hub). Always restore it on Cancel/Esc and
            // when a Stop & Exit request fails, even outside the Main route.
            self.context_help = self.exit_suspended_help.take();
            if let Some(parent) = self.overlay_stack.pop() {
                self.overlay = Some(self.refresh_restored_overlay(parent));
            }
            return true;
        }
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
            Route::SessionWork(id) => self.handle_session_work_route_key(key, *id),
            Route::Btw(_) => self.handle_btw_route_key(key),
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
            Route::SessionNavigation(dialog) => self.handle_session_navigation_key(key, dialog),
            Route::SelectionPicker(dialog) => self.handle_selection_picker_key(key, dialog),
            Route::SessionModelChooser(dialog) => {
                self.handle_session_model_chooser_overlay_key(key, dialog)
            }
            Route::Timeline(dialog) => self.handle_timeline_overlay_key(key, dialog),
            Route::PluginWorkbench(dialog) => self.handle_plugin_workbench_key(key, dialog),
            Route::SkillsWorkbench(dialog) => self.handle_skills_workbench_key(key, dialog),
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

use crate::{App, KeyEvent, KeyEventKind, Overlay, OverlayCommit, Route};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;
use crossterm::event::{KeyCode, KeyModifiers};
