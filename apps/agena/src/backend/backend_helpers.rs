use anyhow::anyhow;

pub(super) fn parse_snapshot_payload(
    payload: Option<serde_json::Value>,
) -> Result<SnapshotCommandOutput> {
    let payload = payload.ok_or_else(|| anyhow!("snapshot tool returned no payload"))?;
    serde_json::from_value(payload).map_err(|error| anyhow!(error.to_string()))
}

pub(crate) fn provider_native_tools_config_for_preset(
    preset: ProviderNativeToolsPreset,
    custom: &ProviderNativeToolsConfig,
) -> ProviderNativeToolsConfig {
    match preset {
        ProviderNativeToolsPreset::Disabled => ProviderNativeToolsConfig::default(),
        ProviderNativeToolsPreset::OpenAiHostedDefaults => ProviderNativeToolsConfig {
            routes: agena_provider::ProviderNativeToolRoutesConfig {
                web_search: Some(ProviderNativeToolRoute::ProviderHosted),
                image_generation: Some(ProviderNativeToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderNativeToolsPreset::AnthropicHostedDefaults => ProviderNativeToolsConfig {
            routes: agena_provider::ProviderNativeToolRoutesConfig {
                web_search: Some(ProviderNativeToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderNativeToolsPreset::GeminiHostedDefaults => ProviderNativeToolsConfig {
            routes: agena_provider::ProviderNativeToolRoutesConfig {
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

use crate::backend::Result;
use crate::backend::{
    ProviderNativeToolRoute, ProviderNativeToolsConfig, ProviderNativeToolsPreset,
    SnapshotCommandOutput,
};
