impl App {
    pub(in crate::app) fn activate_provider_studio_focus(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        match dialog.selection.focus() {
            ProviderStudioFocus::Fields => {
                let fields = provider_studio_visible_fields(dialog);
                let Some(field) = fields.get(dialog.selection.top_selected()).copied() else {
                    return;
                };
                match field {
                    ProviderStudioField::StartAuthAction => {
                        self.activate_provider_studio_start_auth(dialog);
                    }
                    ProviderStudioField::ContinueAuthAction => {
                        self.activate_provider_studio_continue_auth(dialog);
                    }
                    ProviderStudioField::EditAuthDetailsAction => {
                        self.open_provider_studio_detail_page(dialog);
                    }
                    ProviderStudioField::DeleteProviderAction => {
                        if let Some(provider_id) = dialog.draft.source_provider_id.clone() {
                            self.open_provider_studio_delete_provider_confirm(provider_id);
                        }
                    }
                    ProviderStudioField::LoadModelsAction => {
                        self.request_provider_studio_adapter_models(dialog);
                    }
                    ProviderStudioField::AddModelAction => {
                        self.open_provider_studio_new_model_editor(dialog);
                    }
                    ProviderStudioField::DeleteAdapterAction => {
                        self.open_provider_studio_delete_selected_adapter_confirm(dialog);
                    }
                    ProviderStudioField::DeleteModelAction => {
                        self.open_provider_studio_delete_selected_model_confirm(dialog);
                    }
                    ProviderStudioField::SaveAdapterAction => {
                        if provider_studio_selected_adapter_models(dialog).is_none() {
                            self.flash_warning(ui_text::t(
                                &self.i18n,
                                "flash-provider-studio-adapter-required",
                            ));
                            return;
                        }
                        dialog.saving = true;
                        self.request_provider_studio_save_selected_adapter(dialog.clone());
                    }
                    ProviderStudioField::SaveProviderAction => {
                        dialog.saving = true;
                        self.request_provider_studio_save_draft(dialog.clone());
                    }
                    _ => self.activate_provider_studio_field_editor(dialog, field),
                }
            }
            ProviderStudioFocus::Adapters => {
                if let Some(adapter_id) = provider_studio_selected_adapter_models(dialog)
                    .map(|adapter_models| adapter_models.adapter_id.clone())
                {
                    dialog.draft.default_adapter = adapter_id;
                }
            }
            ProviderStudioFocus::Models => {
                if let Some((adapter_id, model_id, provider_model)) =
                    provider_studio_selected_model_target(dialog)
                {
                    self.open_provider_studio_model_page(
                        dialog,
                        adapter_id,
                        model_id,
                        provider_model,
                    );
                }
            }
        }
    }

