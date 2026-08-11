pub(crate) fn settings_edit_title(i18n: &I18n, field: &str) -> String {
    i18n.text_args(
        "overlay-settings-edit-title",
        &agena_tui::fl_args!("field" => field),
    )
}

pub(crate) fn editor_save_footer(i18n: &I18n, multiline: bool) -> String {
    ui_text::t(
        i18n,
        if multiline {
            "overlay-editor-footer-multiline"
        } else {
            "overlay-editor-footer-single-line"
        },
    )
}

pub(crate) fn settings_clear_label(i18n: &I18n) -> String {
    ui_text::t(i18n, "overlay-choice-clear-value")
}

pub(crate) fn settings_path_updated_message(i18n: &I18n, path: &str) -> String {
    i18n.text_args(
        "flash-settings-updated",
        &agena_tui::fl_args!("path" => path),
    )
}

pub(crate) fn settings_path_cleared_message(i18n: &I18n, path: &str) -> String {
    i18n.text_args(
        "flash-settings-cleared",
        &agena_tui::fl_args!("path" => path),
    )
}

pub(crate) fn provider_studio_no_auth_details_message(i18n: &I18n) -> String {
    ui_text::t(i18n, "flash-provider-studio-no-auth-details")
}

pub(crate) fn provider_draft_auth_action_message(
    i18n: &I18n,
    message: &agena_application::provider_studio::ProviderDraftAuthMessage,
) -> String {
    match message {
        agena_application::provider_studio::ProviderDraftAuthMessage::OpenaiBrowserStarted => {
            ui_text::t(i18n, "flash-provider-auth-openai-browser-started")
        }
        agena_application::provider_studio::ProviderDraftAuthMessage::OpenaiDeviceStarted { user_code } => i18n
            .text_args(
                "flash-provider-auth-openai-device-started",
                &agena_tui::fl_args!("code" => user_code.clone()),
            ),
        agena_application::provider_studio::ProviderDraftAuthMessage::CopilotDeviceStarted { user_code } => i18n
            .text_args(
                "flash-provider-auth-copilot-device-started",
                &agena_tui::fl_args!("code" => user_code.clone()),
            ),
        agena_application::provider_studio::ProviderDraftAuthMessage::GitlabBrowserStarted => {
            ui_text::t(i18n, "flash-provider-auth-gitlab-browser-started")
        }
        agena_application::provider_studio::ProviderDraftAuthMessage::OpenaiCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-openai-captured")
        }
        agena_application::provider_studio::ProviderDraftAuthMessage::OpenaiPending => {
            ui_text::t(i18n, "flash-provider-auth-openai-pending")
        }
        agena_application::provider_studio::ProviderDraftAuthMessage::CopilotPending => {
            ui_text::t(i18n, "flash-provider-auth-copilot-pending")
        }
        agena_application::provider_studio::ProviderDraftAuthMessage::CopilotCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-copilot-captured")
        }
        agena_application::provider_studio::ProviderDraftAuthMessage::GitlabCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-gitlab-captured")
        }
    }
}

pub(crate) fn provider_draft_auth_message_is_pending(
    message: &agena_application::provider_studio::ProviderDraftAuthMessage,
) -> bool {
    matches!(
        message,
        agena_application::provider_studio::ProviderDraftAuthMessage::OpenaiPending
            | agena_application::provider_studio::ProviderDraftAuthMessage::CopilotPending
    )
}

pub(crate) fn provider_draft_auth_error_message(
    i18n: &I18n,
    error: &agena_application::provider_studio::ProviderDraftAuthError,
) -> String {
    match error {
        agena_application::provider_studio::ProviderDraftAuthError::UnsupportedInteractiveLogin => {
            ui_text::t(i18n, "flash-provider-auth-error-unsupported")
        }
        agena_application::provider_studio::ProviderDraftAuthError::StartBrowserAuthFirst => {
            ui_text::t(i18n, "flash-provider-auth-error-start-browser-first")
        }
        agena_application::provider_studio::ProviderDraftAuthError::StartDeviceAuthFirst => {
            ui_text::t(i18n, "flash-provider-auth-error-start-device-first")
        }
        agena_application::provider_studio::ProviderDraftAuthError::RequiredField(field) => i18n.text_args(
            "flash-provider-auth-error-required-field",
            &agena_tui::fl_args!("field" => provider_draft_auth_field_label(i18n, field)),
        ),
        agena_application::provider_studio::ProviderDraftAuthError::Other(problem) => problem.user.fallback.clone(),
    }
}

