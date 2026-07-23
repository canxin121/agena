use std::collections::BTreeMap;

use agena_provider::{
    CatalogModelDefinition, ModelCatalogDocument, enrich_catalog_document_thinking_modes,
    merge_live_provider_catalog_document,
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
    let mut document = if has_live_provider_models {
        curate_live_catalog_document(merged)?
    } else {
        curate_catalog_document(merged)?
    };
    enrich_catalog_document_thinking_modes(&mut document);
    Ok((
        document,
        (!warnings.is_empty()).then(|| warnings.join("; ")),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use agena_provider::{CatalogModelDefinition, ModelCatalogDocument};

    use super::{ModelCatalogPublicSourceResult, compose_model_catalog_document};

    #[test]
    fn public_success_survives_a_live_provider_failure_as_a_warning() {
        let public = ModelCatalogPublicSourceResult {
            models: BTreeMap::from([("gpt-4o".to_owned(), CatalogModelDefinition::default())]),
            warnings: Vec::new(),
            succeeded: 1,
        };
        let (document, warning) = compose_model_catalog_document(
            public,
            Err(crate::LiveProviderCatalogBuildError::Collection(
                "provider: unavailable".to_owned(),
            )),
        )
        .expect("public source should be retained");

        assert!(document.models.contains_key("gpt-4o"));
        assert!(
            warning
                .as_deref()
                .is_some_and(|value| value.contains("provider: unavailable"))
        );
    }

    #[test]
    fn empty_sources_report_a_generation_failure() {
        let error = compose_model_catalog_document(
            ModelCatalogPublicSourceResult {
                models: BTreeMap::new(),
                warnings: Vec::new(),
                succeeded: 0,
            },
            Ok((None::<ModelCatalogDocument>, None)),
        )
        .expect_err("no successful sources must fail");
        assert!(
            error
                .to_string()
                .contains("no public catalog sources or live provider model lists succeeded")
        );
    }
}
