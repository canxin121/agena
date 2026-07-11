pub(in crate::app) fn settings_edit_title(i18n: &I18n, field: &str) -> String {
    i18n.text_args(
        "overlay-settings-edit-title",
        &crate::fl_args!("field" => field),
    )
}

pub(in crate::app) fn editor_save_footer(i18n: &I18n, multiline: bool) -> String {
    ui_text::t(
        i18n,
        if multiline {
            "overlay-editor-footer-multiline"
        } else {
            "overlay-editor-footer-single-line"
        },
    )
}

pub(in crate::app) fn settings_clear_label(i18n: &I18n) -> String {
    ui_text::t(i18n, "overlay-choice-clear-value")
}

pub(in crate::app) fn settings_path_updated_message(i18n: &I18n, path: &str) -> String {
    i18n.text_args("flash-settings-updated", &crate::fl_args!("path" => path))
}

pub(in crate::app) fn settings_path_cleared_message(i18n: &I18n, path: &str) -> String {
    i18n.text_args("flash-settings-cleared", &crate::fl_args!("path" => path))
}

pub(in crate::app) fn agent_read_only_edit_message(i18n: &I18n) -> String {
    ui_text::t(i18n, "flash-agent-read-only-edit")
}

pub(in crate::app) fn agent_read_only_permissions_message(i18n: &I18n) -> String {
    ui_text::t(i18n, "flash-agent-read-only-permissions")
}

pub(in crate::app) fn provider_studio_no_auth_details_message(i18n: &I18n) -> String {
    ui_text::t(i18n, "flash-provider-studio-no-auth-details")
}

pub(in crate::app) fn provider_draft_auth_action_message(
    i18n: &I18n,
    message: &crate::backend::ProviderDraftAuthMessage,
) -> String {
    match message {
        crate::backend::ProviderDraftAuthMessage::OpenaiBrowserStarted => {
            ui_text::t(i18n, "flash-provider-auth-openai-browser-started")
        }
        crate::backend::ProviderDraftAuthMessage::OpenaiDeviceStarted { user_code } => i18n
            .text_args(
                "flash-provider-auth-openai-device-started",
                &crate::fl_args!("code" => user_code.clone()),
            ),
        crate::backend::ProviderDraftAuthMessage::CopilotDeviceStarted { user_code } => i18n
            .text_args(
                "flash-provider-auth-copilot-device-started",
                &crate::fl_args!("code" => user_code.clone()),
            ),
        crate::backend::ProviderDraftAuthMessage::GitlabBrowserStarted => {
            ui_text::t(i18n, "flash-provider-auth-gitlab-browser-started")
        }
        crate::backend::ProviderDraftAuthMessage::OpenaiCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-openai-captured")
        }
        crate::backend::ProviderDraftAuthMessage::OpenaiPending => {
            ui_text::t(i18n, "flash-provider-auth-openai-pending")
        }
        crate::backend::ProviderDraftAuthMessage::CopilotPending => {
            ui_text::t(i18n, "flash-provider-auth-copilot-pending")
        }
        crate::backend::ProviderDraftAuthMessage::CopilotCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-copilot-captured")
        }
        crate::backend::ProviderDraftAuthMessage::GitlabCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-gitlab-captured")
        }
    }
}

pub(in crate::app) fn provider_draft_auth_message_is_pending(
    message: &crate::backend::ProviderDraftAuthMessage,
) -> bool {
    matches!(
        message,
        crate::backend::ProviderDraftAuthMessage::OpenaiPending
            | crate::backend::ProviderDraftAuthMessage::CopilotPending
    )
}

