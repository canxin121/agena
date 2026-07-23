use std::{collections::BTreeMap, sync::Arc};

use agena_provider::{
    CatalogDefinitionSourcePriority, CatalogModelDefinition, ModelCatalogDocument,
    merge_public_source_catalog_document,
};
use async_trait::async_trait;
use futures_util::future::join_all;

use crate::{ModelCatalogPublicSource, ModelCatalogPublicSourceResult};

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

/// Default public model-catalog endpoints used by runtime composition. URL
/// fetching and response parsing remain concrete adapter behavior.
pub fn default_public_model_catalog_sources() -> Vec<ModelCatalogRemoteSource> {
    vec![
        ModelCatalogRemoteSource::new(
            "models.dev",
            ModelCatalogRemoteSourceKind::ModelsDev,
            [String::from("https://models.dev/api.json")],
        ),
        ModelCatalogRemoteSource::new(
            "openai-codex-models",
            ModelCatalogRemoteSourceKind::OpenAiCodexModels,
            [String::from(
                "https://raw.githubusercontent.com/openai/codex/main/codex-rs/models-manager/models.json",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "router-for-me",
            ModelCatalogRemoteSourceKind::RouterForMe,
            [
                String::from("https://models.router-for.me/models.json"),
                String::from(
                    "https://raw.githubusercontent.com/router-for-me/models/refs/heads/main/models.json",
                ),
            ],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-google-gemma",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=google&search=gemma&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-google-codegemma",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=google&search=codegemma&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-google-recurrentgemma",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=google&search=recurrentgemma&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-google-deplot",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=google&search=deplot&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-mistralai-mixtral",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=mistralai&search=mixtral&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-mistralai-codestral",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=mistralai&search=codestral&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-mistralai-mistral-large",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=mistralai&search=Mistral-Large-Instruct-2407&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-meta-llama2",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=meta-llama&search=Llama-2-&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-codellama",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=codellama&search=CodeLlama-&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-ibm-granite",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=ibm-granite&search=granite-&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-microsoft-phi-vision",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=microsoft&search=Phi-3-vision-128k-instruct&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-microsoft-kosmos",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=microsoft&search=Kosmos-2&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-bigcode-starcoder2",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=bigcode&search=starcoder2&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-snowflake-arctic-embed",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=Snowflake&search=snowflake-arctic-embed-l&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-adept-fuyu",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=adept&search=fuyu&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-aisingapore-sea-lion",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=aisingapore&search=SEA-LION-v1-7B-IT&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-stockmark-2",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=stockmark&search=Stockmark-2-100B-Instruct&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-zyphra-zamba2",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=Zyphra&search=Zamba2-7B-Instruct&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-deepseek-coder",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=deepseek-ai&search=deepseek-coder-6.7b-instruct&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-deepseek-v3.1",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=deepseek-ai&search=DeepSeek-V3.1&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-ai21-jamba",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=ai21labs&search=AI21-Jamba-&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-writer-palmyra-med",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=Writer&search=Palmyra-Med-70B&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-writer-palmyra-fin",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=Writer&search=Palmyra-Fin-70B-32K&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-writer-palmyra-creative",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=Writer&search=Palmyra-Creative&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-nemotron",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=Llama-3_1-Nemotron&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-nemotron-70b",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=Llama-3.1-Nemotron-70B-Instruct&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-nemotron-4",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=Nemotron-4-340B&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-nemoguard",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=Nemoguard&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-embed-vl",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=embed-vl-1b-v2&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-embed",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=llama-nemotron-embed-1b-v2&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-chatqa",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=ChatQA-1.5-70B&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-cosmos-reason",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=Cosmos-Reason2-8B&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-minitron",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=Mistral-NeMo-Minitron-8B-Instruct&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-nemotron-nano",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=NVIDIA-Nemotron-3-Nano-30B-A3B-BF16&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-nemotron-parse",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=Nemotron-Parse&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "huggingface-nvidia-riva-translate",
            ModelCatalogRemoteSourceKind::HuggingFaceOfficial,
            [String::from(
                "https://huggingface.co/api/models?author=nvidia&search=Riva-Translate-4B-Instruct&limit=100",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "nvidia-nim-reference-models",
            ModelCatalogRemoteSourceKind::OfficialHtmlSignals,
            [String::from(
                "https://docs.api.nvidia.com/nim/reference/models-1",
            )],
        ),
        ModelCatalogRemoteSource::new(
            "nvidia-build-synthetic-video-detector",
            ModelCatalogRemoteSourceKind::OfficialHtmlSignals,
            [String::from(
                "https://build.nvidia.com/nvidia/synthetic-video-detector/modelcard",
            )],
        ),
    ]
}

/// Concrete remote-source parser family used by runtime catalog composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCatalogRemoteSourceKind {
    ModelsDev,
    OpenAiCodexModels,
    HuggingFaceOfficial,
    OfficialHtmlSignals,
    RouterForMe,
}

impl ModelCatalogRemoteSourceKind {
    pub const fn priority(self) -> u8 {
        match self {
            Self::ModelsDev => 4,
            Self::OpenAiCodexModels => 3,
            Self::HuggingFaceOfficial => 2,
            Self::OfficialHtmlSignals => 0,
            Self::RouterForMe => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCatalogRemoteSourceTier {
    OfficialStructured,
    CuratedStructured,
    OfficialRegistry,
    CuratedAggregator,
    OfficialHtmlSignal,
}

/// Concrete source-quality metadata used while ordering and merging a refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCatalogRemoteSourceGrade {
    pub tier: ModelCatalogRemoteSourceTier,
    pub sort_priority: i32,
    pub descriptive_priority: i32,
    pub limits_priority: i32,
    pub capability_priority: i32,
    pub semantics_priority: i32,
    pub pricing_priority: i32,
    pub mode_priority: i32,
}

impl ModelCatalogRemoteSourceGrade {
    pub fn definition_priority(self) -> CatalogDefinitionSourcePriority {
        CatalogDefinitionSourcePriority {
            sort_priority: self.sort_priority,
            descriptive_priority: self.descriptive_priority,
            limits_priority: self.limits_priority,
            capability_priority: self.capability_priority,
            semantics_priority: self.semantics_priority,
            pricing_priority: self.pricing_priority,
            mode_priority: self.mode_priority,
        }
    }
}

/// Default quality policy for one configured public source. URL selection and
/// fetching stay with the concrete source adapter; this pure policy belongs to
/// runtime composition alongside the source kind/tier values.
pub fn default_model_catalog_source_grade(
    source_name: &str,
    kind: ModelCatalogRemoteSourceKind,
) -> ModelCatalogRemoteSourceGrade {
    let normalized = source_name.trim().to_ascii_lowercase();
    match kind {
        ModelCatalogRemoteSourceKind::OpenAiCodexModels => ModelCatalogRemoteSourceGrade {
            tier: ModelCatalogRemoteSourceTier::OfficialStructured,
            sort_priority: 900,
            descriptive_priority: 700,
            limits_priority: 850,
            capability_priority: 900,
            semantics_priority: 1_000,
            pricing_priority: 100,
            mode_priority: 1_000,
        },
        ModelCatalogRemoteSourceKind::ModelsDev if normalized == "models.dev" => {
            ModelCatalogRemoteSourceGrade {
                tier: ModelCatalogRemoteSourceTier::CuratedStructured,
                sort_priority: 950,
                descriptive_priority: 950,
                limits_priority: 950,
                capability_priority: 950,
                semantics_priority: 825,
                pricing_priority: 1_000,
                mode_priority: 900,
            }
        }
        ModelCatalogRemoteSourceKind::RouterForMe => ModelCatalogRemoteSourceGrade {
            tier: ModelCatalogRemoteSourceTier::CuratedAggregator,
            sort_priority: 650,
            descriptive_priority: 550,
            limits_priority: 800,
            capability_priority: 750,
            semantics_priority: 500,
            pricing_priority: 0,
            mode_priority: 850,
        },
        ModelCatalogRemoteSourceKind::HuggingFaceOfficial => ModelCatalogRemoteSourceGrade {
            tier: ModelCatalogRemoteSourceTier::OfficialRegistry,
            sort_priority: 600,
            descriptive_priority: 700,
            limits_priority: 0,
            capability_priority: 550,
            semantics_priority: 0,
            pricing_priority: 0,
            mode_priority: 0,
        },
        ModelCatalogRemoteSourceKind::OfficialHtmlSignals => ModelCatalogRemoteSourceGrade {
            tier: ModelCatalogRemoteSourceTier::OfficialHtmlSignal,
            sort_priority: 400,
            descriptive_priority: 300,
            limits_priority: 975,
            capability_priority: 250,
            semantics_priority: 0,
            pricing_priority: 0,
            mode_priority: 0,
        },
        ModelCatalogRemoteSourceKind::ModelsDev => ModelCatalogRemoteSourceGrade {
            tier: ModelCatalogRemoteSourceTier::CuratedStructured,
            sort_priority: 900,
            descriptive_priority: 900,
            limits_priority: 900,
            capability_priority: 900,
            semantics_priority: 800,
            pricing_priority: 950,
            mode_priority: 850,
        },
    }
}

/// One configured public model-catalog source. Runtime owns this concrete
/// composition value; URL fetching and response parsing remain adapter work.
#[derive(Debug, Clone)]
pub struct ModelCatalogRemoteSource {
    pub name: String,
    pub kind: ModelCatalogRemoteSourceKind,
    pub grade: ModelCatalogRemoteSourceGrade,
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
            grade: default_model_catalog_source_grade(name.as_str(), kind),
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
    pub grade: ModelCatalogRemoteSourceGrade,
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
    grade: ModelCatalogRemoteSourceGrade,
) -> ModelCatalogDocument {
    let base = grade.definition_priority();
    for definition in document.models.values_mut() {
        definition.source_priority = source_priority_for_definition(definition, &base);
    }
    document
}

fn source_priority_for_definition(
    definition: &CatalogModelDefinition,
    base: &CatalogDefinitionSourcePriority,
) -> CatalogDefinitionSourcePriority {
    CatalogDefinitionSourcePriority {
        sort_priority: if definition.is_empty() {
            0
        } else {
            base.sort_priority
        },
        descriptive_priority: if definition_has_descriptive_fields(definition) {
            base.descriptive_priority
        } else {
            0
        },
        limits_priority: if definition_has_limit_fields(definition) {
            base.limits_priority
        } else {
            0
        },
        capability_priority: if definition_has_capability_fields(definition) {
            base.capability_priority
        } else {
            0
        },
        semantics_priority: if definition_has_semantic_fields(definition) {
            base.semantics_priority
        } else {
            0
        },
        pricing_priority: if definition.pricing.is_some() {
            base.pricing_priority
        } else {
            0
        },
        mode_priority: if definition_has_mode_fields(definition) {
            base.mode_priority
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

    use agena_provider::{
        CatalogDefinitionSourcePriority, CatalogModelDefinition, ModelCatalogDocument,
    };

    use super::{
        ModelCatalogConfiguredPublicSource, ModelCatalogFetchedDocument,
        ModelCatalogRemoteDocumentFetcher, ModelCatalogRemoteSource, ModelCatalogRemoteSourceGrade,
        ModelCatalogRemoteSourceKind, ModelCatalogRemoteSourceTier,
        default_model_catalog_source_grade, fetch_public_model_catalog_documents,
        merge_public_model_catalog_documents, public_model_catalog_sources_enabled,
    };
    use crate::ModelCatalogPublicSource;

    #[test]
    fn canonical_models_dev_source_receives_the_curated_top_priority() {
        let grade = default_model_catalog_source_grade(
            " models.dev ",
            ModelCatalogRemoteSourceKind::ModelsDev,
        );
        assert_eq!(grade.tier, ModelCatalogRemoteSourceTier::CuratedStructured);
        assert_eq!(grade.sort_priority, 950);
        assert_eq!(grade.pricing_priority, 1_000);
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
    fn merge_orders_documents_by_runtime_grade_before_applying_provider_merge() {
        let mut preferred = CatalogModelDefinition {
            description: Some("preferred".to_owned()),
            ..Default::default()
        };
        preferred.source_priority = CatalogDefinitionSourcePriority {
            descriptive_priority: 100,
            ..Default::default()
        };
        let mut fallback = CatalogModelDefinition {
            description: Some("fallback".to_owned()),
            ..Default::default()
        };
        fallback.source_priority = CatalogDefinitionSourcePriority {
            descriptive_priority: 10,
            ..Default::default()
        };
        let result = merge_public_model_catalog_documents(
            vec![
                ModelCatalogFetchedDocument {
                    name: "fallback".to_owned(),
                    kind: ModelCatalogRemoteSourceKind::ModelsDev,
                    grade: ModelCatalogRemoteSourceGrade {
                        sort_priority: 10,
                        ..default_model_catalog_source_grade(
                            "fallback",
                            ModelCatalogRemoteSourceKind::ModelsDev,
                        )
                    },
                    document: ModelCatalogDocument {
                        models: BTreeMap::from([("model".to_owned(), fallback)]),
                    },
                },
                ModelCatalogFetchedDocument {
                    name: "preferred".to_owned(),
                    kind: ModelCatalogRemoteSourceKind::ModelsDev,
                    grade: ModelCatalogRemoteSourceGrade {
                        sort_priority: 100,
                        ..default_model_catalog_source_grade(
                            "preferred",
                            ModelCatalogRemoteSourceKind::ModelsDev,
                        )
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
                ModelCatalogRemoteSourceKind::ModelsDev,
                ["https://example.invalid/available".to_owned()],
            ),
            ModelCatalogRemoteSource::new(
                "unavailable",
                ModelCatalogRemoteSourceKind::ModelsDev,
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
                .descriptive_priority,
            sources[0].grade.descriptive_priority
        );
    }

    #[tokio::test]
    async fn configured_public_source_composes_the_concrete_fetcher() {
        let source = ModelCatalogConfiguredPublicSource::new(
            Arc::new(FixtureFetcher),
            vec![ModelCatalogRemoteSource::new(
                "available",
                ModelCatalogRemoteSourceKind::ModelsDev,
                ["https://example.invalid/available".to_owned()],
            )],
        );
        let result = source.fetch().await;
        assert_eq!(result.succeeded, 1);
        assert!(result.models.contains_key("model"));
    }
}
