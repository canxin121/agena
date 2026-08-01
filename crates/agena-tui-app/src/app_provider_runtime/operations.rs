impl App {
    pub(crate) fn session_model_chooser_items(&self) -> UiResult<Vec<SessionModelChoiceItem>> {
        let providers = self.backend.list_configured_providers();
        let mut items = Vec::new();
        for provider in providers {
            let default_adapter = provider.defaults.adapter.clone();
            let models = self
                .backend
                .list_local_provider_models(provider.provider_id.as_str())
                .map_err(crate::UiFailure::internal)?;
            for model in models {
                items.push(session_model_choice_item(
                    &self.i18n,
                    provider.provider_id.as_str(),
                    default_adapter.as_deref(),
                    model,
                ));
            }
        }
        items.sort_by(|left, right| {
            (
                left.identity.provider_id.clone(),
                left.identity.adapter_id.clone().unwrap_or_default(),
                left.identity.model_id.clone(),
            )
                .cmp(&(
                    right.identity.provider_id.clone(),
                    right.identity.adapter_id.clone().unwrap_or_default(),
                    right.identity.model_id.clone(),
                ))
        });
        Ok(items)
    }

    pub(crate) fn request_provider_studio_save_draft(&mut self, dialog: ProviderStudioOverlay) {
        // Convert at the adapter boundary; the feature crate never sees the
        // concrete backend draft or authentication types.
        let _protocol_draft = provider_studio_snapshot(&dialog);
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let adapter_ids = provider_studio_request_adapter_ids(&dialog);
        tokio::spawn(async move {
            let result = backend
                .save_provider_draft(
                    dialog.draft.clone(),
                    dialog.adapter_models.as_slice(),
                    &adapter_ids,
                    &dialog.selected_model_keys,
                )
                .await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: dialog.draft.provider_id.clone(),
                result,
            });
        });
    }

    pub(crate) fn request_provider_studio_save_selected_adapter(
        &mut self,
        dialog: ProviderStudioOverlay,
    ) {
        let Some(adapter_models) = provider_studio_selected_adapter_models_for_save(&dialog) else {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-required",
            ));
            return;
        };
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .save_provider_adapter_matches(dialog.draft.clone(), adapter_models)
                .await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: dialog.draft.provider_id.clone(),
                result,
            });
        });
    }

    pub(crate) fn request_provider_studio_save_model_value(
        &mut self,
        draft: ProviderConfigDraft,
        adapter_id: String,
        model_id: String,
        model_value: JsonValue,
    ) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .save_provider_model_value(
                    draft.clone(),
                    adapter_id.as_str(),
                    model_id.as_str(),
                    model_value,
                )
                .await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: draft.provider_id.clone(),
                result,
            });
        });
    }

    pub(crate) fn request_provider_studio_delete_model(
        &mut self,
        draft: ProviderConfigDraft,
        adapter_id: String,
        model_id: String,
    ) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .delete_provider_model(draft.clone(), adapter_id.as_str(), model_id.as_str())
                .await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: draft.provider_id.clone(),
                result,
            });
        });
    }

    pub(crate) fn request_provider_studio_delete_provider(&mut self, provider_id: String) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend.delete_provider(provider_id.as_str()).await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id,
                result,
            });
        });
    }

    pub(crate) fn request_provider_studio_delete_adapter(
        &mut self,
        draft: ProviderConfigDraft,
        adapter_id: String,
    ) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .delete_provider_adapter(draft.clone(), adapter_id.as_str())
                .await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: draft.provider_id.clone(),
                result,
            });
        });
    }

    pub(crate) fn move_provider_studio_selection(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        delta: isize,
    ) {
        match dialog.selection.focus() {
            ProviderStudioFocus::Fields => dialog
                .selection
                .move_top(provider_studio_visible_fields(dialog).len(), delta),
            ProviderStudioFocus::Adapters => {
                dialog
                    .selection
                    .move_left(dialog.adapter_candidate_ids.len(), delta);
                dialog.selection.clamp_right(
                    provider_studio_selected_adapter_models(dialog)
                        .map(|adapter| adapter.models.len())
                        .unwrap_or_default(),
                );
            }
            ProviderStudioFocus::Models => dialog.selection.move_right(
                provider_studio_selected_adapter_models(dialog)
                    .map(|adapter| adapter.models.len())
                    .unwrap_or_default(),
                delta,
            ),
        }
    }

    pub(crate) fn open_provider_studio_detail_page(&mut self, dialog: &mut ProviderStudioOverlay) {
        if provider_studio_detail_fields(dialog).is_empty() {
            self.flash_warning(provider_studio_no_auth_details_message(&self.i18n));
            return;
        }
        dialog.model_page = None;
        dialog.detail_page = Some(ProviderStudioDetailPage {
            title: ui_text::t(&self.i18n, "overlay-provider-studio-detail"),
            footer: ui_text::t(&self.i18n, "overlay-provider-studio-detail-footer"),
            selection: SelectionCursor::new(provider_studio_preferred_detail_field_index(dialog)),
        });
    }

    pub(crate) fn guide_provider_studio_auth_field(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        field: ProviderStudioField,
    ) -> bool {
        if provider_studio_detail_field_index(dialog, field).is_none() {
            return false;
        }
        dialog.detail_page = None;
        self.activate_provider_studio_field_editor(dialog, field);
        true
    }

    pub(crate) fn activate_provider_studio_start_auth(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        if let Some(field) = provider_studio_missing_start_auth_field(dialog) {
            let _ = self.guide_provider_studio_auth_field(dialog, field);
            return;
        }
        self.request_provider_studio_start_auth(dialog);
    }

    pub(crate) fn activate_provider_studio_continue_auth(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        if let Some(field) = provider_studio_missing_continue_auth_field(dialog) {
            let _ = self.guide_provider_studio_auth_field(dialog, field);
            return;
        }
        self.request_provider_studio_continue_auth(dialog);
    }

    pub(crate) fn activate_provider_studio_field_editor(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        field: ProviderStudioField,
    ) {
        if !provider_studio_field_editable(dialog, field) {
            return;
        }
        if let Some(all_items) = self.provider_studio_field_choice_items(dialog, field) {
            let allow_clear = provider_studio_field_allows_clear(field);
            let current_value = provider_studio_field_value(&dialog.draft, field);
            let current_value = if allow_clear && current_value.trim().is_empty() {
                None
            } else {
                Some(current_value)
            };
            self.open_choice_overlay(self.build_choice_overlay(
                ui_text::t(&self.i18n, "overlay-provider-studio-edit-title"),
                provider_studio_field_prompt(&self.i18n, field),
                current_value,
                all_items,
                ChoiceOverlayAction::ProviderStudioField(field),
                allow_clear,
                Self::provider_studio_field_choice_overlay_style(field),
            ));
            return;
        }
        dialog.editor = Some(ProviderStudioEditor::new(
            ui_text::t(&self.i18n, "overlay-provider-studio-edit-title"),
            provider_studio_field_prompt(&self.i18n, field),
            ui_text::t(&self.i18n, "overlay-provider-studio-edit-footer"),
            false,
            Editor::from_text(provider_studio_field_value(&dialog.draft, field)),
            ProviderStudioEditorAction::Field(field),
        ));
    }

    pub(crate) fn activate_provider_studio_detail_page_selection(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let Some(selected_field) = dialog
            .detail_page
            .as_ref()
            .map(|page| page.selection.selected)
        else {
            return;
        };
        let fields = provider_studio_detail_fields(dialog);
        let Some(field) = fields.get(selected_field).copied() else {
            return;
        };
        self.activate_provider_studio_field_editor(dialog, field);
    }

    pub(crate) fn handle_provider_studio_detail_page_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ProviderStudioOverlay,
    ) -> bool {
        let field_count = provider_studio_detail_fields(dialog).len();
        let Some(detail_page) = dialog.detail_page.as_mut() else {
            return false;
        };
        match resolve_tui_key(KeyContext::ProviderDetail, key) {
            Some(KeyAction::Back) => {
                dialog.detail_page = None;
                false
            }
            Some(KeyAction::Activate) => {
                self.activate_provider_studio_detail_page_selection(dialog);
                false
            }
            _ if detail_page
                .selection
                .handle_structural_navigation_key(key, field_count, 10) =>
            {
                false
            }
            _ => false,
        }
    }

    pub(crate) fn open_provider_studio_model_page(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        adapter_id: String,
        model_id: String,
        provider_model: Option<ProviderModelResource>,
    ) {
        match self.backend.provider_model_draft_value(
            &dialog.draft,
            adapter_id.as_str(),
            model_id.as_str(),
            provider_model.as_ref(),
        ) {
            Ok(model_value) => {
                match provider_model_config_draft_from_value(model_id.as_str(), model_value) {
                    Ok(mut draft) => {
                        apply_provider_model_config_supported_modes(
                            provider_model.as_ref(),
                            &mut draft,
                        );
                        dialog.detail_page = None;
                        dialog.model_page = Some(ProviderStudioModelPage {
                            title: self.i18n.text_args(
                                "overlay-provider-studio-model-title",
                                &agena_tui::fl_args!(
                                    "adapter" => adapter_id.clone(),
                                    "model" => model_id.clone(),
                                ),
                            ),
                            footer: ui_text::t(&self.i18n, "overlay-provider-studio-model-footer"),
                            adapter_id,
                            original_model_id: model_id,
                            draft,
                            selection: SelectionCursor::default(),
                        });
                    }
                    Err(error) => self.flash_error(error),
                }
            }
            Err(error) => self.flash_error(crate::UiFailure::internal(error)),
        }
    }

    pub(crate) fn open_provider_studio_new_model_editor(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let Some(adapter_id) = provider_studio_selected_adapter_id(dialog) else {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-required",
            ));
            return;
        };
        if !provider_studio_adapter_selectable(dialog, adapter_id.as_str()) {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-unavailable",
            ));
            return;
        }
        dialog.editor = Some(ProviderStudioEditor::new(
            ui_text::t(&self.i18n, "overlay-provider-studio-new-model-title"),
            ui_text::t(&self.i18n, "overlay-provider-studio-new-model-prompt"),
            ui_text::t(&self.i18n, "overlay-provider-studio-edit-footer"),
            false,
            Editor::from_text(String::new()),
            ProviderStudioEditorAction::NewModel { adapter_id },
        ));
    }

    pub(crate) fn open_provider_studio_delete_provider_confirm(&mut self, provider_id: String) {
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-provider-delete-title"),
            vec![self.i18n.text_args(
                "overlay-provider-delete-body",
                &agena_tui::fl_args!("provider" => provider_id.clone()),
            )],
            ConfirmAction::ProviderStudioDeleteProvider { provider_id },
        )));
    }

    pub(crate) fn open_provider_studio_delete_selected_confirm(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        match dialog.selection.focus() {
            ProviderStudioFocus::Fields => {
                if let Some(provider_id) = dialog.draft.source_provider_id.clone() {
                    self.open_provider_studio_delete_provider_confirm(provider_id);
                }
            }
            ProviderStudioFocus::Adapters => {
                self.open_provider_studio_delete_selected_adapter_confirm(dialog);
            }
            ProviderStudioFocus::Models => {
                self.open_provider_studio_delete_selected_model_confirm(dialog);
            }
        }
    }

    pub(crate) fn open_provider_studio_delete_adapter_confirm(
        &mut self,
        dialog: &ProviderStudioOverlay,
        adapter_id: String,
    ) {
        let mut body_lines = vec![self.i18n.text_args(
            "overlay-provider-delete-adapter-body",
            &agena_tui::fl_args!(
                "provider" => dialog.draft.provider_id.clone(),
                "adapter" => adapter_id.clone(),
            ),
        )];
        if dialog.draft.source_provider_id.is_some()
            && dialog.configured_adapter_ids.len() == 1
            && dialog.configured_adapter_ids.contains(adapter_id.as_str())
        {
            body_lines.push(ui_text::t(
                &self.i18n,
                "overlay-provider-delete-adapter-last-body",
            ));
        }
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-provider-delete-adapter-title"),
            body_lines,
            ConfirmAction::ProviderStudioDeleteAdapter { adapter_id },
        )));
    }

    pub(crate) fn open_provider_studio_delete_model_confirm(
        &mut self,
        dialog: &ProviderStudioOverlay,
        adapter_id: String,
        model_id: String,
    ) {
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-provider-delete-model-title"),
            vec![self.i18n.text_args(
                "overlay-provider-delete-model-body",
                &agena_tui::fl_args!(
                    "provider" => dialog.draft.provider_id.clone(),
                    "adapter" => adapter_id.clone(),
                    "model" => model_id.clone(),
                ),
            )],
            ConfirmAction::ProviderStudioDeleteModel {
                adapter_id,
                model_id,
            },
        )));
    }

    pub(crate) fn open_provider_studio_delete_selected_adapter_confirm(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let Some(adapter_id) = provider_studio_selected_adapter_id(dialog) else {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-required",
            ));
            return;
        };
        let has_state = dialog.configured_adapter_ids.contains(adapter_id.as_str())
            || dialog.selected_adapter_ids.contains(adapter_id.as_str())
            || dialog
                .adapter_models
                .iter()
                .any(|adapter_models| adapter_models.adapter_id == adapter_id)
            || dialog.draft.default_adapter == adapter_id;
        if !has_state {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-delete-empty",
            ));
            return;
        }
        self.open_provider_studio_delete_adapter_confirm(dialog, adapter_id);
    }

    pub(crate) fn open_provider_studio_delete_selected_model_confirm(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let target = if let Some(page) = dialog.model_page.as_ref() {
            Some((page.adapter_id.clone(), page.original_model_id.clone()))
        } else {
            provider_studio_selected_model_target(dialog)
                .map(|(adapter_id, model_id, _)| (adapter_id, model_id))
        };
        let Some((adapter_id, model_id)) = target else {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-model-required",
            ));
            return;
        };
        self.open_provider_studio_delete_model_confirm(dialog, adapter_id, model_id);
    }
}
use crate::provider_studio::provider_studio_snapshot;
use crate::{
    App, AppMessage, ChoiceOverlayAction, ConfirmAction, Editor, JsonValue, KeyEvent, Overlay,
    ProviderConfigDraft, ProviderStudioDetailPage, ProviderStudioEditor,
    ProviderStudioEditorAction, ProviderStudioField, ProviderStudioFocus, ProviderStudioModelPage,
    ProviderStudioOverlay, SelectionCursor, SessionModelChoiceItem, UiResult,
    apply_provider_model_config_supported_modes, provider_model_config_draft_from_value,
    provider_studio_adapter_selectable, provider_studio_detail_field_index,
    provider_studio_detail_fields, provider_studio_field_allows_clear,
    provider_studio_field_editable, provider_studio_field_prompt, provider_studio_field_value,
    provider_studio_missing_continue_auth_field, provider_studio_missing_start_auth_field,
    provider_studio_no_auth_details_message, provider_studio_preferred_detail_field_index,
    provider_studio_request_adapter_ids, provider_studio_selected_adapter_id,
    provider_studio_selected_adapter_models, provider_studio_selected_adapter_models_for_save,
    provider_studio_selected_model_target, provider_studio_visible_fields,
    session_model_choice_item, ui_text,
};
use agena_api::resource::ProviderModelResource;
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
