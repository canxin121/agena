use std::collections::BTreeMap;

use crate::{
    AppError,
    model::{
        CapabilitySupport, ModelInputModality, ModelLifecycle, ModelPricing, ModelPricingTier,
        ModelSpeedModeRequestOverride,
    },
    provider::{
        CapabilitySelectionPatch, ConfiguredModelSpeedMode, ConfiguredModelThinkingMode,
        ModelCapabilityFeature, ModelCapabilityPatch, ReasoningEffort, ThinkingRequest,
    },
};
use futures_util::{StreamExt, future::join_all, stream};
use regex::Regex;
use serde::Deserialize;
use std::sync::OnceLock;

use super::{
    CatalogDefinitionSourcePriority, CatalogModelDefinition, ModelCatalogDocument,
    canonical_model_catalog_id, merge_catalog_definition,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCatalogRemoteSourceKind {
    ModelsDev,
    OpenAiCodexModels,
    HuggingFaceOfficial,
    OfficialHtmlSignals,
    RouterForMe,
}

impl ModelCatalogRemoteSourceKind {
    pub fn priority(self) -> u8 {
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
            grade: default_source_grade(name.as_str(), kind),
            name,
            kind,
            urls: urls.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FetchedModelCatalogDocument {
    pub name: String,
    pub kind: ModelCatalogRemoteSourceKind,
    pub grade: ModelCatalogRemoteSourceGrade,
    pub document: ModelCatalogDocument,
}

pub fn default_public_sources() -> Vec<ModelCatalogRemoteSource> {
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

pub async fn fetch_documents(
    client: &reqwest::Client,
    sources: &[ModelCatalogRemoteSource],
) -> (Vec<FetchedModelCatalogDocument>, Vec<String>) {
    let mut documents = Vec::new();
    let mut warnings = Vec::new();

    let results = join_all(
        sources
            .iter()
            .map(|source| async move { (source, fetch_source_document(client, source).await) }),
    )
    .await;

    for (source, result) in results {
        match result {
            Ok(document) => documents.push(FetchedModelCatalogDocument {
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

async fn fetch_source_document(
    client: &reqwest::Client,
    source: &ModelCatalogRemoteSource,
) -> Result<ModelCatalogDocument, AppError> {
    let mut last_error = None;

    for url in &source.urls {
        match fetch_and_parse_source_document(client, source.kind, url).await {
            Ok(document) => return Ok(document),
            Err(error) => last_error = Some(format!("{url}: {error}")),
        }
    }

    Err(AppError::Config(format!(
        "all source URLs failed for {}: {}",
        source.name,
        last_error.unwrap_or_else(|| "no URLs configured".to_owned())
    )))
}

async fn fetch_and_parse_source_document(
    client: &reqwest::Client,
    kind: ModelCatalogRemoteSourceKind,
    url: &str,
) -> Result<ModelCatalogDocument, AppError> {
    let response = client.get(url).send().await?.error_for_status()?;
    let body = response.text().await?;
    match kind {
        ModelCatalogRemoteSourceKind::ModelsDev => parse_models_dev_document(body.as_str()),
        ModelCatalogRemoteSourceKind::OpenAiCodexModels => {
            parse_openai_codex_models_document(body.as_str())
        }
        ModelCatalogRemoteSourceKind::HuggingFaceOfficial => {
            parse_hugging_face_official_document(body.as_str())
        }
        ModelCatalogRemoteSourceKind::OfficialHtmlSignals => {
            parse_official_html_source_document(client, url, body.as_str()).await
        }
        ModelCatalogRemoteSourceKind::RouterForMe => parse_router_document(body.as_str()),
    }
}

async fn parse_official_html_source_document(
    client: &reqwest::Client,
    url: &str,
    body: &str,
) -> Result<ModelCatalogDocument, AppError> {
    let mut document = parse_official_html_signals_document(body)?;

    if !is_nvidia_reference_index_url(url) {
        return Ok(document);
    }

    let pages = parse_official_html_reference_index_document(body)?;
    if pages.is_empty() {
        return Ok(document);
    }

    let results = stream::iter(pages.into_iter().map(|page| async move {
        fetch_official_html_reference_page_document(client, page).await
    }))
    .buffer_unordered(24)
    .collect::<Vec<_>>()
    .await;

    for result in results {
        let Ok(Some((aliases, definition))) = result else {
            continue;
        };
        for alias in aliases {
            merge_document_entry(&mut document.models, alias, definition.clone());
        }
    }

    Ok(document)
}

fn parse_models_dev_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let providers: BTreeMap<String, ModelsDevProvider> = serde_json::from_str(body)?;
    let mut providers: Vec<_> = providers.into_iter().collect();
    providers.sort_by(|(left_key, left), (right_key, right)| {
        models_dev_provider_rank(right_key, right)
            .cmp(&models_dev_provider_rank(left_key, left))
            .then_with(|| left_key.cmp(right_key))
    });

    let mut document_models = BTreeMap::new();

    for (provider_key, provider) in providers {
        let origin = models_dev_origin(provider_key.as_str(), &provider);
        let adapter_id = models_dev_adapter_id(provider_key.as_str(), &provider);
        let mut models: Vec<_> = provider.models.into_iter().collect();
        models.sort_by(|(left, _), (right, _)| left.cmp(right));

        for (fallback_model_id, model) in models {
            let model_id = normalize_optional_string(model.id)
                .unwrap_or_else(|| fallback_model_id.trim().to_owned());
            if model_id.is_empty() {
                continue;
            }

            let definition = CatalogModelDefinition {
                lifecycle: parse_models_dev_lifecycle(model.status.as_deref()),
                context_window_tokens: model
                    .limit
                    .as_ref()
                    .and_then(|limits| limits.context.or(limits.input))
                    .map(clamp_u64_to_u32),
                max_input_tokens: model
                    .limit
                    .as_ref()
                    .and_then(|limits| limits.input)
                    .map(clamp_u64_to_u32),
                max_output_tokens: model
                    .limit
                    .as_ref()
                    .and_then(|limits| limits.output)
                    .map(clamp_u64_to_u32),
                description: normalize_optional_string(model.description),
                knowledge_cutoff: normalize_optional_string(model.knowledge),
                release_date: normalize_optional_string(model.release_date),
                last_updated: normalize_optional_string(model.last_updated),
                open_weights: model.open_weights,
                default_thinking_mode: None,
                supports_parallel_tool_calls: None,
                supports_verbosity: None,
                default_verbosity: None,
                default_temperature: None,
                default_top_p: None,
                default_top_k: None,
                assistant_reasoning_interleaved: models_dev_assistant_reasoning_interleaved(
                    model.interleaved.as_ref(),
                ),
                assistant_reasoning_field: models_dev_assistant_reasoning_field(
                    model.interleaved.as_ref(),
                ),
                output_modalities: models_dev_output_modalities(model.modalities.as_ref()),
                pricing: models_dev_pricing(model.cost.as_ref()),
                display_name: normalize_optional_string(model.name),
                origin: origin.clone(),
                thinking_modes: BTreeMap::new(),
                speed_modes: models_dev_speed_modes(
                    model.experimental.as_ref(),
                    adapter_id.as_deref(),
                ),
                capabilities: model_capability_patch(
                    models_dev_input_support(model.modalities.as_ref(), model.attachment),
                    features_from_bool_flags(&[
                        (ModelCapabilityFeature::Reasoning, model.reasoning),
                        (ModelCapabilityFeature::ToolCalling, model.tool_call),
                        (
                            ModelCapabilityFeature::StructuredOutput,
                            model.structured_output,
                        ),
                        (ModelCapabilityFeature::Temperature, model.temperature),
                    ]),
                ),
                source_priority: CatalogDefinitionSourcePriority::default(),
            };

            merge_document_entry(&mut document_models, model_id, definition);
        }
    }

    Ok(ModelCatalogDocument {
        models: document_models,
    })
}

fn parse_router_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let sections: BTreeMap<String, Vec<RouterModel>> = serde_json::from_str(body)?;
    let mut document_models = BTreeMap::new();

    for (section, models) in sections {
        for model in models {
            let model_id = normalize_optional_string(model.id.clone())
                .or_else(|| normalize_optional_string(model.name.clone()))
                .unwrap_or_default();
            if model_id.is_empty() {
                continue;
            }

            let origin = router_origin(section.as_str(), &model);
            let input_support = router_input_support(&model);
            let mut supported_features = Vec::new();
            let unsupported_features = Vec::new();
            if model.thinking.is_some() {
                supported_features.push(ModelCapabilityFeature::Reasoning);
            }
            if let Some(parameters) = model.supported_parameters.as_ref() {
                if parameters
                    .iter()
                    .any(|parameter| parameter.eq_ignore_ascii_case("temperature"))
                {
                    supported_features.push(ModelCapabilityFeature::Temperature);
                }
                if parameters
                    .iter()
                    .any(|parameter| parameter.eq_ignore_ascii_case("tools"))
                {
                    supported_features.push(ModelCapabilityFeature::ToolCalling);
                }
                if parameters.iter().any(|parameter| {
                    parameter.eq_ignore_ascii_case("response_format")
                        || parameter.eq_ignore_ascii_case("json_schema")
                }) {
                    supported_features.push(ModelCapabilityFeature::StructuredOutput);
                }
            }

            let definition = CatalogModelDefinition {
                lifecycle: None,
                context_window_tokens: model
                    .context_length
                    .or(model.input_token_limit)
                    .map(clamp_u64_to_u32),
                max_input_tokens: model.input_token_limit.map(clamp_u64_to_u32),
                max_output_tokens: model
                    .max_completion_tokens
                    .or(model.output_token_limit)
                    .map(clamp_u64_to_u32),
                description: normalize_optional_string(model.description),
                knowledge_cutoff: None,
                release_date: None,
                last_updated: None,
                open_weights: None,
                default_thinking_mode: None,
                supports_parallel_tool_calls: None,
                supports_verbosity: None,
                default_verbosity: None,
                default_temperature: None,
                default_top_p: None,
                default_top_k: None,
                assistant_reasoning_interleaved: None,
                assistant_reasoning_field: None,
                output_modalities: Vec::new(),
                pricing: None,
                display_name: normalize_optional_string(model.display_name),
                origin,
                speed_modes: BTreeMap::new(),
                thinking_modes: router_thinking_modes(model.thinking.as_ref()),
                capabilities: model_capability_patch(
                    input_support,
                    (supported_features, unsupported_features),
                ),
                source_priority: CatalogDefinitionSourcePriority::default(),
            };

            merge_document_entry(&mut document_models, model_id, definition);
        }
    }

    Ok(ModelCatalogDocument {
        models: document_models,
    })
}

fn parse_openai_codex_models_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let payload: OpenAiCodexModelsDocument = serde_json::from_str(body)?;
    let mut document_models = BTreeMap::new();

    for model in payload.models {
        let model_id = normalize_optional_string(model.slug).unwrap_or_default();
        if model_id.is_empty() {
            continue;
        }

        let description = normalize_optional_string(model.description);
        let display_name = normalize_optional_string(model.display_name);
        let thinking_modes = codex_thinking_modes(model.supported_reasoning_levels.as_deref());
        let speed_modes = codex_speed_modes(
            model.service_tiers.as_deref(),
            model.additional_speed_tiers.as_deref(),
        );
        let definition = CatalogModelDefinition {
            lifecycle: None,
            context_window_tokens: model
                .context_window
                .max(model.max_context_window)
                .map(clamp_u64_to_u32),
            max_input_tokens: None,
            max_output_tokens: None,
            description,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: codex_default_thinking_mode(
                model.default_reasoning_level.as_deref(),
            ),
            supports_parallel_tool_calls: model.supports_parallel_tool_calls,
            supports_verbosity: model.support_verbosity,
            default_verbosity: normalize_optional_string(model.default_verbosity),
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
            display_name,
            origin: Some("OpenAI".to_owned()),
            thinking_modes,
            speed_modes,
            capabilities: model_capability_patch(
                (
                    codex_input_support(model.input_modalities.as_deref()),
                    Vec::new(),
                ),
                (
                    [
                        (!model
                            .supported_reasoning_levels
                            .as_deref()
                            .unwrap_or(&[])
                            .is_empty())
                        .then_some(ModelCapabilityFeature::Reasoning),
                        (model.supports_parallel_tool_calls == Some(true))
                            .then_some(ModelCapabilityFeature::ToolCalling),
                    ]
                    .into_iter()
                    .flatten()
                    .collect(),
                    Vec::new(),
                ),
            ),
            source_priority: CatalogDefinitionSourcePriority::default(),
        };

        merge_document_entry(&mut document_models, model_id, definition);
    }

    Ok(ModelCatalogDocument {
        models: document_models,
    })
}

fn parse_hugging_face_official_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let payload: Vec<HuggingFaceHubModel> = serde_json::from_str(body)?;
    let mut document_models = BTreeMap::new();

    for model in payload {
        let repo_id = normalize_optional_string(model.id.or(model.model_id)).unwrap_or_default();
        if repo_id.is_empty() || model.private || !hugging_face_model_is_supported(repo_id.as_str())
        {
            continue;
        }

        let Some((owner, repo_name)) = repo_id.split_once('/') else {
            continue;
        };
        let Some(origin) = hugging_face_owner_origin(owner) else {
            continue;
        };

        let definition = CatalogModelDefinition {
            lifecycle: None,
            context_window_tokens: None,
            max_input_tokens: None,
            max_output_tokens: None,
            description: None,
            knowledge_cutoff: None,
            release_date: hugging_face_release_date(model.created_at.as_deref()),
            last_updated: None,
            open_weights: Some(true),
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: hugging_face_output_modalities(
                model.pipeline_tag.as_deref(),
                repo_name,
                model.tags.as_deref(),
            ),
            pricing: None,
            display_name: Some(repo_name.to_owned()),
            origin: Some(origin.to_owned()),
            thinking_modes: BTreeMap::new(),
            speed_modes: BTreeMap::new(),
            capabilities: hugging_face_capability_patch(
                model.pipeline_tag.as_deref(),
                repo_name,
                model.tags.as_deref(),
            ),
            source_priority: CatalogDefinitionSourcePriority::default(),
        };

        for alias in hugging_face_model_aliases(repo_name) {
            merge_document_entry(&mut document_models, alias, definition.clone());
        }
    }

    Ok(ModelCatalogDocument {
        models: document_models,
    })
}

#[derive(Debug, Clone, Copy)]
struct OfficialHtmlModelSignal {
    model_ids: &'static [&'static str],
    display_name: &'static str,
    origin: &'static str,
    markers: &'static [&'static str],
    input_modalities: &'static [ModelInputModality],
    output_modalities: &'static [&'static str],
}

const OFFICIAL_HTML_MODEL_SIGNALS: &[OfficialHtmlModelSignal] = &[
    OfficialHtmlModelSignal {
        model_ids: &["dbrx-instruct"],
        display_name: "dbrx-instruct",
        origin: "Databricks",
        markers: &["filename\":\"nvidia-nim-api-for-databricksdbrx-instruct.json\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["embed-qa-4"],
        display_name: "embed-qa-4",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-embed-qa-4\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["ising-calibration-1-35b-a3b"],
        display_name: "ising-calibration-1-35b-a3b",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-ising-calibration-1-35b-a3b\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["llama-3.2-nemoretriever-1b-vlm-embed-v1"],
        display_name: "llama-3.2-nemoretriever-1b-vlm-embed-v1",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-llama-3_2-nemoretriever-1b-vlm-embed-v1\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["llama-3.2-nv-embedqa-1b-v1"],
        display_name: "llama-3.2-nv-embedqa-1b-v1",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-llama-3_2-nv-embedqa-1b-v1\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["llama-3.2-nv-embedqa-1b-v2"],
        display_name: "llama-3.2-nv-embedqa-1b-v2",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-llama-3_2-nv-embedqa-1b-v2\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["nemoretriever-parse"],
        display_name: "nemoretriever-parse",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-nemoretriever-parse\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["nemotron-4-340b-reward"],
        display_name: "nemotron-4-340b-reward",
        origin: "NVIDIA",
        markers: &["filename\":\"nvidia-nim-api-for-nvidianemotron-4-340b-reward.json\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["neva-22b"],
        display_name: "neva-22b",
        origin: "NVIDIA",
        markers: &["filename\":\"nvidia-nim-api-for-nvidianeva-22b.json\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["nv-embedqa-e5-v5"],
        display_name: "nv-embedqa-e5-v5",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-nv-embedqa-e5-v5\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["nv-embedqa-mistral-7b-v2"],
        display_name: "nv-embedqa-mistral-7b-v2",
        origin: "NVIDIA",
        markers: &["filename\":\"nvidia-nim-api-for-nvidianv-embedqa-mistral-7b-v2.json\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["nvclip"],
        display_name: "nvclip",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-nvclip\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["vila"],
        display_name: "vila",
        origin: "NVIDIA",
        markers: &["href=\"/nim/reference/nvidia-vila\""],
        input_modalities: &[],
        output_modalities: &[],
    },
    OfficialHtmlModelSignal {
        model_ids: &["ai-synthetic-video-detector", "synthetic-video-detector"],
        display_name: "synthetic-video-detector",
        origin: "NVIDIA",
        markers: &["synthetic-video-detector"],
        input_modalities: &[ModelInputModality::Video],
        output_modalities: &["text"],
    },
];

fn parse_official_html_signals_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let normalized = body.trim().to_ascii_lowercase();
    let mut document_models = BTreeMap::new();

    for signal in OFFICIAL_HTML_MODEL_SIGNALS {
        if !signal
            .markers
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            continue;
        }

        let definition = CatalogModelDefinition {
            lifecycle: None,
            context_window_tokens: None,
            max_input_tokens: None,
            max_output_tokens: None,
            description: None,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: signal
                .output_modalities
                .iter()
                .map(|modality| (*modality).to_owned())
                .collect(),
            pricing: None,
            display_name: Some(signal.display_name.to_owned()),
            origin: Some(signal.origin.to_owned()),
            thinking_modes: BTreeMap::new(),
            speed_modes: BTreeMap::new(),
            capabilities: model_capability_patch(
                (signal.input_modalities.to_vec(), Vec::new()),
                (Vec::new(), Vec::new()),
            ),
            source_priority: CatalogDefinitionSourcePriority::default(),
        };

        for model_id in signal.model_ids {
            merge_document_entry(
                &mut document_models,
                (*model_id).to_owned(),
                definition.clone(),
            );
        }
    }

    Ok(ModelCatalogDocument {
        models: document_models,
    })
}

fn is_nvidia_reference_index_url(url: &str) -> bool {
    url.trim()
        .to_ascii_lowercase()
        .contains("docs.api.nvidia.com/nim/reference/models-1")
}

fn parse_official_html_reference_index_document(
    body: &str,
) -> Result<Vec<OfficialHtmlReferencePage>, AppError> {
    let Some(json) = extract_official_html_ssr_props_json(body) else {
        return Ok(Vec::new());
    };

    let props: OfficialHtmlSsrProps = serde_json::from_str(json)?;
    let mut pages = Vec::new();
    for reference in props.sidebars.refs {
        collect_official_html_reference_pages(&reference.pages, &mut pages);
    }
    Ok(pages)
}

fn collect_official_html_reference_pages(
    candidates: &[OfficialHtmlSsrPage],
    pages: &mut Vec<OfficialHtmlReferencePage>,
) {
    for page in candidates {
        if let Some(reference_page) = page.as_reference_page() {
            if !pages
                .iter()
                .any(|existing| existing.slug == reference_page.slug)
            {
                pages.push(reference_page);
            }
        }
        collect_official_html_reference_pages(&page.children, pages);
    }
}

async fn fetch_official_html_reference_page_document(
    client: &reqwest::Client,
    page: OfficialHtmlReferencePage,
) -> Result<Option<(Vec<String>, CatalogModelDefinition)>, AppError> {
    let url = format!("https://docs.api.nvidia.com/nim/reference/{}", page.slug);
    let mut last_error = None;

    for _ in 0..2 {
        match client.get(&url).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(response) => {
                    let body = response.text().await?;
                    return Ok(parse_official_html_reference_page_document(
                        page.title.as_str(),
                        page.slug.as_str(),
                        body.as_str(),
                    ));
                }
                Err(error) => last_error = Some(error),
            },
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error
        .map(AppError::from)
        .unwrap_or_else(|| AppError::Config(format!("fetch official html page failed: {url}"))))
}

fn parse_official_html_reference_page_document(
    title: &str,
    slug: &str,
    body: &str,
) -> Option<(Vec<String>, CatalogModelDefinition)> {
    let limits = parse_official_html_token_limits(body)?;
    let aliases = official_html_reference_page_aliases(title, slug);
    if aliases.is_empty() {
        return None;
    }

    let (display_name, origin) = official_html_display_name_and_origin(title);
    Some((
        aliases,
        CatalogModelDefinition {
            lifecycle: None,
            context_window_tokens: Some(limits.context_window_tokens),
            max_input_tokens: Some(
                limits
                    .max_input_tokens
                    .unwrap_or(limits.context_window_tokens),
            ),
            max_output_tokens: None,
            description: None,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
            display_name,
            origin,
            thinking_modes: BTreeMap::new(),
            speed_modes: BTreeMap::new(),
            capabilities: ModelCapabilityPatch::default(),
            source_priority: CatalogDefinitionSourcePriority::default(),
        },
    ))
}

fn official_html_reference_page_aliases(title: &str, slug: &str) -> Vec<String> {
    let mut aliases = Vec::new();

    if let Some((owner, model_name)) = split_official_html_reference_title(title) {
        push_official_html_alias(&mut aliases, model_name);
        push_official_html_alias(&mut aliases, format!("{owner}/{model_name}"));
        push_official_html_alias(&mut aliases, format!("{owner}-{model_name}"));
    }

    push_official_html_alias(&mut aliases, slug);
    aliases
}

fn push_official_html_alias(aliases: &mut Vec<String>, value: impl AsRef<str>) {
    let canonical = canonical_model_catalog_id(value.as_ref());
    if !canonical.is_empty() && !aliases.contains(&canonical) {
        aliases.push(canonical);
    }
}

fn official_html_display_name_and_origin(title: &str) -> (Option<String>, Option<String>) {
    let Some((owner, model_name)) = split_official_html_reference_title(title) else {
        return (None, None);
    };
    let origin = official_html_owner_origin(owner)
        .map(str::to_owned)
        .or_else(|| Some(title_case_tokenized(owner)));
    (Some(model_name.to_owned()), origin)
}

fn split_official_html_reference_title(title: &str) -> Option<(&str, &str)> {
    let (owner, model_name) = title.split_once('/')?;
    let owner = owner.trim();
    let model_name = model_name.trim();
    (!owner.is_empty() && !model_name.is_empty()).then_some((owner, model_name))
}

fn official_html_owner_origin(owner: &str) -> Option<&'static str> {
    match owner.trim().to_ascii_lowercase().as_str() {
        "abacusai" => Some("Abacus.AI"),
        "bytedance" => Some("ByteDance"),
        "meta" => Some("Meta"),
        "minimaxai" => Some("MiniMax"),
        "moonshotai" => Some("Moonshot AI"),
        "openai" => Some("OpenAI"),
        "z-ai" | "zai" => Some("Z.AI"),
        other => hugging_face_owner_origin(other),
    }
}

fn parse_official_html_token_limits(body: &str) -> Option<OfficialHtmlTokenLimits> {
    let plain_text = official_html_plain_text(body);

    let mut context_values =
        capture_token_values(plain_text.as_str(), official_html_input_context_length_re());
    context_values.extend(capture_token_values(
        plain_text.as_str(),
        official_html_context_length_re(),
    ));
    context_values.extend(capture_token_values(
        plain_text.as_str(),
        official_html_context_window_re(),
    ));
    context_values.extend(capture_token_values(
        plain_text.as_str(),
        official_html_leading_context_size_re(),
    ));
    context_values.extend(capture_token_values(
        plain_text.as_str(),
        official_html_supports_context_size_re(),
    ));

    let context_window_tokens = context_values.into_iter().max()?;
    let max_input_tokens =
        capture_token_values(plain_text.as_str(), official_html_input_context_length_re())
            .into_iter()
            .max()
            .or(Some(context_window_tokens));

    Some(OfficialHtmlTokenLimits {
        context_window_tokens,
        max_input_tokens,
    })
}

fn capture_token_values(text: &str, pattern: &Regex) -> Vec<u32> {
    pattern
        .captures_iter(text)
        .filter_map(|capture| capture.name("value"))
        .filter_map(|value| parse_token_quantity(value.as_str()))
        .collect()
}

fn parse_token_quantity(raw: &str) -> Option<u32> {
    let normalized = raw.trim().to_ascii_lowercase().replace(',', "");
    let (numeric, multiplier) = if let Some(value) = normalized.strip_suffix("thousand") {
        (value.trim(), 1_000.0)
    } else if let Some(value) = normalized.strip_suffix("million") {
        (value.trim(), 1_000_000.0)
    } else if let Some(value) = normalized.strip_suffix("billion") {
        (value.trim(), 1_000_000_000.0)
    } else if let Some(value) = normalized.strip_suffix('k') {
        (value.trim(), 1_000.0)
    } else if let Some(value) = normalized.strip_suffix('m') {
        (value.trim(), 1_000_000.0)
    } else if let Some(value) = normalized.strip_suffix('b') {
        (value.trim(), 1_000_000_000.0)
    } else {
        (normalized.trim(), 1.0)
    };

    let value = numeric.parse::<f64>().ok()?;
    (value.is_finite() && value > 0.0)
        .then(|| clamp_u64_to_u32((value * multiplier).round() as u64))
}

fn official_html_plain_text(body: &str) -> String {
    let meta_content = official_html_meta_content_re()
        .captures_iter(body)
        .filter_map(|capture| capture.name("content"))
        .map(|content| content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let without_scripts = official_html_script_re().replace_all(body, " ");
    let without_styles = official_html_style_re().replace_all(without_scripts.as_ref(), " ");
    let without_tags = official_html_tag_re().replace_all(without_styles.as_ref(), " ");
    let combined = format!("{meta_content} {}", without_tags.as_ref());
    let decoded = html_escape::decode_html_entities(combined.as_str());
    official_html_whitespace_re()
        .replace_all(decoded.as_ref(), " ")
        .trim()
        .to_owned()
}

fn extract_official_html_ssr_props_json(body: &str) -> Option<&str> {
    official_html_ssr_props_re()
        .captures(body)
        .and_then(|capture| capture.name("json"))
        .map(|json| json.as_str())
}

fn official_html_ssr_props_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?s)<script id="ssr-props" type="application/json">(?P<json>.*?)</script>"#)
            .expect("valid ssr props regex")
    })
}

