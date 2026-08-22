use super::provider_selection::*;
use agena_api::resource::ProviderModelResource;
use agena_provider::{AgenaToolMode, AgenaToolsConfig, ResolvedProviderModelConfig};

pub(crate) fn provider_studio_field_label_key(field: ProviderStudioField) -> &'static str {
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
        ProviderStudioField::RequestTimeoutSecs => "provider-field-request-timeout",
        ProviderStudioField::ConnectTimeoutSecs => "provider-field-connect-timeout",
    }
}

pub(crate) fn provider_studio_field_label(i18n: &I18n, field: ProviderStudioField) -> String {
    ui_text::t(i18n, provider_studio_field_label_key(field))
}

pub(crate) fn provider_studio_field_prompt(i18n: &I18n, field: ProviderStudioField) -> String {
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
            &agena_tui::fl_args!("field" => provider_studio_field_label(i18n, field)),
        ),
    }
}

pub(crate) fn provider_studio_field_value(
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
        ProviderStudioField::RequestTimeoutSecs => draft.request_timeout_secs.to_string(),
        ProviderStudioField::ConnectTimeoutSecs => draft.connect_timeout_secs.to_string(),
    }
}

pub(crate) fn provider_studio_field_editable(
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> bool {
    match field {
        ProviderStudioField::ProviderId => true,
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
        ProviderStudioField::RequestTimeoutSecs | ProviderStudioField::ConnectTimeoutSecs => true,
    }
}

pub(crate) fn provider_studio_model_key(adapter_id: &str, model_id: &str) -> String {
    format!("{adapter_id}\u{1f}{model_id}")
}

pub(crate) fn remove_provider_studio_model_from_dialog(
    dialog: &mut ProviderStudioOverlay,
    adapter_id: &str,
    model_id: &str,
) {
    if let Some(adapter_models) = dialog
        .adapter_models
        .iter_mut()
        .find(|adapter_models| adapter_models.adapter_id == adapter_id)
    {
        adapter_models.models.retain(|model| model.id != model_id);
    }
    dialog
        .selected_model_keys
        .remove(provider_studio_model_key(adapter_id, model_id).as_str());
    dialog.model_page = None;
    dialog.selection.clamp_right(
        provider_studio_selected_adapter_models(dialog)
            .map(|adapter| adapter.models.len())
            .unwrap_or_default(),
    );
}

pub(crate) fn remove_provider_studio_adapter_from_dialog(
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
}

const PROVIDER_MODEL_CONFIG_FIELDS: [ProviderModelConfigField; 15] = [
    ProviderModelConfigField::ModelId,
    ProviderModelConfigField::Enabled,
    ProviderModelConfigField::NativeCompaction,
    ProviderModelConfigField::AgenaToolMode,
    ProviderModelConfigField::DisplayName,
    ProviderModelConfigField::Lifecycle,
    ProviderModelConfigField::ContextWindowTokens,
    ProviderModelConfigField::MaxInputTokens,
    ProviderModelConfigField::MaxOutputTokens,
    ProviderModelConfigField::Features,
    ProviderModelConfigField::InputModalities,
    ProviderModelConfigField::OutputModalities,
    ProviderModelConfigField::ThinkingModes,
    ProviderModelConfigField::SpeedModes,
    ProviderModelConfigField::Description,
];

pub(crate) fn provider_model_config_fields() -> &'static [ProviderModelConfigField] {
    &PROVIDER_MODEL_CONFIG_FIELDS
}

pub(crate) fn provider_model_config_draft_from_value(
    model_id: &str,
    value: JsonValue,
) -> crate::UiResult<ProviderModelConfigDraft> {
    let overlay = serde_json::from_value::<ResolvedProviderModelConfig>(value)
        .map_err(crate::UiFailure::internal)?;
    Ok(provider_model_config_draft_from_overlay(model_id, overlay))
}

pub(crate) fn provider_model_config_draft_from_overlay(
    model_id: &str,
    overlay: ResolvedProviderModelConfig,
) -> ProviderModelConfigDraft {
    let definition = overlay.definition;
    ProviderModelConfigDraft {
        model_id: model_id.to_owned(),
        enabled: overlay.enabled,
        native_compaction: overlay.native_compaction,
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
        supported_thinking_modes: definition.thinking_modes.keys().cloned().collect(),
        supported_speed_modes: definition.speed_modes.keys().cloned().collect(),
        description: definition.description.clone().unwrap_or_default(),
        definition,
    }
}

