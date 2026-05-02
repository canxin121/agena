use std::sync::LazyLock;

use crate::model::{CapabilitySupport, ModelCapabilities};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityFamily {
    OpenAi,
    OpenAiCompatible,
    Anthropic,
    Gemini,
    Bedrock,
    Gitlab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRegistry {
    families: Vec<CapabilityFamilyProfile>,
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self {
            families: vec![
                CapabilityFamilyProfile::new(
                    CapabilityFamily::OpenAi,
                    openai_default_capabilities(),
                    vec![
                        ModelCapabilityRule::new(
                            ModelMatcher::any_prefix(["gpt-4o", "gpt-4.1", "gpt-5"]),
                            openai_multimodal_capabilities(),
                        )
                        .or(ModelMatcher::contains("codex")),
                        ModelCapabilityRule::new(
                            ModelMatcher::any_prefix(["o1", "o3", "o4"]),
                            openai_reasoning_capabilities(),
                        ),
                    ],
                ),
                CapabilityFamilyProfile::new(
                    CapabilityFamily::OpenAiCompatible,
                    openai_default_capabilities(),
                    vec![
                        ModelCapabilityRule::new(
                            ModelMatcher::any_prefix(["gpt-4o", "gpt-4.1", "gpt-5"]),
                            openai_multimodal_capabilities(),
                        )
                        .or(ModelMatcher::contains("codex")),
                        ModelCapabilityRule::new(
                            ModelMatcher::any_prefix(["o1", "o3", "o4"]),
                            openai_reasoning_capabilities(),
                        ),
                    ],
                ),
                CapabilityFamilyProfile::new(
                    CapabilityFamily::Anthropic,
                    anthropic_default_capabilities(),
                    vec![ModelCapabilityRule::new(
                        ModelMatcher::contains("claude"),
                        anthropic_claude_capabilities(),
                    )],
                ),
                CapabilityFamilyProfile::new(
                    CapabilityFamily::Gemini,
                    gemini_default_capabilities(),
                    vec![ModelCapabilityRule::new(
                        ModelMatcher::contains("gemini"),
                        gemini_multimodal_capabilities(),
                    )],
                ),
                CapabilityFamilyProfile::new(
                    CapabilityFamily::Bedrock,
                    bedrock_default_capabilities(),
                    vec![ModelCapabilityRule::new(
                        ModelMatcher::contains("claude"),
                        anthropic_claude_capabilities(),
                    )],
                ),
                CapabilityFamilyProfile::new(
                    CapabilityFamily::Gitlab,
                    gitlab_default_capabilities(),
                    vec![
                        ModelCapabilityRule::new(
                            ModelMatcher::contains("claude"),
                            anthropic_claude_capabilities(),
                        ),
                        ModelCapabilityRule::new(
                            ModelMatcher::any_prefix(["gpt-4o", "gpt-4.1", "gpt-5"]),
                            openai_multimodal_capabilities(),
                        )
                        .or(ModelMatcher::contains("gpt")),
                        ModelCapabilityRule::new(
                            ModelMatcher::any_prefix(["o1", "o3", "o4"]),
                            openai_reasoning_capabilities(),
                        ),
                    ],
                ),
            ],
        }
    }
}