fn official_html_script_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<script\b.*?</script>").expect("valid script regex"))
}

fn official_html_meta_content_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?is)<meta\b[^>]*\bcontent="(?P<content>[^"]*)"[^>]*>"#)
            .expect("valid meta content regex")
    })
}

fn official_html_style_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<style\b.*?</style>").expect("valid style regex"))
}

fn official_html_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<[^>]+>").expect("valid tag regex"))
}

fn official_html_whitespace_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s+").expect("valid whitespace regex"))
}

fn official_html_input_context_length_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\binput\s+context\s+length\s*\(isl\)[^0-9]{0,24}(?P<value>\d+(?:,\d{3})*(?:\.\d+)?(?:\s*(?:k|m|b|thousand|million|billion))?)\b",
        )
        .expect("valid input context regex")
    })
}

fn official_html_context_length_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:maximum\s+context\s+length|context\s+length)\b[^0-9]{0,24}(?:of\s+|up\s+to\s+|is\s+)?(?P<value>\d+(?:,\d{3})*(?:\.\d+)?(?:\s*(?:k|m|b|thousand|million|billion))?)(?:\s*tokens?)?\b",
        )
        .expect("valid context length regex")
    })
}

fn official_html_context_window_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\bcontext\s+window\b[^0-9]{0,32}(?:of\s+up\s+to\s+|up\s+to\s+|of\s+|is\s+)?(?P<value>\d+(?:,\d{3})*(?:\.\d+)?(?:\s*(?:k|m|b|thousand|million|billion))?)(?:\s*tokens?)?\b",
        )
        .expect("valid context window regex")
    })
}

