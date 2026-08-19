impl App {
    pub(crate) fn handle_choice_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ChoiceOverlay,
    ) -> bool {
        let action = match resolve_tui_key(KeyContext::Choice, key) {
            Some(KeyAction::Accept) => agena_tui::choice::ChoicePresentationAction::Accept,
            _ => agena_tui::choice::ChoicePresentationAction::Input(key),
        };
        match agena_tui::choice::reduce(&mut dialog.presentation, action) {
            agena_tui::choice::ChoicePresentationEffect::Close => true,
            agena_tui::choice::ChoicePresentationEffect::KeepOpen => false,
            agena_tui::choice::ChoicePresentationEffect::Commit(selection) => {
                self.commit_choice_overlay(dialog, selection)
            }
        }
    }

    pub(crate) fn handle_session_search_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SessionSearchOverlay,
    ) -> bool {
        match resolve_tui_key(KeyContext::SessionSearch, key) {
            Some(KeyAction::Open) => {
                let Some(session) = dialog.selected_item().cloned() else {
                    return false;
                };
                self.open_session(session.session_id, session.title);
                self.focus = Focus::Composer;
                true
            }
            _ => {
                let selected_before = dialog.selected;
                match dialog.handle_input_key(key) {
                    SearchPickerInputResult::Close => true,
                    SearchPickerInputResult::Navigated => {
                        let stayed_at_boundary = dialog.selected == selected_before;
                        if stayed_at_boundary
                            && matches!(
                                key.code,
                                crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Right
                            )
                        {
                            self.request_more_session_search_results(dialog);
                        }
                        false
                    }
                    SearchPickerInputResult::Edited { changed } => {
                        if changed {
                            self.reset_session_search_query(
                                dialog,
                                dialog.input.text().trim().to_string(),
                            );
                        }
                        false
                    }
                }
            }
        }
    }

    fn request_more_session_search_results(&mut self, dialog: &mut SessionSearchOverlay) {
        if dialog.is_loading() || !dialog.meta.has_more {
            return;
        }
        if dialog.meta.mode == SessionViewMode::Subtree {
            return;
        }
        let Some(SessionSearchEffect::LoadPage { page_index, cursor }) =
            dialog.meta.request_next_page()
        else {
            return;
        };
        dialog.begin_append();
        dialog.footer = self.session_search_footer(dialog);
        self.request_session_search_page(
            dialog.meta.mode,
            dialog.input.text().trim().to_string(),
            page_index,
            cursor,
        );
    }

    pub(crate) fn reset_session_search_query(
        &mut self,
        dialog: &mut SessionSearchOverlay,
        query: String,
    ) {
        let _ = dialog.meta.reset_for_query();
        dialog.selected = 0;
        dialog.set_loading(true);
        dialog.footer = self.session_search_footer(dialog);
        match dialog.meta.mode {
            SessionViewMode::Subtree => {
                if let Some(session_id) = dialog.meta.scope_session_id {
                    self.request_session_search_subtree(session_id, query);
                }
            }
            SessionViewMode::All | SessionViewMode::Roots => {
                self.request_session_search_page(dialog.meta.mode, query, 0, None);
            }
        }
    }

    pub(crate) fn refresh_session_search_overlay_local(&self, dialog: &mut SessionSearchOverlay) {
        let query = dialog.input.text().trim();
        let filtered = dialog
            .meta
            .all_items
            .iter()
            .filter(|session| session.matches_query(query))
            .cloned()
            .collect::<Vec<_>>();
        let _ = dialog.meta.reset_for_query();
        dialog.replace_items(filtered);
        dialog.set_loading(false);
        dialog.footer = self.session_search_footer(dialog);
    }

    pub(crate) fn session_search_footer(&self, dialog: &SessionSearchOverlay) -> String {
        let scope = match dialog.meta.mode {
            SessionViewMode::All => ui_text::t(&self.i18n, "overlay-session-search-scope-all"),
            SessionViewMode::Roots => ui_text::t(&self.i18n, "overlay-session-search-scope-roots"),
            SessionViewMode::Subtree => {
                ui_text::t(&self.i18n, "overlay-session-search-scope-subtree")
            }
        };
        if dialog.meta.mode == SessionViewMode::Subtree {
            let total = dialog
                .meta
                .all_items
                .iter()
                .filter(|session| session.matches_query(dialog.input.text().trim()))
                .count();
            return self.i18n.text_args(
                "overlay-session-search-footer-local",
                &agena_tui::fl_args!(
                    "scope" => scope,
                    "total" => total as i64,
                ),
            );
        }

        let end_state = if dialog.meta.has_more {
            ui_text::t(&self.i18n, "overlay-session-search-tail-more")
        } else {
            ui_text::t(&self.i18n, "overlay-session-search-tail-end")
        };
        self.i18n.text_args(
            "overlay-session-search-footer-remote",
            &agena_tui::fl_args!(
                "scope" => scope,
                "loaded" => dialog.items.len() as i64,
                "tail" => end_state,
            ),
        )
    }

    pub(crate) fn handle_selection_picker_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SelectionPickerOverlay,
    ) -> bool {
        let action = match resolve_tui_key(KeyContext::Picker, key) {
            Some(KeyAction::Accept) => agena_tui::selection_picker::SelectionPickerAction::Accept,
            _ => agena_tui::selection_picker::SelectionPickerAction::Input(key),
        };
        match agena_tui::selection_picker::reduce(&mut dialog.presentation, action) {
            agena_tui::selection_picker::SelectionPickerEffect::Close => true,
            agena_tui::selection_picker::SelectionPickerEffect::KeepOpen => false,
            agena_tui::selection_picker::SelectionPickerEffect::Activate { key } => {
                let Some(action) = dialog.actions.get(key.as_str()).cloned() else {
                    return false;
                };
                match action {
                    SelectionPickerCommand::ProviderCreate => {
                        self.route_stack
                            .push(Route::SelectionPicker(dialog.clone()));
                        self.open_provider_studio(None);
                        false
                    }
                    SelectionPickerCommand::Provider { provider_id } => {
                        self.route_stack
                            .push(Route::SelectionPicker(dialog.clone()));
                        self.open_provider_studio(Some(provider_id.as_str()));
                        false
                    }
                }
            }
        }
    }

    pub(crate) fn handle_command_palette_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut CommandPaletteOverlay,
    ) -> bool {
        let action = match resolve_tui_key(KeyContext::Picker, key) {
            Some(KeyAction::Accept) => agena_tui::command_palette::CommandPaletteAction::Accept,
            _ => agena_tui::command_palette::CommandPaletteAction::Input(key),
        };
        match agena_tui::command_palette::reduce(&mut dialog.presentation, action) {
            agena_tui::command_palette::CommandPaletteEffect::Close => true,
            agena_tui::command_palette::CommandPaletteEffect::KeepOpen => false,
            agena_tui::command_palette::CommandPaletteEffect::Activate { key } => {
                let Some(action) = dialog.actions.get(key.as_str()).cloned() else {
                    return false;
                };
                match action {
                    CommandPaletteCommand::BuiltIn(spec) => {
                        if spec.requires_arguments() {
                            self.prepare_composer_command(spec.name);
                        } else {
                            self.execute_command(spec, "");
                        }
                    }
                    CommandPaletteCommand::Plugin(entry) => {
                        if plugin_operation_accepts_empty_arguments(&entry) {
                            self.execute_plugin_slash_operation(*entry, "");
                        } else if let Some(command_name) = plugin_operation_slash_name(&entry) {
                            self.prepare_composer_command(command_name.as_str());
                        }
                    }
                }
                true
            }
        }
    }

    pub(crate) fn handle_session_navigation_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SessionNavigationOverlay,
    ) -> bool {
        let action = match resolve_tui_key(KeyContext::Picker, key) {
            Some(KeyAction::Accept) => {
                agena_tui_session::session_navigation::SessionNavigationAction::Accept
            }
            _ => agena_tui_session::session_navigation::SessionNavigationAction::Input(key),
        };
        match agena_tui_session::session_navigation::reduce(&mut dialog.presentation, action) {
            agena_tui_session::session_navigation::SessionNavigationEffect::Close => true,
            agena_tui_session::session_navigation::SessionNavigationEffect::KeepOpen => false,
            agena_tui_session::session_navigation::SessionNavigationEffect::Open { key } => {
                let Some(SessionNavigationCommand::OpenSession { session_id }) =
                    dialog.actions.get(key.as_str()).cloned()
                else {
                    return false;
                };
                self.open_session(
                    session_id,
                    ui_text::session_fallback_title(&self.i18n, session_id),
                );
                self.focus = Focus::Transcript;
                true
            }
            agena_tui_session::session_navigation::SessionNavigationEffect::Rewind { key } => {
                let Some(SessionNavigationCommand::Rewind {
                    session_id,
                    turn_id,
                    message_text,
                    target,
                }) = dialog.actions.get(key.as_str()).cloned()
                else {
                    return false;
                };
                self.open_rewind_confirm_overlay(session_id, turn_id, message_text, target);
                true
            }
        }
    }

    pub(crate) fn handle_session_model_chooser_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SessionModelChooserOverlay,
    ) -> bool {
        match agena_tui::model_chooser::reduce(
            dialog,
            agena_tui::model_chooser::SessionModelChooserAction::Input(key),
        ) {
            agena_tui::model_chooser::SessionModelChooserReducerEffect::Close => true,
            agena_tui::model_chooser::SessionModelChooserReducerEffect::KeepOpen => false,
            agena_tui::model_chooser::SessionModelChooserReducerEffect::Select {
                purpose: SessionModelChooserPurpose::RuntimeOverride,
                identity,
            } => self.apply_model_override(model_ref_from_session_model_identity(identity)),
            agena_tui::model_chooser::SessionModelChooserReducerEffect::Select {
                purpose: SessionModelChooserPurpose::ProviderDefault,
                identity,
            } => {
                let model = model_ref_from_session_model_identity(identity);
                self.open_model_selection_mode_step_or_finish(
                    SessionModelChooserPurpose::ProviderDefault,
                    model,
                    None,
                    None,
                    None,
                    SessionModelModeStep::ThinkingMode,
                );
                false
            }
            agena_tui::model_chooser::SessionModelChooserReducerEffect::Select {
                purpose: SessionModelChooserPurpose::PermissionApproval,
                identity,
            } => {
                let model = model_ref_from_session_model_identity(identity);
                self.open_model_selection_mode_step_or_finish(
                    SessionModelChooserPurpose::PermissionApproval,
                    model,
                    None,
                    None,
                    None,
                    SessionModelModeStep::ThinkingMode,
                );
                false
            }
        }
    }

    pub(crate) fn handle_timeline_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut TimelineOverlay,
    ) -> bool {
        match dialog.handle_input_key(key) {
            SearchPickerInputResult::Close => true,
            SearchPickerInputResult::Navigated => false,
            SearchPickerInputResult::Edited { .. } => false,
        }
    }

    pub(crate) fn handle_provider_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ProviderStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => return false,
                EditorDialogKeyResult::Close => {
                    dialog.editor = None;
                    return false;
                }
                EditorDialogKeyResult::Submit(action, input) => match action {
                    ProviderStudioEditorAction::Field(field) => {
                        let value = input.trim().to_string();
                        if let Err(error) = self.commit_provider_studio_field(dialog, field, value)
                        {
                            self.flash_error(error);
                            return false;
                        }
                        dialog.editor = None;
                        return false;
                    }
                    ProviderStudioEditorAction::NewModel { adapter_id } => {
                        let value = input.trim().to_string();
                        match self.add_provider_studio_manual_model(dialog, adapter_id, value) {
                            Ok(()) => dialog.editor = None,
                            Err(error) => self.flash_error(error),
                        }
                        return false;
                    }
                    ProviderStudioEditorAction::ModelField(field) => {
                        let value = input.trim().to_string();
                        if let Err(error) =
                            self.commit_provider_studio_model_field(dialog, field, value)
                        {
                            self.flash_error(error);
                            return false;
                        }
                        dialog.editor = None;
                        return false;
                    }
                },
            }
        }

        if dialog.model_page.is_some() {
            return self.handle_provider_studio_model_page_key(key, dialog);
        }

        if dialog.detail_page.is_some() {
            return self.handle_provider_studio_detail_page_key(key, dialog);
        }

        match resolve_tui_key(KeyContext::ProviderStudio, key) {
            Some(KeyAction::Close) => true,
            Some(KeyAction::NextTab) => {
                dialog.selection.next_focus();
                false
            }
            Some(KeyAction::PreviousTab) => {
                dialog.selection.prev_focus();
                false
            }
            Some(KeyAction::Toggle)
                if dialog.selection.focus() == ProviderStudioFocus::Adapters =>
            {
                self.toggle_provider_studio_selected_adapter(dialog);
                false
            }
            Some(KeyAction::Toggle) if dialog.selection.focus() == ProviderStudioFocus::Models => {
                self.toggle_provider_studio_selected_model(dialog);
                false
            }
            Some(KeyAction::MoveUp) => {
                self.move_provider_studio_selection(dialog, -1);
                false
            }
            Some(KeyAction::MoveDown) => {
                self.move_provider_studio_selection(dialog, 1);
                false
            }
            Some(KeyAction::Delete) => {
                self.open_provider_studio_delete_selected_confirm(dialog);
                false
            }
            Some(KeyAction::ProviderRefreshModels) => {
                self.request_provider_studio_adapter_models(dialog);
                false
            }
            Some(KeyAction::ProviderAddModel) => {
                self.open_provider_studio_new_model_editor(dialog);
                false
            }
            Some(KeyAction::ProviderSaveAdapter) => {
                if provider_studio_selected_adapter_models(dialog).is_none() {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-provider-studio-adapter-required",
                    ));
                    return false;
                }
                dialog.saving = true;
                self.request_provider_studio_save_selected_adapter(dialog.clone());
                false
            }
            Some(KeyAction::ProviderSave) => {
                dialog.saving = true;
                self.request_provider_studio_save_draft(dialog.clone());
                false
            }
            Some(KeyAction::Activate) => {
                self.activate_provider_studio_focus(dialog);
                false
            }
            _ => false,
        }
    }

    pub(crate) fn handle_model_catalog_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ModelCatalogStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.editor.as_mut() {
            return match drive_input_dialog_key(editor, key) {
                InputDialogKeyResult::Close => {
                    dialog.editor = None;
                    false
                }
                InputDialogKeyResult::Submit(_, value) => {
                    let effect = dialog.presentation.begin_query(value.trim());
                    dialog.editor = None;
                    if let agena_tui::model_catalog::ModelCatalogEffect::LoadPage {
                        query,
                        offset,
                    } = effect
                    {
                        self.request_model_catalog_page(query, offset);
                    }
                    false
                }
                InputDialogKeyResult::Continue => false,
            };
        }

        match agena_tui::model_catalog::handle_key(&mut dialog.presentation, key) {
            agena_tui::model_catalog::ModelCatalogEffect::Close => true,
            agena_tui::model_catalog::ModelCatalogEffect::OpenSearch => {
                dialog.editor =
                    Some(self.build_model_catalog_search_overlay(dialog.presentation.query()));
                false
            }
            agena_tui::model_catalog::ModelCatalogEffect::Refresh => {
                self.request_model_catalog_refresh();
                false
            }
            agena_tui::model_catalog::ModelCatalogEffect::LoadPage { query, offset } => {
                self.request_model_catalog_page(query, offset);
                false
            }
            agena_tui::model_catalog::ModelCatalogEffect::KeepOpen => false,
        }
    }
}

