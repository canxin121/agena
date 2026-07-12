use super::provider_selection::*;

pub(in crate::app) fn provider_studio_field_label_key(field: ProviderStudioField) -> &'static str {
    match field {
        ProviderStudioField::ProviderId => "provider-field-provider-id",
        ProviderStudioField::AuthMode => "provider-field-auth-mode",
        ProviderStudioField::AuthSubtype => "provider-field-auth-subtype",
        ProviderStudioField::AuthLoginMethod => "provider-field-auth-login-method",
        ProviderStudioField::StartAuthAction => "provider-field-start-auth",
        ProviderStudioField::ContinueAuthAction => "provider-field-continue-auth",
        ProviderStudioField::EditAuthDetailsAction => "provider-field-auth-details",
        ProviderStudioField::BaseUrl => "provider-field-base-url",
        ProviderStudioField::InstanceUrl => "provider-field-instance-url",
        ProviderStudioField::ApiKeySource => "provider-field-api-key-source",
        ProviderStudioField::ApiKeyValue => "provider-field-api-key-value",
        ProviderStudioField::RedirectUri => "provider-field-redirect-uri",
        ProviderStudioField::CallbackUrl => "provider-field-callback-url",
        ProviderStudioField::RefreshToken => "provider-field-refresh-token",
        ProviderStudioField::AccessToken => "provider-field-access-token",
        ProviderStudioField::ExpiresAtMs => "provider-field-expires-at-ms",
        ProviderStudioField::AccountId => "provider-field-account-id",
        ProviderStudioField::EnterpriseDomain => "provider-field-enterprise-domain",
        ProviderStudioField::Region => "provider-field-region",
        ProviderStudioField::Profile => "provider-field-profile",
        ProviderStudioField::AccessKeyId => "provider-field-access-key-id",
        ProviderStudioField::SecretAccessKey => "provider-field-secret-access-key",
        ProviderStudioField::SessionToken => "provider-field-session-token",
        ProviderStudioField::ServiceKeyEnv => "provider-field-service-key-env",
        ProviderStudioField::DefaultAdapter => "provider-field-default-adapter",
        ProviderStudioField::DefaultModel => "provider-field-default-model",
    }
}

pub(in crate::app) fn provider_studio_field_label(
    i18n: &I18n,
    field: ProviderStudioField,
) -> String {
    ui_text::t(i18n, provider_studio_field_label_key(field))
}

pub(in crate::app) fn provider_studio_field_prompt(
    i18n: &I18n,
    field: ProviderStudioField,
) -> String {
    match field {
        ProviderStudioField::AuthMode => {
            ui_text::t(i18n, "overlay-provider-studio-edit-auth-mode-prompt")
        }
        ProviderStudioField::AuthSubtype => {
            ui_text::t(i18n, "overlay-provider-studio-edit-auth-subtype-prompt")
        }
        ProviderStudioField::AuthLoginMethod => ui_text::t(
            i18n,
            "overlay-provider-studio-edit-auth-login-method-prompt",
        ),
        ProviderStudioField::StartAuthAction
        | ProviderStudioField::ContinueAuthAction
        | ProviderStudioField::EditAuthDetailsAction => String::new(),
        _ => i18n.text_args(
            "overlay-provider-studio-edit-prompt",
            &crate::fl_args!("field" => provider_studio_field_label(i18n, field)),
        ),
    }
}