pub(in crate::app) fn provider_draft_auth_error_message(
    i18n: &I18n,
    error: &crate::backend::ProviderDraftAuthError,
) -> String {
    match error {
        crate::backend::ProviderDraftAuthError::UnsupportedInteractiveLogin => {
            ui_text::t(i18n, "flash-provider-auth-error-unsupported")
        }
        crate::backend::ProviderDraftAuthError::StartBrowserAuthFirst => {
            ui_text::t(i18n, "flash-provider-auth-error-start-browser-first")
        }
        crate::backend::ProviderDraftAuthError::StartDeviceAuthFirst => {
            ui_text::t(i18n, "flash-provider-auth-error-start-device-first")
        }
        crate::backend::ProviderDraftAuthError::RequiredField(field) => i18n.text_args(
            "flash-provider-auth-error-required-field",
            &crate::fl_args!("field" => provider_draft_auth_field_label(i18n, field)),
        ),
        crate::backend::ProviderDraftAuthError::Other(error) => error.clone(),
    }
}

pub(in crate::app) fn provider_draft_auth_field_label(
    i18n: &I18n,
    field: &crate::backend::ProviderDraftAuthField,
) -> String {
    ui_text::t(
        i18n,
        match field {
            crate::backend::ProviderDraftAuthField::RedirectUri => "provider-field-redirect-uri",
            crate::backend::ProviderDraftAuthField::InstanceUrl => "provider-field-instance-url",
            crate::backend::ProviderDraftAuthField::CallbackUrl => "provider-field-callback-url",
        },
    )
}

pub(in crate::app) fn provider_studio_save_result_message(
    i18n: &I18n,
    result: &crate::backend::ProviderStudioSaveResult,
) -> String {
    match result {
        crate::backend::ProviderStudioSaveResult::ProviderDraftSaved {
            provider_id,
            default_adapter,
            default_model,
        } => match default_model {
            Some(default_model) => i18n.text_args(
                "flash-provider-save-draft",
                &crate::fl_args!(
                    "provider" => provider_id.clone(),
                    "adapter" => default_adapter.clone(),
                    "model" => default_model.clone(),
                ),
            ),
            None => i18n.text_args(
                "flash-provider-save-draft-no-model",
                &crate::fl_args!(
                    "provider" => provider_id.clone(),
                    "adapter" => default_adapter.clone(),
                ),
            ),
        },
        crate::backend::ProviderStudioSaveResult::AdapterMatchesSaved {
            provider_id,
            adapter_id,
            listed_model_count,
            matched_model_count,
        } => i18n.text_args(
            "flash-provider-save-adapter-matches",
            &crate::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "listed" => *listed_model_count as i64,
                "matched" => *matched_model_count as i64,
            ),
        ),
        crate::backend::ProviderStudioSaveResult::ConfiguredModelSaved {
            provider_id,
            adapter_id,
            model_id,
        } => i18n.text_args(
            "flash-provider-save-configured-model",
            &crate::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "model" => model_id.clone(),
            ),
        ),
        crate::backend::ProviderStudioSaveResult::ProviderDeleted { provider_id } => i18n
            .text_args(
                "flash-provider-delete-provider",
                &crate::fl_args!("provider" => provider_id.clone()),
            ),
        crate::backend::ProviderStudioSaveResult::AdapterDeleted {
            provider_id,
            adapter_id,
            removed_model_count,
        } => i18n.text_args(
            "flash-provider-delete-adapter",
            &crate::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "count" => *removed_model_count as i64,
            ),
        ),
        crate::backend::ProviderStudioSaveResult::ModelDeleted {
            provider_id,
            adapter_id,
            model_id,
        } => i18n.text_args(
            "flash-provider-delete-model",
            &crate::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "model" => model_id.clone(),
            ),
        ),
    }
}

pub(in crate::app) fn provider_studio_save_error_message(
    i18n: &I18n,
    error: &crate::backend::ProviderStudioSaveError,
) -> String {
    match error {
        crate::backend::ProviderStudioSaveError::Validation(error) => {
            provider_studio_save_validation_error_message(i18n, error)
        }
        crate::backend::ProviderStudioSaveError::ExistingProviderSettingsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-settings-object")
        }
        crate::backend::ProviderStudioSaveError::ProviderAdapterMustBeObject { adapter_id } => i18n
            .text_args(
                "flash-provider-save-error-adapter-object",
                &crate::fl_args!("adapter" => adapter_id.clone()),
            ),
        crate::backend::ProviderStudioSaveError::ProviderModelConfigMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-model-object")
        }
        crate::backend::ProviderStudioSaveError::ConfiguredProviderAdapterSettingsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-configured-adapter-object")
        }
        crate::backend::ProviderStudioSaveError::ConfiguredProviderAdapterModelsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-configured-models-object")
        }
        crate::backend::ProviderStudioSaveError::Other(error) => error.clone(),
    }
}

