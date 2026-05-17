use std::collections::BTreeMap;

use crate::{
    AppError,
    model::{CapabilitySupport, ModelInputModality, ModelLifecycle, ModelVariantRequestOverride},
    provider::{
        ConfiguredModelVariant, FeatureCapabilityPatch, FeatureCapabilityPatchBody,
        InputCapabilityPatch, InputCapabilityPatchBody, ModelCapabilityFeature,
        ModelCapabilityPatch, ReasoningEffort, ThinkingRequest,
    },
};
use serde::Deserialize;

use super::{CatalogModelDefinition, ModelCatalogDocument, merge_catalog_definition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCatalogRemoteSourceKind {
    ModelsDev,
    RouterForMe,
}

#[derive(Debug, Clone)]
pub struct ModelCatalogRemoteSource {
    pub name: String,
    pub kind: ModelCatalogRemoteSourceKind,
    pub urls: Vec<String>,
}

impl ModelCatalogRemoteSource {
    pub fn new(
        name: impl Into<String>,
        kind: ModelCatalogRemoteSourceKind,
        urls: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            urls: urls.into_iter().collect(),
        }
    }
}

pub fn default_public_sources() -> Vec<ModelCatalogRemoteSource> {
    vec![
        ModelCatalogRemoteSource::new(
            "models.dev",
            ModelCatalogRemoteSourceKind::ModelsDev,
            [String::from("https://models.dev/api.json")],
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
    ]
}

pub async fn fetch_documents(
    client: &reqwest::Client,
    sources: &[ModelCatalogRemoteSource],
) -> (Vec<(String, ModelCatalogDocument)>, Vec<String>) {
    let mut documents = Vec::new();
    let mut warnings = Vec::new();

    for source in sources {
        match fetch_source_document(client, source).await {
            Ok(document) => documents.push((source.name.clone(), document)),
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
        ModelCatalogRemoteSourceKind::RouterForMe => parse_router_document(body.as_str()),
    }
}

fn parse_models_dev_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let providers: BTreeMap<String, ModelsDevProvider> = serde_json::from_str(body)?;
    let mut providers: Vec<_> = providers.into_iter().collect();
    providers.sort_by(|(left_key, left), (right_key, right)| {
        models_dev_provider_rank(right_key, right)
            .cmp(&models_dev_provider_rank(left_key, left))
            .then_with(|| left_key.cmp(right_key))
    });

    let mut document = ModelCatalogDocument::default();

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
                max_output_tokens: model
                    .limit
                    .as_ref()
                    .and_then(|limits| limits.output)
                    .map(clamp_u64_to_u32),
                description: None,
                display_name: normalize_optional_string(model.name),
                origin: origin.clone(),
                variants: models_dev_variants(model.experimental.as_ref(), adapter_id.as_deref()),
                capabilities: model_capability_patch(
                    modalities_to_support(model.modalities.as_ref()),
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
            };

            merge_document_entry(&mut document, model_id, definition);
        }
    }

    Ok(document)
}

fn parse_router_document(body: &str) -> Result<ModelCatalogDocument, AppError> {
    let sections: BTreeMap<String, Vec<RouterModel>> = serde_json::from_str(body)?;
    let mut document = ModelCatalogDocument::default();

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
            }

            let definition = CatalogModelDefinition {
                lifecycle: None,
                context_window_tokens: model
                    .context_length
                    .or(model.input_token_limit)
                    .map(clamp_u64_to_u32),
                max_output_tokens: model
                    .max_completion_tokens
                    .or(model.output_token_limit)
                    .map(clamp_u64_to_u32),
                description: normalize_optional_string(model.description),
                display_name: normalize_optional_string(model.display_name),
                origin,
                variants: router_variants(model.thinking.as_ref()),
                capabilities: model_capability_patch(
                    input_support,
                    (supported_features, unsupported_features),
                ),
            };

            merge_document_entry(&mut document, model_id, definition);
        }
    }

    Ok(document)
}

