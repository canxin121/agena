impl App {
    pub(in crate::app) fn has_active_text_input(&self) -> bool {
        if self.context_help.is_some() {
            return false;
        }
        if let Some(overlay) = &self.overlay {
            return !matches!(overlay, Overlay::Confirm(_) | Overlay::Permission(_));
        }
        match &self.current_route {
            Route::Main => self.focus == Focus::Composer,
            Route::Usage(_) | Route::SettingsStudio(_) => false,
            Route::AgentStudio(dialog) => dialog.editor.is_some(),
            Route::PermissionStudio(dialog) => dialog.editor.is_some(),
            Route::PermissionRuleStudio(dialog) => dialog.editor.is_some(),
            Route::SessionSearch(_)
            | Route::CommandPalette(_)
            | Route::SessionNavigation(_)
            | Route::SelectionPicker(_)
            | Route::SessionModelChooser(_)
            | Route::Timeline(_)
            | Route::PluginWorkbench(_) => true,
            Route::ProviderStudio(dialog) => dialog.editor.is_some(),
            Route::ModelCatalogStudio(dialog) => dialog.editor.is_some(),
        }
    }

    pub(in crate::app) fn handle_paste(&mut self, text: String) {
        if self.context_help.is_some() {
            return;
        }
        let backend = self.backend.clone();
        let mut pending_session_search_request: Option<(SessionViewMode, Option<i64>, String)> =
            None;
        if self.overlay.is_none() {
            let mut handled_route = false;
            match &mut self.current_route {
                Route::Main => {}
                Route::Usage(_) | Route::SettingsStudio(_) => {
                    handled_route = true;
                }
                Route::AgentStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::PermissionStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::PermissionRuleStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::SessionSearch(dialog) => {
                    let before = dialog.input.text().trim().to_string();
                    dialog.input.insert_str(text.as_str());
                    let after = dialog.input.text().trim().to_string();
                    if before != after {
                        let _ = dialog.meta.reset_for_query();
                        dialog.selected = 0;
                        dialog.set_loading(true);
                        pending_session_search_request =
                            Some((dialog.meta.mode, dialog.meta.scope_session_id, after));
                    }
                    handled_route = true;
                }
                Route::CommandPalette(dialog) => {
                    let _ = agena_tui::command_palette::reduce(
                        &mut dialog.presentation,
                        agena_tui::command_palette::CommandPaletteAction::Paste(text.clone()),
                    );
                    handled_route = true;
                }
                Route::SessionNavigation(dialog) => {
                    let _ = agena_tui::session_navigation::reduce(
                        &mut dialog.presentation,
                        agena_tui::session_navigation::SessionNavigationAction::Paste(text.clone()),
                    );
                    handled_route = true;
                }
                Route::SelectionPicker(dialog) => {
                    let _ = agena_tui::selection_picker::reduce(
                        &mut dialog.presentation,
                        agena_tui::selection_picker::SelectionPickerAction::Paste(text.clone()),
                    );
                    handled_route = true;
                }
                Route::SessionModelChooser(dialog) => {
                    let _ = agena_tui::model_chooser::reduce(
                        dialog,
                        agena_tui::model_chooser::SessionModelChooserAction::Paste(text.clone()),
                    );
                    handled_route = true;
                }
                Route::Timeline(dialog) => {
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_timeline_overlay(dialog);
                    handled_route = true;
                }
                Route::PluginWorkbench(dialog) => {
                    Self::paste_plugin_workbench(dialog, text.as_str());
                    handled_route = true;
                }
                Route::ProviderStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::ModelCatalogStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
            }
            if handled_route {
                if let Some((mode, scope_session_id, query)) = pending_session_search_request {
                    match mode {
                        SessionViewMode::Subtree => {
                            if let Some(session_id) = scope_session_id {
                                self.request_session_search_subtree(session_id, query);
                            }
                        }
                        SessionViewMode::All | SessionViewMode::Roots => {
                            self.request_session_search_page(mode, query, 0, None);
                        }
                    }
                }
                return;
            }
        }
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::TranscriptSearch(dialog)
                | Overlay::SessionRename(dialog)
                | Overlay::AgentCreate(dialog) => {
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::SettingsValueEdit(dialog) => {
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::Choice(dialog) => {
                    let _ = agena_tui::choice::reduce(
                        &mut dialog.presentation,
                        agena_tui::choice::ChoicePresentationAction::Paste(text.clone()),
                    );
                }
                Overlay::FileAttach(dialog) => {
                    dialog.presentation.input.insert_str(text.as_str());
                    Self::refresh_file_attach_overlay_with_backend(&backend, dialog);
                }
                Overlay::PathBrowser(dialog) => {
                    dialog.presentation.input.insert_str(text.as_str());
                    Self::refresh_path_browser_overlay_with_root(
                        self.backend.workspace_root(),
                        dialog,
                    );
                }
                Overlay::UserInputReply(dialog) => {
                    let _ = dialog.presentation.insert_custom_text(text.as_str());
                }
                Overlay::SessionSearch(dialog) => {
                    let before = dialog.input.text().trim().to_string();
                    dialog.input.insert_str(text.as_str());
                    let after = dialog.input.text().trim().to_string();
                    if before != after {
                        let _ = dialog.meta.reset_for_query();
                        dialog.selected = 0;
                        dialog.set_loading(true);
                        pending_session_search_request =
                            Some((dialog.meta.mode, dialog.meta.scope_session_id, after));
                    }
                }
                Overlay::ProviderStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.insert_str(text.as_str());
                    }
                }
                Overlay::ModelCatalogStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.insert_str(text.as_str());
                    }
                }
                Overlay::Timeline(dialog) => {
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_timeline_overlay(dialog);
                }
                Overlay::Confirm(_) => {}
                Overlay::Permission(_) => {}
            }
            if let Some((mode, scope_session_id, query)) = pending_session_search_request {
                match mode {
                    SessionViewMode::Subtree => {
                        if let Some(session_id) = scope_session_id {
                            self.request_session_search_subtree(session_id, query);
                        }
                    }
                    SessionViewMode::All | SessionViewMode::Roots => {
                        self.request_session_search_page(mode, query, 0, None);
                    }
                }
            }
            return;
        }

        if self.focus == Focus::Composer {
            self.reset_prompt_history_recall();
            if self.try_stage_pasted_path(text.as_str()) {
                return;
            }

            // TerminalRuntime normalizes bracketed paste and the bounded
            // legacy fallback into the same application event.
            self.composer.insert_str(text.as_str());
            self.after_composer_text_mutated();
        }
    }
}
use crate::app::{App, Overlay, Route};
use agena_tui::main_focus::Focus;
use agena_tui::session_view::SessionViewMode;