fn official_html_leading_context_size_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?P<value>\d+(?:,\d{3})*(?:\.\d+)?(?:\s*(?:k|m|b|thousand|million|billion))?)\s*(?:-|\s)?tokens?\s+context\s+(?:window|length|size)\b",
        )
        .expect("valid leading context regex")
    })
}

fn official_html_supports_context_size_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\bsupports\s+up\s+to\s+(?:a|an)?\s*(?P<value>\d+(?:,\d{3})*(?:\.\d+)?(?:\s*(?:k|m|b|thousand|million|billion))?)\s+context\s+(?:window|length|size)\b",
        )
        .expect("valid supports context regex")
    })
}

fn default_source_grade(
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
        sort_priority: if !definition.is_empty() {
            base.sort_priority
        } else {
            Default::default()
        },
        descriptive_priority: if definition_has_descriptive_fields(definition) {
            base.descriptive_priority
        } else {
            Default::default()
        },
        limits_priority: if definition_has_limit_fields(definition) {
            base.limits_priority
        } else {
            Default::default()
        },
        capability_priority: if definition_has_capability_fields(definition) {
            base.capability_priority
        } else {
            Default::default()
        },
        semantics_priority: if definition_has_semantic_fields(definition) {
            base.semantics_priority
        } else {
            Default::default()
        },
        pricing_priority: if definition.pricing.is_some() {
            base.pricing_priority
        } else {
            Default::default()
        },
        mode_priority: if definition_has_mode_fields(definition) {
            base.mode_priority
        } else {
            Default::default()
        },
    }
}

