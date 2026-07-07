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
                            ModelMatcher::any(vec![
                                ModelMatcher::any_prefix(["gpt-4o", "gpt-4.1", "gpt-5"]),
                                ModelMatcher::contains("codex"),
                            ]),
                            openai_multimodal_capabilities(),
                        ),
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
                            ModelMatcher::any(vec![
                                ModelMatcher::any_prefix(["gpt-4o", "gpt-4.1", "gpt-5"]),
                                ModelMatcher::contains("codex"),
                            ]),
                            openai_multimodal_capabilities(),
                        ),
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
                    vec![
                        ModelCapabilityRule::new(
                            ModelMatcher::any(vec![
                                ModelMatcher::contains("opus-4-7"),
                                ModelMatcher::contains("opus-4.7"),
                            ]),
                            ModelCapabilities {
                                temperature_supported: CapabilitySupport::Unsupported,
                                ..anthropic_claude_capabilities()
                            },
                        ),
                        ModelCapabilityRule::new(
                            ModelMatcher::contains("claude"),
                            anthropic_claude_capabilities(),
                        ),
                    ],
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
                            ModelMatcher::any(vec![
                                ModelMatcher::any_prefix(["gpt-4o", "gpt-4.1", "gpt-5"]),
                                ModelMatcher::contains("gpt"),
                            ]),
                            openai_multimodal_capabilities(),
                        ),
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

    fn any(values: Vec<ModelMatcher>) -> Self {
        Self::AnyOf(values)
    }

    fn contains(value: impl Into<String>) -> Self {
        Self::Contains(value.into())
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
    ModelCapabilities {
        tool_calling: CapabilitySupport::Supported,
        streaming: CapabilitySupport::Supported,
        structured_output: CapabilitySupport::Unknown,
        reasoning: CapabilitySupport::Unsupported,
        temperature_supported: CapabilitySupport::Supported,
        ..ModelCapabilities::default()
    }
}

fn openai_multimodal_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        image_input: CapabilitySupport::Supported,
        document_input: CapabilitySupport::Supported,
        file_input: CapabilitySupport::Supported,
        structured_output: CapabilitySupport::Supported,
        ..openai_default_capabilities()
    }
}

fn openai_reasoning_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        reasoning: CapabilitySupport::Supported,
        structured_output: CapabilitySupport::Supported,
        temperature_supported: CapabilitySupport::Unsupported,
        ..openai_multimodal_capabilities()
    }
}

fn anthropic_default_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        tool_calling: CapabilitySupport::Supported,
        streaming: CapabilitySupport::Supported,
        audio_input: CapabilitySupport::Unsupported,
        video_input: CapabilitySupport::Unsupported,
        reasoning: CapabilitySupport::Unsupported,
        structured_output: CapabilitySupport::Unsupported,
        ..ModelCapabilities::default()
    }
}

fn anthropic_claude_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        image_input: CapabilitySupport::Supported,
        document_input: CapabilitySupport::Supported,
        file_input: CapabilitySupport::Unsupported,
        reasoning: CapabilitySupport::Supported,
        ..anthropic_default_capabilities()
    }
}

fn gemini_default_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        tool_calling: CapabilitySupport::Unsupported,
        streaming: CapabilitySupport::Supported,
        structured_output: CapabilitySupport::Supported,
        reasoning: CapabilitySupport::Unsupported,
        ..ModelCapabilities::default()
    }
}

fn gemini_multimodal_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        tool_calling: CapabilitySupport::Supported,
        image_input: CapabilitySupport::Supported,
        document_input: CapabilitySupport::Supported,
        file_input: CapabilitySupport::Supported,
        reasoning: CapabilitySupport::Supported,
        ..gemini_default_capabilities()
    }
}

fn bedrock_default_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        tool_calling: CapabilitySupport::Supported,
        streaming: CapabilitySupport::Supported,
        ..ModelCapabilities::default()
    }
}

fn gitlab_default_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        tool_calling: CapabilitySupport::Supported,
        streaming: CapabilitySupport::Supported,
        ..ModelCapabilities::default()
    }
}