fn merge_document_entry(
    document: &mut ModelCatalogDocument,
    model_id: String,
    definition: CatalogModelDefinition,
) {
    document
        .models
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
        Some("deprecated") | Some("legacy") | Some("sunset") => Some(ModelLifecycle::Deprecated),
        _ => None,
    }
}

fn models_dev_variants(
    experimental: Option<&ModelsDevExperimental>,
    adapter_id: Option<&str>,
) -> BTreeMap<String, ConfiguredModelVariant> {
    let mut variants = BTreeMap::new();
    let Some(experimental) = experimental else {
        return variants;
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
        let mut adapter_overrides = BTreeMap::new();
        let mut default_request_override = ModelVariantRequestOverride::default();
        if request_override.is_empty() {
        } else if let Some(adapter_id) = adapter_id {
            adapter_overrides.insert(adapter_id.to_owned(), request_override);
        } else {
            default_request_override = request_override;
        }
        variants.insert(
            normalized.to_owned(),
            ConfiguredModelVariant {
                display_name: Some(title_case_tokenized(normalized)),
                description: None,
                thinking: effort_for_variant_name(normalized)
                    .map(|effort| ThinkingRequest::Effort { effort }),
                request_override: default_request_override,
                adapter_overrides,
                disabled: false,
            },
        );
    }
    variants
}

fn models_dev_request_override(provider: &ModelsDevModeProvider) -> ModelVariantRequestOverride {
    let headers = provider.headers.clone().unwrap_or_default();
    let body_patch = provider.body.clone().unwrap_or_default();
    ModelVariantRequestOverride {
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
        if let Some(mapped) = map_modality_name(modality) {
            if mapped != ModelInputModality::Text && !supported.contains(&mapped) {
                supported.push(mapped);
            }
        }
    }
    (supported, unsupported)
}

fn router_variants(thinking: Option<&RouterThinking>) -> BTreeMap<String, ConfiguredModelVariant> {
    let mut variants = BTreeMap::new();
    let Some(thinking) = thinking else {
        return variants;
    };

    if thinking.zero_allowed.unwrap_or(false) {
        variants.insert(
            "no-thinking".to_owned(),
            ConfiguredModelVariant {
                display_name: Some("No Thinking".to_owned()),
                description: None,
                thinking: Some(ThinkingRequest::Disabled),
                request_override: ModelVariantRequestOverride::default(),
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
            variants
                .entry(variant_name)
                .or_insert_with(|| ConfiguredModelVariant {
                    display_name: Some(format!("Thinking {}", title_case_tokenized(level))),
                    description: None,
                    thinking: Some(ThinkingRequest::Effort { effort }),
                    request_override: ModelVariantRequestOverride::default(),
                    adapter_overrides: BTreeMap::new(),
                    disabled: false,
                });
        }
    }

    if thinking.levels.is_empty() {
        if let Some(high_budget) = router_high_budget(thinking) {
            variants.insert(
                "thinking-high".to_owned(),
                ConfiguredModelVariant {
                    display_name: Some("Thinking High".to_owned()),
                    description: None,
                    thinking: Some(ThinkingRequest::Budget {
                        budget_tokens: high_budget,
                    }),
                    request_override: ModelVariantRequestOverride::default(),
                    adapter_overrides: BTreeMap::new(),
                    disabled: false,
                },
            );
        }

        if let Some(max_budget) = thinking.max.map(clamp_u64_to_u32) {
            variants.insert(
                "thinking-max".to_owned(),
                ConfiguredModelVariant {
                    display_name: Some("Thinking Max".to_owned()),
                    description: None,
                    thinking: Some(ThinkingRequest::Budget {
                        budget_tokens: max_budget,
                    }),
                    request_override: ModelVariantRequestOverride::default(),
                    adapter_overrides: BTreeMap::new(),
                    disabled: false,
                },
            );
        }
    }

    variants
}

fn router_high_budget(thinking: &RouterThinking) -> Option<u32> {
    let max_budget = thinking.max?;
    let min_budget = thinking.min.unwrap_or(0);
    let target = max_budget.min(16_384).max(min_budget);
    Some(clamp_u64_to_u32(target))
}

fn modalities_to_support(
    modalities: Option<&ModelsDevModalities>,
) -> (Vec<ModelInputModality>, Vec<ModelInputModality>) {
    let mut supported = Vec::new();
    let unsupported = Vec::new();
    let Some(modalities) = modalities else {
        return (supported, unsupported);
    };
    for modality in &modalities.input {
        if let Some(mapped) = map_modality_name(modality) {
            if mapped != ModelInputModality::Text && !supported.contains(&mapped) {
                supported.push(mapped);
            }
        }
    }
    (supported, unsupported)
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
        input: (!supported_inputs.is_empty() || !unsupported_inputs.is_empty()).then_some(
            InputCapabilityPatch::Patch(InputCapabilityPatchBody {
                supported: supported_inputs,
                unsupported: unsupported_inputs,
            }),
        ),
        features: (!supported_features.is_empty() || !unsupported_features.is_empty()).then_some(
            FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
                supported: supported_features,
                unsupported: unsupported_features,
            }),
        ),
        ..ModelCapabilityPatch::default()
    }
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