pub(crate) fn provider_draft_auth_field_label(
    i18n: &I18n,
    field: &agena_application::provider_studio::ProviderDraftAuthField,
) -> String {
    ui_text::t(
        i18n,
        match field {
            agena_application::provider_studio::ProviderDraftAuthField::RedirectUri => "provider-field-redirect-uri",
            agena_application::provider_studio::ProviderDraftAuthField::InstanceUrl => "provider-field-instance-url",
            agena_application::provider_studio::ProviderDraftAuthField::CallbackUrl => "provider-field-callback-url",
        },
    )
}

pub(crate) fn provider_studio_save_result_message(
    i18n: &I18n,
    result: &agena_application::provider_studio::ProviderStudioSaveResult,
) -> String {
    match result {
        agena_application::provider_studio::ProviderStudioSaveResult::ProviderDraftSaved {
            provider_id,
            default_adapter,
            ..
        } => i18n.text_args(
            "flash-provider-save-draft",
            &agena_tui::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => default_adapter.clone(),
            ),
        ),
        agena_application::provider_studio::ProviderStudioSaveResult::AdapterMatchesSaved {
            provider_id,
            adapter_id,
            listed_model_count,
            matched_model_count,
        } => i18n.text_args(
            "flash-provider-save-adapter-matches",
            &agena_tui::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "listed" => *listed_model_count as i64,
                "matched" => *matched_model_count as i64,
            ),
        ),
        agena_application::provider_studio::ProviderStudioSaveResult::ConfiguredModelSaved {
            provider_id,
            adapter_id,
            model_id,
        } => i18n.text_args(
            "flash-provider-save-configured-model",
            &agena_tui::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "model" => model_id.clone(),
            ),
        ),
        agena_application::provider_studio::ProviderStudioSaveResult::ProviderDeleted { provider_id } => i18n
            .text_args(
                "flash-provider-delete-provider",
                &agena_tui::fl_args!("provider" => provider_id.clone()),
            ),
        agena_application::provider_studio::ProviderStudioSaveResult::AdapterDeleted {
            provider_id,
            adapter_id,
            removed_model_count,
        } => i18n.text_args(
            "flash-provider-delete-adapter",
            &agena_tui::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "count" => *removed_model_count as i64,
            ),
        ),
        agena_application::provider_studio::ProviderStudioSaveResult::ModelDeleted {
            provider_id,
            adapter_id,
            model_id,
        } => i18n.text_args(
            "flash-provider-delete-model",
            &agena_tui::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "model" => model_id.clone(),
            ),
        ),
    }
}

pub(crate) fn provider_studio_save_error_message(
    i18n: &I18n,
    error: &agena_application::provider_studio::ProviderStudioSaveError,
) -> String {
    match error {
        agena_application::provider_studio::ProviderStudioSaveError::Validation(error) => {
            provider_studio_save_validation_error_message(i18n, error)
        }
        agena_application::provider_studio::ProviderStudioSaveError::ExistingProviderSettingsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-settings-object")
        }
        agena_application::provider_studio::ProviderStudioSaveError::ProviderAdapterMustBeObject { adapter_id } => i18n
            .text_args(
                "flash-provider-save-error-adapter-object",
                &agena_tui::fl_args!("adapter" => adapter_id.clone()),
            ),
        agena_application::provider_studio::ProviderStudioSaveError::ProviderModelConfigMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-model-object")
        }
        agena_application::provider_studio::ProviderStudioSaveError::ConfiguredProviderAdapterSettingsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-configured-adapter-object")
        }
        agena_application::provider_studio::ProviderStudioSaveError::ConfiguredProviderAdapterModelsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-configured-models-object")
        }
        agena_application::provider_studio::ProviderStudioSaveError::Other(problem) => {
            problem.user.fallback.clone()
        }
    }
}