pub(crate) fn definition_has_descriptive_fields(definition: &CatalogModelDefinition) -> bool {
    definition.lifecycle.is_some()
        || definition.description.is_some()
        || definition.knowledge_cutoff.is_some()
        || definition.release_date.is_some()
        || definition.last_updated.is_some()
        || definition.open_weights.is_some()
        || definition.display_name.is_some()
        || definition.origin.is_some()
}

pub(crate) fn definition_has_limit_fields(definition: &CatalogModelDefinition) -> bool {
    definition.context_window_tokens.is_some()
        || definition.max_input_tokens.is_some()
        || definition.max_output_tokens.is_some()
}

pub(crate) fn definition_has_capability_fields(definition: &CatalogModelDefinition) -> bool {
    !definition.output_modalities.is_empty() || !definition.capabilities.is_empty()
}

pub(crate) fn definition_has_semantic_fields(definition: &CatalogModelDefinition) -> bool {
    definition.default_thinking_mode.is_some()
        || definition.supports_parallel_tool_calls.is_some()
        || definition.supports_verbosity.is_some()
        || definition.default_verbosity.is_some()
        || definition.default_temperature.is_some()
        || definition.default_top_p.is_some()
        || definition.default_top_k.is_some()
        || definition.assistant_reasoning_interleaved.is_some()
        || definition.assistant_reasoning_field.is_some()
}

pub(crate) fn definition_has_mode_fields(definition: &CatalogModelDefinition) -> bool {
    !definition.thinking_modes.is_empty() || !definition.speed_modes.is_empty()
}

fn merge_document_entry(
    models: &mut BTreeMap<String, CatalogModelDefinition>,
    model_id: String,
    definition: CatalogModelDefinition,
) {
    models
        .entry(model_id)
        .and_modify(|existing| merge_catalog_definition(existing, &definition))
        .or_insert(definition);
}

fn models_dev_provider_rank(provider_key: &str, provider: &ModelsDevProvider) -> i32 {
    if models_dev_origin(provider_key, provider).is_some() {
        200
    } else if provider
        .name
        .as_deref()
        .is_some_and(|name| name.to_ascii_lowercase().contains("gateway"))
    {
        10
    } else {
        0
    }
}

fn models_dev_origin(provider_key: &str, provider: &ModelsDevProvider) -> Option<String> {
    let normalized = provider
        .id
        .as_deref()
        .unwrap_or(provider_key)
        .trim()
        .to_ascii_lowercase();
    match normalized.as_str() {
        "openai" => Some("OpenAI".to_owned()),
        "anthropic" => Some("Anthropic".to_owned()),
        "google" => Some("Google".to_owned()),
        "deepseek" => Some("DeepSeek".to_owned()),
        "xai" => Some("xAI".to_owned()),
        "cohere" => Some("Cohere".to_owned()),
        "mistral" => Some("Mistral AI".to_owned()),
        "moonshotai" | "kimi-for-coding" => Some("Moonshot AI".to_owned()),
        "alibaba" | "alibaba-cn" => Some("Alibaba".to_owned()),
        "nvidia" => Some("NVIDIA".to_owned()),
        "minimax" | "minimax-cn" => Some("MiniMax".to_owned()),
        "perplexity" | "perplexity-agent" => Some("Perplexity".to_owned()),
        "upstage" => Some("Upstage".to_owned()),
        "xiaomi" => Some("Xiaomi".to_owned()),
        "sarvam" => Some("Sarvam AI".to_owned()),
        "stepfun" => Some("StepFun".to_owned()),
        "databricks" => Some("Databricks".to_owned()),
        "llama" => Some("Meta".to_owned()),
        _ => None,
    }
}

fn models_dev_adapter_id(provider_key: &str, provider: &ModelsDevProvider) -> Option<String> {
    let normalized = provider
        .id
        .as_deref()
        .unwrap_or(provider_key)
        .trim()
        .to_ascii_lowercase();
    match normalized.as_str() {
        "openai" => Some("openai".to_owned()),
        "anthropic" => Some("anthropic".to_owned()),
        "google" | "gemini" => Some("gemini".to_owned()),
        _ => None,
    }
}