pub(in crate::app) fn provider_studio_field_value(
    draft: &ProviderConfigDraft,
    field: ProviderStudioField,
) -> String {
    match field {
        ProviderStudioField::ProviderId => draft.provider_id.clone(),
        ProviderStudioField::AuthMode => draft.auth_kind.mode_label().to_owned(),
        ProviderStudioField::AuthSubtype => draft.auth_kind.subtype_label().to_owned(),
        ProviderStudioField::AuthLoginMethod => draft
            .interactive_login_kind()
            .map(|kind| kind.token().to_owned())
            .unwrap_or_default(),
        ProviderStudioField::StartAuthAction
        | ProviderStudioField::ContinueAuthAction
        | ProviderStudioField::EditAuthDetailsAction => String::new(),
        ProviderStudioField::BaseUrl => draft.auth.base_url.clone(),
        ProviderStudioField::InstanceUrl => draft.auth.instance_url.clone(),
        ProviderStudioField::ApiKeySource => draft.auth.secret_source_kind.token().to_owned(),
        ProviderStudioField::ApiKeyValue => draft.auth.secret_source_value.clone(),
        ProviderStudioField::RedirectUri => draft.redirect_uri().unwrap_or_default().to_owned(),
        ProviderStudioField::CallbackUrl => draft.callback_url().unwrap_or_default().to_owned(),
        ProviderStudioField::RefreshToken => draft
            .active_tokens()
            .map(|tokens| tokens.refresh_token.clone())
            .unwrap_or_default(),
        ProviderStudioField::AccessToken => draft
            .active_tokens()
            .map(|tokens| tokens.access_token.clone())
            .unwrap_or_default(),
        ProviderStudioField::ExpiresAtMs => draft
            .active_tokens()
            .map(|tokens| tokens.expires_at_ms.clone())
            .unwrap_or_default(),
        ProviderStudioField::AccountId => draft.account_id().unwrap_or_default().to_owned(),
        ProviderStudioField::EnterpriseDomain => draft
            .credential_drafts
            .github_copilot
            .enterprise_domain
            .clone(),
        ProviderStudioField::Region => draft.auth.region.clone(),
        ProviderStudioField::Profile => draft.auth.profile.clone(),
        ProviderStudioField::AccessKeyId => draft.auth.access_key_id.clone(),
        ProviderStudioField::SecretAccessKey => draft.auth.secret_access_key.clone(),
        ProviderStudioField::SessionToken => draft.auth.session_token.clone(),
        ProviderStudioField::ServiceKeyEnv => draft.auth.service_key_env.clone(),
        ProviderStudioField::DefaultAdapter => draft.default_adapter.clone(),
        ProviderStudioField::DefaultModel => draft.default_model.clone(),
    }
}