pub fn enrich_catalog_document_variants(document: &mut ModelCatalogDocument) {
    for (model_id, definition) in &mut document.models {
        let inferred = inferred_variants(model_id.as_str(), definition);
        for (name, variant) in inferred {
            definition.variants.entry(name).or_insert(variant);
        }
    }
}

fn inferred_variants(
    model_id: &str,
    definition: &CatalogModelDefinition,
) -> BTreeMap<String, ConfiguredModelVariant> {
    let mut variants = BTreeMap::new();
    if !matches!(
        definition
            .capabilities
            .feature_support(ModelCapabilityFeature::Reasoning),
        Some(CapabilitySupport::Supported)
    ) {
        return variants;
    }

    let normalized = model_id.trim().to_ascii_lowercase();
    if normalized.contains("gpt-5")
        || normalized.starts_with("o1")
        || normalized.starts_with("o3")
        || normalized.starts_with("o4")
    {
        for effort in openai_reasoning_efforts(normalized.as_str()) {
            insert_effort_variant(&mut variants, effort);
        }
        return variants;
    }

    if normalized.contains("gemini-3") {
        insert_effort_variant(&mut variants, ReasoningEffort::Low);
        insert_effort_variant(&mut variants, ReasoningEffort::High);
        return variants;
    }

    if normalized.contains("gemini-2.5") {
        insert_effort_variant(&mut variants, ReasoningEffort::High);
        insert_effort_variant(&mut variants, ReasoningEffort::Max);
        return variants;
    }

    if normalized.contains("claude") && definition.variants.is_empty() {
        insert_effort_variant(&mut variants, ReasoningEffort::High);
        insert_effort_variant(&mut variants, ReasoningEffort::Max);
    }

    variants
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

fn insert_effort_variant(
    variants: &mut BTreeMap<String, ConfiguredModelVariant>,
    effort: ReasoningEffort,
) {
    let effort_name = effort.as_str();
    variants
        .entry(format!("thinking-{effort_name}"))
        .or_insert_with(|| ConfiguredModelVariant {
            display_name: Some(format!("Thinking {}", title_case_tokenized(effort_name))),
            description: None,
            thinking: Some(ThinkingRequest::Effort { effort }),
            request_override: ModelVariantRequestOverride::default(),
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
    modalities: Option<ModelsDevModalities>,
    #[serde(default)]
    limit: Option<ModelsDevLimits>,
    #[serde(default)]
    experimental: Option<ModelsDevExperimental>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevModalities {
    #[serde(default)]
    input: Vec<String>,
    #[serde(default, rename = "output")]
    _output: Vec<String>,
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