fn parse_models_dev_lifecycle(status: Option<&str>) -> Option<ModelLifecycle> {
    match status
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("active") | Some("stable") | Some("ga") => Some(ModelLifecycle::Active),
        Some("preview") => Some(ModelLifecycle::Preview),
        Some("beta") => Some(ModelLifecycle::Beta),
        Some("alpha") => Some(ModelLifecycle::Alpha),
        Some("experimental") => Some(ModelLifecycle::Experimental),
        Some("deprecated") | Some("sunset") => Some(ModelLifecycle::Deprecated),
        _ => None,
    }
}

fn models_dev_speed_modes(
    experimental: Option<&ModelsDevExperimental>,
    adapter_id: Option<&str>,
) -> BTreeMap<String, ConfiguredModelSpeedMode> {
    let mut modes = BTreeMap::new();
    let Some(experimental) = experimental else {
        return modes;
    };
    for (name, mode) in &experimental.modes {
        let normalized = name.trim();
        if normalized.is_empty() {
            continue;
        }
        let request_override = mode
            .provider
            .as_ref()
            .map(models_dev_request_override)
            .unwrap_or_default();
        let (default_request_override, adapter_overrides) = if request_override.is_empty() {
            (ModelSpeedModeRequestOverride::default(), BTreeMap::new())
        } else if let Some(adapter_id) = adapter_id {
            (
                ModelSpeedModeRequestOverride::default(),
                BTreeMap::from([(adapter_id.to_owned(), request_override)]),
            )
        } else {
            (request_override, BTreeMap::new())
        };
        modes.insert(
            normalized.to_owned(),
            ConfiguredModelSpeedMode {
                display_name: Some(title_case_tokenized(normalized)),
                description: None,
                request_override: default_request_override,
                adapter_overrides,
                disabled: false,
            },
        );
    }
    modes
}

fn models_dev_assistant_reasoning_field(
    interleaved: Option<&ModelsDevInterleaved>,
) -> Option<String> {
    match interleaved {
        Some(ModelsDevInterleaved::Field(field)) => normalize_optional_string(field.field.clone())
            .and_then(|value| match value.as_str() {
                "reasoning_content" | "reasoning_details" => Some(value),
                _ => None,
            }),
        Some(ModelsDevInterleaved::Enabled(_)) => None,
        _ => None,
    }
}

fn models_dev_assistant_reasoning_interleaved(
    interleaved: Option<&ModelsDevInterleaved>,
) -> Option<bool> {
    match interleaved {
        Some(ModelsDevInterleaved::Enabled(enabled)) => Some(*enabled),
        Some(ModelsDevInterleaved::Field(_)) => Some(true),
        None => None,
    }
}

fn models_dev_request_override(provider: &ModelsDevModeProvider) -> ModelSpeedModeRequestOverride {
    let headers = provider.headers.clone().unwrap_or_default();
    let body_patch = provider.body.clone().unwrap_or_default();
    ModelSpeedModeRequestOverride {
        headers,
        body_patch,
    }
}

fn router_origin(section: &str, model: &RouterModel) -> Option<String> {
    if let Some(owned_by) = model.owned_by.as_deref() {
        let owned = owned_by.trim().to_ascii_lowercase();
        match owned.as_str() {
            "google" => return Some("Google".to_owned()),
            "anthropic" => return Some("Anthropic".to_owned()),
            "openai" => return Some("OpenAI".to_owned()),
            "xai" => return Some("xAI".to_owned()),
            "moonshot" | "moonshotai" | "kimi" => return Some("Moonshot AI".to_owned()),
            _ => {}
        }
    }

    match section {
        "claude" => Some("Anthropic".to_owned()),
        "gemini" | "gemini-cli" | "aistudio" | "vertex" => Some("Google".to_owned()),
        "xai" => Some("xAI".to_owned()),
        "kimi" => Some("Moonshot AI".to_owned()),
        "codex-free" | "codex-team" | "codex-plus" | "codex-pro" => Some("OpenAI".to_owned()),
        "antigravity" => Some("Antigravity".to_owned()),
        _ => None,
    }
}

fn router_input_support(model: &RouterModel) -> (Vec<ModelInputModality>, Vec<ModelInputModality>) {
    let mut supported = Vec::new();
    let unsupported = Vec::new();
    let Some(modalities) = model.supported_input_modalities.as_ref() else {
        return (supported, unsupported);
    };
    for modality in modalities {
        if let Some(mapped) = map_modality_name(modality)
            && mapped != ModelInputModality::Text
            && !supported.contains(&mapped)
        {
            supported.push(mapped);
        }
    }
    (supported, unsupported)
}

fn router_thinking_modes(
    thinking: Option<&RouterThinking>,
) -> BTreeMap<String, ConfiguredModelThinkingMode> {
    let mut modes = BTreeMap::new();
    let Some(thinking) = thinking else {
        return modes;
    };

    if thinking.zero_allowed.unwrap_or(false) {
        modes.insert(
            "no-thinking".to_owned(),
            ConfiguredModelThinkingMode {
                display_name: Some("Off".to_owned()),
                description: None,
                thinking: Some(ThinkingRequest::Disabled),
                request_override: Default::default(),
                adapter_overrides: BTreeMap::new(),
                disabled: false,
            },
        );
    }

    for level in &thinking.levels {
        if let Some(effort) = effort_for_variant_name(level) {
            let normalized = level.trim().to_ascii_lowercase();
            let variant_name = if normalized == "none" {
                "no-thinking".to_owned()
            } else {
                format!("thinking-{normalized}")
            };
            modes
                .entry(variant_name)
                .or_insert_with(|| ConfiguredModelThinkingMode {
                    display_name: Some(format!("Think {}", title_case_tokenized(level))),
                    description: None,
                    thinking: Some(ThinkingRequest::Effort { effort }),
                    request_override: Default::default(),
                    adapter_overrides: BTreeMap::new(),
                    disabled: false,
                });
        }
    }

    if thinking.levels.is_empty() {
        if let Some(high_budget) = router_high_budget(thinking) {
            modes.insert(
                "thinking-high".to_owned(),
                ConfiguredModelThinkingMode {
                    display_name: Some("Think High".to_owned()),
                    description: None,
                    thinking: Some(ThinkingRequest::Budget {
                        budget_tokens: high_budget,
                    }),
                    request_override: Default::default(),
                    adapter_overrides: BTreeMap::new(),
                    disabled: false,
                },
            );
        }

        if let Some(max_budget) = thinking.max.map(clamp_u64_to_u32) {
            modes.insert(
                "thinking-max".to_owned(),
                ConfiguredModelThinkingMode {
                    display_name: Some("Think Max".to_owned()),
                    description: None,
                    thinking: Some(ThinkingRequest::Budget {
                        budget_tokens: max_budget,
                    }),
                    request_override: Default::default(),
                    adapter_overrides: BTreeMap::new(),
                    disabled: false,
                },
            );
        }
    }

    modes
}

fn codex_thinking_modes(
    supported_reasoning_levels: Option<&[OpenAiCodexReasoningLevel]>,
) -> BTreeMap<String, ConfiguredModelThinkingMode> {
    let mut modes = BTreeMap::new();
    let Some(supported_reasoning_levels) = supported_reasoning_levels else {
        return modes;
    };

    for level in supported_reasoning_levels {
        let Some(effort) = effort_for_variant_name(level.effort.as_str()) else {
            continue;
        };
        let effort_name = effort.as_str();
        modes.insert(
            format!("thinking-{effort_name}"),
            ConfiguredModelThinkingMode {
                display_name: Some(format!("Think {}", title_case_tokenized(effort_name))),
                description: normalize_optional_string(level.description.clone()),
                thinking: Some(ThinkingRequest::Effort { effort }),
                request_override: Default::default(),
                adapter_overrides: BTreeMap::new(),
                disabled: false,
            },
        );
    }

    modes
}

