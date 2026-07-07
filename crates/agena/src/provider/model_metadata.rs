use std::sync::LazyLock;

use crate::model::{
    ModelLifecycle, ModelMetadata, ModelTokenLimits, normalize_model_default_temperature,
    normalize_model_default_top_k, normalize_model_default_top_p,
};

use super::CapabilityFamily;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelMetadataRegistry;

impl ModelMetadataRegistry {
    pub fn metadata_for_family(&self, _family: CapabilityFamily, model: &str) -> ModelMetadata {
        let normalized_model = normalize_model(model);
        ModelMetadata {
            lifecycle: detect_model_lifecycle(normalized_model.as_str()),
            limits: ModelTokenLimits {
                context_window_tokens: detect_context_window_tokens(normalized_model.as_str()),
                max_input_tokens: None,
                max_output_tokens: None,
            },
            description: None,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: normalize_model_default_temperature(
                detect_default_temperature(normalized_model.as_str()).map(ToOwned::to_owned),
            ),
            default_top_p: normalize_model_default_top_p(
                detect_default_top_p(normalized_model.as_str()).map(ToOwned::to_owned),
            ),
            default_top_k: normalize_model_default_top_k(detect_default_top_k(
                normalized_model.as_str(),
            )),
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
        }
    }
}

pub fn default_model_metadata_registry() -> &'static ModelMetadataRegistry {
    static REGISTRY: LazyLock<ModelMetadataRegistry> =
        LazyLock::new(ModelMetadataRegistry::default);
    &REGISTRY
}

fn normalize_model(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

fn detect_model_lifecycle(model: &str) -> Option<ModelLifecycle> {
    if model.contains("claude-3-opus")
        || model.contains("claude-3-7-sonnet")
        || model.contains("claude-3-5-haiku")
        || model.contains("deprecated")
    {
        return Some(ModelLifecycle::Deprecated);
    }
    if model.contains("preview") {
        return Some(ModelLifecycle::Preview);
    }
    if model.contains("beta") {
        return Some(ModelLifecycle::Beta);
    }
    if model.contains("alpha") {
        return Some(ModelLifecycle::Alpha);
    }
    if model.starts_with("exp-") || model.contains("experimental") {
        return Some(ModelLifecycle::Experimental);
    }
    Some(ModelLifecycle::Active)
}

fn detect_context_window_tokens(model: &str) -> Option<u32> {
    [
        ("1m", 1_000_000_u32),
        ("200k", 200_000_u32),
        ("128k", 128_000_u32),
        ("64k", 64_000_u32),
        ("32k", 32_000_u32),
        ("16k", 16_000_u32),
    ]
    .into_iter()
    .find_map(|(pattern, tokens)| model.contains(pattern).then_some(tokens))
}

fn detect_default_temperature(model: &str) -> Option<&'static str> {
    if model.contains("qwen") {
        return Some("0.55");
    }
    if model.contains("claude") {
        return None;
    }
    if model.contains("gemini")
        || model.contains("glm-4.6")
        || model.contains("glm-4.7")
        || model.contains("minimax-m2")
    {
        return Some("1.0");
    }
    if model.contains("kimi-k2") {
        if ["thinking", "k2.", "k2p", "k2-5"]
            .into_iter()
            .any(|pattern| model.contains(pattern))
        {
            return Some("1.0");
        }
        return Some("0.6");
    }
    None
}

fn detect_default_top_p(model: &str) -> Option<&'static str> {
    if model.contains("qwen") {
        return Some("1.0");
    }
    if [
        "minimax-m2",
        "gemini",
        "kimi-k2.5",
        "kimi-k2p5",
        "kimi-k2-5",
    ]
    .into_iter()
    .any(|pattern| model.contains(pattern))
    {
        return Some("0.95");
    }
    None
}

fn detect_default_top_k(model: &str) -> Option<u32> {
    if model.contains("minimax-m2") {
        if ["m2.", "m25", "m21"]
            .into_iter()
            .any(|pattern| model.contains(pattern))
        {
            return Some(40);
        }
        return Some(20);
    }
    model.contains("gemini").then_some(64)
}
