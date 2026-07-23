use std::{collections::BTreeMap, sync::Arc};

use agena_domain::{ModelInputModality, ModelLifecycle, ModelSpeedModeRequestOverride};
use agena_provider::{
    CatalogDefinitionSourcePriority, CatalogModelDefinition, ModelCatalogDocument,
    merge_catalog_definition,
};
use futures_util::stream;
use regex::Regex;
use serde::Deserialize;
use std::sync::OnceLock;
use thiserror::Error;

use crate::{
    ModelCatalogConfiguredPublicSource, ModelCatalogPublicSource,
    ModelCatalogRemoteDocumentFetcher, ModelCatalogRemoteSource, ModelCatalogRemoteSourceKind,
};
use crate::{default_public_model_catalog_sources, public_model_catalog_sources_enabled};
use agena_provider::normalized_catalog_model_id as canonical_model_catalog_id;

mod sources_enrichment;
mod sources_fetch;

use sources_enrichment::*;
pub(crate) use sources_fetch::fetch_source_document;

#[derive(Debug, Error)]
pub enum ModelCatalogHttpError {
    #[error("model catalog source error: {0}")]
    Source(String),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub fn build_model_catalog_public_source(
    user_agent: impl Into<String>,
    sources: Vec<ModelCatalogRemoteSource>,
) -> Result<Arc<dyn ModelCatalogPublicSource>, ModelCatalogHttpError> {
    let user_agent = user_agent.into();
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(user_agent)
        .build()?;
    Ok(Arc::new(ModelCatalogConfiguredPublicSource::new(
        Arc::new(HttpModelCatalogDocumentFetcher { client }),
        sources,
    )))
}

/// Builds the enabled default public-source adapter. Runtime owns both the
/// executable source list and the environment enablement policy; composition
/// supplies only process-specific HTTP identity.
pub fn build_default_public_model_catalog_source(
    user_agent: impl Into<String>,
) -> Result<Arc<dyn ModelCatalogPublicSource>, ModelCatalogHttpError> {
    let sources = public_model_catalog_sources_enabled()
        .then(default_public_model_catalog_sources)
        .unwrap_or_default();
    build_model_catalog_public_source(user_agent, sources)
}

struct HttpModelCatalogDocumentFetcher {
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl ModelCatalogRemoteDocumentFetcher for HttpModelCatalogDocumentFetcher {
    async fn fetch_document(
        &self,
        source: &ModelCatalogRemoteSource,
    ) -> Result<ModelCatalogDocument, String> {
        fetch_source_document(&self.client, source)
            .await
            .map_err(|error| error.to_string())
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
