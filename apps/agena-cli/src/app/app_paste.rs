impl App {
    pub(in crate::app) fn handle_paste(&mut self, text: String) {
        let backend = self.backend.clone();
        let mut pending_session_search_request: Option<(SessionViewMode, Option<i64>, String)> =
            None;
        if self.overlay.is_none() {
            let mut handled_route = false;
            match &mut self.current_route {
                Route::Main => {}
                Route::Help(_) | Route::SettingsStudio(_) => {}
                Route::AgentStudio(dialog) => {
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::PermissionStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::PermissionRuleStudio(dialog) => {
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::SessionSearch(dialog) => {
                    let before = dialog.input.text().trim().to_string();
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    let after = dialog.input.text().trim().to_string();
                    if before != after {
                        dialog.meta.page_index = 0;
                        dialog.selected = 0;
                        dialog.meta.offset = 0;
                        dialog.meta.cursors.clear();
                        dialog.meta.cursors.push(None);
                        dialog.meta.next_cursor = None;
                        dialog.meta.has_more = false;
                        dialog.loading = true;
                        pending_session_search_request =
                            Some((dialog.meta.mode, dialog.meta.scope_session_id, after));
                    }
                    handled_route = true;
                }
                Route::Picker(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_picker_overlay(dialog);
                    handled_route = true;
                }
                Route::SessionModelChooser(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_session_model_chooser_overlay(dialog, false, None);
                    handled_route = true;
                }
                Route::Timeline(dialog) => {
                    dialog.input.flush_all_pending_input();
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
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::ModelCatalogStudio(dialog) => {
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
                        editor.input.flush_all_pending_input();
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
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::SettingsValueEdit(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::RuntimeSettingEdit(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::Choice(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::sync_choice_overlay_input(dialog, true);
                }
                Overlay::FileAttach(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    dialog.items = backend
                        .search_workspace_files(dialog.input.text(), 24)
                        .unwrap_or_default();
                    dialog.clamp_selection();
                }
                Overlay::PathBrowser(dialog) => {
                    dialog.input.flush_all_pending_input();
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
                    dialog.custom_input.flush_all_pending_input();
                    dialog.custom_input.insert_str(text.as_str());
                }
                Overlay::SessionSearch(dialog) => {
                    let before = dialog.input.text().trim().to_string();
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    let after = dialog.input.text().trim().to_string();
                    if before != after {
                        dialog.meta.page_index = 0;
                        dialog.selected = 0;
                        dialog.meta.offset = 0;
                        dialog.meta.cursors.clear();
                        dialog.meta.cursors.push(None);
                        dialog.meta.next_cursor = None;
                        dialog.meta.has_more = false;
                        dialog.loading = true;
                        pending_session_search_request =
                            Some((dialog.meta.mode, dialog.meta.scope_session_id, after));
                    }
                }
                Overlay::Picker(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_picker_overlay(dialog);
                }
                Overlay::ProviderStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                    }
                }
                Overlay::ModelCatalogStudio(dialog) => {
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                    }
                }
                Overlay::Timeline(dialog) => {
                    dialog.input.flush_all_pending_input();
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
            self.composer.flush_all_pending_input();
            if self.try_stage_pasted_path(text.as_str()) {
                return;
            }

            // Bracketed-paste capable terminals deliver one `Event::Paste`,
            // while other terminals deliver the same clipboard contents as a
            // burst of key events. Insert both paths as ordinary composer
            // text so the visible result never depends on terminal support
            // or a hidden character-count threshold.
            self.composer.insert_str(text.as_str());
            self.after_composer_text_mutated();
        }
    }
}
use crate::app::{App, Focus, Overlay, QuestionFlowScreen, Route, SessionViewMode};