pub(crate) fn apply_provider_model_config_supported_modes(
    provider_model: Option<&ProviderModelResource>,
    draft: &mut ProviderModelConfigDraft,
) {
    let Some(provider_model) = provider_model else {
        return;
    };
    // Selectors come from `preset` when set, otherwise from the request
    // shape (effort name / `off`), matching the domain's `selector()`. Live
    // models (e.g. cpa's deepseek-v4) advertise effort modes with `preset`
    // unset, so deriving the selector here keeps the detail panel in sync
    // with what the mode selector offers.
    draft.supported_thinking_modes = provider_model
        .thinking_modes
        .iter()
        .filter_map(|mode| mode.selector().map(std::borrow::Cow::into_owned))
        .collect();
    draft.supported_speed_modes = provider_model.speed_modes.keys().cloned().collect();
}

pub(crate) fn provider_model_config_draft_to_model_value(
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
        agena_provider::CapabilitySelectionPatch::optional_from_supported_unsupported(
            input_supported,
            input_unsupported,
        );
    definition.capabilities.features =
        agena_provider::CapabilitySelectionPatch::optional_from_supported_unsupported(
            features_supported,
            features_unsupported,
        );
    let overlay = ResolvedProviderModelConfig {
        enabled: draft.enabled,
        native_compaction: draft.native_compaction,
        agena_tools: AgenaToolsConfig {
            mode: draft.agena_tool_mode,
            provider_native: Default::default(),
        },
        definition,
    };

    let value = provider_model_overlay_to_json_local(overlay)?;
    Ok((model_id.to_owned(), value))
}

pub(crate) fn provider_model_config_field_label(
    i18n: &I18n,
    field: ProviderModelConfigField,
) -> String {
    ui_text::t(i18n, provider_model_config_field_label_key(field))
}

pub(crate) fn provider_model_config_field_prompt(
    i18n: &I18n,
    field: ProviderModelConfigField,
) -> String {
    i18n.text_args(
        "overlay-provider-studio-model-field-prompt",
        &agena_tui::fl_args!("field" => provider_model_config_field_label(i18n, field)),
    )
}

pub(crate) fn provider_model_config_field_value(
    draft: &ProviderModelConfigDraft,
    field: ProviderModelConfigField,
) -> String {
    match field {
        ProviderModelConfigField::ModelId => draft.model_id.clone(),
        ProviderModelConfigField::Enabled => draft.enabled.to_string(),
        ProviderModelConfigField::NativeCompaction => draft.native_compaction.to_string(),
        ProviderModelConfigField::AgenaToolMode => match draft.agena_tool_mode {
            AgenaToolMode::ProviderProtocol => "provider_protocol".to_owned(),
            AgenaToolMode::Disabled => "disabled".to_owned(),
        },
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
        ProviderModelConfigField::ThinkingModes => {
            let mut names = draft.supported_thinking_modes.iter().collect::<Vec<_>>();
            names.sort_by(|left, right| {
                let left =
                    provider_studio_runtime_thinking_mode(left, &draft.definition.thinking_modes);
                let right =
                    provider_studio_runtime_thinking_mode(right, &draft.definition.thinking_modes);
                agena_domain::compare_thinking_mode_strength(&left, &right)
            });
            names.into_iter().cloned().collect::<Vec<_>>().join(", ")
        }
        ProviderModelConfigField::SpeedModes => draft
            .supported_speed_modes
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        ProviderModelConfigField::Description => draft.description.clone(),
    }
}

fn provider_studio_runtime_thinking_mode(
    selector: &str,
    configured: &agena_provider::ConfiguredModelModeMap<
        agena_provider::ConfiguredModelThinkingMode,
    >,
) -> agena_domain::ModelThinkingMode {
    let configured = configured.get(selector);
    configured
        .map(|mode| agena_provider::configured_thinking_mode_to_model(selector, mode))
        .unwrap_or_else(|| agena_domain::ModelThinkingMode {
            preset: Some(selector.to_owned()),
            ..Default::default()
        })
}

pub(crate) fn provider_model_config_field_display(
    i18n: &I18n,
    draft: &ProviderModelConfigDraft,
    field: ProviderModelConfigField,
) -> String {
    let value = provider_model_config_field_value(draft, field);
    if value.trim().is_empty() {
        ui_text::t(i18n, "value-unset")
    } else if field == ProviderModelConfigField::AgenaToolMode {
        let key = match draft.agena_tool_mode {
            AgenaToolMode::ProviderProtocol => "agena-tool-mode-provider-protocol-label",
            AgenaToolMode::Disabled => "agena-tool-mode-disabled-label",
        };
        ui_text::t(i18n, key)
    } else {
        value
    }
}