impl CapabilityRegistry {
    pub fn capabilities_for_family(
        &self,
        family: CapabilityFamily,
        model: &str,
    ) -> ModelCapabilities {
        let normalized_model = normalize_model(model);
        self.families
            .iter()
            .find(|profile| profile.family == family)
            .map(|profile| profile.capabilities_for(normalized_model.as_str()))
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapabilityFamilyProfile {
    family: CapabilityFamily,
    fallback: ModelCapabilities,
    rules: Vec<ModelCapabilityRule>,
}

impl CapabilityFamilyProfile {
    fn new(
        family: CapabilityFamily,
        fallback: ModelCapabilities,
        rules: Vec<ModelCapabilityRule>,
    ) -> Self {
        Self {
            family,
            fallback,
            rules,
        }
    }

    fn capabilities_for(&self, normalized_model: &str) -> ModelCapabilities {
        self.rules
            .iter()
            .find(|rule| rule.matcher.matches(normalized_model))
            .map(|rule| rule.capabilities.clone())
            .unwrap_or_else(|| self.fallback.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelCapabilityRule {
    matcher: ModelMatcher,
    capabilities: ModelCapabilities,
}

impl ModelCapabilityRule {
    fn new(matcher: ModelMatcher, capabilities: ModelCapabilities) -> Self {
        Self {
            matcher,
            capabilities,
        }
    }

    fn or(mut self, matcher: ModelMatcher) -> Self {
        self.matcher = self.matcher.or(matcher);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModelMatcher {
    Prefix(String),
    Contains(String),
    AnyOf(Vec<ModelMatcher>),
}

impl ModelMatcher {
    fn prefix(value: impl Into<String>) -> Self {
        Self::Prefix(value.into())
    }

    fn any_prefix<const N: usize>(values: [&str; N]) -> Self {
        Self::AnyOf(values.into_iter().map(Self::prefix).collect())
    }

    fn contains(value: impl Into<String>) -> Self {
        Self::Contains(value.into())
    }

    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::AnyOf(mut left), Self::AnyOf(right)) => {
                left.extend(right);
                Self::AnyOf(left)
            }
            (Self::AnyOf(mut left), right) => {
                left.push(right);
                Self::AnyOf(left)
            }
            (left, Self::AnyOf(mut right)) => {
                let mut values = vec![left];
                values.append(&mut right);
                Self::AnyOf(values)
            }
            (left, right) => Self::AnyOf(vec![left, right]),
        }
    }

    fn matches(&self, normalized_model: &str) -> bool {
        match self {
            Self::Prefix(prefix) => normalized_model.starts_with(prefix),
            Self::Contains(fragment) => normalized_model.contains(fragment),
            Self::AnyOf(values) => values.iter().any(|value| value.matches(normalized_model)),
        }
    }
}

pub fn default_capability_registry() -> &'static CapabilityRegistry {
    static REGISTRY: LazyLock<CapabilityRegistry> = LazyLock::new(CapabilityRegistry::default);
    &REGISTRY
}

fn normalize_model(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

fn openai_default_capabilities() -> ModelCapabilities {
    ModelCapabilities::default()
        .with_tool_calling(CapabilitySupport::Supported)
        .with_streaming(CapabilitySupport::Supported)
        .with_structured_output(CapabilitySupport::Unknown)
        .with_reasoning(CapabilitySupport::Unsupported)
        .with_temperature_supported(CapabilitySupport::Supported)
}

fn openai_multimodal_capabilities() -> ModelCapabilities {
    openai_default_capabilities()
        .with_image_input(CapabilitySupport::Supported)
        .with_document_input(CapabilitySupport::Supported)
        .with_file_input(CapabilitySupport::Supported)
        .with_structured_output(CapabilitySupport::Supported)
}

fn openai_reasoning_capabilities() -> ModelCapabilities {
    openai_multimodal_capabilities()
        .with_reasoning(CapabilitySupport::Supported)
        .with_structured_output(CapabilitySupport::Supported)
        .with_temperature_supported(CapabilitySupport::Unsupported)
}

fn anthropic_default_capabilities() -> ModelCapabilities {
    ModelCapabilities::default()
        .with_tool_calling(CapabilitySupport::Supported)
        .with_streaming(CapabilitySupport::Supported)
        .with_audio_input(CapabilitySupport::Unsupported)
        .with_video_input(CapabilitySupport::Unsupported)
        .with_reasoning(CapabilitySupport::Unsupported)
        .with_structured_output(CapabilitySupport::Unsupported)
}

fn anthropic_claude_capabilities() -> ModelCapabilities {
    anthropic_default_capabilities()
        .with_image_input(CapabilitySupport::Supported)
        .with_document_input(CapabilitySupport::Supported)
        .with_file_input(CapabilitySupport::Unsupported)
        .with_reasoning(CapabilitySupport::Supported)
}

fn gemini_default_capabilities() -> ModelCapabilities {
    ModelCapabilities::default()
        .with_tool_calling(CapabilitySupport::Unsupported)
        .with_streaming(CapabilitySupport::Supported)
        .with_structured_output(CapabilitySupport::Supported)
        .with_reasoning(CapabilitySupport::Unsupported)
}

fn gemini_multimodal_capabilities() -> ModelCapabilities {
    gemini_default_capabilities()
        .with_tool_calling(CapabilitySupport::Supported)
        .with_image_input(CapabilitySupport::Supported)
        .with_document_input(CapabilitySupport::Supported)
        .with_file_input(CapabilitySupport::Supported)
        .with_reasoning(CapabilitySupport::Supported)
}

fn bedrock_default_capabilities() -> ModelCapabilities {
    ModelCapabilities::default()
        .with_tool_calling(CapabilitySupport::Supported)
        .with_streaming(CapabilitySupport::Supported)
}

fn gitlab_default_capabilities() -> ModelCapabilities {
    ModelCapabilities::default()
        .with_tool_calling(CapabilitySupport::Supported)
        .with_streaming(CapabilitySupport::Supported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_family_marks_gpt5_as_multimodal() {
        let registry = CapabilityRegistry::default();
        let capabilities = registry.capabilities_for_family(CapabilityFamily::OpenAi, "gpt-5");
        assert_eq!(capabilities.image_input, CapabilitySupport::Supported);
        assert_eq!(capabilities.document_input, CapabilitySupport::Supported);
        assert_eq!(capabilities.file_input, CapabilitySupport::Supported);
        assert_eq!(capabilities.structured_output, CapabilitySupport::Supported);
        assert_eq!(capabilities.reasoning, CapabilitySupport::Unsupported);
        assert_eq!(
            capabilities.temperature_supported,
            CapabilitySupport::Supported
        );
    }

    #[test]
    fn openai_family_marks_o_series_as_reasoning_no_temperature() {
        let registry = CapabilityRegistry::default();
        for model in ["o1", "o3-mini", "o4-mini"] {
            let capabilities = registry.capabilities_for_family(CapabilityFamily::OpenAi, model);
            assert_eq!(
                capabilities.reasoning,
                CapabilitySupport::Supported,
                "{model} should support reasoning"
            );
            assert_eq!(
                capabilities.temperature_supported,
                CapabilitySupport::Unsupported,
                "{model} should not support temperature"
            );
            assert_eq!(capabilities.structured_output, CapabilitySupport::Supported);
        }
    }

    #[test]
    fn anthropic_family_marks_claude_files_as_unsupported() {
        let registry = CapabilityRegistry::default();
        let capabilities =
            registry.capabilities_for_family(CapabilityFamily::Anthropic, "claude-sonnet-4-5");
        assert_eq!(capabilities.image_input, CapabilitySupport::Supported);
        assert_eq!(capabilities.document_input, CapabilitySupport::Supported);
        assert_eq!(capabilities.file_input, CapabilitySupport::Unsupported);
        assert_eq!(capabilities.reasoning, CapabilitySupport::Supported);
        assert_eq!(
            capabilities.structured_output,
            CapabilitySupport::Unsupported
        );
    }

    #[test]
    fn gemini_family_marks_gemini_models_as_multimodal_with_reasoning() {
        let registry = CapabilityRegistry::default();
        let capabilities =
            registry.capabilities_for_family(CapabilityFamily::Gemini, "gemini-2.0-flash");
        assert_eq!(capabilities.image_input, CapabilitySupport::Supported);
        assert_eq!(capabilities.reasoning, CapabilitySupport::Supported);
        assert_eq!(capabilities.structured_output, CapabilitySupport::Supported);
    }
}