fn codex_speed_modes(
    service_tiers: Option<&[OpenAiCodexServiceTier]>,
    additional_speed_tiers: Option<&[String]>,
) -> BTreeMap<String, ConfiguredModelSpeedMode> {
    let mut modes = BTreeMap::new();
    let Some(service_tiers) = service_tiers else {
        return modes;
    };

    let aliases = additional_speed_tiers.unwrap_or(&[]);
    for (index, tier) in service_tiers.iter().enumerate() {
        let Some(tier_id) = normalize_optional_string(tier.id.clone()) else {
            continue;
        };
        let alias = aliases
            .get(index)
            .and_then(|value| normalize_optional_string(Some(value.clone())));
        let name = alias.unwrap_or_else(|| {
            normalize_optional_string(tier.name.clone())
                .map(|value| value.to_ascii_lowercase().replace(' ', "-"))
                .unwrap_or_else(|| tier_id.clone())
        });
        if name.is_empty() {
            continue;
        }

        modes.insert(
            name,
            ConfiguredModelSpeedMode {
                display_name: normalize_optional_string(tier.name.clone())
                    .or_else(|| Some(title_case_tokenized(tier_id.as_str()))),
                description: normalize_optional_string(tier.description.clone()),
                request_override: ModelSpeedModeRequestOverride {
                    headers: BTreeMap::new(),
                    body_patch: BTreeMap::from([(
                        "service_tier".to_owned(),
                        serde_json::Value::String(tier_id),
                    )]),
                },
                adapter_overrides: BTreeMap::new(),
                disabled: false,
            },
        );
    }

    modes
}

fn codex_default_thinking_mode(default_reasoning_level: Option<&str>) -> Option<String> {
    let effort = default_reasoning_level.and_then(effort_for_variant_name)?;
    Some(format!("thinking-{}", effort.as_str()))
}

fn router_high_budget(thinking: &RouterThinking) -> Option<u32> {
    let max_budget = thinking.max?;
    let min_budget = thinking.min.unwrap_or(0);
    let target = max_budget.min(16_384).max(min_budget);
    Some(clamp_u64_to_u32(target))
}

fn codex_input_support(input_modalities: Option<&[String]>) -> Vec<ModelInputModality> {
    let mut supported = Vec::new();
    let Some(input_modalities) = input_modalities else {
        return supported;
    };
    for modality in input_modalities {
        if let Some(mapped) = map_modality_name(modality)
            && mapped != ModelInputModality::Text
            && !supported.contains(&mapped)
        {
            supported.push(mapped);
        }
    }
    supported
}

fn models_dev_input_support(
    modalities: Option<&ModelsDevModalities>,
    attachment: Option<bool>,
) -> (Vec<ModelInputModality>, Vec<ModelInputModality>) {
    let mut supported = Vec::new();
    let unsupported = Vec::new();
    if let Some(modalities) = modalities {
        for modality in &modalities.input {
            if let Some(mapped) = map_modality_name(modality)
                && mapped != ModelInputModality::Text
                && !supported.contains(&mapped)
            {
                supported.push(mapped);
            }
        }
    }
    if attachment == Some(true) && !supported.contains(&ModelInputModality::File) {
        supported.push(ModelInputModality::File);
    }
    (supported, unsupported)
}

fn models_dev_output_modalities(modalities: Option<&ModelsDevModalities>) -> Vec<String> {
    let mut output = Vec::new();
    let Some(modalities) = modalities else {
        return output;
    };
    for modality in &modalities.output {
        let normalized = normalize_modality_label(modality);
        if !normalized.is_empty() && !output.contains(&normalized) {
            output.push(normalized);
        }
    }
    output
}

fn models_dev_pricing(cost: Option<&ModelsDevCost>) -> Option<ModelPricing> {
    let cost = cost?;
    let mut tiers = cost
        .tiers
        .iter()
        .filter_map(models_dev_pricing_tier)
        .collect::<Vec<_>>();

    let context_over_200k_tier = cost
        .context_over_200k
        .as_ref()
        .and_then(|context_over_200k| {
            let tier = ModelPricingTier {
                tier_type: Some("context".to_owned()),
                size_tokens: Some(200_000),
                input_usd_per_million_tokens: pricing_value(context_over_200k.input.as_ref()),
                output_usd_per_million_tokens: pricing_value(context_over_200k.output.as_ref()),
                cache_read_usd_per_million_tokens: pricing_value(
                    context_over_200k.cache_read.as_ref(),
                ),
                cache_write_usd_per_million_tokens: pricing_value(
                    context_over_200k.cache_write.as_ref(),
                ),
            };
            (!tier.is_empty()).then_some(tier)
        });
    if let Some(tier) = context_over_200k_tier
        && !tiers.iter().any(|existing| {
            existing.tier_type.as_deref() == Some("context")
                && existing.size_tokens == Some(200_000)
        })
    {
        tiers.push(tier);
    }

    let pricing = ModelPricing {
        input_usd_per_million_tokens: pricing_value(cost.input.as_ref()),
        output_usd_per_million_tokens: pricing_value(cost.output.as_ref()),
        cache_read_usd_per_million_tokens: pricing_value(cost.cache_read.as_ref()),
        cache_write_usd_per_million_tokens: pricing_value(cost.cache_write.as_ref()),
        tiers,
    };
    (!pricing.is_empty()).then_some(pricing)
}

fn models_dev_pricing_tier(tier: &ModelsDevCostTier) -> Option<ModelPricingTier> {
    let tier = ModelPricingTier {
        tier_type: normalize_optional_string(tier.tier_type.clone()),
        size_tokens: tier.size.map(clamp_u64_to_u32),
        input_usd_per_million_tokens: pricing_value(tier.input.as_ref()),
        output_usd_per_million_tokens: pricing_value(tier.output.as_ref()),
        cache_read_usd_per_million_tokens: pricing_value(tier.cache_read.as_ref()),
        cache_write_usd_per_million_tokens: pricing_value(tier.cache_write.as_ref()),
    };
    (!tier.is_empty()).then_some(tier)
}

fn pricing_value(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::Number(value)) => Some(value.to_string()),
        Some(serde_json::Value::String(value)) => normalize_optional_string(Some(value.clone())),
        _ => None,
    }
}

fn features_from_bool_flags(
    flags: &[(ModelCapabilityFeature, Option<bool>)],
) -> (Vec<ModelCapabilityFeature>, Vec<ModelCapabilityFeature>) {
    let mut supported = Vec::new();
    let mut unsupported = Vec::new();
    for (feature, value) in flags {
        match value {
            Some(true) => supported.push(*feature),
            Some(false) => unsupported.push(*feature),
            None => {}
        }
    }
    (supported, unsupported)
}

fn model_capability_patch(
    (supported_inputs, unsupported_inputs): (Vec<ModelInputModality>, Vec<ModelInputModality>),
    (supported_features, unsupported_features): (
        Vec<ModelCapabilityFeature>,
        Vec<ModelCapabilityFeature>,
    ),
) -> ModelCapabilityPatch {
    ModelCapabilityPatch {
        input: CapabilitySelectionPatch::optional_from_supported_unsupported(
            supported_inputs,
            unsupported_inputs,
        ),
        features: CapabilitySelectionPatch::optional_from_supported_unsupported(
            supported_features,
            unsupported_features,
        ),
        ..ModelCapabilityPatch::default()
    }
}

fn hugging_face_capability_patch(
    pipeline_tag: Option<&str>,
    repo_name: &str,
    tags: Option<&[String]>,
) -> ModelCapabilityPatch {
    let normalized_repo = repo_name.trim().to_ascii_lowercase();
    let normalized_pipeline = pipeline_tag
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let mut supported_inputs = Vec::new();
    let tag_set = normalized_tag_set(tags);

    if matches!(
        normalized_pipeline.as_str(),
        "image-text-to-text"
            | "image-to-text"
            | "visual-question-answering"
            | "image-classification"
            | "object-detection"
            | "image-segmentation"
            | "document-question-answering"
            | "any-to-any"
    ) || normalized_repo.contains("vision")
        || normalized_repo.contains("kosmos")
        || tag_set.iter().any(|tag| {
            matches!(
                tag.as_str(),
                "vision"
                    | "image-text-to-text"
                    | "image-to-text"
                    | "visual-question-answering"
                    | "image-classification"
                    | "object-detection"
                    | "image-segmentation"
                    | "document-question-answering"
            )
        })
    {
        supported_inputs.push(ModelInputModality::Image);
    }
    if matches!(
        normalized_pipeline.as_str(),
        "automatic-speech-recognition"
            | "audio-to-audio"
            | "audio-classification"
            | "voice-activity-detection"
    ) || tag_set.iter().any(|tag| {
        matches!(
            tag.as_str(),
            "audio" | "audio-to-audio" | "automatic-speech-recognition" | "audio-classification"
        )
    }) {
        supported_inputs.push(ModelInputModality::Audio);
    }
    if matches!(
        normalized_pipeline.as_str(),
        "video-text-to-text" | "video-classification" | "video-text-retrieval"
    ) || tag_set.iter().any(|tag| {
        matches!(
            tag.as_str(),
            "video" | "video-text-to-text" | "video-classification"
        )
    }) {
        supported_inputs.push(ModelInputModality::Video);
    }
    if matches!(normalized_pipeline.as_str(), "document-question-answering")
        || tag_set
            .iter()
            .any(|tag| matches!(tag.as_str(), "document-question-answering" | "document"))
    {
        supported_inputs.push(ModelInputModality::Document);
    }
    supported_inputs.dedup();

    model_capability_patch((supported_inputs, Vec::new()), (Vec::new(), Vec::new()))
}

