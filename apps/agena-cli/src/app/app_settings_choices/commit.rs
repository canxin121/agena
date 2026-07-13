impl App {
    pub(in crate::app) fn commit_choice_overlay(&mut self, dialog: &mut ChoiceOverlay) -> bool {
        let Some(selection) = dialog.selected_row() else {
            return false;
        };
        match dialog.meta.action.clone() {
            ChoiceOverlayAction::SettingsField(field) => {
                let input = match selection {
                    SearchPickerSelection::Clear(_) => String::new(),
                    SearchPickerSelection::Custom(value) => value.raw,
                    SearchPickerSelection::Item(item) => item.value,
                };
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
            ChoiceOverlayAction::RuntimeSetting(field) => {
                let input = match selection {
                    SearchPickerSelection::Clear(_) => String::new(),
                    SearchPickerSelection::Custom(value) => value.raw,
                    SearchPickerSelection::Item(item) => item.value,
                };
                match self.run_options.apply_runtime_setting_input(
                    &self.i18n,
                    field,
                    input.as_str(),
                ) {
                    Ok(message) => {
                        self.flash_success(message);
                        self.refresh_current_route_after_local_edit();
                        true
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::SessionModelVariant(step) => {
                let input = match selection {
                    SearchPickerSelection::Clear(_) => String::new(),
                    SearchPickerSelection::Custom(value) => value.raw,
                    SearchPickerSelection::Item(item) => item.value,
                };
                let field = session_model_variant_field(step);
                match self.run_options.apply_runtime_setting_input(
                    &self.i18n,
                    field,
                    input.as_str(),
                ) {
                    Ok(_) => {
                        self.advance_session_model_variant_step(step);
                        true
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::ProviderStudioField(field) => {
                let value = match selection {
                    SearchPickerSelection::Clear(_) => String::new(),
                    SearchPickerSelection::Custom(value) => value.raw,
                    SearchPickerSelection::Item(item) => item.value,
                };
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
                let value = match selection {
                    SearchPickerSelection::Clear(_) => String::new(),
                    SearchPickerSelection::Custom(value) => value.raw,
                    SearchPickerSelection::Item(item) => item.value,
                };
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
                let value = match selection {
                    SearchPickerSelection::Clear(_) => String::new(),
                    SearchPickerSelection::Custom(value) => value.raw,
                    SearchPickerSelection::Item(item) => item.value,
                };
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
                let value = match selection {
                    SearchPickerSelection::Clear(_) => String::new(),
                    SearchPickerSelection::Custom(value) => value.raw,
                    SearchPickerSelection::Item(item) => item.value,
                };
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
        }
    }

    pub(in crate::app) fn open_settings_field_editor(
        &mut self,
        field: SettingsFieldSpec,
        _return_query: &str,
    ) {
        let sources = match self.backend.config_json_sources() {
            Ok(sources) => sources,
            Err(error) => {
                self.flash_error(error.to_string());
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
use crate::app::{
    App, ChoiceOverlay, ChoiceOverlayAction, Editor, JsonValue, Overlay, PermissionMode,
    PermissionRuleStudioChoiceField, PermissionRuleSubjectKind, Route, SearchPickerSelection,
    SettingsFieldSpec, SettingsValueEditOverlay, apply_permission_studio_mode_input, get_json_path,
    parse_settings_field_input, refresh_permission_rule_studio_dialog, session_model_variant_field,
    setting_value_input_text, settings_edit_title, settings_field_edit_title,
    settings_path_cleared_message, settings_path_updated_message, settings_value_edit_prompt,
    ui_text,
};
