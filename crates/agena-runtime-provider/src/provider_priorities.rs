use std::collections::BTreeMap;

use agena_provider::{ProviderCapabilityFamilyConfig, ProviderModelPriorities};

use agena_runtime_config::{ProviderAdapterDefinition, ResolvedProviderConfig};

/// Resolve the provider ranking policy used by live model-catalog refresh.
///
/// The calculation only interprets Runtime-owned provider configuration and
/// returns the provider-owned priority contract.  Keeping it here prevents a
/// concrete configuration registry helper from becoming an accidental catalog
/// policy boundary.
pub fn provider_model_catalog_priorities(
    providers: &BTreeMap<String, ResolvedProviderConfig>,
) -> ProviderModelPriorities {
    ProviderModelPriorities::new(
        providers
            .iter()
            .map(|(provider_id, provider)| {
                let priority = provider
                    .adapters
                    .values()
                    .filter(|adapter| adapter.enabled)
                    .map(|adapter| match &adapter.definition {
                        ProviderAdapterDefinition::Anthropic(_)
                        | ProviderAdapterDefinition::Gemini(_) => 500,
                        ProviderAdapterDefinition::OpenAiResponses(config) => {
                            provider_capability_family_priority(
                                config.options.capability_family.as_ref(),
                            )
                        }
                        ProviderAdapterDefinition::OpenAiChatCompletions(config) => {
                            provider_capability_family_priority(
                                config.options.capability_family.as_ref(),
                            )
                        }
                        ProviderAdapterDefinition::OpenAiRealtime(config) => {
                            provider_capability_family_priority(
                                config.options.capability_family.as_ref(),
                            )
                        }
                        ProviderAdapterDefinition::AmazonBedrock(_) => 200,
                        ProviderAdapterDefinition::Gitlab(_) => 150,
                        ProviderAdapterDefinition::Ollama(_) => 50,
                    })
                    .max()
                    .unwrap_or_default();
                (provider_id.clone(), priority)
            })
            .collect(),
    )
}

const fn provider_capability_family_priority(
    family: Option<&ProviderCapabilityFamilyConfig>,
) -> i32 {
    match family {
        Some(ProviderCapabilityFamilyConfig::OpenAi)
        | Some(ProviderCapabilityFamilyConfig::OpenAiCompatible)
        | None => 450,
        Some(ProviderCapabilityFamilyConfig::Anthropic)
        | Some(ProviderCapabilityFamilyConfig::Gemini) => 350,
        Some(ProviderCapabilityFamilyConfig::Bedrock)
        | Some(ProviderCapabilityFamilyConfig::Gitlab) => 200,
    }
}

#[cfg(test)]
mod tests {
    use super::provider_model_catalog_priorities;

    #[test]
    fn empty_provider_configuration_has_no_priorities() {
        let priorities = provider_model_catalog_priorities(&Default::default());
        assert!(priorities.is_empty());
    }
}