pub(crate) fn provider_model_config_field_editable(field: ProviderModelConfigField) -> bool {
    !matches!(
        field,
        ProviderModelConfigField::ModelId
            | ProviderModelConfigField::ThinkingModes
            | ProviderModelConfigField::SpeedModes
    )
}

pub(crate) fn commit_provider_model_config_field(
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
        ProviderModelConfigField::NativeCompaction => {
            draft.native_compaction = parse_bool_token(value.as_str())?;
        }
        ProviderModelConfigField::AgenaToolMode => {
            draft.agena_tool_mode = match value.trim() {
                "provider_protocol" => AgenaToolMode::ProviderProtocol,
                "disabled" => AgenaToolMode::Disabled,
                other => return Err(format!("unsupported Agena tool mode `{other}`")),
            };
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
        ProviderModelConfigField::ThinkingModes | ProviderModelConfigField::SpeedModes => {}
    }
    Ok(())
}
use super::{
    CredentialIssuer, I18n, JsonValue, ProviderConfigDraft, ProviderDraftAuthKind,
    ProviderDraftInteractiveLoginKind, ProviderModelConfigDraft, ProviderModelConfigField,
    ProviderStudioField, ProviderStudioOverlay, model_lifecycle_token, parse_bool_token,
    parse_model_capability_feature, parse_model_capability_feature_set, parse_model_input_modality,
    parse_model_input_modality_set, parse_optional_model_lifecycle, parse_optional_u32_field,
    provider_model_config_field_label_key, provider_model_overlay_to_json_local,
    provider_studio_auth_login_kind, provider_studio_available_login_kinds,
    provider_studio_detail_fields, split_csv_tokens, trimmed_owned_local,
};
use crate::ui_text;

#[cfg(test)]
mod tests {
    use super::{
        ProviderModelConfigField, apply_provider_model_config_supported_modes,
        commit_provider_model_config_field, provider_model_config_draft_from_overlay,
        provider_model_config_draft_to_model_value, provider_model_config_field_value,
        provider_model_config_fields,
    };
    use agena_provider::{AgenaToolMode, AgenaToolsConfig, ResolvedProviderModelConfig};

    #[test]
    fn model_detail_orders_capabilities_and_has_no_action_rows() {
        assert_eq!(
            provider_model_config_fields(),
            &[
                ProviderModelConfigField::ModelId,
                ProviderModelConfigField::Enabled,
                ProviderModelConfigField::NativeCompaction,
                ProviderModelConfigField::AgenaToolMode,
                ProviderModelConfigField::DisplayName,
                ProviderModelConfigField::Lifecycle,
                ProviderModelConfigField::ContextWindowTokens,
                ProviderModelConfigField::MaxInputTokens,
                ProviderModelConfigField::MaxOutputTokens,
                ProviderModelConfigField::Features,
                ProviderModelConfigField::InputModalities,
                ProviderModelConfigField::OutputModalities,
                ProviderModelConfigField::ThinkingModes,
                ProviderModelConfigField::SpeedModes,
                ProviderModelConfigField::Description,
            ],
        );
    }

    #[test]
    fn saving_model_detail_preserves_modes_and_hidden_metadata() {
        let mut definition = agena_provider::ConfiguredModelDefinition {
            knowledge_cutoff: Some("2025-01".to_owned()),
            ..Default::default()
        };
        definition.thinking_modes.insert(
            "deep".to_owned(),
            agena_provider::ConfiguredModelThinkingMode {
                strategy: Some(agena_provider::ConfiguredThinkingStrategy::RequestOnly),
                ..Default::default()
            },
        );
        definition.thinking_modes.default =
            agena_provider::ConfiguredModeDefault::Mode("deep".to_owned());
        definition.speed_modes.insert(
            "fast".to_owned(),
            agena_provider::ConfiguredModelSpeedMode::default(),
        );
        let overlay = ResolvedProviderModelConfig {
            enabled: true,
            native_compaction: false,
            agena_tools: AgenaToolsConfig {
                mode: AgenaToolMode::Disabled,
                provider_native: Default::default(),
            },
            definition: definition.clone(),
        };

        let draft = provider_model_config_draft_from_overlay("model-a", overlay);
        let (_, value) = provider_model_config_draft_to_model_value(&draft).unwrap();
        let saved: ResolvedProviderModelConfig = serde_json::from_value(value).unwrap();

        assert_eq!(
            draft.supported_thinking_modes,
            std::collections::BTreeSet::from(["deep".to_owned()]),
        );
        assert_eq!(
            draft.supported_speed_modes,
            std::collections::BTreeSet::from(["fast".to_owned()]),
        );
        assert_eq!(saved.definition.thinking_modes, definition.thinking_modes);
        assert_eq!(saved.definition.speed_modes, definition.speed_modes);
        assert_eq!(
            saved.definition.knowledge_cutoff,
            definition.knowledge_cutoff
        );
        assert_eq!(saved.agena_tools.mode, AgenaToolMode::Disabled);
        assert!(!saved.native_compaction);
    }