fn hugging_face_output_modalities(
    pipeline_tag: Option<&str>,
    repo_name: &str,
    tags: Option<&[String]>,
) -> Vec<String> {
    let normalized_repo = repo_name.trim().to_ascii_lowercase();
    let normalized_pipeline = pipeline_tag
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let tag_set = normalized_tag_set(tags);

    if normalized_pipeline == "any-to-any" {
        return vec!["text".to_owned(), "image".to_owned()];
    }
    if matches!(
        normalized_pipeline.as_str(),
        "image-text-to-text"
            | "image-to-text"
            | "visual-question-answering"
            | "text-generation"
            | "text2text-generation"
            | "translation"
            | "summarization"
            | "automatic-speech-recognition"
            | "document-question-answering"
            | "video-text-to-text"
    ) {
        return vec!["text".to_owned()];
    }
    if matches!(
        normalized_pipeline.as_str(),
        "text-to-image" | "image-to-image"
    ) {
        return vec!["image".to_owned()];
    }
    if matches!(
        normalized_pipeline.as_str(),
        "text-to-audio" | "text-to-speech" | "audio-to-audio"
    ) {
        return vec!["audio".to_owned()];
    }
    if matches!(
        normalized_pipeline.as_str(),
        "text-to-video" | "image-to-video"
    ) {
        return vec!["video".to_owned()];
    }
    if normalized_repo.contains("vision") || normalized_repo.contains("kosmos") {
        return vec!["text".to_owned()];
    }
    if tag_set.iter().any(|tag| tag == "vision") {
        return vec!["text".to_owned()];
    }
    Vec::new()
}

fn normalized_tag_set(tags: Option<&[String]>) -> Vec<String> {
    let mut values = Vec::new();
    for tag in tags.unwrap_or(&[]) {
        let normalized = tag.trim().to_ascii_lowercase();
        if !normalized.is_empty() && !values.contains(&normalized) {
            values.push(normalized);
        }
    }
    values
}

fn hugging_face_owner_origin(owner: &str) -> Option<&'static str> {
    match owner.trim().to_ascii_lowercase().as_str() {
        "google" => Some("Google"),
        "meta-llama" | "codellama" => Some("Meta"),
        "mistralai" => Some("Mistral AI"),
        "ibm-granite" => Some("IBM"),
        "microsoft" => Some("Microsoft"),
        "bigcode" => Some("BigCode"),
        "snowflake" => Some("Snowflake"),
        "adept" => Some("Adept"),
        "aisingapore" => Some("AI Singapore"),
        "stockmark" => Some("Stockmark"),
        "zyphra" => Some("Zyphra"),
        "deepseek-ai" => Some("DeepSeek"),
        "ai21labs" => Some("AI21 Labs"),
        "nvidia" => Some("NVIDIA"),
        "writer" => Some("Writer"),
        _ => None,
    }
}

fn hugging_face_release_date(created_at: Option<&str>) -> Option<String> {
    let created_at = created_at?.trim();
    let date = created_at.split('T').next().unwrap_or_default().trim();
    (!date.is_empty()).then(|| date.to_owned())
}

fn hugging_face_model_is_supported(repo_id: &str) -> bool {
    let Some((owner, repo_name)) = repo_id.split_once('/') else {
        return false;
    };
    if hugging_face_owner_origin(owner).is_none() {
        return false;
    }

    let normalized = repo_name.trim().to_ascii_lowercase();
    if normalized == "nvidia-nemotron-3-nano-30b-a3b-bf16" {
        return true;
    }
    !matches!(
        normalized.as_str(),
        value
            if value.ends_with("-gguf")
                || value.ends_with("-onnx")
                || value.contains("-onnx-")
                || value.ends_with("-tflite")
                || value.ends_with("-keras")
                || value.ends_with("-pytorch")
                || value.ends_with("-ov")
                || value.ends_with("-accelerator")
                || value.ends_with("-assistant")
                || value.ends_with("-fp8")
                || value.ends_with("-bf16")
                || value.ends_with("-nvfp4")
                || value.ends_with("-awq")
                || value.ends_with("-gptq")
                || value.ends_with("-exl2")
                || value.ends_with("-flax")
                || value.ends_with("-vllm")
                || value.ends_with("-dummy-weights")
                || value.contains("tiny-random")
                || value.contains("-mlx-")
                || value.contains("-sfp-cpp")
                || value.contains("-reward")
                || value.contains("-base")
    )
}

fn hugging_face_model_aliases(repo_name: &str) -> Vec<String> {
    let normalized = repo_name.trim().to_ascii_lowercase();
    let mut aliases = vec![repo_name.trim().to_owned()];

    if let Some(stripped) = normalized.strip_suffix("-hf") {
        aliases.push(stripped.to_owned());
    }
    if normalized.starts_with("codegemma-") && normalized.ends_with("-it") {
        aliases.push(normalized.trim_end_matches("-it").to_owned());
    }
    if normalized.starts_with("llama-2-") {
        aliases.push(normalized.replacen("llama-2-", "llama2-", 1));
    }
    if normalized == "codestral-22b-v0.1" {
        aliases.push("codestral-22b-instruct-v0.1".to_owned());
    }
    if normalized == "mistral-large-instruct-2407" {
        aliases.push("mistral-large-2-instruct".to_owned());
    }
    if normalized == "kosmos-2-patch14-224" {
        aliases.push("kosmos-2".to_owned());
    }
    if normalized == "snowflake-arctic-embed-l" {
        aliases.push("arctic-embed-l".to_owned());
    }
    if normalized == "sea-lion-v1-7b-it" {
        aliases.push("sea-lion-7b-instruct".to_owned());
    }
    if normalized == "granite-34b-code-instruct-8k" {
        aliases.push("granite-34b-code-instruct".to_owned());
    }
    if matches!(
        normalized.as_str(),
        "granite-8b-code-instruct-4k" | "granite-8b-code-instruct-128k"
    ) {
        aliases.push("granite-8b-code-instruct".to_owned());
    }
    if normalized == "palmyra-creative" {
        aliases.push("palmyra-creative-122b".to_owned());
    }
    if normalized == "mistral-nemo-minitron-8b-instruct" {
        aliases.push("mistral-nemo-minitron-8b-8k-instruct".to_owned());
    }
    if normalized == "nvidia-nemotron-3-nano-30b-a3b-bf16" {
        aliases.push("nemotron-nano-3-30b-a3b".to_owned());
    }
    if normalized.starts_with("nvidia-nemotron-parse-") {
        aliases.push("nemotron-parse".to_owned());
    }
    if let Some((size, version)) = normalized
        .strip_prefix("ai21-jamba-")
        .and_then(|value| value.rsplit_once('-'))
    {
        aliases.push(format!("jamba-{size}-{version}"));
        aliases.push(format!("jamba-{version}-{size}-instruct"));
    }

    aliases.sort();
    aliases.dedup();
    aliases
}

fn map_modality_name(value: &str) -> Option<ModelInputModality> {
    match value.trim().to_ascii_lowercase().as_str() {
        "text" => Some(ModelInputModality::Text),
        "image" => Some(ModelInputModality::Image),
        "audio" => Some(ModelInputModality::Audio),
        "video" => Some(ModelInputModality::Video),
        "document" | "pdf" => Some(ModelInputModality::Document),
        "file" => Some(ModelInputModality::File),
        _ => None,
    }
}

fn normalize_modality_label(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "pdf" => "document".to_owned(),
        other => other.to_owned(),
    }
}

fn effort_for_variant_name(name: &str) -> Option<ReasoningEffort> {
    match name.trim().to_ascii_lowercase().as_str() {
        "minimal" => Some(ReasoningEffort::Minimal),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" => Some(ReasoningEffort::Xhigh),
        "max" => Some(ReasoningEffort::Max),
        _ => None,
    }
}

pub fn enrich_catalog_document_thinking_modes(document: &mut ModelCatalogDocument) {
    for (model_id, definition) in &mut document.models {
        let inferred = inferred_thinking_modes(model_id.as_str(), definition);
        for (name, mode) in inferred {
            definition.thinking_modes.entry(name).or_insert(mode);
        }
    }
}

