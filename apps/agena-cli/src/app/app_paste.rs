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
            Route::Usage(_) | Route::SettingsStudio(_) | Route::PluginPolicyStudio(_) => false,
            Route::AgentStudio(dialog) => dialog.workbench.editor.is_some(),
            Route::PermissionStudio(dialog) => dialog.editor.is_some(),
            Route::PermissionRuleStudio(dialog) => dialog.workbench.editor.is_some(),
            Route::SessionSearch(_)
            | Route::Picker(_)
            | Route::SessionModelChooser(_)
            | Route::Timeline(_)
            | Route::PluginWorkbench(_) => true,
            Route::ProviderStudio(dialog) => dialog.editor.is_some(),
            Route::ModelCatalogStudio(dialog) => dialog.workbench.editor.is_some(),
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
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
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
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::SessionSearch(dialog) => {
                    let before = dialog.input.text().trim().to_string();
                    dialog.input.insert_str(text.as_str());
                    let after = dialog.input.text().trim().to_string();
                    if before != after {
                        dialog.meta.page_index = 0;
                        dialog.selected = 0;
                        dialog.meta.next_cursor = None;
                        dialog.meta.has_more = false;
                        dialog.set_loading(true);
                        pending_session_search_request =
                            Some((dialog.meta.mode, dialog.meta.scope_session_id, after));
                    }
                    handled_route = true;
                }
                Route::Picker(dialog) => {
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_picker_overlay(dialog);
                    handled_route = true;
                }
                Route::SessionModelChooser(dialog) => {
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_session_model_chooser_overlay(dialog, false);
                    handled_route = true;
                }
                Route::Timeline(dialog) => {
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_timeline_overlay(dialog);
                    handled_route = true;
                }
                Route::PluginPolicyStudio(_) => {
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
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
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
                Overlay::RuntimeSettingEdit(dialog) => {
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::Choice(dialog) => {
                    dialog.input.insert_str(text.as_str());
                    Self::sync_choice_overlay_input(dialog);
                }
                Overlay::FileAttach(dialog) => {
                    dialog.input.insert_str(text.as_str());
                    let items = backend
                        .search_workspace_files(dialog.input.text(), 24)
                        .unwrap_or_default();
                    dialog.replace_items(items);
                }
                Overlay::PathBrowser(dialog) => {
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_path_browser_overlay_with_root(
                        self.backend.workspace_root(),
                        dialog,
                    );
                }
                Overlay::UserInputReply(dialog) => {
                    if dialog.state.screen() == QuestionFlowScreen::Review {
                        Self::focus_user_input_question(dialog, dialog.state.selected_question());
                    }
                    if !dialog.editing_custom && !Self::begin_user_input_custom_edit(dialog) {
                        return;
                    }
                    dialog.custom_input.insert_str(text.as_str());
                }
                Overlay::SessionSearch(dialog) => {
                    let before = dialog.input.text().trim().to_string();
                    dialog.input.insert_str(text.as_str());
                    let after = dialog.input.text().trim().to_string();
                    if before != after {
                        dialog.meta.page_index = 0;
                        dialog.selected = 0;
                        dialog.meta.next_cursor = None;
                        dialog.meta.has_more = false;
                        dialog.set_loading(true);
                        pending_session_search_request =
                            Some((dialog.meta.mode, dialog.meta.scope_session_id, after));
                    }
                }
                Overlay::Picker(dialog) => {
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_picker_overlay(dialog);
                }
                Overlay::ProviderStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.insert_str(text.as_str());
                    }
                }
                Overlay::ModelCatalogStudio(dialog) => {
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
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
use crate::app::{App, Focus, Overlay, QuestionFlowScreen, Route, SessionViewMode};