use crate::{
    App, ChoiceOverlay, CommandPaletteCommand, CommandPaletteOverlay, EditorDialogKeyResult,
    InputDialogKeyResult, KeyEvent, ModelCatalogStudioOverlay, ModelRef,
    ProviderStudioEditorAction, ProviderStudioFocus, ProviderStudioOverlay, Route,
    SearchPickerInputResult, SelectionPickerCommand, SelectionPickerOverlay,
    SessionModelChooserOverlay, SessionModelChooserPurpose, SessionModelModeStep,
    SessionNavigationCommand, SessionNavigationOverlay, SessionSearchOverlay, TimelineOverlay,
    drive_editor_dialog_key, drive_input_dialog_key, plugin_operation_accepts_empty_arguments,
    plugin_operation_slash_name, provider_studio_selected_adapter_models, ui_text,
};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;
use agena_tui::model_chooser::SessionModelIdentity;
use agena_tui_session::{session_search::SessionSearchEffect, session_view::SessionViewMode};

fn model_ref_from_session_model_identity(identity: SessionModelIdentity) -> ModelRef {
    match identity.adapter_id {
        Some(adapter_id) => {
            ModelRef::new_with_adapter(identity.provider_id, adapter_id, identity.model_id)
        }
        None => ModelRef::new(identity.provider_id, identity.model_id),
    }
}
