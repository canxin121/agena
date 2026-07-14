use anyhow::anyhow;

pub(super) fn parse_snapshot_payload(
    payload: Option<serde_json::Value>,
) -> Result<SnapshotCommandOutput> {
    let payload = payload.ok_or_else(|| anyhow!("snapshot tool returned no payload"))?;
    serde_json::from_value(payload).map_err(|error| anyhow!(error.to_string()))
}

pub(super) fn provider_native_tools_summary_resource(
    provider: &agena::config::ResolvedProviderConfig,
) -> ProviderNativeToolsSummaryResource {
    let (enabled, default_bindings) = provider
        .defaults
        .adapter
        .as_ref()
        .zip(provider.defaults.model.as_ref())
        .and_then(|(adapter_id, model_id)| {
            provider
                .models
                .get(format!("{adapter_id}/{model_id}").as_str())
        })
        .map(|model| (model.native_tools.enabled, model.native_tool_bindings()))
        .unwrap_or((false, Vec::new()));
    ProviderNativeToolsSummaryResource {
        enabled,
        model_count: provider
            .models
            .values()
            .filter(|model| model.native_tools.enabled)
            .count(),
        bindings: default_bindings
            .into_iter()
            .map(|binding| ProviderNativeToolBindingResource {
                tool: binding.tool.config_key().to_owned(),
                route: serde_json::to_string(&binding.route)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned(),
            })
            .collect(),
    }
}

pub(crate) fn provider_native_tools_config_for_preset(
    preset: ProviderNativeToolsPreset,
    custom: &ProviderNativeToolsConfig,
) -> ProviderNativeToolsConfig {
    match preset {
        ProviderNativeToolsPreset::Disabled => ProviderNativeToolsConfig::default(),
        ProviderNativeToolsPreset::OpenAiHostedDefaults => ProviderNativeToolsConfig {
            enabled: true,
            routes: agena::config::ProviderNativeToolRoutesConfig {
                web_search: Some(ProviderNativeToolRoute::ProviderHosted),
                image_generation: Some(ProviderNativeToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderNativeToolsPreset::AnthropicHostedDefaults => ProviderNativeToolsConfig {
            enabled: true,
            routes: agena::config::ProviderNativeToolRoutesConfig {
                web_search: Some(ProviderNativeToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderNativeToolsPreset::GeminiHostedDefaults => ProviderNativeToolsConfig {
            enabled: true,
            routes: agena::config::ProviderNativeToolRoutesConfig {
                web_search: Some(ProviderNativeToolRoute::ProviderHosted),
                code_execution: Some(ProviderNativeToolRoute::ProviderHosted),
                url_context: Some(ProviderNativeToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderNativeToolsPreset::Custom => custom.clone(),
    }
}

pub(crate) fn provider_native_tools_preset_from_config(
    config: &ProviderNativeToolsConfig,
) -> ProviderNativeToolsPreset {
    if *config
        == provider_native_tools_config_for_preset(
            ProviderNativeToolsPreset::OpenAiHostedDefaults,
            &ProviderNativeToolsConfig::default(),
        )
    {
        ProviderNativeToolsPreset::OpenAiHostedDefaults
    } else if *config
        == provider_native_tools_config_for_preset(
            ProviderNativeToolsPreset::AnthropicHostedDefaults,
            &ProviderNativeToolsConfig::default(),
        )
    {
        ProviderNativeToolsPreset::AnthropicHostedDefaults
    } else if *config
        == provider_native_tools_config_for_preset(
            ProviderNativeToolsPreset::GeminiHostedDefaults,
            &ProviderNativeToolsConfig::default(),
        )
    {
        ProviderNativeToolsPreset::GeminiHostedDefaults
    } else if config.is_empty() {
        ProviderNativeToolsPreset::Disabled
    } else {
        ProviderNativeToolsPreset::Custom
    }
}

pub(crate) fn provider_native_tools_suggested_preset_for_draft(
    draft: &ProviderConfigDraft,
    adapter_id: &str,
) -> Option<ProviderNativeToolsPreset> {
    match (draft.auth_kind.credential_issuer(), adapter_id.trim()) {
        (Some(CredentialIssuer::OpenaiChatgpt), "openai_responses") => {
            Some(ProviderNativeToolsPreset::OpenAiHostedDefaults)
        }
        (Some(CredentialIssuer::GoogleAdc), "openai_chat_completions") => {
            Some(ProviderNativeToolsPreset::GeminiHostedDefaults)
        }
        _ => None,
    }
}

pub(super) fn apply_provider_native_tools_defaults_to_model_value(
    draft: &ProviderConfigDraft,
    adapter_id: &str,
    model_value: &mut JsonValue,
) -> std::result::Result<(), ProviderStudioSaveError> {
    let Some(preset) = provider_native_tools_suggested_preset_for_draft(draft, adapter_id) else {
        return Ok(());
    };
    let model_object = model_value
        .as_object_mut()
        .ok_or(ProviderStudioSaveError::ProviderModelConfigMustBeObject)?;
    if model_object.contains_key("native_tools") {
        return Ok(());
    }
    model_object.insert(
        "native_tools".to_owned(),
        serde_json::to_value(provider_native_tools_config_for_preset(
            preset,
            &ProviderNativeToolsConfig::default(),
        ))
        .map_err(ProviderStudioSaveError::other)?,
    );
    Ok(())
}
use crate::backend::Result;
use crate::backend::{
    CredentialIssuer, JsonValue, ProviderConfigDraft, ProviderNativeToolBindingResource,
    ProviderNativeToolRoute, ProviderNativeToolsConfig, ProviderNativeToolsPreset,
    ProviderNativeToolsSummaryResource, ProviderStudioSaveError, SnapshotCommandOutput,
};
