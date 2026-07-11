impl App {
    pub(in crate::app) fn add_provider_studio_manual_model(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        adapter_id: String,
        model_id: String,
    ) -> UiResult<()> {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(ui_text::t(
                &self.i18n,
                "flash-provider-studio-model-id-required",
            ));
        }
        if !dialog.selected_adapter_ids.contains(adapter_id.as_str()) {
            dialog.selected_adapter_ids.insert(adapter_id.clone());
            dialog.adapter_selection_touched = true;
        }
        if !dialog
            .adapter_models
            .iter()
            .any(|adapter_models| adapter_models.adapter_id == adapter_id)
        {
            dialog.adapter_models.push(ProviderAdapterModelsResource {
                adapter_id: adapter_id.clone(),
                enabled: true,
                resolved_base_url: None,
                models: Vec::new(),
                error: None,
            });
        }
        let adapter_index = dialog
            .adapter_models
            .iter()
            .position(|adapter_models| adapter_models.adapter_id == adapter_id)
            .expect("adapter models entry must exist");
        if !dialog.adapter_models[adapter_index]
            .models
            .iter()
            .any(|model| model.id.as_ref() == model_id)
        {
            dialog.adapter_models[adapter_index]
                .models
                .push(ProviderModel::new(adapter_id.as_str(), model_id));
            dialog.adapter_models[adapter_index]
                .models
                .sort_by(|left, right| left.id.as_ref().cmp(right.id.as_ref()));
        }
        let selected_model_index = dialog.adapter_models[adapter_index]
            .models
            .iter()
            .position(|model| model.id.as_ref() == model_id)
            .unwrap_or_default();
        if let Some(left_index) = dialog
            .adapter_candidate_ids
            .iter()
            .position(|candidate| candidate == &adapter_id)
        {
            dialog.selection.set_left_selected(left_index);
        }
        dialog.selection.set_right_selected(selected_model_index);
        dialog
            .selected_model_keys
            .insert(provider_studio_model_key(adapter_id.as_str(), model_id));
        provider_studio_ensure_default_selection(dialog);
        self.open_provider_studio_model_page(dialog, adapter_id, model_id.to_owned(), None);
        Ok(())
    }

    pub(in crate::app) fn activate_provider_studio_model_field_editor(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        field: ProviderModelConfigField,
    ) {
        if !provider_model_config_field_editable(field) {
            return;
        }
        if let Some(items) = self.provider_model_config_field_choice_items(dialog, field) {
            let current = dialog
                .model_page
                .as_ref()
                .map(|page| provider_model_config_field_value(&page.draft, field))
                .unwrap_or_default();
            self.open_choice_overlay(self.build_choice_overlay(
                ui_text::t(&self.i18n, "overlay-provider-studio-model-edit-title"),
                provider_model_config_field_prompt(&self.i18n, field),
                Editor::from_text(current),
                items,
                ChoiceOverlayAction::ProviderStudioModelField(field),
                !matches!(field, ProviderModelConfigField::Enabled),
                Self::provider_model_config_field_choice_overlay_style(field),
            ));
            return;
        }
        let current = dialog
            .model_page
            .as_ref()
            .map(|page| provider_model_config_field_value(&page.draft, field))
            .unwrap_or_default();
        dialog.editor = Some(ProviderStudioEditor::new(
            ui_text::t(&self.i18n, "overlay-provider-studio-model-edit-title"),
            provider_model_config_field_prompt(&self.i18n, field),
            ui_text::t(&self.i18n, "overlay-provider-studio-edit-footer"),
            matches!(field, ProviderModelConfigField::Description),
            Editor::from_text(current),
            ProviderStudioEditorAction::ModelField(field),
        ));
    }

    pub(in crate::app) fn activate_provider_studio_model_page_selection(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let Some(selected) = dialog
            .model_page
            .as_ref()
            .map(|page| page.selection.selected)
        else {
            return;
        };
        let Some(field) = provider_model_config_fields().get(selected).copied() else {
            return;
        };
        match field {
            ProviderModelConfigField::SaveAction => self.save_provider_studio_model_page(dialog),
            ProviderModelConfigField::DeleteAction => {
                self.open_provider_studio_delete_selected_model_confirm(dialog)
            }
            _ => self.activate_provider_studio_model_field_editor(dialog, field),
        }
    }

    pub(in crate::app) fn commit_provider_studio_model_field(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        field: ProviderModelConfigField,
        value: String,
    ) -> UiResult<()> {
        let Some(page) = dialog.model_page.as_mut() else {
            return Err(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
        };
        commit_provider_model_config_field(&mut page.draft, field, value)
    }

    pub(in crate::app) fn save_provider_studio_model_page(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let Some(page) = dialog.model_page.as_ref() else {
            return;
        };
        let (model_id, model_value) = match provider_model_config_draft_to_model_value(&page.draft)
        {
            Ok(value) => value,
            Err(error) => {
                self.flash_error(error);
                return;
            }
        };
        dialog.saving = true;
        self.request_provider_studio_save_model_value(
            dialog.draft.clone(),
            page.adapter_id.clone(),
            model_id,
            model_value,
        );
    }

    pub(in crate::app) fn delete_provider_studio_model(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        adapter_id: String,
        model_id: String,
    ) {
        if dialog.draft.source_provider_id.is_some() {
            dialog.saving = true;
            self.request_provider_studio_delete_model(dialog.draft.clone(), adapter_id, model_id);
        } else {
            remove_provider_studio_model_from_dialog(
                dialog,
                adapter_id.as_str(),
                model_id.as_str(),
            );
            dialog.selection.set_focus(ProviderStudioFocus::Models);
        }
    }

    pub(in crate::app) fn delete_provider_studio_adapter(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        adapter_id: String,
    ) {
        if dialog.draft.source_provider_id.is_some()
            && dialog.configured_adapter_ids.contains(adapter_id.as_str())
        {
            dialog.saving = true;
            self.request_provider_studio_delete_adapter(dialog.draft.clone(), adapter_id);
        } else {
            remove_provider_studio_adapter_from_dialog(dialog, adapter_id.as_str());
            dialog.selection.set_focus(ProviderStudioFocus::Adapters);
        }
    }

    pub(in crate::app) fn handle_provider_studio_model_page_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ProviderStudioOverlay,
    ) -> bool {
        let field_count = provider_model_config_fields().len();
        if dialog.model_page.is_none() {
            return false;
        }
        match resolve_tui_key(KeyContext::ProviderModel, key) {
            Some(KeyAction::Back) => {
                dialog.model_page = None;
                false
            }
            Some(KeyAction::Activate) => {
                self.activate_provider_studio_model_page_selection(dialog);
                false
            }
            _ if dialog.model_page.as_mut().is_some_and(|page| {
                page.selection
                    .handle_structural_navigation_key(key, field_count, 10)
            }) =>
            {
                false
            }
            _ => false,
        }
    }
}
use crate::app::{
    App, ChoiceOverlayAction, Editor, KeyEvent, ProviderAdapterModelsResource, ProviderModel,
    ProviderModelConfigField, ProviderStudioEditor, ProviderStudioEditorAction,
    ProviderStudioFocus, ProviderStudioOverlay, UiResult, commit_provider_model_config_field,
    provider_model_config_draft_to_model_value, provider_model_config_field_editable,
    provider_model_config_field_prompt, provider_model_config_field_value,
    provider_model_config_fields, provider_studio_ensure_default_selection,
    provider_studio_model_key, remove_provider_studio_adapter_from_dialog,
    remove_provider_studio_model_from_dialog, ui_text,
};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
