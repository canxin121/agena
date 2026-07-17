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
        ProviderStudioField::RequestTimeoutSecs => "provider-field-request-timeout",
        ProviderStudioField::ConnectTimeoutSecs => "provider-field-connect-timeout",
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
        ProviderStudioField::RequestTimeoutSecs => draft.request_timeout_secs.to_string(),
        ProviderStudioField::ConnectTimeoutSecs => draft.connect_timeout_secs.to_string(),
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
        ProviderStudioField::DefaultAdapter
        | ProviderStudioField::DefaultModel
        | ProviderStudioField::RequestTimeoutSecs
        | ProviderStudioField::ConnectTimeoutSecs => true,
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

const PROVIDER_MODEL_CONFIG_FIELDS: [ProviderModelConfigField; 15] = [
    ProviderModelConfigField::ModelId,
    ProviderModelConfigField::Enabled,
    ProviderModelConfigField::AgenaToolMode,
    ProviderModelConfigField::ProviderNativeTools,
    ProviderModelConfigField::DisplayName,
    ProviderModelConfigField::Lifecycle,
    ProviderModelConfigField::ContextWindowTokens,
    ProviderModelConfigField::MaxInputTokens,
    ProviderModelConfigField::MaxOutputTokens,
    ProviderModelConfigField::Features,
    ProviderModelConfigField::InputModalities,
    ProviderModelConfigField::OutputModalities,
    ProviderModelConfigField::ThinkingModeVariants,
    ProviderModelConfigField::SpeedModeVariants,
    ProviderModelConfigField::Description,
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

pub(in crate::app) fn provider_model_config_draft_from_overlay(
    model_id: &str,
    overlay: agena::config::ProviderModelOverlay,
) -> ProviderModelConfigDraft {
    let definition = overlay.definition;
    ProviderModelConfigDraft {
        model_id: model_id.to_owned(),
        enabled: overlay.enabled,
        agena_tool_mode: overlay.agena_tools.mode,
        display_name: definition.display_name.clone().unwrap_or_default(),
        lifecycle: definition
            .lifecycle
            .map(model_lifecycle_token)
            .unwrap_or_default(),
        context_window_tokens: definition
            .context_window_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        max_input_tokens: definition
            .max_input_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        max_output_tokens: definition
            .max_output_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        input_modalities: definition
            .capabilities
            .input
            .as_ref()
            .map(|patch| patch.supported().iter().map(ToString::to_string).collect())
            .unwrap_or_default(),
        features: definition
            .capabilities
            .features
            .as_ref()
            .map(|patch| patch.supported().iter().map(ToString::to_string).collect())
            .unwrap_or_default(),
        output_modalities: definition.output_modalities.join(", "),
        thinking_mode_variants: definition.thinking_modes.keys().cloned().collect(),
        speed_mode_variants: definition.speed_modes.keys().cloned().collect(),
        description: definition.description.clone().unwrap_or_default(),
        provider_native_tools_preset: provider_native_tools_preset_from_config(
            &overlay.agena_tools.provider_native,
        ),
        provider_native_tools_custom: overlay.agena_tools.provider_native,
        definition,
    }
}

pub(in crate::app) fn apply_provider_model_config_supported_variants(
    provider_model: Option<&ProviderModel>,
    draft: &mut ProviderModelConfigDraft,
) {
    let Some(provider_model) = provider_model else {
        return;
    };
    draft.thinking_mode_variants = provider_model.thinking_modes.keys().cloned().collect();
    draft.speed_mode_variants = provider_model.speed_modes.keys().cloned().collect();
}

pub(in crate::app) fn provider_model_config_draft_to_model_value(
    draft: &ProviderModelConfigDraft,
) -> std::result::Result<(String, JsonValue), String> {
    let model_id = draft.model_id.trim();
    if model_id.is_empty() {
        return Err("model id is required".to_owned());
    }

    let input_supported = draft
        .input_modalities
        .iter()
        .filter_map(|value| parse_model_input_modality(value.as_str()))
        .collect();
    let input_unsupported = draft
        .definition
        .capabilities
        .input
        .as_ref()
        .map(|patch| patch.unsupported().to_vec())
        .unwrap_or_default();
    let features_supported = draft
        .features
        .iter()
        .filter_map(|value| parse_model_capability_feature(value.as_str()))
        .collect();
    let features_unsupported = draft
        .definition
        .capabilities
        .features
        .as_ref()
        .map(|patch| patch.unsupported().to_vec())
        .unwrap_or_default();
    let mut definition = draft.definition.clone();
    definition.lifecycle = parse_optional_model_lifecycle(draft.lifecycle.as_str())?;
    definition.context_window_tokens = parse_optional_u32_field(
        draft.context_window_tokens.as_str(),
        "context_window_tokens",
    )?;
    definition.max_input_tokens =
        parse_optional_u32_field(draft.max_input_tokens.as_str(), "max_input_tokens")?;
    definition.max_output_tokens =
        parse_optional_u32_field(draft.max_output_tokens.as_str(), "max_output_tokens")?;
    definition.display_name = trimmed_owned_local(draft.display_name.as_str());
    definition.description = trimmed_owned_local(draft.description.as_str());
    definition.output_modalities = split_csv_tokens(draft.output_modalities.as_str());
    definition.capabilities.input =
        agena::provider::CapabilitySelectionPatch::optional_from_supported_unsupported(
            input_supported,
            input_unsupported,
        );
    definition.capabilities.features =
        agena::provider::CapabilitySelectionPatch::optional_from_supported_unsupported(
            features_supported,
            features_unsupported,
        );
    let overlay = agena::config::ProviderModelOverlay {
        enabled: draft.enabled,
        agena_tools: agena::config::AgenaToolsConfig {
            mode: draft.agena_tool_mode,
            provider_native: provider_native_tools_config_for_preset(
                draft.provider_native_tools_preset,
                &draft.provider_native_tools_custom,
            ),
        },
        definition,
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
        ProviderModelConfigField::AgenaToolMode => match draft.agena_tool_mode {
            agena::config::AgenaToolMode::ProviderProtocol => "provider_protocol".to_owned(),
            agena::config::AgenaToolMode::PromptEnvelope => "prompt_envelope".to_owned(),
            agena::config::AgenaToolMode::Disabled => "disabled".to_owned(),
        },
        ProviderModelConfigField::ProviderNativeTools => {
            draft.provider_native_tools_preset.token().to_owned()
        }
        ProviderModelConfigField::DisplayName => draft.display_name.clone(),
        ProviderModelConfigField::Lifecycle => draft.lifecycle.clone(),
        ProviderModelConfigField::ContextWindowTokens => draft.context_window_tokens.clone(),
        ProviderModelConfigField::MaxInputTokens => draft.max_input_tokens.clone(),
        ProviderModelConfigField::MaxOutputTokens => draft.max_output_tokens.clone(),
        ProviderModelConfigField::Features => draft
            .features
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        ProviderModelConfigField::InputModalities => draft
            .input_modalities
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        ProviderModelConfigField::OutputModalities => draft.output_modalities.clone(),
        ProviderModelConfigField::ThinkingModeVariants => draft
            .thinking_mode_variants
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        ProviderModelConfigField::SpeedModeVariants => draft
            .speed_mode_variants
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        ProviderModelConfigField::Description => draft.description.clone(),
    }
}

pub(in crate::app) fn provider_model_config_field_display(
    i18n: &I18n,
    draft: &ProviderModelConfigDraft,
    field: ProviderModelConfigField,
) -> String {
    let value = provider_model_config_field_value(draft, field);
    if value.trim().is_empty() {
        ui_text::t(i18n, "value-unset")
    } else if field == ProviderModelConfigField::ProviderNativeTools {
        provider_native_tools_preset_label(i18n, draft.provider_native_tools_preset)
    } else if field == ProviderModelConfigField::AgenaToolMode {
        let key = match draft.agena_tool_mode {
            agena::config::AgenaToolMode::ProviderProtocol => {
                "agena-tool-mode-provider-protocol-label"
            }
            agena::config::AgenaToolMode::PromptEnvelope => "agena-tool-mode-prompt-envelope-label",
            agena::config::AgenaToolMode::Disabled => "agena-tool-mode-disabled-label",
        };
        ui_text::t(i18n, key)
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
            | ProviderModelConfigField::ThinkingModeVariants
            | ProviderModelConfigField::SpeedModeVariants
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
        ProviderModelConfigField::AgenaToolMode => {
            draft.agena_tool_mode = match value.trim() {
                "provider_protocol" => agena::config::AgenaToolMode::ProviderProtocol,
                "prompt_envelope" => {
                    draft.provider_native_tools_preset = ProviderNativeToolsPreset::Disabled;
                    agena::config::AgenaToolMode::PromptEnvelope
                }
                "disabled" => {
                    draft.provider_native_tools_preset = ProviderNativeToolsPreset::Disabled;
                    agena::config::AgenaToolMode::Disabled
                }
                other => return Err(format!("unsupported Agena tool mode `{other}`")),
            };
        }
        ProviderModelConfigField::ProviderNativeTools => {
            let preset = if value.trim().is_empty() {
                ProviderNativeToolsPreset::Disabled
            } else {
                ProviderNativeToolsPreset::parse(value.as_str())
                    .ok_or_else(|| format!("unsupported provider-native tools preset `{value}`"))?
            };
            if preset != ProviderNativeToolsPreset::Disabled
                && !draft.agena_tool_mode.is_provider_protocol()
            {
                return Err(
                    "provider-native tools require agena_tools.mode `provider_protocol`".to_owned(),
                );
            }
            draft.provider_native_tools_preset = preset;
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
        ProviderModelConfigField::ThinkingModeVariants
        | ProviderModelConfigField::SpeedModeVariants => {}
    }
    Ok(())
}
use super::{
    CredentialIssuer, I18n, JsonValue, ProviderConfigDraft, ProviderDraftAuthKind,
    ProviderDraftInteractiveLoginKind, ProviderModel, ProviderModelConfigDraft,
    ProviderModelConfigField, ProviderNativeToolsPreset, ProviderStudioField,
    ProviderStudioOverlay, model_lifecycle_token, parse_bool_token, parse_model_capability_feature,
    parse_model_capability_feature_set, parse_model_input_modality, parse_model_input_modality_set,
    parse_optional_model_lifecycle, parse_optional_u32_field,
    provider_model_config_field_label_key, provider_model_overlay_to_json_local,
    provider_native_tools_config_for_preset, provider_native_tools_preset_from_config,
    provider_native_tools_preset_label, provider_studio_auth_login_kind,
    provider_studio_available_login_kinds, provider_studio_detail_fields, split_csv_tokens,
    trimmed_owned_local, ui_text,
};

#[cfg(test)]
mod tests {
    use super::{
        ProviderModel, ProviderModelConfigField, ProviderNativeToolsPreset,
        apply_provider_model_config_supported_variants, commit_provider_model_config_field,
        provider_model_config_draft_from_overlay, provider_model_config_draft_to_model_value,
        provider_model_config_fields,
    };

    #[test]
    fn model_detail_orders_capabilities_and_has_no_action_rows() {
        assert_eq!(
            provider_model_config_fields(),
            &[
                ProviderModelConfigField::ModelId,
                ProviderModelConfigField::Enabled,
                ProviderModelConfigField::AgenaToolMode,
                ProviderModelConfigField::ProviderNativeTools,
                ProviderModelConfigField::DisplayName,
                ProviderModelConfigField::Lifecycle,
                ProviderModelConfigField::ContextWindowTokens,
                ProviderModelConfigField::MaxInputTokens,
                ProviderModelConfigField::MaxOutputTokens,
                ProviderModelConfigField::Features,
                ProviderModelConfigField::InputModalities,
                ProviderModelConfigField::OutputModalities,
                ProviderModelConfigField::ThinkingModeVariants,
                ProviderModelConfigField::SpeedModeVariants,
                ProviderModelConfigField::Description,
            ],
        );
    }

    #[test]
    fn saving_model_detail_preserves_modes_and_hidden_metadata() {
        let mut definition = agena::provider::ConfiguredModelDefinition {
            knowledge_cutoff: Some("2025-01".to_owned()),
            default_thinking_mode: Some("deep".to_owned()),
            ..Default::default()
        };
        definition.thinking_modes.insert(
            "deep".to_owned(),
            agena::provider::ConfiguredModelThinkingMode::default(),
        );
        definition.speed_modes.insert(
            "fast".to_owned(),
            agena::provider::ConfiguredModelSpeedMode::default(),
        );
        let overlay = agena::config::ProviderModelOverlay {
            enabled: true,
            agena_tools: agena::config::AgenaToolsConfig {
                mode: agena::config::AgenaToolMode::PromptEnvelope,
                provider_native: Default::default(),
            },
            definition: definition.clone(),
        };

        let draft = provider_model_config_draft_from_overlay("model-a", overlay);
        let (_, value) = provider_model_config_draft_to_model_value(&draft).unwrap();
        let saved: agena::config::ProviderModelOverlay = serde_json::from_value(value).unwrap();

        assert_eq!(
            draft.thinking_mode_variants,
            std::collections::BTreeSet::from(["deep".to_owned()]),
        );
        assert_eq!(
            draft.speed_mode_variants,
            std::collections::BTreeSet::from(["fast".to_owned()]),
        );
        assert_eq!(saved.definition.thinking_modes, definition.thinking_modes);
        assert_eq!(saved.definition.speed_modes, definition.speed_modes);
        assert_eq!(
            saved.definition.knowledge_cutoff,
            definition.knowledge_cutoff
        );
        assert_eq!(
            saved.definition.default_thinking_mode,
            definition.default_thinking_mode,
        );
        assert_eq!(
            saved.agena_tools.mode,
            agena::config::AgenaToolMode::PromptEnvelope
        );
    }

    #[test]
    fn all_agena_tool_modes_are_selectable_and_persisted() {
        for (token, expected) in [
            (
                "provider_protocol",
                agena::config::AgenaToolMode::ProviderProtocol,
            ),
            (
                "prompt_envelope",
                agena::config::AgenaToolMode::PromptEnvelope,
            ),
            ("disabled", agena::config::AgenaToolMode::Disabled),
        ] {
            let mut draft = provider_model_config_draft_from_overlay(
                "model-a",
                agena::config::ProviderModelOverlay::default(),
            );
            commit_provider_model_config_field(
                &mut draft,
                ProviderModelConfigField::AgenaToolMode,
                token.to_owned(),
            )
            .unwrap();

            let (_, value) = provider_model_config_draft_to_model_value(&draft).unwrap();
            let saved: agena::config::ProviderModelOverlay = serde_json::from_value(value).unwrap();
            assert_eq!(saved.agena_tools.mode, expected, "mode token: {token}");
        }
    }

    #[test]
    fn provider_native_tools_require_an_explicit_provider_protocol_mode() {
        let mut draft = provider_model_config_draft_from_overlay(
            "model-a",
            agena::config::ProviderModelOverlay::default(),
        );
        draft.provider_native_tools_preset = ProviderNativeToolsPreset::OpenAiHostedDefaults;

        commit_provider_model_config_field(
            &mut draft,
            ProviderModelConfigField::AgenaToolMode,
            "prompt_envelope".to_owned(),
        )
        .unwrap();
        assert_eq!(
            draft.agena_tool_mode,
            agena::config::AgenaToolMode::PromptEnvelope
        );
        assert_eq!(
            draft.provider_native_tools_preset,
            ProviderNativeToolsPreset::Disabled
        );

        let error = commit_provider_model_config_field(
            &mut draft,
            ProviderModelConfigField::ProviderNativeTools,
            ProviderNativeToolsPreset::OpenAiHostedDefaults
                .token()
                .to_owned(),
        )
        .expect_err("provider-native tools must not change Agena tools mode implicitly");
        assert!(error.contains("require agena_tools.mode `provider_protocol`"));
        assert_eq!(
            draft.provider_native_tools_preset,
            ProviderNativeToolsPreset::Disabled
        );

        commit_provider_model_config_field(
            &mut draft,
            ProviderModelConfigField::AgenaToolMode,
            "provider_protocol".to_owned(),
        )
        .unwrap();
        commit_provider_model_config_field(
            &mut draft,
            ProviderModelConfigField::ProviderNativeTools,
            ProviderNativeToolsPreset::OpenAiHostedDefaults
                .token()
                .to_owned(),
        )
        .unwrap();
        assert_eq!(
            draft.agena_tool_mode,
            agena::config::AgenaToolMode::ProviderProtocol
        );
        assert_eq!(
            draft.provider_native_tools_preset,
            ProviderNativeToolsPreset::OpenAiHostedDefaults
        );
    }

    #[test]
    fn live_provider_variants_drive_the_model_detail() {
        let mut draft = provider_model_config_draft_from_overlay(
            "model-a",
            agena::config::ProviderModelOverlay::default(),
        );
        let mut model = ProviderModel::new("openai_responses", "model-a");
        model.thinking_modes.insert(
            "high".to_owned(),
            agena::model::ModelThinkingMode::default(),
        );
        model
            .speed_modes
            .insert("fast".to_owned(), agena::model::ModelSpeedMode::default());

        apply_provider_model_config_supported_variants(Some(&model), &mut draft);

        assert_eq!(
            draft.thinking_mode_variants,
            std::collections::BTreeSet::from(["high".to_owned()]),
        );
        assert_eq!(
            draft.speed_mode_variants,
            std::collections::BTreeSet::from(["fast".to_owned()]),
        );
    }
}
