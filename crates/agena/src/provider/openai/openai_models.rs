use super::{
    BTreeSet, CapabilitySupport, ChatCompletionResponse, CopilotModelExtension, Deserialize,
    ModelCapabilities, ModelInputModality, ModelMetadata, ModelThinkingMode, ModelTokenLimits,
    clamp_u64_to_u32, model_supports_input_modality, utils,
};
use crate::provider::{ReasoningEffort, ThinkingRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DashscopeReasoningProfile {
    Toggleable,
    AlwaysOn,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum OpenAiModelListResponse {
    CodexWrapped {
        models: Vec<OpenAiCodexModel>,
    },
    Wrapped {
        data: Vec<OpenAiCompatibleModel>,
    },
    ClineRecommended {
        #[serde(default)]
        recommended: Vec<OpenAiRecommendedModel>,
        #[serde(default)]
        free: Vec<OpenAiRecommendedModel>,
        #[serde(rename = "clinePass", default)]
        cline_pass: Vec<OpenAiRecommendedModel>,
    },
    Bare(Vec<OpenAiCompatibleModel>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum OpenAiChatCompletionResponse {
    Wrapped {
        data: ChatCompletionResponse,
        #[serde(default, rename = "success")]
        _success: Option<bool>,
    },
    Bare(ChatCompletionResponse),
}

impl OpenAiModelListResponse {
    pub(super) fn into_items(
        self,
        provider_id: &str,
        models_url: Option<&str>,
    ) -> Vec<OpenAiListedModel> {
        match self {
            Self::CodexWrapped { models } => {
                models.into_iter().map(OpenAiListedModel::Codex).collect()
            }
            Self::Wrapped { data } => data
                .into_iter()
                .map(OpenAiListedModel::Compatible)
                .collect(),
            Self::ClineRecommended {
                recommended,
                free,
                cline_pass,
            } => cline_recommended_models_for_provider(
                provider_id,
                models_url,
                recommended,
                free,
                cline_pass,
            )
            .into_iter()
            .map(OpenAiListedModel::Recommended)
            .collect(),
            Self::Bare(data) => data
                .into_iter()
                .map(OpenAiListedModel::Compatible)
                .collect(),
        }
    }
}

#[derive(Debug)]
pub(super) enum OpenAiListedModel {
    Compatible(OpenAiCompatibleModel),
    Recommended(OpenAiRecommendedModel),
    Codex(OpenAiCodexModel),
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiCompatibleModel {
    pub(super) id: String,
    #[serde(default, flatten)]
    pub(super) copilot: CopilotModelExtension,
    #[serde(default)]
    pub(super) display_name: Option<String>,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default, alias = "context_length")]
    pub(super) context_window_tokens: Option<u64>,
    #[serde(default, alias = "input_token_limit")]
    pub(super) max_input_tokens: Option<u64>,
    #[serde(default, alias = "max_completion_tokens")]
    pub(super) max_output_tokens: Option<u64>,
}

impl OpenAiCompatibleModel {
    pub(super) fn metadata(&self) -> ModelMetadata {
        let metadata = ModelMetadata {
            lifecycle: None,
            limits: ModelTokenLimits {
                context_window_tokens: self
                    .context_window_tokens
                    .or(self.max_input_tokens)
                    .map(clamp_u64_to_u32),
                max_input_tokens: self.max_input_tokens.map(clamp_u64_to_u32),
                max_output_tokens: self.max_output_tokens.map(clamp_u64_to_u32),
            },
            description: None,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
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
        };
        metadata.merged_with_fallbacks_from(&self.copilot.metadata(self.id.as_str()))
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiRecommendedModel {
    pub(super) id: String,
    #[serde(default)]
    pub(super) display_name: Option<String>,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
}

impl OpenAiRecommendedModel {
    pub(super) fn metadata(&self) -> ModelMetadata {
        ModelMetadata {
            lifecycle: None,
            limits: ModelTokenLimits::default(),
            description: self
                .description
                .clone()
                .and_then(|value| utils::normalize_optional_text(Some(value))),
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
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

pub(super) fn cline_recommended_models_for_provider(
    provider_id: &str,
    models_url: Option<&str>,
    recommended: Vec<OpenAiRecommendedModel>,
    free: Vec<OpenAiRecommendedModel>,
    cline_pass: Vec<OpenAiRecommendedModel>,
) -> Vec<OpenAiRecommendedModel> {
    let provider_key = provider_id.trim().to_ascii_lowercase();
    let models_url_key = models_url.unwrap_or_default().trim().to_ascii_lowercase();
    let is_cline_pass_provider = provider_key.contains("cline-pass")
        || provider_key.contains("cline_pass")
        || provider_key.contains("cline_api")
        || provider_key.contains("clineapi")
        || models_url_key.contains("/ai/cline/recommended-models");
    let mut selected = if is_cline_pass_provider {
        cline_pass
    } else {
        let mut combined = recommended;
        combined.extend(free);
        if combined.is_empty() {
            cline_pass
        } else {
            combined
        }
    };

    let mut seen = BTreeSet::new();
    selected.retain(|model| {
        let Some(id) = utils::normalize_optional_text(Some(model.id.clone())) else {
            return false;
        };
        seen.insert(id)
    });
    selected
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiCodexModel {
    #[serde(default)]
    pub(super) slug: String,
    #[serde(default)]
    pub(super) display_name: Option<String>,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) default_reasoning_level: Option<String>,
    #[serde(default)]
    pub(super) supported_reasoning_levels: Vec<OpenAiCodexReasoningLevel>,
    #[serde(default)]
    pub(super) support_verbosity: Option<bool>,
    #[serde(default)]
    pub(super) default_verbosity: Option<String>,
    #[serde(default)]
    pub(super) supports_parallel_tool_calls: Option<bool>,
    #[serde(default)]
    pub(super) context_window: Option<u64>,
    #[serde(default)]
    pub(super) max_context_window: Option<u64>,
    #[serde(default)]
    pub(super) input_modalities: Vec<String>,
}

impl OpenAiCodexModel {
    pub(super) fn thinking_modes(&self) -> Vec<ModelThinkingMode> {
        let default_selector = self
            .default_reasoning_level
            .as_deref()
            .and_then(codex_reasoning_selector);
        let mut modes = Vec::new();
        for level in &self.supported_reasoning_levels {
            let Some(selector) = codex_reasoning_selector(level.effort.as_str()) else {
                continue;
            };
            let thinking = if selector == "off" {
                ThinkingRequest::Disabled
            } else {
                let Some(effort) = reasoning_effort_from_selector(selector) else {
                    continue;
                };
                ThinkingRequest::Effort { effort }
            };
            if modes
                .iter()
                .any(|mode: &ModelThinkingMode| mode.selector().as_deref() == Some(selector))
            {
                continue;
            }
            modes.push(ModelThinkingMode {
                is_default: default_selector == Some(selector),
                display_name: None,
                description: level
                    .description
                    .as_ref()
                    .and_then(|value| utils::normalize_optional_text(Some(value.clone()))),
                preset: None,
                thinking: Some(thinking),
                request_override: Default::default(),
                adapter_overrides: Default::default(),
            });
        }
        if let Some(default_selector) = default_selector
            && !modes
                .iter()
                .any(|mode| mode.selector().as_deref() == Some(default_selector))
            && let Some(mode) = codex_thinking_mode(default_selector, None, true)
        {
            modes.push(mode);
        }
        modes
    }

    pub(super) fn metadata(&self) -> ModelMetadata {
        let description = self
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let default_verbosity = self
            .default_verbosity
            .as_ref()
            .and_then(|value| utils::normalize_optional_text(Some(value.clone())));
        ModelMetadata {
            lifecycle: None,
            limits: ModelTokenLimits {
                context_window_tokens: self
                    .context_window
                    .or(self.max_context_window)
                    .map(clamp_u64_to_u32),
                max_input_tokens: None,
                max_output_tokens: None,
            },
            description,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            supports_parallel_tool_calls: self.supports_parallel_tool_calls,
            supports_verbosity: self.support_verbosity,
            default_verbosity,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
        }
    }

    pub(super) fn capabilities(&self) -> ModelCapabilities {
        let supports = |modality: ModelInputModality| {
            if self.input_modalities.is_empty() {
                return CapabilitySupport::Unknown;
            }
            if self
                .input_modalities
                .iter()
                .any(|value| model_supports_input_modality(value.as_str(), modality))
            {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unsupported
            }
        };
        let text_input = match supports(ModelInputModality::Text) {
            CapabilitySupport::Unsupported => CapabilitySupport::Unsupported,
            _ => CapabilitySupport::Supported,
        };
        ModelCapabilities {
            text_input,
            image_input: supports(ModelInputModality::Image),
            document_input: supports(ModelInputModality::Document),
            audio_input: supports(ModelInputModality::Audio),
            video_input: supports(ModelInputModality::Video),
            file_input: supports(ModelInputModality::File),
            tool_calling: if self.supports_parallel_tool_calls == Some(true) {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unknown
            },
            reasoning: if self.supported_reasoning_levels.is_empty() {
                CapabilitySupport::Unknown
            } else {
                CapabilitySupport::Supported
            },
            ..ModelCapabilities::default()
        }
    }
}

fn codex_thinking_mode(
    selector: &str,
    description: Option<String>,
    is_default: bool,
) -> Option<ModelThinkingMode> {
    let thinking = if selector == "off" {
        ThinkingRequest::Disabled
    } else {
        ThinkingRequest::Effort {
            effort: reasoning_effort_from_selector(selector)?,
        }
    };
    Some(ModelThinkingMode {
        is_default,
        display_name: None,
        description,
        preset: None,
        thinking: Some(thinking),
        request_override: Default::default(),
        adapter_overrides: Default::default(),
    })
}

fn codex_reasoning_selector(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" | "off" | "disabled" => Some("off"),
        "minimal" => Some("minimal"),
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" => Some("max"),
        _ => None,
    }
}

fn reasoning_effort_from_selector(selector: &str) -> Option<ReasoningEffort> {
    match selector {
        "minimal" => Some(ReasoningEffort::Minimal),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" => Some(ReasoningEffort::Xhigh),
        "max" => Some(ReasoningEffort::Max),
        _ => None,
    }
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(super) struct OpenAiCodexReasoningLevel {
    effort: String,
    #[serde(default)]
    description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{OpenAiCodexModel, OpenAiCodexReasoningLevel};

    #[test]
    fn codex_default_reasoning_level_marks_the_matching_mode() {
        let model = OpenAiCodexModel {
            default_reasoning_level: Some("medium".to_owned()),
            supported_reasoning_levels: vec![
                OpenAiCodexReasoningLevel {
                    effort: "low".to_owned(),
                    description: None,
                },
                OpenAiCodexReasoningLevel {
                    effort: "medium".to_owned(),
                    description: None,
                },
            ],
            slug: String::new(),
            display_name: None,
            name: None,
            description: None,
            support_verbosity: None,
            default_verbosity: None,
            supports_parallel_tool_calls: None,
            context_window: None,
            max_context_window: None,
            input_modalities: Vec::new(),
        };

        let modes = model.thinking_modes();

        assert_eq!(modes.iter().filter(|mode| mode.is_default).count(), 1);
        assert_eq!(
            modes
                .iter()
                .find(|mode| mode.is_default)
                .and_then(|mode| mode.selector())
                .as_deref(),
            Some("medium")
        );
    }

    #[test]
    fn codex_missing_default_level_is_materialized_as_a_mode() {
        let model = OpenAiCodexModel {
            default_reasoning_level: Some("none".to_owned()),
            supported_reasoning_levels: Vec::new(),
            slug: String::new(),
            display_name: None,
            name: None,
            description: None,
            support_verbosity: None,
            default_verbosity: None,
            supports_parallel_tool_calls: None,
            context_window: None,
            max_context_window: None,
            input_modalities: Vec::new(),
        };

        let modes = model.thinking_modes();

        assert_eq!(modes.len(), 1);
        assert!(modes[0].is_default);
        assert_eq!(modes[0].selector().as_deref(), Some("off"));
    }
}