fn inferred_thinking_modes(
    model_id: &str,
    definition: &CatalogModelDefinition,
) -> BTreeMap<String, ConfiguredModelThinkingMode> {
    let mut modes = BTreeMap::new();
    if !matches!(
        definition
            .capabilities
            .feature_support(ModelCapabilityFeature::Reasoning),
        Some(CapabilitySupport::Supported)
    ) {
        return modes;
    }

    let normalized = model_id.trim().to_ascii_lowercase();
    if normalized.contains("gpt-5")
        || normalized.starts_with("o1")
        || normalized.starts_with("o3")
        || normalized.starts_with("o4")
    {
        for effort in openai_reasoning_efforts(normalized.as_str()) {
            insert_effort_mode(&mut modes, effort);
        }
        return modes;
    }

    if normalized.contains("gemini-3") {
        insert_effort_mode(&mut modes, ReasoningEffort::Low);
        insert_effort_mode(&mut modes, ReasoningEffort::High);
        return modes;
    }

    if normalized.contains("gemini-2.5") {
        insert_effort_mode(&mut modes, ReasoningEffort::High);
        insert_effort_mode(&mut modes, ReasoningEffort::Max);
        return modes;
    }

    if normalized.contains("claude") && definition.thinking_modes.is_empty() {
        insert_effort_mode(&mut modes, ReasoningEffort::High);
        insert_effort_mode(&mut modes, ReasoningEffort::Max);
    }

    modes
}

fn openai_reasoning_efforts(model_id: &str) -> Vec<ReasoningEffort> {
    let mut efforts = Vec::new();
    if model_id.contains("gpt-5") {
        efforts.push(ReasoningEffort::Minimal);
    }
    efforts.extend([
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
    ]);
    if model_id.contains("gpt-5") {
        efforts.push(ReasoningEffort::Xhigh);
    }
    efforts
}

fn insert_effort_mode(
    modes: &mut BTreeMap<String, ConfiguredModelThinkingMode>,
    effort: ReasoningEffort,
) {
    let effort_name = effort.as_str();
    modes
        .entry(format!("thinking-{effort_name}"))
        .or_insert_with(|| ConfiguredModelThinkingMode {
            display_name: Some(format!("Think {}", title_case_tokenized(effort_name))),
            description: None,
            thinking: Some(ThinkingRequest::Effort { effort }),
            request_override: Default::default(),
            adapter_overrides: BTreeMap::new(),
            disabled: false,
        });
}

fn title_case_tokenized(value: &str) -> String {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            format!(
                "{}{}",
                first.to_ascii_uppercase(),
                chars.as_str().to_ascii_lowercase()
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn clamp_u64_to_u32(value: u64) -> u32 {
    value.min(u32::MAX as u64) as u32
}

#[derive(Debug, Deserialize)]
struct ModelsDevProvider {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    models: BTreeMap<String, ModelsDevModel>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevModel {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    knowledge: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    last_updated: Option<String>,
    #[serde(default)]
    open_weights: Option<bool>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    reasoning: Option<bool>,
    #[serde(default, rename = "tool_call")]
    tool_call: Option<bool>,
    #[serde(default)]
    structured_output: Option<bool>,
    #[serde(default)]
    temperature: Option<bool>,
    #[serde(default)]
    attachment: Option<bool>,
    #[serde(default)]
    modalities: Option<ModelsDevModalities>,
    #[serde(default)]
    limit: Option<ModelsDevLimits>,
    #[serde(default)]
    cost: Option<ModelsDevCost>,
    #[serde(default)]
    interleaved: Option<ModelsDevInterleaved>,
    #[serde(default)]
    experimental: Option<ModelsDevExperimental>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevModalities {
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    output: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevCost {
    #[serde(default)]
    input: Option<serde_json::Value>,
    #[serde(default)]
    output: Option<serde_json::Value>,
    #[serde(default)]
    cache_read: Option<serde_json::Value>,
    #[serde(default)]
    cache_write: Option<serde_json::Value>,
    #[serde(default)]
    context_over_200k: Option<ModelsDevCostTierContext>,
    #[serde(default)]
    tiers: Vec<ModelsDevCostTier>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevCostTierContext {
    #[serde(default)]
    input: Option<serde_json::Value>,
    #[serde(default)]
    output: Option<serde_json::Value>,
    #[serde(default)]
    cache_read: Option<serde_json::Value>,
    #[serde(default)]
    cache_write: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevCostTier {
    #[serde(default, rename = "type")]
    tier_type: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    input: Option<serde_json::Value>,
    #[serde(default)]
    output: Option<serde_json::Value>,
    #[serde(default)]
    cache_read: Option<serde_json::Value>,
    #[serde(default)]
    cache_write: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevLimits {
    #[serde(default)]
    context: Option<u64>,
    #[serde(default)]
    input: Option<u64>,
    #[serde(default)]
    output: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevExperimental {
    #[serde(default)]
    modes: BTreeMap<String, ModelsDevMode>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ModelsDevInterleaved {
    Enabled(bool),
    Field(ModelsDevInterleavedField),
}

#[derive(Debug, Deserialize)]
struct ModelsDevInterleavedField {
    #[serde(default)]
    field: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ModelsDevMode {
    #[serde(default)]
    provider: Option<ModelsDevModeProvider>,
}

#[derive(Debug, Deserialize, Default)]
struct ModelsDevModeProvider {
    #[serde(default)]
    body: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct RouterModel {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default, rename = "max_completion_tokens")]
    max_completion_tokens: Option<u64>,
    #[serde(default, rename = "inputTokenLimit")]
    input_token_limit: Option<u64>,
    #[serde(default, rename = "outputTokenLimit")]
    output_token_limit: Option<u64>,
    #[serde(default, rename = "supportedInputModalities")]
    supported_input_modalities: Option<Vec<String>>,
    #[serde(default, rename = "supported_parameters")]
    supported_parameters: Option<Vec<String>>,
    #[serde(default)]
    thinking: Option<RouterThinking>,
}

#[derive(Debug, Deserialize)]
struct RouterThinking {
    #[serde(default, rename = "min")]
    min: Option<u64>,
    #[serde(default, rename = "max")]
    max: Option<u64>,
    #[serde(default)]
    zero_allowed: Option<bool>,
    #[serde(default, rename = "dynamic_allowed")]
    _dynamic_allowed: Option<bool>,
    #[serde(default)]
    levels: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiCodexModelsDocument {
    #[serde(default)]
    models: Vec<OpenAiCodexModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiCodexModel {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    default_reasoning_level: Option<String>,
    #[serde(default)]
    supports_parallel_tool_calls: Option<bool>,
    #[serde(default)]
    support_verbosity: Option<bool>,
    #[serde(default)]
    default_verbosity: Option<String>,
    #[serde(default)]
    input_modalities: Option<Vec<String>>,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    max_context_window: Option<u64>,
    #[serde(default)]
    supported_reasoning_levels: Option<Vec<OpenAiCodexReasoningLevel>>,
    #[serde(default)]
    service_tiers: Option<Vec<OpenAiCodexServiceTier>>,
    #[serde(default)]
    additional_speed_tiers: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiCodexReasoningLevel {
    effort: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiCodexServiceTier {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OfficialHtmlReferencePage {
    title: String,
    slug: String,
}

#[derive(Debug, Default, Deserialize)]
struct OfficialHtmlSsrProps {
    #[serde(default)]
    sidebars: OfficialHtmlSsrSidebars,
}

#[derive(Debug, Default, Deserialize)]
struct OfficialHtmlSsrSidebars {
    #[serde(default)]
    refs: Vec<OfficialHtmlSsrSection>,
}

#[derive(Debug, Default, Deserialize)]
struct OfficialHtmlSsrSection {
    #[serde(default)]
    pages: Vec<OfficialHtmlSsrPage>,
}

#[derive(Debug, Default, Deserialize)]
struct OfficialHtmlSsrPage {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    children: Vec<OfficialHtmlSsrPage>,
}

impl OfficialHtmlSsrPage {
    fn as_reference_page(&self) -> Option<OfficialHtmlReferencePage> {
        if self.hidden {
            return None;
        }

        let title = normalize_optional_string(self.title.clone())?;
        let slug = normalize_optional_string(self.slug.clone())?;
        if !title.contains(" / ") || slug.ends_with("-infer") {
            return None;
        }

        Some(OfficialHtmlReferencePage { title, slug })
    }
}

#[derive(Debug, Clone, Copy)]
struct OfficialHtmlTokenLimits {
    context_window_tokens: u32,
    max_input_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct HuggingFaceHubModel {
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "modelId")]
    model_id: Option<String>,
    #[serde(default)]
    pipeline_tag: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default, rename = "createdAt")]
    created_at: Option<String>,
    #[serde(default)]
    private: bool,
}