pub(crate) fn provider_studio_save_validation_error_message(
    i18n: &I18n,
    error: &agena_application::provider_studio::ProviderStudioSaveValidationError,
) -> String {
    match error {
        agena_application::provider_studio::ProviderStudioSaveValidationError::FieldRequired(field) => i18n
            .text_args(
                "flash-provider-save-error-required-field",
                &agena_tui::fl_args!("field" => provider_studio_save_field_label(i18n, field)),
            ),
        agena_application::provider_studio::ProviderStudioSaveValidationError::UnsupportedDefaultAdapter {
            auth_kind,
            adapter,
            supported,
        } => i18n.text_args(
            "flash-provider-save-error-unsupported-default-adapter",
            &agena_tui::fl_args!(
                "auth" => provider_draft_auth_kind_label(i18n, auth_kind),
                "adapter" => adapter.clone(),
                "supported" => supported.clone(),
            ),
        ),
        agena_application::provider_studio::ProviderStudioSaveValidationError::UnsupportedAdapters {
            auth_kind,
            adapters,
            supported,
        } => i18n.text_args(
            "flash-provider-save-error-unsupported-adapters",
            &agena_tui::fl_args!(
                "auth" => provider_draft_auth_kind_label(i18n, auth_kind),
                "adapters" => adapters.join(", "),
                "supported" => supported.clone(),
            ),
        ),
        agena_application::provider_studio::ProviderStudioSaveValidationError::ApiBaseUrlRequired => {
            ui_text::t(i18n, "flash-provider-save-error-api-base-url")
        }
        agena_application::provider_studio::ProviderStudioSaveValidationError::GitlabApiKeyOrEnvRequired => {
            ui_text::t(i18n, "flash-provider-save-error-gitlab-token")
        }
        agena_application::provider_studio::ProviderStudioSaveValidationError::CredentialBaseUrlRequired {
            issuer,
        } => i18n.text_args(
            "flash-provider-save-error-credential-base-url",
            &agena_tui::fl_args!(
                "issuer" => provider_credential_issuer_label_localized(i18n, *issuer),
            ),
        ),
        agena_application::provider_studio::ProviderStudioSaveValidationError::CredentialServiceKeyEnvRequired {
            issuer,
        } => i18n.text_args(
            "flash-provider-save-error-credential-service-key-env",
            &agena_tui::fl_args!(
                "issuer" => provider_credential_issuer_label_localized(i18n, *issuer),
            ),
        ),
        agena_application::provider_studio::ProviderStudioSaveValidationError::BedrockKeyPairRequired => {
            ui_text::t(i18n, "flash-provider-save-error-bedrock-key-pair")
        }
    }
}

pub(crate) fn provider_studio_save_field_label(
    i18n: &I18n,
    field: &agena_application::provider_studio::ProviderStudioSaveField,
) -> String {
    ui_text::t(
        i18n,
        match field {
            agena_application::provider_studio::ProviderStudioSaveField::ProviderId => "provider-field-provider-id",
            agena_application::provider_studio::ProviderStudioSaveField::DefaultAdapter => {
                "provider-field-default-adapter"
            }
            agena_application::provider_studio::ProviderStudioSaveField::AdapterId => "provider-field-adapter-id",
            agena_application::provider_studio::ProviderStudioSaveField::ModelId => "provider-field-model-id",
            agena_application::provider_studio::ProviderStudioSaveField::AuthMode => "provider-field-auth-mode",
            agena_application::provider_studio::ProviderStudioSaveField::AuthSubtype => {
                "provider-field-auth-subtype"
            }
            agena_application::provider_studio::ProviderStudioSaveField::CredentialIssuer => {
                "provider-field-auth-subtype"
            }
        },
    )
}