    #[test]
    fn all_agena_tool_modes_are_selectable_and_persisted() {
        for (token, expected) in [
            ("provider_protocol", AgenaToolMode::ProviderProtocol),
            ("disabled", AgenaToolMode::Disabled),
        ] {
            let mut draft = provider_model_config_draft_from_overlay(
                "model-a",
                ResolvedProviderModelConfig::default(),
            );
            commit_provider_model_config_field(
                &mut draft,
                ProviderModelConfigField::AgenaToolMode,
                token.to_owned(),
            )
            .unwrap();

            let (_, value) = provider_model_config_draft_to_model_value(&draft).unwrap();
            let saved: ResolvedProviderModelConfig = serde_json::from_value(value).unwrap();
            assert_eq!(saved.agena_tools.mode, expected, "mode token: {token}");
        }
    }

    #[test]
    fn live_provider_modes_drive_the_model_detail() {
        let mut draft = provider_model_config_draft_from_overlay(
            "model-a",
            ResolvedProviderModelConfig::default(),
        );
        let mut model =
            agena_api::resource::ProviderModelResource::configured("openai_responses", "model-a");
        model
            .thinking_modes
            .push(agena_api::resource::ProviderModelThinkingModeResource {
                preset: Some("high".to_owned()),
                is_default: false,
                display_name: None,
                description: None,
                thinking: None,
                request_override: Default::default(),
                adapter_overrides: Default::default(),
            });
        model.speed_modes.insert(
            "fast".to_owned(),
            agena_api::resource::ProviderModelSpeedModeResource {
                is_default: false,
                display_name: None,
                description: None,
                request_override: Default::default(),
                adapter_overrides: Default::default(),
            },
        );

        apply_provider_model_config_supported_modes(Some(&model), &mut draft);

        assert_eq!(
            draft.supported_thinking_modes,
            std::collections::BTreeSet::from(["high".to_owned()]),
        );
        assert_eq!(
            draft.supported_speed_modes,
            std::collections::BTreeSet::from(["fast".to_owned()]),
        );
    }

    #[test]
    fn live_effort_modes_without_preset_still_show_in_the_model_detail() {
        // Live OpenAI-compatible models (e.g. cpa's deepseek-v4) advertise
        // effort modes with `preset` unset; the selector derives from the
        // request shape. The detail panel must show them, not 未设置.
        let mut draft = provider_model_config_draft_from_overlay(
            "deepseek-v4-pro",
            ResolvedProviderModelConfig::default(),
        );
        let mut model = agena_api::resource::ProviderModelResource::configured(
            "openai_responses",
            "deepseek-v4-pro",
        );
        for effort in ["low", "medium", "high", "max"] {
            model
                .thinking_modes
                .push(agena_api::resource::ProviderModelThinkingModeResource {
                    preset: None,
                    is_default: false,
                    display_name: None,
                    description: None,
                    thinking: Some(agena_api::resource::ThinkingRequestResource::Effort {
                        effort: match effort {
                            "low" => agena_api::resource::ReasoningEffortResource::Low,
                            "medium" => agena_api::resource::ReasoningEffortResource::Medium,
                            "high" => agena_api::resource::ReasoningEffortResource::High,
                            _ => agena_api::resource::ReasoningEffortResource::Max,
                        },
                    }),
                    request_override: Default::default(),
                    adapter_overrides: Default::default(),
                });
        }
        model
            .thinking_modes
            .push(agena_api::resource::ProviderModelThinkingModeResource {
                preset: None,
                is_default: false,
                display_name: None,
                description: None,
                thinking: Some(agena_api::resource::ThinkingRequestResource::Disabled),
                request_override: Default::default(),
                adapter_overrides: Default::default(),
            });

        apply_provider_model_config_supported_modes(Some(&model), &mut draft);

        assert_eq!(
            draft.supported_thinking_modes,
            ["low", "medium", "high", "max", "off"]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
        );
        assert!(
            !provider_model_config_field_value(&draft, ProviderModelConfigField::ThinkingModes)
                .trim()
                .is_empty()
        );
    }
}
