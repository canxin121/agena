impl App {
    pub(crate) fn has_active_text_input(&self) -> bool {
        if self.context_help.is_some() {
            return false;
        }
        if let Some(overlay) = &self.overlay {
            return !matches!(overlay, Overlay::Confirm(_) | Overlay::Permission(_));
        }
        match &self.current_route {
            Route::Main => self.focus == Focus::Composer,
            Route::Usage(_)
            | Route::Activities(_)
            | Route::PlanViewer(_)
            | Route::SettingsStudio(_)
            | Route::ClientVersionsStudio(_)
            | Route::Hub(_) => false,
            Route::PermissionStudio(dialog) => dialog.editor.is_some(),
            Route::PermissionRuleStudio(dialog) => dialog.editor.is_some(),
            Route::SessionSearch(_)
            | Route::CommandPalette(_)
            | Route::SkillPicker(_)
            | Route::SessionNavigation(_)
            | Route::SelectionPicker(_)
            | Route::SessionModelChooser(_)
            | Route::Timeline(_)
            | Route::PluginWorkbench(_) => true,
            Route::SkillStudio(dialog) => dialog.editor.is_some() || dialog.detail.is_none(),
            Route::ProviderStudio(dialog) => dialog.editor.is_some(),
            Route::ModelCatalogStudio(dialog) => dialog.editor.is_some(),
        }
    }

    pub(crate) fn handle_paste(&mut self, text: String) {
        if self.context_help.is_some() {
            return;
        }
        let mut pending_session_search_request: Option<(SessionViewMode, Option<i64>, String)> =
            None;
        let mut pending_path_browser_refresh = None;
        if self.overlay.is_none() {
            // Pasting onto an expanded pending interaction part inserts the
            // text into its custom-feedback field ("everything is a part").
            if self.focus == Focus::Transcript && self.paste_into_active_interaction(&text) {
                return;
            }
            let mut handled_route = false;
            match &mut self.current_route {
                Route::Main => {}
                Route::Usage(_)
                | Route::Activities(_)
                | Route::PlanViewer(_)
                | Route::SettingsStudio(_)
                | Route::ClientVersionsStudio(_)
                | Route::Hub(_) => {
                    handled_route = true;
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
                Route::SkillPicker(dialog) => {
                    let _ = agena_tui::selection_picker::reduce(
                        &mut dialog.presentation,
                        agena_tui::selection_picker::SelectionPickerAction::Paste(text.clone()),
                    );
                    handled_route = true;
                }
                Route::SkillStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.insert_str(text.as_str());
                    } else if dialog.detail.is_none() {
                        let _ = agena_tui::selection_picker::reduce(
                            &mut dialog.presentation,
                            agena_tui::selection_picker::SelectionPickerAction::Paste(text.clone()),
                        );
                    }
                    handled_route = true;
                }
                Route::SessionNavigation(dialog) => {
                    let _ = agena_tui_session::session_navigation::reduce(
                        &mut dialog.presentation,
                        agena_tui_session::session_navigation::SessionNavigationAction::Paste(
                            text.clone(),
                        ),
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
                Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
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
                Overlay::PathBrowser(dialog) => {
                    dialog.presentation.input.insert_str(text.as_str());
                    Self::refresh_path_browser_overlay(&self.application, dialog);
                    pending_path_browser_refresh = Some(
                        Self::path_browser_directory_and_needle_for_overlay(
                            &self.application,
                            dialog,
                        )
                        .0,
                    );
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
            if let Some(directory) = pending_path_browser_refresh {
                self.request_path_browser_directory_refresh(directory);
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

            if paste_requires_text_artifact(text.as_str()) {
                self.stage_text_artifact(text);
                self.after_composer_text_mutated();
                return;
            }

            // TerminalRuntime normalizes bracketed paste and the bounded
            // legacy fallback into the same application event.
            self.composer.insert_str(text.as_str());
            self.after_composer_text_mutated();
        }
    }
}

fn paste_requires_text_artifact(text: &str) -> bool {
    text.chars().count() >= 1_000
}

#[cfg(test)]
mod tests {
    use super::paste_requires_text_artifact;

    #[test]
    fn thousand_character_pastes_use_text_artifacts() {
        assert!(!paste_requires_text_artifact(&"x".repeat(999)));
        assert!(paste_requires_text_artifact(&"x".repeat(1_000)));
    }
}
use crate::{App, Overlay, Route};
use agena_tui::main_focus::Focus;
use agena_tui_session::session_view::SessionViewMode;
