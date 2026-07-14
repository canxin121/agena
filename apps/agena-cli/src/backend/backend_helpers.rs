use anyhow::anyhow;

pub(super) fn parse_snapshot_payload(
    payload: Option<serde_json::Value>,
) -> Result<SnapshotCommandOutput> {
    let payload = payload.ok_or_else(|| anyhow!("snapshot tool returned no payload"))?;
    serde_json::from_value(payload).map_err(|error| anyhow!(error.to_string()))
}

pub(super) fn provider_tools_summary_resource(
    provider: &agena::config::ResolvedProviderConfig,
) -> ProviderToolsSummaryResource {
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
        .map(|model| (model.provider_tools.enabled, model.provider_tool_bindings()))
        .unwrap_or((false, Vec::new()));
    ProviderToolsSummaryResource {
        enabled,
        model_count: provider
            .models
            .values()
            .filter(|model| model.provider_tools.enabled)
            .count(),
        bindings: default_bindings
            .into_iter()
            .map(|binding| ProviderToolBindingResource {
                tool: binding.tool.config_key().to_owned(),
                route: serde_json::to_string(&binding.route)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned(),
            })
            .collect(),
    }
}

pub(crate) fn provider_tools_config_for_preset(
    preset: ProviderToolsPreset,
    custom: &ProviderToolsConfig,
) -> ProviderToolsConfig {
    match preset {
        ProviderToolsPreset::Disabled => ProviderToolsConfig::default(),
        ProviderToolsPreset::OpenAiHostedDefaults => ProviderToolsConfig {
            enabled: true,
            routes: agena::config::ProviderToolRoutesConfig {
                web_search: Some(ProviderToolRoute::ProviderHosted),
                image_generation: Some(ProviderToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderToolsPreset::AnthropicHostedDefaults => ProviderToolsConfig {
            enabled: true,
            routes: agena::config::ProviderToolRoutesConfig {
                web_search: Some(ProviderToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderToolsPreset::GeminiHostedDefaults => ProviderToolsConfig {
            enabled: true,
            routes: agena::config::ProviderToolRoutesConfig {
                web_search: Some(ProviderToolRoute::ProviderHosted),
                code_execution: Some(ProviderToolRoute::ProviderHosted),
                url_context: Some(ProviderToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderToolsPreset::Custom => custom.clone(),
    }
}

pub(crate) fn provider_tools_preset_from_config(
    config: &ProviderToolsConfig,
) -> ProviderToolsPreset {
    if *config
        == provider_tools_config_for_preset(
            ProviderToolsPreset::OpenAiHostedDefaults,
            &ProviderToolsConfig::default(),
        )
    {
        ProviderToolsPreset::OpenAiHostedDefaults
    } else if *config
        == provider_tools_config_for_preset(
            ProviderToolsPreset::AnthropicHostedDefaults,
            &ProviderToolsConfig::default(),
        )
    {
        ProviderToolsPreset::AnthropicHostedDefaults
    } else if *config
        == provider_tools_config_for_preset(
            ProviderToolsPreset::GeminiHostedDefaults,
            &ProviderToolsConfig::default(),
        )
    {
        ProviderToolsPreset::GeminiHostedDefaults
    } else if config.is_empty() {
        ProviderToolsPreset::Disabled
    } else {
        ProviderToolsPreset::Custom
    }
}

pub(crate) fn provider_tools_suggested_preset_for_draft(
    draft: &ProviderConfigDraft,
    adapter_id: &str,
) -> Option<ProviderToolsPreset> {
    match (draft.auth_kind.credential_issuer(), adapter_id.trim()) {
        (Some(CredentialIssuer::OpenaiChatgpt), "openai_responses") => {
            Some(ProviderToolsPreset::OpenAiHostedDefaults)
        }
        (Some(CredentialIssuer::GoogleAdc), "openai_chat_completions") => {
            Some(ProviderToolsPreset::GeminiHostedDefaults)
        }
        _ => None,
    }
}

pub(super) fn apply_provider_tools_defaults_to_model_value(
    draft: &ProviderConfigDraft,
    adapter_id: &str,
    model_value: &mut JsonValue,
) -> std::result::Result<(), ProviderStudioSaveError> {
    let Some(preset) = provider_tools_suggested_preset_for_draft(draft, adapter_id) else {
        return Ok(());
    };
    let model_object = model_value
        .as_object_mut()
        .ok_or(ProviderStudioSaveError::ProviderModelConfigMustBeObject)?;
    if model_object.contains_key("provider_tools") {
        return Ok(());
    }
    model_object.insert(
        "provider_tools".to_owned(),
        serde_json::to_value(provider_tools_config_for_preset(
            preset,
            &ProviderToolsConfig::default(),
        ))
        .map_err(ProviderStudioSaveError::other)?,
    );
    Ok(())
}
use crate::backend::Result;
use crate::backend::{
    CredentialIssuer, JsonValue, ProviderConfigDraft, ProviderStudioSaveError,
    ProviderToolBindingResource, ProviderToolRoute, ProviderToolsConfig, ProviderToolsPreset,
    ProviderToolsSummaryResource, SnapshotCommandOutput,
};
