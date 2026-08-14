use std::{collections::BTreeMap, sync::Arc};

use agena_provider::{
    CatalogDefinitionSourcePriority, CatalogModelDefinition, ModelCatalogDocument,
    merge_public_source_catalog_document,
};
use async_trait::async_trait;
use futures_util::future::join_all;

use crate::{ModelCatalogPublicSource, ModelCatalogPublicSourceResult};

/// Default GitHub-hosted model catalog document. The catalog is maintained in
/// the `agena-model-catalog` repository as a single hand-curated JSON file;
/// the runtime consumes it directly instead of crawling registries at runtime.
pub const DEFAULT_GITHUB_CATALOG_URL: &str =
    "https://raw.githubusercontent.com/canxin121/agena-model-catalog/main/models.json";

/// Whether public model-catalog sources are enabled for this runtime process.
/// The concrete fetch adapter owns URL parsing; runtime owns this composition
/// policy and its environment override.
pub fn public_model_catalog_sources_enabled() -> bool {
    !std::env::var_os("AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES")
        .map(|value| {
            matches!(
                value.to_string_lossy().trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Default public model-catalog endpoint used by runtime composition. URL
/// fetching and response parsing remain concrete adapter behavior.
pub fn default_public_model_catalog_sources() -> Vec<ModelCatalogRemoteSource> {
    vec![ModelCatalogRemoteSource::new(
        "agena-github-catalog",
        ModelCatalogRemoteSourceKind::GithubCatalog,
        [DEFAULT_GITHUB_CATALOG_URL.to_owned()],
    )]
}

/// Concrete remote-source parser family used by runtime catalog composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCatalogRemoteSourceKind {
    GithubCatalog,
}

impl ModelCatalogRemoteSourceKind {
    pub const fn priority(self) -> u8 {
        match self {
            Self::GithubCatalog => 100,
        }
    }
}

/// One configured public model-catalog source. Runtime owns this concrete
/// composition value; URL fetching and response parsing remain adapter work.
#[derive(Debug, Clone)]
pub struct ModelCatalogRemoteSource {
    pub name: String,
    pub kind: ModelCatalogRemoteSourceKind,
    pub grade: CatalogDefinitionSourcePriority,
    pub urls: Vec<String>,
}

impl ModelCatalogRemoteSource {
    pub fn new(
        name: impl Into<String>,
        kind: ModelCatalogRemoteSourceKind,
        urls: impl IntoIterator<Item = String>,
    ) -> Self {
        let name = name.into();
        Self {
            // The maintained catalog is the single source of truth, so every
            // field carries the same full priority; curation still resolves
            // cross-document aliases when a caller supplies extra sources.
            grade: CatalogDefinitionSourcePriority {
                sort_priority: 1_000,
                descriptive_priority: 1_000,
                limits_priority: 1_000,
                capability_priority: 1_000,
                semantics_priority: 1_000,
                pricing_priority: 1_000,
                mode_priority: 1_000,
            },
            name,
            kind,
            urls: urls.into_iter().collect(),
        }
    }
}

/// Concrete HTTP/source adapters implement one-document retrieval and parsing.
/// Runtime owns collection, error aggregation, source-priority annotation, and
/// cross-source merge without taking a dependency on any parser or HTTP crate.
#[async_trait]
pub trait ModelCatalogRemoteDocumentFetcher: Send + Sync {
    async fn fetch_document(
        &self,
        source: &ModelCatalogRemoteSource,
    ) -> Result<ModelCatalogDocument, String>;
}

/// Runtime-owned public-source composition adapter. Concrete layers supply
/// only the HTTP/parser fetcher; source collection and document composition
/// stay with runtime.
#[derive(Clone)]
pub struct ModelCatalogConfiguredPublicSource {
    fetcher: Arc<dyn ModelCatalogRemoteDocumentFetcher>,
    sources: Vec<ModelCatalogRemoteSource>,
}

impl ModelCatalogConfiguredPublicSource {
    pub fn new(
        fetcher: Arc<dyn ModelCatalogRemoteDocumentFetcher>,
        sources: Vec<ModelCatalogRemoteSource>,
    ) -> Self {
        Self { fetcher, sources }
    }
}

#[async_trait]
impl ModelCatalogPublicSource for ModelCatalogConfiguredPublicSource {
    async fn fetch(&self) -> ModelCatalogPublicSourceResult {
        let (documents, warnings) =
            fetch_public_model_catalog_documents(self.fetcher.as_ref(), &self.sources).await;
        merge_public_model_catalog_documents(documents, warnings)
    }
}

/// One successfully fetched public catalog document. The HTTP adapter owns
/// retrieval and parsing; runtime owns cross-source ordering and merge
/// composition.
#[derive(Debug, Clone)]
pub struct ModelCatalogFetchedDocument {
    pub name: String,
    pub kind: ModelCatalogRemoteSourceKind,
    pub grade: CatalogDefinitionSourcePriority,
    pub document: ModelCatalogDocument,
}

/// Fetches every configured public source through the concrete adapter,
/// retaining successful documents and source-qualified warnings.
pub async fn fetch_public_model_catalog_documents(
    fetcher: &dyn ModelCatalogRemoteDocumentFetcher,
    sources: &[ModelCatalogRemoteSource],
) -> (Vec<ModelCatalogFetchedDocument>, Vec<String>) {
    let mut documents = Vec::new();
    let mut warnings = Vec::new();
    let results = join_all(
        sources
            .iter()
            .map(|source| async move { (source, fetcher.fetch_document(source).await) }),
    )
    .await;
    for (source, result) in results {
        match result {
            Ok(document) => documents.push(ModelCatalogFetchedDocument {
                name: source.name.clone(),
                kind: source.kind,
                grade: source.grade,
                document: annotate_document_source_priority(document, source.grade),
            }),
            Err(error) => warnings.push(format!("{}: {error}", source.name)),
        }
    }
    (documents, warnings)
}

fn annotate_document_source_priority(
    mut document: ModelCatalogDocument,
    grade: CatalogDefinitionSourcePriority,
) -> ModelCatalogDocument {
    for definition in document.models.values_mut() {
        definition.source_priority = source_priority_for_definition(definition, &grade);
    }
    document
}

fn source_priority_for_definition(
    definition: &CatalogModelDefinition,
    grade: &CatalogDefinitionSourcePriority,
) -> CatalogDefinitionSourcePriority {
    CatalogDefinitionSourcePriority {
        sort_priority: if definition.is_empty() {
            0
        } else {
            grade.sort_priority
        },
        descriptive_priority: if definition_has_descriptive_fields(definition) {
            grade.descriptive_priority
        } else {
            0
        },
        limits_priority: if definition_has_limit_fields(definition) {
            grade.limits_priority
        } else {
            0
        },
        capability_priority: if definition_has_capability_fields(definition) {
            grade.capability_priority
        } else {
            0
        },
        semantics_priority: if definition_has_semantic_fields(definition) {
            grade.semantics_priority
        } else {
            0
        },
        pricing_priority: if definition.pricing.is_some() {
            grade.pricing_priority
        } else {
            0
        },
        mode_priority: if definition_has_mode_fields(definition) {
            grade.mode_priority
        } else {
            0
        },
    }
}

fn definition_has_descriptive_fields(definition: &CatalogModelDefinition) -> bool {
    definition.lifecycle.is_some()
        || definition.description.is_some()
        || definition.knowledge_cutoff.is_some()
        || definition.release_date.is_some()
        || definition.last_updated.is_some()
        || definition.open_weights.is_some()
        || definition.display_name.is_some()
        || definition.origin.is_some()
}

fn definition_has_limit_fields(definition: &CatalogModelDefinition) -> bool {
    definition.context_window_tokens.is_some()
        || definition.max_input_tokens.is_some()
        || definition.max_output_tokens.is_some()
}

fn definition_has_capability_fields(definition: &CatalogModelDefinition) -> bool {
    !definition.output_modalities.is_empty() || !definition.capabilities.is_empty()
}

fn definition_has_semantic_fields(definition: &CatalogModelDefinition) -> bool {
    definition.supports_parallel_tool_calls.is_some()
        || definition.supports_verbosity.is_some()
        || definition.default_verbosity.is_some()
        || definition.default_temperature.is_some()
        || definition.default_top_p.is_some()
        || definition.default_top_k.is_some()
        || definition.assistant_reasoning_interleaved.is_some()
        || definition.assistant_reasoning_field.is_some()
}

fn definition_has_mode_fields(definition: &CatalogModelDefinition) -> bool {
    !definition.thinking_modes.is_empty() || !definition.speed_modes.is_empty()
}

/// Orders successfully fetched documents by their runtime source policy and
/// merges them into the public catalog result consumed by refresh composition.
pub fn merge_public_model_catalog_documents(
    mut documents: Vec<ModelCatalogFetchedDocument>,
    warnings: Vec<String>,
) -> ModelCatalogPublicSourceResult {
    documents.sort_by(|left, right| {
        right
            .grade
            .sort_priority
            .cmp(&left.grade.sort_priority)
            .then_with(|| right.kind.priority().cmp(&left.kind.priority()))
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut models = BTreeMap::new();
    for fetched in documents.iter() {
        merge_public_source_catalog_document(&mut models, fetched.document.clone());
    }
    ModelCatalogPublicSourceResult {
        models,
        warnings,
        succeeded: documents.len(),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use agena_provider::{CatalogModelDefinition, ModelCatalogDocument};

    use super::{
        ModelCatalogConfiguredPublicSource, ModelCatalogFetchedDocument,
        ModelCatalogRemoteDocumentFetcher, ModelCatalogRemoteSource, ModelCatalogRemoteSourceKind,
        default_public_model_catalog_sources, fetch_public_model_catalog_documents,
        merge_public_model_catalog_documents, public_model_catalog_sources_enabled,
    };
    use crate::ModelCatalogPublicSource;

    #[test]
    fn canonical_github_source_receives_full_priority() {
        let source = ModelCatalogRemoteSource::new(
            "github",
            ModelCatalogRemoteSourceKind::GithubCatalog,
            ["https://example.invalid/models.json".to_owned()],
        );
        assert_eq!(source.grade.sort_priority, 1_000);
        assert_eq!(source.grade.pricing_priority, 1_000);
    }

    #[test]
    fn public_source_toggle_defaults_to_enabled() {
        // This test intentionally does not mutate the process environment,
        // which would race with other runtime tests.
        if std::env::var_os("AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES").is_none() {
            assert!(public_model_catalog_sources_enabled());
        }
    }

    #[test]
    fn merge_keeps_the_highest_priority_definition() {
        let mut preferred = CatalogModelDefinition {
            description: Some("preferred".to_owned()),
            ..Default::default()
        };
        preferred.source_priority = super::CatalogDefinitionSourcePriority {
            sort_priority: 1_000,
            ..Default::default()
        };
        let mut fallback = CatalogModelDefinition {
            description: Some("fallback".to_owned()),
            ..Default::default()
        };
        fallback.source_priority = super::CatalogDefinitionSourcePriority {
            sort_priority: 100,
            ..Default::default()
        };
        let result = merge_public_model_catalog_documents(
            vec![
                ModelCatalogFetchedDocument {
                    name: "fallback".to_owned(),
                    kind: ModelCatalogRemoteSourceKind::GithubCatalog,
                    grade: super::CatalogDefinitionSourcePriority {
                        sort_priority: 100,
                        ..Default::default()
                    },
                    document: ModelCatalogDocument {
                        models: BTreeMap::from([("model".to_owned(), fallback)]),
                    },
                },
                ModelCatalogFetchedDocument {
                    name: "preferred".to_owned(),
                    kind: ModelCatalogRemoteSourceKind::GithubCatalog,
                    grade: super::CatalogDefinitionSourcePriority {
                        sort_priority: 1_000,
                        ..Default::default()
                    },
                    document: ModelCatalogDocument {
                        models: BTreeMap::from([("model".to_owned(), preferred)]),
                    },
                },
            ],
            vec!["source warning".to_owned()],
        );
        assert_eq!(result.succeeded, 2);
        assert_eq!(result.warnings, ["source warning"]);
        assert_eq!(
            result.models["model"].description.as_deref(),
            Some("preferred")
        );
    }

    struct FixtureFetcher;

    #[async_trait::async_trait]
    impl ModelCatalogRemoteDocumentFetcher for FixtureFetcher {
        async fn fetch_document(
            &self,
            source: &ModelCatalogRemoteSource,
        ) -> Result<ModelCatalogDocument, String> {
            if source.name == "unavailable" {
                return Err("offline".to_owned());
            }
            Ok(ModelCatalogDocument {
                models: BTreeMap::from([(
                    "model".to_owned(),
                    CatalogModelDefinition {
                        description: Some("available".to_owned()),
                        ..Default::default()
                    },
                )]),
            })
        }
    }

    #[tokio::test]
    async fn collection_owns_warning_aggregation_and_priority_annotation() {
        let sources = vec![
            ModelCatalogRemoteSource::new(
                "available",
                ModelCatalogRemoteSourceKind::GithubCatalog,
                ["https://example.invalid/available".to_owned()],
            ),
            ModelCatalogRemoteSource::new(
                "unavailable",
                ModelCatalogRemoteSourceKind::GithubCatalog,
                ["https://example.invalid/unavailable".to_owned()],
            ),
        ];
        let (documents, warnings) =
            fetch_public_model_catalog_documents(&FixtureFetcher, &sources).await;

        assert_eq!(documents.len(), 1);
        assert_eq!(warnings, ["unavailable: offline"]);
        assert_eq!(
            documents[0].document.models["model"]
                .source_priority
                .sort_priority,
            sources[0].grade.sort_priority
        );
    }

    #[tokio::test]
    async fn configured_public_source_composes_the_concrete_fetcher() {
        let source = ModelCatalogConfiguredPublicSource::new(
            Arc::new(FixtureFetcher),
            vec![ModelCatalogRemoteSource::new(
                "available",
                ModelCatalogRemoteSourceKind::GithubCatalog,
                ["https://example.invalid/available".to_owned()],
            )],
        );
        let result = source.fetch().await;
        assert_eq!(result.succeeded, 1);
        assert!(result.models.contains_key("model"));
    }

    #[test]
    fn default_sources_point_at_the_github_catalog() {
        let sources = default_public_model_catalog_sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name, "agena-github-catalog");
        assert_eq!(sources[0].kind, ModelCatalogRemoteSourceKind::GithubCatalog);
        assert_eq!(sources[0].urls, [super::DEFAULT_GITHUB_CATALOG_URL]);
    }
}
