use serde::Deserialize;

use crate::model::{
    CapabilitySupport, ModelCapabilities, ModelMetadata, ModelTokenLimits, clamp_u64_to_u32,
};

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct CopilotModelExtension {
    #[serde(default)]
    model_picker_enabled: Option<bool>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    supported_endpoints: Vec<String>,
    #[serde(default)]
    policy: Option<CopilotModelPolicy>,
    #[serde(default)]
    capabilities: Option<CopilotModelCapabilities>,
}

impl CopilotModelExtension {
    pub(crate) fn visible(&self) -> bool {
        self.model_picker_enabled.unwrap_or(true)
            && !self
                .policy
                .as_ref()
                .and_then(|policy| policy.state.as_deref())
                .is_some_and(|state| state.eq_ignore_ascii_case("disabled"))
    }

    pub(crate) fn uses_messages_endpoint(&self) -> bool {
        self.supported_endpoints
            .iter()
            .any(|endpoint| endpoint == "/v1/messages")
    }

    pub(crate) fn metadata(&self, model_id: &str) -> ModelMetadata {
        let Some(capabilities) = self.capabilities.as_ref() else {
            return self.metadata_with_limits(ModelTokenLimits::default(), model_id);
        };
        let limits = capabilities
            .limits
            .as_ref()
            .map(|limits| ModelTokenLimits {
                context_window_tokens: limits.max_context_window_tokens.map(clamp_u64_to_u32),
                max_input_tokens: limits.max_prompt_tokens.map(clamp_u64_to_u32),
                max_output_tokens: limits.max_output_tokens.map(clamp_u64_to_u32),
            })
            .unwrap_or_default();
        self.metadata_with_limits(limits, model_id)
    }

    pub(crate) fn capabilities(&self) -> ModelCapabilities {
        let Some(capabilities) = self.capabilities.as_ref() else {
            return ModelCapabilities::default();
        };
        let Some(supports) = capabilities.supports.as_ref() else {
            return ModelCapabilities::default();
        };

        let image = supports.vision.unwrap_or(false)
            || capabilities
                .limits
                .as_ref()
                .and_then(|limits| limits.vision.as_ref())
                .is_some_and(|vision| {
                    vision
                        .supported_media_types
                        .iter()
                        .any(|media_type| media_type.starts_with("image/"))
                });

        let reasoning = supports.adaptive_thinking.unwrap_or(false)
            || supports
                .reasoning_effort
                .as_ref()
                .is_some_and(|efforts| !efforts.is_empty())
            || supports.max_thinking_budget.is_some()
            || supports.min_thinking_budget.is_some();
        ModelCapabilities {
            streaming: support_bool(supports.streaming),
            tool_calling: support_bool(supports.tool_calls),
            structured_output: support_bool(supports.structured_outputs),
            image_input: if image {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unsupported
            },
            reasoning: if reasoning {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unsupported
            },
            ..ModelCapabilities::default()
        }
    }

    fn metadata_with_limits(&self, limits: ModelTokenLimits, model_id: &str) -> ModelMetadata {
        let release_date = self.version.as_deref().map(|version| {
            version
                .strip_prefix(model_id)
                .and_then(|suffix| suffix.strip_prefix('-'))
                .unwrap_or(version)
                .to_owned()
        });
        ModelMetadata {
            lifecycle: None,
            limits,
            description: None,
            knowledge_cutoff: None,
            release_date,
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
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CopilotModelPolicy {
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CopilotModelCapabilities {
    #[serde(default)]
    limits: Option<CopilotModelLimits>,
    #[serde(default)]
    supports: Option<CopilotModelSupports>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CopilotModelLimits {
    #[serde(default)]
    max_context_window_tokens: Option<u64>,
    #[serde(default)]
    max_output_tokens: Option<u64>,
    #[serde(default)]
    max_prompt_tokens: Option<u64>,
    #[serde(default)]
    vision: Option<CopilotVisionLimits>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CopilotVisionLimits {
    #[serde(default)]
    supported_media_types: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CopilotModelSupports {
    #[serde(default)]
    adaptive_thinking: Option<bool>,
    #[serde(default)]
    max_thinking_budget: Option<u64>,
    #[serde(default)]
    min_thinking_budget: Option<u64>,
    #[serde(default)]
    reasoning_effort: Option<Vec<String>>,
    #[serde(default)]
    streaming: Option<bool>,
    #[serde(default)]
    structured_outputs: Option<bool>,
    #[serde(default)]
    tool_calls: Option<bool>,
    #[serde(default)]
    vision: Option<bool>,
}

fn support_bool(value: Option<bool>) -> CapabilitySupport {
    match value {
        Some(true) => CapabilitySupport::Supported,
        Some(false) => CapabilitySupport::Unsupported,
        None => CapabilitySupport::Unknown,
    }
}
