use std::sync::LazyLock;

use crate::model::{ModelLifecycle, ModelMetadata};

use super::CapabilityFamily;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelMetadataRegistry;

impl ModelMetadataRegistry {
    pub fn metadata_for_family(&self, _family: CapabilityFamily, model: &str) -> ModelMetadata {
        let normalized_model = normalize_model(model);
        let mut metadata = ModelMetadata::default();

        if let Some(lifecycle) = detect_model_lifecycle(normalized_model.as_str()) {
            metadata = metadata.with_lifecycle(lifecycle);
        }
        if let Some(context_window_tokens) = detect_context_window_tokens(normalized_model.as_str())
        {
            metadata = metadata.with_context_window_tokens(context_window_tokens);
        }
        if let Some(default_temperature) = detect_default_temperature(normalized_model.as_str()) {
            metadata = metadata.with_default_temperature(default_temperature);
        }
        if let Some(default_top_p) = detect_default_top_p(normalized_model.as_str()) {
            metadata = metadata.with_default_top_p(default_top_p);
        }
        if let Some(default_top_k) = detect_default_top_k(normalized_model.as_str()) {
            metadata = metadata.with_default_top_k(default_top_k);
        }

        metadata
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