pub(crate) fn provider_credential_issuer_label_localized(
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

pub(crate) fn provider_draft_auth_kind_label(
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
            &agena_tui::fl_args!(
                "issuer" => provider_credential_issuer_label_localized(i18n, *issuer)
            ),
        ),
        ProviderDraftAuthKind::Credential(None) => {
            ui_text::t(i18n, "provider-auth-kind-credential")
        }
        ProviderDraftAuthKind::BedrockSigv4 => ui_text::t(i18n, "provider-auth-kind-bedrock"),
    }
}

pub(crate) fn provider_draft_auth_mode_label(
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

pub(crate) fn provider_draft_auth_subtype_label(
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

pub(crate) fn provider_studio_adapter_rule_detail(
    i18n: &I18n,
    rule: &ProviderDraftAdapterRule,
) -> String {
    ui_text::t(i18n, rule.detail_key)
}

pub(crate) fn provider_studio_model_count_label(i18n: &I18n, count: usize) -> String {
    i18n.text_args(
        "overlay-provider-studio-model-count",
        &agena_tui::fl_args!("count" => count as i64),
    )
}

pub(crate) fn provider_studio_catalog_match_label(i18n: &I18n, model_id: Option<&str>) -> String {
    model_id
        .map(|model| {
            i18n.text_args(
                "overlay-provider-studio-catalog-match",
                &agena_tui::fl_args!("model" => model.to_string()),
            )
        })
        .unwrap_or_else(|| ui_text::t(i18n, "overlay-provider-studio-catalog-unmatched"))
}

pub(crate) fn provider_studio_model_list_detail(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
    model_id: &str,
) -> String {
    let key = provider_studio_model_key(adapter_id, model_id);
    let parts = vec![provider_studio_catalog_match_label(
        i18n,
        dialog
            .catalog_matches
            .get(key.as_str())
            .map(|entry| entry.model_id.as_str()),
    )];
    join_inline_segments(parts)
}

pub(crate) fn provider_studio_adapter_list_detail(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
) -> String {
    if let Some(adapter) = dialog
        .adapter_models
        .iter()
        .find(|adapter_models| adapter_models.adapter_id == adapter_id)
    {
        if adapter.failure.is_none() {
            return join_inline_segments(vec![
                provider_studio_model_count_label(i18n, adapter.models.len()),
                adapter
                    .resolved_base_url
                    .clone()
                    .unwrap_or_else(|| ui_text::t(i18n, "overlay-provider-studio-loaded")),
            ]);
        }
        return adapter.failure.as_ref().map_or_else(
            || ui_text::t(i18n, "overlay-provider-studio-error"),
            |problem| problem.user.fallback.clone(),
        );
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

pub(crate) fn provider_studio_live_listing_unavailable_message(
    i18n: &I18n,
    auth_kind: &ProviderDraftAuthKind,
) -> String {
    i18n.text_args(
        "flash-provider-studio-live-listing-unavailable",
        &agena_tui::fl_args!("auth" => provider_draft_auth_kind_label(i18n, auth_kind)),
    )
}

pub(crate) fn provider_studio_draft_listing_unsupported_message(
    i18n: &I18n,
    unsupported: &[String],
) -> String {
    i18n.text_args(
        "flash-provider-studio-draft-listing-unsupported",
        &agena_tui::fl_args!("adapters" => unsupported.join(", ")),
    )
}

pub(crate) fn provider_studio_listing_auth_required_message(
    i18n: &I18n,
    auth_kind: &ProviderDraftAuthKind,
) -> String {
    i18n.text_args(
        "flash-provider-studio-listing-auth-required",
        &agena_tui::fl_args!("auth" => provider_draft_auth_kind_label(i18n, auth_kind)),
    )
}
use crate::{
    CredentialIssuer, I18n, ProviderDraftAdapterRule, ProviderDraftAuthKind, ProviderStudioOverlay,
    join_inline_segments, provider_studio_adapter_rule, provider_studio_model_key, ui_text,
};
