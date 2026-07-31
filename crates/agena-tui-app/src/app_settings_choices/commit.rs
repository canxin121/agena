impl App {
    pub(crate) fn commit_choice_overlay(
        &mut self,
        dialog: &mut ChoiceOverlay,
        selection: agena_tui::choice::ChoiceSelection,
    ) -> bool {
        match dialog.action.clone() {
            ChoiceOverlayAction::InsertContent => match choice_selection_value(&selection).as_str()
            {
                "skill" => {
                    self.open_skill_picker();
                    true
                }
                "file" => {
                    self.request_file_attachment(false);
                    true
                }
                _ => false,
            },
            ChoiceOverlayAction::SettingsField(field) => {
                let input = choice_selection_value(&selection);
                match parse_settings_field_input(&self.i18n, field, input.as_str()) {
                    Ok(Some(value)) => match self
                        .block_on_async(self.backend.set_config_setting(field.path, value))
                    {
                        Ok(_) => {
                            self.flash_success(settings_path_updated_message(
                                &self.i18n, field.path,
                            ));
                            self.refresh_current_route_after_local_edit();
                            true
                        }
                        Err(error) => {
                            self.flash_error(error);
                            false
                        }
                    },
                    Ok(None) => {
                        match self.block_on_async(self.backend.delete_config_setting(field.path)) {
                            Ok(_) => {
                                self.flash_success(settings_path_cleared_message(
                                    &self.i18n, field.path,
                                ));
                                self.refresh_current_route_after_local_edit();
                                true
                            }
                            Err(error) => {
                                self.flash_error(error);
                                false
                            }
                        }
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::SessionModelMode(step) => {
                let input = choice_selection_value(&selection);
                let previous = self.run_options.clone();
                self.run_options
                    .apply_model_mode_input(step, input.as_str());
                if !self.persist_current_session_model_stack() {
                    self.run_options = previous;
                    return false;
                }
                self.advance_session_model_mode_step(step);
                true
            }
            ChoiceOverlayAction::ProviderDefaultModelMode { model, step } => {
                let input = choice_selection_value(&selection);
                match self.persist_provider_default_model_mode(&model, step, input.as_str()) {
                    Ok(()) => {
                        self.advance_provider_default_model_mode_step(model, step);
                        true
                    }
                    Err(error) => {
                        self.flash_error(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::ProviderStudioField(field) => {
                let value = choice_selection_value(&selection);
                let Some((host, mut parent)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return true;
                };
                match self.commit_provider_studio_field(&mut parent, field, value) {
                    Ok(()) => {
                        self.restore_provider_studio_dialog(host, parent);
                        true
                    }
                    Err(error) => {
                        self.restore_provider_studio_dialog(host, parent);
                        self.flash_error(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::ProviderStudioModelField(field) => {
                let value = choice_selection_value(&selection);
                let Some((host, mut parent)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return true;
                };
                match self.commit_provider_studio_model_field(&mut parent, field, value) {
                    Ok(()) => {
                        self.restore_provider_studio_dialog(host, parent);
                        true
                    }
                    Err(error) => {
                        self.restore_provider_studio_dialog(host, parent);
                        self.flash_error(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::PermissionRuleStudio(field) => {
                let value = choice_selection_value(&selection);
                let current_session_id = self.current_or_selected_session_id();
                match &mut self.current_route {
                    Route::PermissionRuleStudio(parent) => {
                        match field {
                            PermissionRuleStudioChoiceField::SubjectKind => {
                                parent.draft.subject_kind = match value.as_str() {
                                    "path_access" => PermissionRuleSubjectKind::PathAccess,
                                    "network_access" => PermissionRuleSubjectKind::NetworkAccess,
                                    _ => PermissionRuleSubjectKind::Tool,
                                };
                            }
                            PermissionRuleStudioChoiceField::PathAccessKind => {
                                if !value.trim().is_empty() {
                                    parent.draft.path_access_kind = value;
                                }
                            }
                            PermissionRuleStudioChoiceField::Scope => {
                                parent.draft.scope = if value.trim().is_empty() {
                                    "workspace".to_string()
                                } else {
                                    value
                                };
                                if parent.draft.scope != "session" {
                                    parent.draft.session_id.clear();
                                } else if parent.draft.session_id.trim().is_empty()
                                    && let Some(session_id) = current_session_id
                                {
                                    parent.draft.session_id = session_id.to_string();
                                }
                            }
                            PermissionRuleStudioChoiceField::Mode => {
                                parent.draft.mode = match value.as_str() {
                                    "allow" => PermissionMode::Allow,
                                    "deny" => PermissionMode::Deny,
                                    _ => PermissionMode::Ask,
                                };
                            }
                        }
                        refresh_permission_rule_studio_dialog(&self.i18n, parent);
                        true
                    }
                    _ => {
                        self.flash_error(ui_text::t(
                            &self.i18n,
                            "flash-permission-rule-context-lost",
                        ));
                        true
                    }
                }
            }
            ChoiceOverlayAction::PermissionStudioMode(target) => {
                let value = choice_selection_value(&selection);
                let Some((host, mut parent)) = self.take_permission_studio_dialog() else {
                    self.flash_error(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-context-lost",
                    ));
                    return true;
                };
                let mut permission = parent.permission.clone();
                let result = apply_permission_studio_mode_input(
                    &self.i18n,
                    &mut permission,
                    &target,
                    value.as_str(),
                )
                .and_then(|_| self.persist_permission_studio(&mut parent, permission));
                match result {
                    Ok(()) => {
                        self.restore_permission_studio_dialog(host, parent);
                        true
                    }
                    Err(error) => {
                        self.restore_permission_studio_dialog(host, parent);
                        self.flash_warning(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::PermissionStudioAddEntries(kind) => {
                let mut entries = dialog.presentation.checked_keys.clone();
                if entries.is_empty()
                    && let agena_tui::choice::ChoiceSelection::Item { value } = &selection
                {
                    entries.push(value.clone());
                }
                let wants_custom = entries
                    .iter()
                    .any(|entry| entry == PERMISSION_STUDIO_CUSTOM_ENTRY);
                entries.retain(|entry| entry != PERMISSION_STUDIO_CUSTOM_ENTRY);
                entries.sort();
                entries.dedup();
                if entries.is_empty() && !wants_custom {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-catalog-empty",
                    ));
                    return false;
                }

                let Some((host, mut parent)) = self.take_permission_studio_dialog() else {
                    self.flash_error(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-context-lost",
                    ));
                    return true;
                };
                if !entries.is_empty() {
                    self.restore_permission_studio_dialog(host, parent);
                    self.open_permission_studio_add_entries_mode(kind, entries, wants_custom);
                    return true;
                }
                if wants_custom {
                    self.open_permission_studio_creator(
                        &mut parent,
                        match kind {
                            PermissionStudioCatalogKind::ToolTags => {
                                PermissionStudioEditorAction::AddToolTag
                            }
                            PermissionStudioCatalogKind::ToolNames => {
                                PermissionStudioEditorAction::AddToolName
                            }
                        },
                    );
                }
                self.restore_permission_studio_dialog(host, parent);
                true
            }
            ChoiceOverlayAction::PermissionStudioAddEntriesMode {
                kind,
                entries,
                add_custom_after,
            } => {
                let value = choice_selection_value(&selection);
                let mode = match value.as_str() {
                    "allow" => PermissionMode::Allow,
                    "deny" => PermissionMode::Deny,
                    _ => PermissionMode::Ask,
                };
                let Some((host, mut parent)) = self.take_permission_studio_dialog() else {
                    self.flash_error(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-context-lost",
                    ));
                    return true;
                };
                let mut permission = parent.permission.clone();
                apply_permission_studio_entries_mode(&mut permission, kind, entries, mode);
                if let Err(error) = self.persist_permission_studio(&mut parent, permission) {
                    self.restore_permission_studio_dialog(host, parent);
                    self.flash_warning(error);
                    return false;
                }
                if add_custom_after {
                    self.open_permission_studio_creator(
                        &mut parent,
                        match kind {
                            PermissionStudioCatalogKind::ToolTags => {
                                PermissionStudioEditorAction::AddToolTag
                            }
                            PermissionStudioCatalogKind::ToolNames => {
                                PermissionStudioEditorAction::AddToolName
                            }
                        },
                    );
                }
                self.restore_permission_studio_dialog(host, parent);
                true
            }
        }
    }

    pub(crate) fn open_settings_field_editor(
        &mut self,
        field: SettingsFieldSpec,
        _return_query: &str,
    ) {
        let sources = match self.backend.config_json_sources() {
            Ok(sources) => sources,
            Err(error) => {
                self.flash_error(crate::UiFailure::internal(error));
                return;
            }
        };
        let file_value = get_json_path(&sources.file, Some(field.path)).unwrap_or(JsonValue::Null);
        let effective_value =
            get_json_path(&sources.effective, Some(field.path)).unwrap_or(JsonValue::Null);
        let prefill = if !file_value.is_null() {
            file_value.clone()
        } else {
            JsonValue::Null
        };
        if let Some(all_items) = self.settings_field_choice_items(field) {
            let current_value = (!prefill.is_null()).then(|| setting_value_input_text(&prefill));
            self.open_choice_overlay(self.build_choice_overlay(
                settings_edit_title(
                    &self.i18n,
                    settings_field_edit_title(&self.i18n, field).as_str(),
                ),
                settings_value_edit_prompt(&self.i18n, field, &file_value, &effective_value),
                current_value,
                all_items,
                ChoiceOverlayAction::SettingsField(field),
                true,
                Self::settings_field_choice_overlay_style(field),
            ));
            return;
        }
        self.overlay = Some(Overlay::SettingsValueEdit(SettingsValueEditOverlay::new(
            settings_edit_title(
                &self.i18n,
                settings_field_edit_title(&self.i18n, field).as_str(),
            ),
            settings_value_edit_prompt(&self.i18n, field, &file_value, &effective_value),
            Editor::from_text(setting_value_input_text(&prefill)),
            field,
        )));
    }
}

fn choice_selection_value(selection: &agena_tui::choice::ChoiceSelection) -> String {
    match selection {
        agena_tui::choice::ChoiceSelection::Clear => String::new(),
        agena_tui::choice::ChoiceSelection::Custom { raw }
        | agena_tui::choice::ChoiceSelection::Item { value: raw } => raw.clone(),
    }
}
use crate::{
    App, ChoiceOverlay, ChoiceOverlayAction, Editor, JsonValue, Overlay,
    PERMISSION_STUDIO_CUSTOM_ENTRY, PermissionMode, PermissionRuleStudioChoiceField,
    PermissionRuleSubjectKind, PermissionStudioCatalogKind, PermissionStudioEditorAction, Route,
    SettingsFieldSpec, SettingsValueEditOverlay, apply_permission_studio_entries_mode,
    apply_permission_studio_mode_input, get_json_path, parse_settings_field_input,
    refresh_permission_rule_studio_dialog, setting_value_input_text, settings_edit_title,
    settings_field_edit_title, settings_path_cleared_message, settings_path_updated_message,
    settings_value_edit_prompt, ui_text,
};