pub(in crate::app) fn provider_studio_field_editable(
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> bool {
    match field {
        ProviderStudioField::ProviderId => dialog.draft.source_provider_id.is_none(),
        ProviderStudioField::AuthMode => true,
        ProviderStudioField::AuthSubtype => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::ApiPending
                | ProviderDraftAuthKind::Api
                | ProviderDraftAuthKind::ClineApi
                | ProviderDraftAuthKind::Gitlab
                | ProviderDraftAuthKind::BedrockSigv4
                | ProviderDraftAuthKind::Credential(_)
        ),
        ProviderStudioField::AuthLoginMethod => {
            provider_studio_available_login_kinds(dialog).len() > 1
        }
        ProviderStudioField::StartAuthAction | ProviderStudioField::ContinueAuthAction => {
            dialog.draft.supports_interactive_auth()
        }
        ProviderStudioField::EditAuthDetailsAction => {
            !provider_studio_detail_fields(dialog).is_empty()
        }
        ProviderStudioField::BaseUrl => match dialog.draft.auth_kind {
            ProviderDraftAuthKind::Unset => false,
            ProviderDraftAuthKind::ApiPending => false,
            ProviderDraftAuthKind::Api | ProviderDraftAuthKind::BedrockSigv4 => {
                provider_studio_base_url_visible(dialog)
            }
            ProviderDraftAuthKind::ClineApi => false,
            ProviderDraftAuthKind::Credential(_) => provider_studio_base_url_visible(dialog),
            ProviderDraftAuthKind::Gitlab | ProviderDraftAuthKind::None => false,
        },
        ProviderStudioField::InstanceUrl => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Gitlab
                | ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab))
        ),
        ProviderStudioField::ApiKeySource | ProviderStudioField::ApiKeyValue => {
            matches!(
                dialog.draft.auth_kind,
                ProviderDraftAuthKind::Api
                    | ProviderDraftAuthKind::ClineApi
                    | ProviderDraftAuthKind::Gitlab
            )
        }
        ProviderStudioField::RedirectUri => {
            matches!(
                dialog.draft.auth_kind,
                ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab))
            ) || matches!(
                dialog.draft.auth_kind,
                ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt))
            ) && provider_studio_auth_login_kind(dialog)
                == Some(ProviderDraftInteractiveLoginKind::Browser)
        }
        ProviderStudioField::CallbackUrl => {
            matches!(
                dialog.draft.auth_kind,
                ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab))
            ) || matches!(
                dialog.draft.auth_kind,
                ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt))
            ) && provider_studio_auth_login_kind(dialog)
                == Some(ProviderDraftInteractiveLoginKind::Browser)
        }
        ProviderStudioField::RefreshToken
        | ProviderStudioField::AccessToken
        | ProviderStudioField::ExpiresAtMs => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(
                CredentialIssuer::OpenaiChatgpt
                    | CredentialIssuer::GithubCopilot
                    | CredentialIssuer::Gitlab
            ))
        ),
        ProviderStudioField::AccountId => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt))
        ),
        ProviderStudioField::EnterpriseDomain => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot))
        ),
        ProviderStudioField::Region
        | ProviderStudioField::Profile
        | ProviderStudioField::AccessKeyId
        | ProviderStudioField::SecretAccessKey
        | ProviderStudioField::SessionToken => {
            matches!(dialog.draft.auth_kind, ProviderDraftAuthKind::BedrockSigv4)
        }
        ProviderStudioField::ServiceKeyEnv => matches!(
            dialog.draft.auth_kind,
            ProviderDraftAuthKind::Credential(Some(issuer)) if issuer.requires_service_key_env()
        ),
        ProviderStudioField::DefaultAdapter | ProviderStudioField::DefaultModel => true,
    }
}

pub(in crate::app) fn provider_studio_model_key(adapter_id: &str, model_id: &str) -> String {
    format!("{adapter_id}\u{1f}{model_id}")
}

pub(in crate::app) fn remove_provider_studio_model_from_dialog(
    dialog: &mut ProviderStudioOverlay,
    adapter_id: &str,
    model_id: &str,
) {
    if let Some(adapter_models) = dialog
        .adapter_models
        .iter_mut()
        .find(|adapter_models| adapter_models.adapter_id == adapter_id)
    {
        adapter_models
            .models
            .retain(|model| model.id.as_ref() != model_id);
    }
    dialog
        .selected_model_keys
        .remove(provider_studio_model_key(adapter_id, model_id).as_str());
    if dialog.draft.default_adapter == adapter_id && dialog.draft.default_model == model_id {
        dialog.draft.default_model.clear();
    }
    dialog.model_page = None;
    dialog.selection.clamp_right(
        provider_studio_selected_adapter_models(dialog)
            .map(|adapter| adapter.models.len())
            .unwrap_or_default(),
    );
    provider_studio_ensure_default_selection(dialog);
}

pub(in crate::app) fn remove_provider_studio_adapter_from_dialog(
    dialog: &mut ProviderStudioOverlay,
    adapter_id: &str,
) {
    dialog
        .adapter_models
        .retain(|adapter_models| adapter_models.adapter_id != adapter_id);
    dialog.configured_adapter_ids.remove(adapter_id);
    dialog.selected_adapter_ids.remove(adapter_id);
    let prefix = format!("{adapter_id}\u{1f}");
    dialog
        .selected_model_keys
        .retain(|key| !key.starts_with(prefix.as_str()));
    dialog
        .catalog_matches
        .retain(|key, _| !key.starts_with(prefix.as_str()));
    if dialog.draft.default_adapter == adapter_id {
        dialog.draft.default_adapter.clear();
        dialog.draft.default_model.clear();
    }
    if dialog
        .model_page
        .as_ref()
        .is_some_and(|page| page.adapter_id == adapter_id)
    {
        dialog.model_page = None;
    }
    dialog.selection.clamp_right(
        provider_studio_selected_adapter_models(dialog)
            .map(|adapter| adapter.models.len())
            .unwrap_or_default(),
    );
    provider_studio_ensure_default_selection(dialog);
}

