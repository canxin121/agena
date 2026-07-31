impl App {
    pub(crate) fn activate_provider_studio_focus(&mut self, dialog: &mut ProviderStudioOverlay) {
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
                    _ => self.activate_provider_studio_field_editor(dialog, field),
                }
            }
            ProviderStudioFocus::Adapters => {}
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

    pub(crate) fn commit_provider_studio_field(
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
            | ProviderStudioField::EditAuthDetailsAction => {}
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
                    Err(error) => {
                        return Err(crate::UiFailure::invalid_with_diagnostic(
                            "The provider authentication mode is invalid.",
                            error,
                        ));
                    }
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
                    Err(error) => {
                        return Err(crate::UiFailure::invalid_with_diagnostic(
                            "The provider authentication subtype is invalid.",
                            error,
                        ));
                    }
                }
            }
            ProviderStudioField::AuthLoginMethod => {
                let Some(kind) = ProviderDraftInteractiveLoginKind::parse(value.as_str()) else {
                    return Err(crate::UiFailure::message(ui_text::t(
                        &self.i18n,
                        "flash-provider-studio-invalid-auth-login-method",
                    )));
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
                        .map_err(crate::UiFailure::internal)?;
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
            ProviderStudioField::RequestTimeoutSecs => {
                dialog.draft.request_timeout_secs = value
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        "request timeout must be a positive number of seconds".to_owned()
                    })
                    .map_err(crate::UiFailure::message)?;
            }
            ProviderStudioField::ConnectTimeoutSecs => {
                dialog.draft.connect_timeout_secs = value
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        "connect timeout must be a positive number of seconds".to_owned()
                    })
                    .map_err(crate::UiFailure::message)?;
            }
        }
        Ok(())
    }

    pub(crate) fn toggle_provider_studio_selected_adapter(
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
        self.sync_provider_studio_shape(dialog);
    }

    pub(crate) fn toggle_provider_studio_selected_model(
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
use crate::{
    App, ProviderDraftAuthKind, ProviderDraftInteractiveLoginKind, ProviderDraftSecretSourceKind,
    ProviderStudioField, ProviderStudioFocus, ProviderStudioOverlay, UiResult,
    provider_studio_adapter_selectable, provider_studio_ensure_default_selection,
    provider_studio_model_key, provider_studio_selected_adapter_id,
    provider_studio_selected_model_target, provider_studio_visible_fields, ui_text,
};
