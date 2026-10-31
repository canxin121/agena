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
    message: &agena_provider::draft::ProviderDraftAuthMessage,
) -> String {
    match message {
        agena_provider::draft::ProviderDraftAuthMessage::OpenaiBrowserStarted => {
            ui_text::t(i18n, "flash-provider-auth-openai-browser-started")
        }
        agena_provider::draft::ProviderDraftAuthMessage::OpenaiDeviceStarted { user_code } => i18n
            .text_args(
                "flash-provider-auth-openai-device-started",
                &agena_tui::fl_args!("code" => user_code.clone()),
            ),
        agena_provider::draft::ProviderDraftAuthMessage::CopilotDeviceStarted { user_code } => i18n
            .text_args(
                "flash-provider-auth-copilot-device-started",
                &agena_tui::fl_args!("code" => user_code.clone()),
            ),
        agena_provider::draft::ProviderDraftAuthMessage::GitlabBrowserStarted => {
            ui_text::t(i18n, "flash-provider-auth-gitlab-browser-started")
        }
        agena_provider::draft::ProviderDraftAuthMessage::OpenaiCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-openai-captured")
        }
        agena_provider::draft::ProviderDraftAuthMessage::OpenaiPending => {
            ui_text::t(i18n, "flash-provider-auth-openai-pending")
        }
        agena_provider::draft::ProviderDraftAuthMessage::CopilotPending => {
            ui_text::t(i18n, "flash-provider-auth-copilot-pending")
        }
        agena_provider::draft::ProviderDraftAuthMessage::CopilotCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-copilot-captured")
        }
        agena_provider::draft::ProviderDraftAuthMessage::GitlabCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-gitlab-captured")
        }
    }
}

pub(crate) fn provider_draft_auth_message_is_pending(
    message: &agena_provider::draft::ProviderDraftAuthMessage,
) -> bool {
    matches!(
        message,
        agena_provider::draft::ProviderDraftAuthMessage::OpenaiPending
            | agena_provider::draft::ProviderDraftAuthMessage::CopilotPending
    )
}

pub(crate) fn provider_draft_auth_error_message(
    i18n: &I18n,
    error: &agena_provider::draft::ProviderDraftAuthError,
) -> String {
    match error {
        agena_provider::draft::ProviderDraftAuthError::UnsupportedInteractiveLogin => {
            ui_text::t(i18n, "flash-provider-auth-error-unsupported")
        }
        agena_provider::draft::ProviderDraftAuthError::StartBrowserAuthFirst => {
            ui_text::t(i18n, "flash-provider-auth-error-start-browser-first")
        }
        agena_provider::draft::ProviderDraftAuthError::StartDeviceAuthFirst => {
            ui_text::t(i18n, "flash-provider-auth-error-start-device-first")
        }
        agena_provider::draft::ProviderDraftAuthError::RequiredField(field) => i18n.text_args(
            "flash-provider-auth-error-required-field",
            &agena_tui::fl_args!("field" => provider_draft_auth_field_label(i18n, field)),
        ),
        agena_provider::draft::ProviderDraftAuthError::Other(problem) => {
            problem.user.fallback.clone()
        }
    }
}

pub(crate) fn provider_draft_auth_field_label(
    i18n: &I18n,
    field: &agena_provider::draft::ProviderDraftAuthField,
) -> String {
    ui_text::t(
        i18n,
        match field {
            agena_provider::draft::ProviderDraftAuthField::RedirectUri => {
                "provider-field-redirect-uri"
            }
            agena_provider::draft::ProviderDraftAuthField::InstanceUrl => {
                "provider-field-instance-url"
            }
            agena_provider::draft::ProviderDraftAuthField::CallbackUrl => {
                "provider-field-callback-url"
            }
        },
    )
}