pub(in crate::app) fn provider_studio_save_validation_error_message(
    i18n: &I18n,
    error: &crate::backend::ProviderStudioSaveValidationError,
) -> String {
    match error {
        crate::backend::ProviderStudioSaveValidationError::FieldRequired(field) => i18n.text_args(
            "flash-provider-save-error-required-field",
            &crate::fl_args!("field" => provider_studio_save_field_label(i18n, field)),
        ),
        crate::backend::ProviderStudioSaveValidationError::UnsupportedDefaultAdapter {
            auth_kind,
            adapter,
            supported,
        } => i18n.text_args(
            "flash-provider-save-error-unsupported-default-adapter",
            &crate::fl_args!(
                "auth" => provider_draft_auth_kind_label(i18n, auth_kind),
                "adapter" => adapter.clone(),
                "supported" => supported.clone(),
            ),
        ),
        crate::backend::ProviderStudioSaveValidationError::UnsupportedAdapters {
            auth_kind,
            adapters,
            supported,
        } => i18n.text_args(
            "flash-provider-save-error-unsupported-adapters",
            &crate::fl_args!(
                "auth" => provider_draft_auth_kind_label(i18n, auth_kind),
                "adapters" => adapters.join(", "),
                "supported" => supported.clone(),
            ),
        ),
        crate::backend::ProviderStudioSaveValidationError::ApiBaseUrlRequired => {
            ui_text::t(i18n, "flash-provider-save-error-api-base-url")
        }
        crate::backend::ProviderStudioSaveValidationError::GitlabApiKeyOrEnvRequired => {
            ui_text::t(i18n, "flash-provider-save-error-gitlab-token")
        }
        crate::backend::ProviderStudioSaveValidationError::CredentialBaseUrlRequired { issuer } => {
            i18n.text_args(
                "flash-provider-save-error-credential-base-url",
                &crate::fl_args!(
                    "issuer" => provider_credential_issuer_label_localized(i18n, *issuer),
                ),
            )
        }
        crate::backend::ProviderStudioSaveValidationError::CredentialServiceKeyEnvRequired {
            issuer,
        } => i18n.text_args(
            "flash-provider-save-error-credential-service-key-env",
            &crate::fl_args!(
                "issuer" => provider_credential_issuer_label_localized(i18n, *issuer),
            ),
        ),
        crate::backend::ProviderStudioSaveValidationError::BedrockKeyPairRequired => {
            ui_text::t(i18n, "flash-provider-save-error-bedrock-key-pair")
        }
    }
}

pub(in crate::app) fn provider_studio_save_field_label(
    i18n: &I18n,
    field: &crate::backend::ProviderStudioSaveField,
) -> String {
    ui_text::t(
        i18n,
        match field {
            crate::backend::ProviderStudioSaveField::ProviderId => "provider-field-provider-id",
            crate::backend::ProviderStudioSaveField::DefaultAdapter => {
                "provider-field-default-adapter"
            }
            crate::backend::ProviderStudioSaveField::AdapterId => "provider-field-adapter-id",
            crate::backend::ProviderStudioSaveField::ModelId => "provider-field-model-id",
            crate::backend::ProviderStudioSaveField::AuthMode => "provider-field-auth-mode",
            crate::backend::ProviderStudioSaveField::AuthSubtype => "provider-field-auth-subtype",
            crate::backend::ProviderStudioSaveField::CredentialIssuer => {
                "provider-field-auth-subtype"
            }
        },
    )
}

