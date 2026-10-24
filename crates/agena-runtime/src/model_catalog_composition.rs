use std::collections::BTreeMap;

use agena_provider::{
    CatalogModelDefinition, ModelCatalogDocument, merge_live_provider_catalog_document,
};

use crate::{
    LiveProviderCatalogBuildError, ModelCatalogCurationError, curate_catalog_document,
    curate_live_catalog_document,
};

/// Public-source fetch output supplied by a concrete HTTP adapter.
pub struct ModelCatalogPublicSourceResult {
    pub models: BTreeMap<String, CatalogModelDefinition>,
    pub warnings: Vec<String>,
    pub succeeded: usize,
}

/// Failure while merging public and live catalog source results.
#[derive(Debug, thiserror::Error)]
pub enum ModelCatalogCompositionError {
    #[error(transparent)]
    LiveProvider(#[from] LiveProviderCatalogBuildError),
    #[error("model catalog generation failed: {0}")]
    Generation(String),
    #[error("curate generated model catalog: {0}")]
    Curation(#[from] ModelCatalogCurationError),
}

/// Merges public-source and live-provider results into the generated catalog.
pub fn compose_model_catalog_document(
    public: ModelCatalogPublicSourceResult,
    live: Result<(Option<ModelCatalogDocument>, Option<String>), LiveProviderCatalogBuildError>,
) -> Result<(ModelCatalogDocument, Option<String>), ModelCatalogCompositionError> {
    let mut merged_models = public.models;
    let mut warnings = public.warnings;
    let mut succeeded = public.succeeded;
    let mut has_live_provider_models = false;

    match live {
        Ok((Some(document), warning)) => {
            merge_live_provider_catalog_document(&mut merged_models, document);
            has_live_provider_models = true;
            succeeded += 1;
            if let Some(warning) = warning {
                warnings.push(warning);
            }
        }
        Ok((None, warning)) => {
            if let Some(warning) = warning {
                warnings.push(warning);
            }
        }
        Err(error) => {
            warnings.push(format!("live provider model list: {error}"));
            if succeeded == 0 {
                return Err(ModelCatalogCompositionError::LiveProvider(error));
            }
        }
    }

    if succeeded == 0 {
        let detail = if warnings.is_empty() {
            "no public catalog sources or live provider model lists succeeded".to_owned()
        } else {
            warnings.join("; ")
        };
        return Err(ModelCatalogCompositionError::Generation(detail));
    }

    let merged = ModelCatalogDocument {
        models: merged_models,
    };
    let document = if has_live_provider_models {
        curate_live_catalog_document(merged)?
    } else {
        curate_catalog_document(merged)?
    };
    Ok((
        document,
        (!warnings.is_empty()).then(|| warnings.join("; ")),
    ))
}