pub(crate) fn provider_studio_save_result_message(
    i18n: &I18n,
    result: &agena_provider::draft::ProviderStudioSaveResult,
) -> String {
    match result {
        agena_provider::draft::ProviderStudioSaveResult::ProviderDraftSaved { provider_id } => i18n
            .text_args(
                "flash-provider-save-draft",
                &agena_tui::fl_args!(
                    "provider" => provider_id.clone(),
                ),
            ),
        agena_provider::draft::ProviderStudioSaveResult::AdapterMatchesSaved {
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
        agena_provider::draft::ProviderStudioSaveResult::ConfiguredModelSaved {
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
        agena_provider::draft::ProviderStudioSaveResult::ProviderDeleted { provider_id } => i18n
            .text_args(
                "flash-provider-delete-provider",
                &agena_tui::fl_args!("provider" => provider_id.clone()),
            ),
        agena_provider::draft::ProviderStudioSaveResult::AdapterDeleted {
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
        agena_provider::draft::ProviderStudioSaveResult::ModelDeleted {
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
    error: &agena_provider::draft::ProviderStudioSaveError,
) -> String {
    match error {
        agena_provider::draft::ProviderStudioSaveError::Validation(error) => {
            provider_studio_save_validation_error_message(i18n, error)
        }
        agena_provider::draft::ProviderStudioSaveError::ExistingProviderSettingsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-settings-object")
        }
        agena_provider::draft::ProviderStudioSaveError::ProviderAdapterMustBeObject { adapter_id } => i18n
            .text_args(
                "flash-provider-save-error-adapter-object",
                &agena_tui::fl_args!("adapter" => adapter_id.clone()),
            ),
        agena_provider::draft::ProviderStudioSaveError::ProviderModelConfigMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-model-object")
        }
        agena_provider::draft::ProviderStudioSaveError::ConfiguredProviderAdapterSettingsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-configured-adapter-object")
        }
        agena_provider::draft::ProviderStudioSaveError::ConfiguredProviderAdapterModelsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-configured-models-object")
        }
        agena_provider::draft::ProviderStudioSaveError::Other(problem) => {
            problem.user.fallback.clone()
        }
    }
}

pub(crate) fn provider_studio_save_validation_error_message(
    i18n: &I18n,
    error: &agena_provider::draft::ProviderStudioSaveValidationError,
) -> String {
    match error {
        agena_provider::draft::ProviderStudioSaveValidationError::FieldRequired(field) => i18n
            .text_args(
                "flash-provider-save-error-required-field",
                &agena_tui::fl_args!("field" => provider_studio_save_field_label(i18n, field)),
            ),
        agena_provider::draft::ProviderStudioSaveValidationError::UnsupportedAdapters {
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
        agena_provider::draft::ProviderStudioSaveValidationError::ApiBaseUrlRequired => {
            ui_text::t(i18n, "flash-provider-save-error-api-base-url")
        }
        agena_provider::draft::ProviderStudioSaveValidationError::GitlabApiKeyOrEnvRequired => {
            ui_text::t(i18n, "flash-provider-save-error-gitlab-token")
        }
        agena_provider::draft::ProviderStudioSaveValidationError::CredentialBaseUrlRequired {
            issuer,
        } => i18n.text_args(
            "flash-provider-save-error-credential-base-url",
            &agena_tui::fl_args!(
                "issuer" => provider_credential_issuer_label_localized(i18n, *issuer),
            ),
        ),
        agena_provider::draft::ProviderStudioSaveValidationError::CredentialServiceKeyEnvRequired {
            issuer,
        } => i18n.text_args(
            "flash-provider-save-error-credential-service-key-env",
            &agena_tui::fl_args!(
                "issuer" => provider_credential_issuer_label_localized(i18n, *issuer),
            ),
        ),
        agena_provider::draft::ProviderStudioSaveValidationError::BedrockKeyPairRequired => {
            ui_text::t(i18n, "flash-provider-save-error-bedrock-key-pair")
        }
    }
}

pub(crate) fn provider_studio_save_field_label(
    i18n: &I18n,
    field: &agena_provider::draft::ProviderStudioSaveField,
) -> String {
    ui_text::t(
        i18n,
        match field {
            agena_provider::draft::ProviderStudioSaveField::ProviderId => {
                "provider-field-provider-id"
            }
            agena_provider::draft::ProviderStudioSaveField::AdapterId => {
                "provider-field-adapter-id"
            }
            agena_provider::draft::ProviderStudioSaveField::ModelId => "provider-field-model-id",
            agena_provider::draft::ProviderStudioSaveField::AuthMode => "provider-field-auth-mode",
            agena_provider::draft::ProviderStudioSaveField::AuthSubtype => {
                "provider-field-auth-subtype"
            }
            agena_provider::draft::ProviderStudioSaveField::CredentialIssuer => {
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

/// Detail line for a Provider Studio model list item, rendered from the
/// catalog-enriched `ProviderModelResource` (the listing is enriched against
/// the model catalog at the application chokepoint). Shows the catalog
/// display name, lifecycle, context window, max output tokens, and pricing
/// when the catalog advertises them.
pub(crate) fn provider_studio_model_list_detail(
    i18n: &I18n,
    model: &ProviderModelResource,
) -> String {
    let mut parts = Vec::new();
    if let Some(display_name) = model
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(display_name.to_owned());
    }
    // Active is the default state; only surface non-default lifecycle stages.
    if let Some(lifecycle) = model
        .metadata
        .lifecycle
        .filter(|lifecycle| !matches!(lifecycle, agena_api::resource::ModelLifecycle::Active))
    {
        parts.push(provider_studio_model_lifecycle_label(i18n, lifecycle));
    }
    if let Some(context_window) = model.metadata.context_window_tokens.map(|value| {
        i18n.text_args(
            "session-model-context-window",
            &agena_tui::fl_args!(
                "value" => crate::app_choice_helpers::format_compact_token_count(value as u64)
            ),
        )
    }) {
        parts.push(context_window);
    }
    if let Some(max_output) = model.metadata.max_output_tokens.map(|value| {
        i18n.text_args(
            "session-model-max-output",
            &agena_tui::fl_args!(
                "value" => crate::app_choice_helpers::format_compact_token_count(value as u64)
            ),
        )
    }) {
        parts.push(max_output);
    }
    let pricing = provider_studio_model_pricing_summary(i18n, model.metadata.pricing.as_ref());
    if !pricing.is_empty() {
        parts.push(pricing);
    }
    join_inline_segments(parts)
}

/// Localized lifecycle label for a model resource.
fn provider_studio_model_lifecycle_label(
    i18n: &I18n,
    lifecycle: agena_api::resource::ModelLifecycle,
) -> String {
    let key = match lifecycle {
        agena_api::resource::ModelLifecycle::Active => "overlay-model-catalog-lifecycle-active",
        agena_api::resource::ModelLifecycle::Preview => "overlay-model-catalog-lifecycle-preview",
        agena_api::resource::ModelLifecycle::Beta => "overlay-model-catalog-lifecycle-beta",
        agena_api::resource::ModelLifecycle::Alpha => "overlay-model-catalog-lifecycle-alpha",
        agena_api::resource::ModelLifecycle::Experimental => {
            "overlay-model-catalog-lifecycle-experimental"
        }
        agena_api::resource::ModelLifecycle::Deprecated => {
            "overlay-model-catalog-lifecycle-deprecated"
        }
    };
    ui_text::t(i18n, key)
}

/// Compact input/output pricing summary for a model resource, e.g.
/// `in $1.25/M · out $10/M`. Empty when no prices are advertised.
fn provider_studio_model_pricing_summary(
    i18n: &I18n,
    pricing: Option<&agena_api::resource::ModelPricing>,
) -> String {
    let Some(pricing) = pricing.filter(|pricing| !pricing.is_empty()) else {
        return String::new();
    };
    let mut parts = Vec::new();
    if let Some(value) = pricing.input_usd_per_million_tokens.as_deref() {
        parts.push(i18n.text_args(
            "overlay-model-catalog-price-input",
            &agena_tui::fl_args!("value" => value),
        ));
    }
    if let Some(value) = pricing.output_usd_per_million_tokens.as_deref() {
        parts.push(i18n.text_args(
            "overlay-model-catalog-price-output",
            &agena_tui::fl_args!("value" => value),
        ));
    }
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
    CredentialIssuer, I18n, ProviderDraftAdapterRule, ProviderDraftAuthKind, ProviderModelResource,
    ProviderStudioOverlay, join_inline_segments, provider_studio_adapter_rule, ui_text,
};