pub(in crate::app) fn provider_credential_issuer_label_localized(
    i18n: &I18n,
    issuer: CredentialIssuer,
) -> String {
    ui_text::t(
        i18n,
        match issuer {
            CredentialIssuer::OpenaiChatgpt => "provider-issuer-openai-chatgpt-label",
            CredentialIssuer::GithubCopilot => "provider-issuer-github-copilot-label",
            CredentialIssuer::Gitlab => "provider-issuer-gitlab-label",
            CredentialIssuer::GoogleAdc => "provider-issuer-google-adc-label",
            CredentialIssuer::SapAiCore => "provider-issuer-sap-ai-core-label",
        },
    )
}

pub(in crate::app) fn provider_draft_auth_kind_label(
    i18n: &I18n,
    auth_kind: &ProviderDraftAuthKind,
) -> String {
    match auth_kind {
        ProviderDraftAuthKind::Unset => ui_text::t(i18n, "provider-auth-kind-unset"),
        ProviderDraftAuthKind::None => ui_text::t(i18n, "provider-auth-kind-none"),
        ProviderDraftAuthKind::ApiPending => ui_text::t(i18n, "provider-auth-kind-api"),
        ProviderDraftAuthKind::Api => ui_text::t(i18n, "provider-auth-kind-api"),
        ProviderDraftAuthKind::ClineApi => ui_text::t(i18n, "provider-auth-kind-cline"),
        ProviderDraftAuthKind::Gitlab => ui_text::t(i18n, "provider-auth-kind-gitlab"),
        ProviderDraftAuthKind::Credential(Some(issuer)) => i18n.text_args(
            "provider-auth-kind-credential-with-issuer",
            &crate::fl_args!(
                "issuer" => provider_credential_issuer_label_localized(i18n, *issuer)
            ),
        ),
        ProviderDraftAuthKind::Credential(None) => {
            ui_text::t(i18n, "provider-auth-kind-credential")
        }
        ProviderDraftAuthKind::BedrockSigv4 => ui_text::t(i18n, "provider-auth-kind-bedrock"),
    }
}

pub(in crate::app) fn provider_draft_auth_mode_label(
    i18n: &I18n,
    auth_kind: &ProviderDraftAuthKind,
) -> String {
    match auth_kind {
        ProviderDraftAuthKind::Unset => ui_text::t(i18n, "provider-auth-kind-unset"),
        ProviderDraftAuthKind::None => ui_text::t(i18n, "provider-auth-kind-none"),
        ProviderDraftAuthKind::ApiPending
        | ProviderDraftAuthKind::Api
        | ProviderDraftAuthKind::ClineApi
        | ProviderDraftAuthKind::Gitlab
        | ProviderDraftAuthKind::BedrockSigv4 => ui_text::t(i18n, "provider-auth-kind-api"),
        ProviderDraftAuthKind::Credential(_) => ui_text::t(i18n, "provider-auth-kind-credential"),
    }
}

pub(in crate::app) fn provider_draft_auth_subtype_label(
    i18n: &I18n,
    auth_kind: &ProviderDraftAuthKind,
) -> String {
    match auth_kind {
        ProviderDraftAuthKind::Unset
        | ProviderDraftAuthKind::None
        | ProviderDraftAuthKind::ApiPending
        | ProviderDraftAuthKind::Credential(None) => String::new(),
        ProviderDraftAuthKind::Api => ui_text::t(i18n, "provider-auth-subtype-custom-label"),
        ProviderDraftAuthKind::ClineApi => ui_text::t(i18n, "provider-auth-kind-cline"),
        ProviderDraftAuthKind::Gitlab => ui_text::t(i18n, "provider-auth-kind-gitlab"),
        ProviderDraftAuthKind::Credential(Some(issuer)) => {
            provider_credential_issuer_label_localized(i18n, *issuer)
        }
        ProviderDraftAuthKind::BedrockSigv4 => ui_text::t(i18n, "provider-auth-kind-bedrock"),
    }
}

pub(in crate::app) fn provider_studio_adapter_rule_detail(
    i18n: &I18n,
    rule: &ProviderDraftAdapterRule,
) -> String {
    ui_text::t(i18n, rule.detail_key)
}

