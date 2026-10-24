use agena_provider::{
    ModelCatalogDocument, ProviderModelPriorities, ProviderModelSource,
    collect_live_provider_models,
};

use crate::{ModelCatalogCurationError, curate_live_catalog_document};

/// Failure while producing a catalog document from live provider model lists.
#[derive(Debug, thiserror::Error)]
pub enum LiveProviderCatalogBuildError {
    #[error("live provider model list failed: {0}")]
    Collection(String),
    #[error("curate live provider model catalog: {0}")]
    Curation(#[from] ModelCatalogCurationError),
}

/// Builds a curated catalog document from live provider adapters.
///
/// The caller supplies configuration-derived priorities, while collection,
/// curation, warnings, and partial-success behavior remain runtime-owned.
pub async fn build_live_provider_catalog_document(
    providers: &dyn ProviderModelSource,
    provider_priorities: Option<&ProviderModelPriorities>,
) -> Result<(Option<ModelCatalogDocument>, Option<String>), LiveProviderCatalogBuildError> {
    let (raw_models, errors, succeeded) = collect_live_provider_models(providers, |provider_id| {
        provider_priorities
            .map(|priorities| priorities.get(provider_id.as_ref()))
            .unwrap_or_default()
    })
    .await;

    if succeeded == 0 {
        let detail = if errors.is_empty() {
            return Ok((None, None));
        } else {
            errors.join("; ")
        };
        return Err(LiveProviderCatalogBuildError::Collection(detail));
    }

    let document = curate_live_catalog_document(ModelCatalogDocument { models: raw_models })?;
    let warning = (!errors.is_empty()).then(|| {
        format!(
            "live provider model lists generated catalog from {succeeded} provider(s); skipped {} provider(s): {}",
            errors.len(),
            errors.join("; ")
        )
    });
    Ok((Some(document), warning))
}