    pub(in crate::app) fn commit_provider_studio_field(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        field: ProviderStudioField,
        value: String,
    ) -> UiResult<()> {
        match field {
            ProviderStudioField::ProviderId => {
                dialog.draft.provider_id = value;
                dialog.draft.normalize_shape();
                self.refresh_provider_studio_adapter_state(dialog);
            }
            ProviderStudioField::StartAuthAction
            | ProviderStudioField::ContinueAuthAction
            | ProviderStudioField::EditAuthDetailsAction
            | ProviderStudioField::DeleteProviderAction
            | ProviderStudioField::LoadModelsAction
            | ProviderStudioField::AddModelAction
            | ProviderStudioField::DeleteAdapterAction
            | ProviderStudioField::DeleteModelAction
            | ProviderStudioField::SaveAdapterAction
            | ProviderStudioField::SaveProviderAction => {}
            ProviderStudioField::AuthMode => {
                match ProviderDraftAuthKind::parse_category(
                    value.as_str(),
                    dialog.draft.auth_kind.clone(),
                ) {
                    Ok(auth_kind) => {
                        dialog.draft.auth_kind = auth_kind;
                        dialog.draft.normalize_shape();
                        self.refresh_provider_studio_adapter_state(dialog);
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            ProviderStudioField::AuthSubtype => {
                match ProviderDraftAuthKind::parse_subtype(
                    value.as_str(),
                    dialog.draft.auth_kind.clone(),
                ) {
                    Ok(auth_kind) => {
                        dialog.draft.auth_kind = auth_kind;
                        dialog.draft.normalize_shape();
                        self.refresh_provider_studio_adapter_state(dialog);
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            ProviderStudioField::AuthLoginMethod => {
                let Some(kind) = ProviderDraftInteractiveLoginKind::parse(value.as_str()) else {
                    return Err(ui_text::t(
                        &self.i18n,
                        "flash-provider-studio-invalid-auth-login-method",
                    ));
                };
                dialog.draft.set_interactive_login_kind(kind);
            }
            ProviderStudioField::BaseUrl => {
                dialog.draft.auth.base_url = value;
            }
            ProviderStudioField::InstanceUrl => {
                dialog.draft.auth.instance_url = value;
            }
            ProviderStudioField::ApiKeySource => {
                dialog.draft.auth.secret_source_kind =
                    ProviderDraftSecretSourceKind::parse(value.as_str())
                        .map_err(|error| error.to_string())?;
            }
            ProviderStudioField::ApiKeyValue => {
                dialog.draft.auth.secret_source_value = value;
            }
            ProviderStudioField::RedirectUri => {
                dialog.draft.set_redirect_uri(value);
            }
            ProviderStudioField::CallbackUrl => {
                dialog.draft.set_callback_url(value);
            }
            ProviderStudioField::RefreshToken => {
                dialog.draft.set_refresh_token(value);
            }
            ProviderStudioField::AccessToken => {
                dialog.draft.set_access_token(value);
            }
            ProviderStudioField::ExpiresAtMs => {
                dialog.draft.set_expires_at_ms(value);
            }
            ProviderStudioField::AccountId => {
                dialog.draft.set_account_id(value);
            }
            ProviderStudioField::EnterpriseDomain => {
                dialog
                    .draft
                    .credential_drafts
                    .github_copilot
                    .enterprise_domain = value;
            }
            ProviderStudioField::Region => {
                dialog.draft.auth.region = value;
            }
            ProviderStudioField::Profile => {
                dialog.draft.auth.profile = value;
            }
            ProviderStudioField::AccessKeyId => {
                dialog.draft.auth.access_key_id = value;
            }
            ProviderStudioField::SecretAccessKey => {
                dialog.draft.auth.secret_access_key = value;
            }
            ProviderStudioField::SessionToken => {
                dialog.draft.auth.session_token = value;
            }
            ProviderStudioField::ServiceKeyEnv => {
                dialog.draft.auth.service_key_env = value;
            }
            ProviderStudioField::DefaultAdapter => {
                dialog.draft.default_adapter = value;
                self.sync_provider_studio_shape(dialog);
            }
            ProviderStudioField::DefaultModel => {
                dialog.draft.default_model = value;
            }
        }
        Ok(())
    }

    pub(in crate::app) fn toggle_provider_studio_selected_adapter(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let Some(adapter_id) = provider_studio_selected_adapter_id(dialog) else {
            return;
        };
        if !provider_studio_adapter_selectable(dialog, adapter_id.as_str()) {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-unavailable",
            ));
            return;
        }
        if !dialog.selected_adapter_ids.remove(adapter_id.as_str()) {
            dialog.selected_adapter_ids.insert(adapter_id);
        }
        dialog.adapter_selection_touched = true;
        self.sync_provider_studio_shape(dialog);
    }

    pub(in crate::app) fn toggle_provider_studio_selected_model(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let Some((adapter_id, model_id, _)) = provider_studio_selected_model_target(dialog) else {
            return;
        };
        let key = provider_studio_model_key(adapter_id.as_str(), model_id.as_str());
        if !dialog.selected_model_keys.remove(key.as_str()) {
            dialog.selected_model_keys.insert(key);
        }
        provider_studio_ensure_default_selection(dialog);
    }
}
use crate::app::{
    App, ProviderDraftAuthKind, ProviderDraftInteractiveLoginKind, ProviderDraftSecretSourceKind,
    ProviderStudioField, ProviderStudioFocus, ProviderStudioOverlay, UiResult,
    provider_studio_adapter_selectable, provider_studio_ensure_default_selection,
    provider_studio_model_key, provider_studio_selected_adapter_id,
    provider_studio_selected_adapter_models, provider_studio_selected_model_target,
    provider_studio_visible_fields, ui_text,
};