const PROVIDER_MODEL_CONFIG_FIELDS: [ProviderModelConfigField; 14] = [
    ProviderModelConfigField::ModelId,
    ProviderModelConfigField::Enabled,
    ProviderModelConfigField::DisplayName,
    ProviderModelConfigField::Lifecycle,
    ProviderModelConfigField::ContextWindowTokens,
    ProviderModelConfigField::MaxInputTokens,
    ProviderModelConfigField::MaxOutputTokens,
    ProviderModelConfigField::InputModalities,
    ProviderModelConfigField::Features,
    ProviderModelConfigField::OutputModalities,
    ProviderModelConfigField::Description,
    ProviderModelConfigField::NativeTools,
    ProviderModelConfigField::SaveAction,
    ProviderModelConfigField::DeleteAction,
];

pub(in crate::app) fn provider_model_config_fields() -> &'static [ProviderModelConfigField] {
    &PROVIDER_MODEL_CONFIG_FIELDS
}

pub(in crate::app) fn provider_model_config_draft_from_value(
    model_id: &str,
    value: JsonValue,
) -> std::result::Result<ProviderModelConfigDraft, String> {
    let overlay = serde_json::from_value::<agena::config::ProviderModelOverlay>(value)
        .map_err(|error| error.to_string())?;
    Ok(provider_model_config_draft_from_overlay(model_id, overlay))
}

pub(in crate::app) fn apply_provider_model_config_native_tools_suggestion(
    provider_draft: &ProviderConfigDraft,
    adapter_id: &str,
    native_tools_present: bool,
    draft: &mut ProviderModelConfigDraft,
) {
    if native_tools_present || draft.native_tools_preset != ProviderNativeToolsPreset::Disabled {
        return;
    }
    if let Some(preset) =
        provider_native_tools_suggested_preset_for_draft(provider_draft, adapter_id)
    {
        draft.native_tools_preset = preset;
    }
}

pub(in crate::app) fn provider_model_config_draft_from_overlay(
    model_id: &str,
    overlay: agena::config::ProviderModelOverlay,
) -> ProviderModelConfigDraft {
    ProviderModelConfigDraft {
        model_id: model_id.to_owned(),
        enabled: overlay.enabled,
        display_name: overlay.definition.display_name.unwrap_or_default(),
        lifecycle: overlay
            .definition
            .lifecycle
            .map(model_lifecycle_token)
            .unwrap_or_default(),
        context_window_tokens: overlay
            .definition
            .context_window_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        max_input_tokens: overlay
            .definition
            .max_input_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        max_output_tokens: overlay
            .definition
            .max_output_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        input_modalities: overlay
            .definition
            .capabilities
            .input
            .as_ref()
            .map(|patch| patch.supported().iter().map(ToString::to_string).collect())
            .unwrap_or_default(),
        features: overlay
            .definition
            .capabilities
            .features
            .as_ref()
            .map(|patch| patch.supported().iter().map(ToString::to_string).collect())
            .unwrap_or_default(),
        output_modalities: overlay.definition.output_modalities.join(", "),
        description: overlay.definition.description.unwrap_or_default(),
        native_tools_preset: provider_native_tools_preset_from_config(&overlay.native_tools),
        native_tools_custom: overlay.native_tools,
    }
}

