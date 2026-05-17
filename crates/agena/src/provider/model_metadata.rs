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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_preview_lifecycle() {
        let metadata = default_model_metadata_registry()
            .metadata_for_family(CapabilityFamily::OpenAi, "gpt-5-preview");
        assert_eq!(metadata.lifecycle, Some(ModelLifecycle::Preview));
    }

    #[test]
    fn infers_context_window_from_model_slug() {
        let metadata = default_model_metadata_registry()
            .metadata_for_family(CapabilityFamily::OpenAi, "gpt-4-32k");
        assert_eq!(metadata.limits.context_window_tokens, Some(32_000));
    }

    #[test]
    fn marks_known_deprecated_claude_lineages() {
        let metadata = default_model_metadata_registry()
            .metadata_for_family(CapabilityFamily::Anthropic, "claude-3-7-sonnet-latest");
        assert_eq!(metadata.lifecycle, Some(ModelLifecycle::Deprecated));
    }
}
