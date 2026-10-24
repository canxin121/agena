use std::collections::BTreeMap;

use agena_domain::ProviderId;

use crate::{
    CatalogModelDefinition, ProviderModelSource, catalog_definition_from_model,
    merge_catalog_definition,
};

/// Collects live model definitions from provider adapters in concrete-layer
/// priority order and merges duplicate model IDs through the provider contract.
///
/// Fetching public sources, curation, persistence, and refresh policy remain
/// composition concerns outside this dependency-light provider collector.
pub async fn collect_live_provider_models(
    providers: &dyn ProviderModelSource,
    priority: impl Fn(&ProviderId) -> i32,
) -> (BTreeMap<String, CatalogModelDefinition>, Vec<String>, usize) {
    let mut provider_ids = providers.provider_ids();
    provider_ids.sort_by(|left, right| {
        priority(right)
            .cmp(&priority(left))
            .then_with(|| left.cmp(right))
    });

    let mut models = BTreeMap::new();
    let mut errors = Vec::new();
    let mut succeeded = 0_usize;
    for provider_id in provider_ids {
        match providers.list_models(&provider_id).await {
            Ok(provider_models) => {
                succeeded += 1;
                for model in provider_models {
                    if model.id.as_ref().trim().is_empty() {
                        continue;
                    }
                    let definition = catalog_definition_from_model(&model);
                    models
                        .entry(model.id.to_string())
                        .and_modify(|current| merge_catalog_definition(current, &definition))
                        .or_insert(definition);
                }
            }
            Err(error) => errors.push(format!("{provider_id}: {error}")),
        }
    }
    (models, errors, succeeded)
}