pub(in crate::app) fn provider_model_config_draft_to_model_value(
    draft: &ProviderModelConfigDraft,
) -> std::result::Result<(String, JsonValue), String> {
    let model_id = draft.model_id.trim();
    if model_id.is_empty() {
        return Err("model id is required".to_owned());
    }

    let input = (!draft.input_modalities.is_empty()).then(|| {
        agena::provider::CapabilitySelectionPatch::Supported(
            draft
                .input_modalities
                .iter()
                .filter_map(|value| parse_model_input_modality(value.as_str()))
                .collect(),
        )
    });
    let features = (!draft.features.is_empty()).then(|| {
        agena::provider::CapabilitySelectionPatch::Supported(
            draft
                .features
                .iter()
                .filter_map(|value| parse_model_capability_feature(value.as_str()))
                .collect(),
        )
    });
    let overlay = agena::config::ProviderModelOverlay {
        enabled: draft.enabled,
        native_tools: provider_native_tools_config_for_preset(
            draft.native_tools_preset,
            &draft.native_tools_custom,
        ),
        definition: agena::provider::ConfiguredModelDefinition {
            lifecycle: parse_optional_model_lifecycle(draft.lifecycle.as_str())?,
            context_window_tokens: parse_optional_u32_field(
                draft.context_window_tokens.as_str(),
                "context_window_tokens",
            )?,
            max_input_tokens: parse_optional_u32_field(
                draft.max_input_tokens.as_str(),
                "max_input_tokens",
            )?,
            max_output_tokens: parse_optional_u32_field(
                draft.max_output_tokens.as_str(),
                "max_output_tokens",
            )?,
            display_name: trimmed_owned_local(draft.display_name.as_str()),
            description: trimmed_owned_local(draft.description.as_str()),
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: split_csv_tokens(draft.output_modalities.as_str()),
            pricing: None,
            thinking_modes: std::collections::BTreeMap::new(),
            speed_modes: std::collections::BTreeMap::new(),
            capabilities: agena::provider::ModelCapabilityPatch { input, features },
        },
    };

    let value = provider_model_overlay_to_json_local(overlay)?;
    Ok((model_id.to_owned(), value))
}

pub(in crate::app) fn provider_model_config_field_label(
    i18n: &I18n,
    field: ProviderModelConfigField,
) -> String {
    ui_text::t(i18n, provider_model_config_field_label_key(field))
}

pub(in crate::app) fn provider_model_config_field_prompt(
    i18n: &I18n,
    field: ProviderModelConfigField,
) -> String {
    i18n.text_args(
        "overlay-provider-studio-model-field-prompt",
        &crate::fl_args!("field" => provider_model_config_field_label(i18n, field)),
    )
}

pub(in crate::app) fn provider_model_config_field_value(
    draft: &ProviderModelConfigDraft,
    field: ProviderModelConfigField,
) -> String {
    match field {
        ProviderModelConfigField::ModelId => draft.model_id.clone(),
        ProviderModelConfigField::Enabled => draft.enabled.to_string(),
        ProviderModelConfigField::DisplayName => draft.display_name.clone(),
        ProviderModelConfigField::Lifecycle => draft.lifecycle.clone(),
        ProviderModelConfigField::ContextWindowTokens => draft.context_window_tokens.clone(),
        ProviderModelConfigField::MaxInputTokens => draft.max_input_tokens.clone(),
        ProviderModelConfigField::MaxOutputTokens => draft.max_output_tokens.clone(),
        ProviderModelConfigField::InputModalities => draft
            .input_modalities
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        ProviderModelConfigField::Features => draft
            .features
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        ProviderModelConfigField::OutputModalities => draft.output_modalities.clone(),
        ProviderModelConfigField::Description => draft.description.clone(),
        ProviderModelConfigField::NativeTools => draft.native_tools_preset.token().to_owned(),
        ProviderModelConfigField::SaveAction | ProviderModelConfigField::DeleteAction => {
            String::new()
        }
    }
}

pub(in crate::app) fn provider_model_config_field_display(
    i18n: &I18n,
    draft: &ProviderModelConfigDraft,
    field: ProviderModelConfigField,
) -> String {
    match field {
        ProviderModelConfigField::SaveAction => {
            return ui_text::t(i18n, "provider-model-action-save-detail");
        }
        ProviderModelConfigField::DeleteAction => {
            return ui_text::t(i18n, "provider-model-action-delete-detail");
        }
        _ => {}
    }
    let value = provider_model_config_field_value(draft, field);
    if value.trim().is_empty() {
        ui_text::t(i18n, "value-unset")
    } else if field == ProviderModelConfigField::NativeTools {
        provider_native_tools_preset_label(i18n, draft.native_tools_preset)
    } else {
        value
    }
}

