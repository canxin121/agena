//! Runtime-owned projections for configured provider model selection.
//!
//! This module deliberately stops at stable domain models.  Provider adapter
//! capability and catalog decoration remain concrete composition adapters, but
//! the no-network route selection policy belongs to Runtime alongside
//! the resolved provider configuration values.

use std::collections::BTreeSet;

use agena_domain::{Model, ModelCapabilities, ModelId, ProviderId};

use agena_runtime_config::ResolvedProviderConfig;

/// Project enabled configured routes without constructing a provider adapter
/// or performing network discovery.
pub fn configured_local_models(
    provider_id: &str,
    configured: &ResolvedProviderConfig,
) -> Vec<Model> {
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();
    for route in configured.models.keys() {
        let Some((adapter_id, model_id)) = route.split_once('/') else {
            continue;
        };
        let adapter_id = adapter_id.trim();
        let model_id = model_id.trim();
        if adapter_id.is_empty()
            || model_id.is_empty()
            || !configured
                .adapters
                .get(adapter_id)
                .map(|adapter| adapter.enabled)
                .unwrap_or(false)
        {
            continue;
        }
        if !seen.insert((adapter_id.to_owned(), model_id.to_owned())) {
            continue;
        }
        models.push(configured_model(
            provider_id,
            adapter_id,
            model_id,
            configured,
        ));
    }

    models
}

/// Return enabled adapter IDs in stable order for catalog/provider adapters.
pub fn configured_enabled_adapter_ids(configured: &ResolvedProviderConfig) -> Vec<String> {
    let mut enabled_adapter_ids = configured
        .adapters
        .iter()
        .filter(|(_, adapter)| adapter.enabled)
        .map(|(adapter_id, _)| adapter_id.clone())
        .collect::<Vec<_>>();
    enabled_adapter_ids.sort();
    enabled_adapter_ids
}

fn configured_model(
    provider_id: &str,
    adapter_id: &str,
    model_id: &str,
    configured: &ResolvedProviderConfig,
) -> Model {
    let route = format!("{adapter_id}/{model_id}");
    Model {
        provider_id: ProviderId::new(provider_id),
        adapter_id: Some(agena_domain::AdapterId::new(adapter_id)),
        id: ModelId::new(model_id),
        catalog_model_id: None,
        display_name: None,
        native_compaction: configured
            .models
            .get(route.as_str())
            .map(|model| model.native_compaction)
            .unwrap_or(true),
        capabilities: ModelCapabilities::default(),
        metadata: agena_domain::ModelMetadata::default(),
        thinking_modes: Vec::new(),
        speed_modes: std::collections::BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::configured_local_models;
    use agena_provider::ProviderModelDiscoveryConfig;
    use agena_runtime_config::{
        OllamaProviderOptions, ProviderAdapterDefinition, ResolvedProviderAdapterConfig,
        ResolvedProviderConfig,
    };
    use std::collections::BTreeMap;

    #[test]
    fn configured_local_models_keep_enabled_routes() {
        let adapter = ResolvedProviderAdapterConfig {
            enabled: true,
            model_discovery: ProviderModelDiscoveryConfig::ConfiguredOnly,
            definition: ProviderAdapterDefinition::Ollama(OllamaProviderOptions { base_url: None }),
        };
        let config = ResolvedProviderConfig {
            enabled: true,
            auth: agena_runtime_config::ProviderAuthConfig::None,
            network: Default::default(),
            adapters: BTreeMap::from([(String::from("local"), adapter)]),
            models: BTreeMap::new(),
        };

        let models = configured_local_models("ollama", &config);
        assert!(models.is_empty());
    }
}