pub(in crate::app) fn provider_studio_model_count_label(i18n: &I18n, count: usize) -> String {
    i18n.text_args(
        "overlay-provider-studio-model-count",
        &crate::fl_args!("count" => count as i64),
    )
}

pub(in crate::app) fn provider_studio_catalog_match_label(
    i18n: &I18n,
    model_id: Option<&str>,
) -> String {
    model_id
        .map(|model| {
            i18n.text_args(
                "overlay-provider-studio-catalog-match",
                &crate::fl_args!("model" => model.to_string()),
            )
        })
        .unwrap_or_else(|| ui_text::t(i18n, "overlay-provider-studio-catalog-unmatched"))
}

pub(in crate::app) fn provider_studio_model_list_detail(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
    model_id: &str,
) -> String {
    let key = provider_studio_model_key(adapter_id, model_id);
    let mut parts = vec![provider_studio_catalog_match_label(
        i18n,
        dialog
            .catalog_matches
            .get(key.as_str())
            .map(|entry| entry.model_id.as_str()),
    )];
    if dialog.draft.default_adapter == adapter_id && dialog.draft.default_model == model_id {
        parts.push(ui_text::t(i18n, "overlay-provider-studio-default"));
    }
    join_inline_segments(parts)
}

pub(in crate::app) fn provider_studio_adapter_list_detail(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
) -> String {
    if let Some(adapter) = dialog
        .adapter_models
        .iter()
        .find(|adapter_models| adapter_models.adapter_id == adapter_id)
    {
        if adapter.error.is_none() {
            return join_inline_segments(vec![
                provider_studio_model_count_label(i18n, adapter.models.len()),
                adapter
                    .resolved_base_url
                    .clone()
                    .unwrap_or_else(|| ui_text::t(i18n, "overlay-provider-studio-loaded")),
            ]);
        }
        return adapter
            .error
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| ui_text::t(i18n, "overlay-provider-studio-error"));
    }
    if let Some(rule) = provider_studio_adapter_rule(dialog, adapter_id) {
        let mut parts = vec![provider_studio_adapter_rule_detail(i18n, rule)];
        if rule.supports_draft_model_listing {
            parts.push(ui_text::t(i18n, "overlay-provider-studio-live-list"));
        }
        if dialog.configured_adapter_ids.contains(adapter_id) {
            parts.push(ui_text::t(i18n, "overlay-provider-studio-configured"));
        }
        return join_inline_segments(parts);
    }
    if dialog.configured_adapter_ids.contains(adapter_id) {
        ui_text::t(i18n, "overlay-provider-studio-configured-disk")
    } else {
        ui_text::t(i18n, "overlay-provider-studio-not-listed")
    }
}

pub(in crate::app) fn provider_studio_live_listing_unavailable_message(
    i18n: &I18n,
    auth_kind: &ProviderDraftAuthKind,
) -> String {
    i18n.text_args(
        "flash-provider-studio-live-listing-unavailable",
        &crate::fl_args!("auth" => provider_draft_auth_kind_label(i18n, auth_kind)),
    )
}

pub(in crate::app) fn provider_studio_draft_listing_unsupported_message(
    i18n: &I18n,
    unsupported: &[String],
) -> String {
    i18n.text_args(
        "flash-provider-studio-draft-listing-unsupported",
        &crate::fl_args!("adapters" => unsupported.join(", ")),
    )
}

pub(in crate::app) fn provider_studio_listing_auth_required_message(
    i18n: &I18n,
    auth_kind: &ProviderDraftAuthKind,
) -> String {
    i18n.text_args(
        "flash-provider-studio-listing-auth-required",
        &crate::fl_args!("auth" => provider_draft_auth_kind_label(i18n, auth_kind)),
    )
}
use crate::app::{
    CredentialIssuer, I18n, ProviderDraftAdapterRule, ProviderDraftAuthKind, ProviderStudioOverlay,
    join_inline_segments, provider_studio_adapter_rule, provider_studio_model_key, ui_text,
};