pub(in crate::app) fn provider_model_config_field_editable(
    field: ProviderModelConfigField,
) -> bool {
    !matches!(
        field,
        ProviderModelConfigField::ModelId
            | ProviderModelConfigField::SaveAction
            | ProviderModelConfigField::DeleteAction
    )
}

pub(in crate::app) fn commit_provider_model_config_field(
    draft: &mut ProviderModelConfigDraft,
    field: ProviderModelConfigField,
    value: String,
) -> std::result::Result<(), String> {
    match field {
        ProviderModelConfigField::ModelId => {
            let value = value.trim();
            if value.is_empty() {
                return Err("model id is required".to_owned());
            }
            draft.model_id = value.to_owned();
        }
        ProviderModelConfigField::Enabled => {
            draft.enabled = parse_bool_token(value.as_str())?;
        }
        ProviderModelConfigField::DisplayName => draft.display_name = value,
        ProviderModelConfigField::Lifecycle => {
            parse_optional_model_lifecycle(value.as_str())?;
            draft.lifecycle = value.trim().to_owned();
        }
        ProviderModelConfigField::ContextWindowTokens => {
            parse_optional_u32_field(value.as_str(), "context_window_tokens")?;
            draft.context_window_tokens = value.trim().to_owned();
        }
        ProviderModelConfigField::MaxInputTokens => {
            parse_optional_u32_field(value.as_str(), "max_input_tokens")?;
            draft.max_input_tokens = value.trim().to_owned();
        }
        ProviderModelConfigField::MaxOutputTokens => {
            parse_optional_u32_field(value.as_str(), "max_output_tokens")?;
            draft.max_output_tokens = value.trim().to_owned();
        }
        ProviderModelConfigField::InputModalities => {
            draft.input_modalities = parse_model_input_modality_set(value.as_str())?;
        }
        ProviderModelConfigField::Features => {
            draft.features = parse_model_capability_feature_set(value.as_str())?;
        }
        ProviderModelConfigField::OutputModalities => {
            draft.output_modalities = split_csv_tokens(value.as_str()).join(", ");
        }
        ProviderModelConfigField::Description => draft.description = value,
        ProviderModelConfigField::NativeTools => {
            let preset = if value.trim().is_empty() {
                ProviderNativeToolsPreset::Disabled
            } else {
                ProviderNativeToolsPreset::parse(value.as_str())
                    .ok_or_else(|| format!("unsupported native tools preset `{value}`"))?
            };
            draft.native_tools_preset = preset;
        }
        ProviderModelConfigField::SaveAction | ProviderModelConfigField::DeleteAction => {}
    }
    Ok(())
}
use super::{
    CredentialIssuer, I18n, JsonValue, ProviderConfigDraft, ProviderDraftAuthKind,
    ProviderDraftInteractiveLoginKind, ProviderModelConfigDraft, ProviderModelConfigField,
    ProviderNativeToolsPreset, ProviderStudioField, ProviderStudioOverlay, model_lifecycle_token,
    parse_bool_token, parse_model_capability_feature, parse_model_capability_feature_set,
    parse_model_input_modality, parse_model_input_modality_set, parse_optional_model_lifecycle,
    parse_optional_u32_field, provider_model_config_field_label_key,
    provider_model_overlay_to_json_local, provider_native_tools_config_for_preset,
    provider_native_tools_preset_from_config, provider_native_tools_preset_label,
    provider_native_tools_suggested_preset_for_draft, provider_studio_auth_login_kind,
    provider_studio_available_login_kinds, provider_studio_detail_fields, split_csv_tokens,
    trimmed_owned_local, ui_text,
};
