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
    let (active, default_bindings) = provider
        .defaults
        .adapter
        .as_ref()
        .zip(provider.defaults.model.as_ref())
        .and_then(|(adapter_id, model_id)| {
            provider
                .models
                .get(format!("{adapter_id}/{model_id}").as_str())
        })
        .map(|model| {
            let bindings = model.provider_native_tool_bindings();
            (!bindings.is_empty(), bindings)
        })
        .unwrap_or((false, Vec::new()));
    ProviderNativeToolsSummaryResource {
        active,
        model_count: provider
            .models
            .values()
            .filter(|model| !model.provider_native_tool_bindings().is_empty())
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
            routes: agena::config::ProviderNativeToolRoutesConfig {
                web_search: Some(ProviderNativeToolRoute::ProviderHosted),
                image_generation: Some(ProviderNativeToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderNativeToolsPreset::AnthropicHostedDefaults => ProviderNativeToolsConfig {
            routes: agena::config::ProviderNativeToolRoutesConfig {
                web_search: Some(ProviderNativeToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderNativeToolsPreset::GeminiHostedDefaults => ProviderNativeToolsConfig {
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

use crate::backend::Result;
use crate::backend::{
    ProviderNativeToolBindingResource, ProviderNativeToolRoute, ProviderNativeToolsConfig,
    ProviderNativeToolsPreset, ProviderNativeToolsSummaryResource, SnapshotCommandOutput,
};
