impl App {
    pub(crate) fn has_active_text_input(&self) -> bool {
        if self.context_help.is_some() {
            return false;
        }
        // An inline user-input answer is a text target even though it lives on
        // the transcript surface: the editor owns the key stream while it is
        // open, so terminal-delivered text (and the Enter that confirmed an
        // input-method candidate) must be normalized as text input.
        if self.interaction_editing.is_some() {
            return true;
        }
        if let Some(overlay) = &self.overlay {
            if let Overlay::SessionSearch(dialog) = overlay {
                return dialog.focus == agena_tui_components::SearchPickerFocus::Input;
            }
            return !matches!(overlay, Overlay::Confirm(_) | Overlay::Permission(_));
        }
        match &self.current_route {
            Route::Main => self.focus == Focus::Composer,
            Route::Btw(_) => true,
            Route::Usage(_)
            | Route::Activities(_)
            | Route::PlanViewer(_)
            | Route::SessionWork(_)
            | Route::SettingsStudio(_)
            | Route::ClientVersionsStudio(_)
            | Route::Hub(_) => false,
            Route::PermissionStudio(dialog) => dialog.editor.is_some(),
            Route::PermissionRuleStudio(dialog) => dialog.editor.is_some(),
            // Results own command keys such as p/f. Treating them as text
            // coalesces fast key presses into a paste and edits the query.
            Route::SessionSearch(dialog) => {
                dialog.focus == agena_tui_components::SearchPickerFocus::Input
            }
            Route::CommandPalette(_)
            | Route::SessionNavigation(_)
            | Route::SelectionPicker(_)
            | Route::SessionModelChooser(_)
            | Route::Timeline(_)
            | Route::PluginWorkbench(_) => true,
            Route::ProviderStudio(dialog) => dialog.editor.is_some(),
            Route::ModelCatalogStudio(dialog) => dialog.editor.is_some(),
        }
    }

    pub(crate) fn handle_paste(&mut self, text: String) {
        if self.context_help.is_some() {
            return;
        }
        if self.btw_has_focus() {
            if let Some(id) = self.transcript.session_id
                && let Some(state) = self.btw_sessions.get_mut(&id)
            {
                state.input.insert_str(&text);
            }
            return;
        }
        if (matches!(self.overlay, Some(Overlay::SessionSearch(_)))
            || (self.overlay.is_none() && matches!(self.current_route, Route::SessionSearch(_))))
            && let Some((host, mut dialog)) = self.take_session_search_dialog()
        {
            let before = dialog.input.text().trim().to_string();
            dialog.input.insert_str(&text);
            dialog.focus = agena_tui_components::SearchPickerFocus::Input;
            let query = dialog.input.text().trim().to_string();
            if before != query {
                self.reset_session_search_query(&mut dialog, query);
            }
            self.restore_session_search_dialog(host, dialog);
            return;
        }
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
                Route::Usage(_) | Route::Activities(_) | Route::PlanViewer(_) | Route::Hub(_) => {
                    handled_route = true;
                }
                Route::SessionWork(_) | Route::Btw(_) => {
                    handled_route = true;
                }
                Route::SettingsStudio(dialog) | Route::ClientVersionsStudio(dialog) => {
                    dialog.state.paste(&text);
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
                Route::SessionSearch(_) => {}
                Route::CommandPalette(dialog) => {
                    let _ = agena_tui::command_palette::reduce(
                        &mut dialog.presentation,
                        agena_tui::command_palette::CommandPaletteAction::Paste(text.clone()),
                    );
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
                return;
            }
        }
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::HubProject(dialog) => {
                    if !dialog.submitting {
                        dialog.error = None;
                        dialog.editor.input.insert_str(text.as_str());
                    }
                }
                Overlay::SettingsValueEdit(dialog) => {
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::TranscriptFoldLoad(dialog) => {
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::QueueEditor(dialog) => dialog.paste(text.as_str()),
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
                Overlay::SessionSearch(_) => {}
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
            return;
        }

        if self.focus == Focus::Composer {
            self.reset_prompt_history_recall();
            if self.try_stage_pasted_path(text.as_str()) {
                return;
            }
            if text.len() > 1024 * 1024 {
                self.flash_warning(self.i18n.text("flash-paste-limit-draft-unchanged"));
                return;
            }
            if paste_requires_workspace_text_file(text.as_str()) {
                self.stage_long_paste_text_file(text);
                return;
            }

            // TerminalRuntime normalizes terminal paste input into one
            // application event before it reaches the composer.
            self.composer.insert_str(text.as_str());
            self.after_composer_text_mutated();
        }
    }
}

fn paste_requires_workspace_text_file(text: &str) -> bool {
    text.chars().count() >= 1_000
}

use crate::{App, Overlay, Route};
use agena_tui::main_focus::Focus;
