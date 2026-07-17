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
use futures_util::{future::join_all, stream};
use regex::Regex;
use serde::Deserialize;
use std::sync::OnceLock;

use super::{
    CatalogDefinitionSourcePriority, CatalogModelDefinition, ModelCatalogDocument,
    canonical_model_catalog_id, merge_catalog_definition,
};

mod sources_enrichment;
mod sources_fetch;

pub(crate) use sources_enrichment::enrich_catalog_document_thinking_modes;
pub(super) use sources_enrichment::*;
pub(crate) use sources_fetch::fetch_documents;

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
        descriptive_priority: if definition.lifecycle.is_some()
            || definition.description.is_some()
            || definition.knowledge_cutoff.is_some()
            || definition.release_date.is_some()
            || definition.last_updated.is_some()
            || definition.open_weights.is_some()
            || definition.display_name.is_some()
            || definition.origin.is_some()
        {
            base.descriptive_priority
        } else {
            Default::default()
        },
        limits_priority: if definition.context_window_tokens.is_some()
            || definition.max_input_tokens.is_some()
            || definition.max_output_tokens.is_some()
        {
            base.limits_priority
        } else {
            Default::default()
        },
        capability_priority: if !definition.output_modalities.is_empty()
            || !definition.capabilities.is_empty()
        {
            base.capability_priority
        } else {
            Default::default()
        },
        semantics_priority: if definition.default_thinking_mode.is_some()
            || definition.supports_parallel_tool_calls.is_some()
            || definition.supports_verbosity.is_some()
            || definition.default_verbosity.is_some()
            || definition.default_temperature.is_some()
            || definition.default_top_p.is_some()
            || definition.default_top_k.is_some()
            || definition.assistant_reasoning_interleaved.is_some()
            || definition.assistant_reasoning_field.is_some()
        {
            base.semantics_priority
        } else {
            Default::default()
        },
        pricing_priority: if definition.pricing.is_some() {
            base.pricing_priority
        } else {
            Default::default()
        },
        mode_priority: if !definition.thinking_modes.is_empty()
            || !definition.speed_modes.is_empty()
        {
            base.mode_priority
        } else {
            Default::default()
        },
    }
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
