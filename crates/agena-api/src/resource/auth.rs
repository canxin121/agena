use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Summary of one provider adapter.
pub struct ProviderAdapterSummaryResource {
    pub adapter_id: String,
    pub enabled: bool,
    pub configured_model_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Default adapter and model of a provider.
pub struct ProviderDefaultsResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Summary of a provider: defaults plus its adapters.
pub struct ProviderSummaryResource {
    pub provider_id: String,
    pub defaults: ProviderDefaultsResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapters: Vec<ProviderAdapterSummaryResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Models of a provider.
pub struct ProviderModelsResponse {
    pub provider_id: String,
    pub models: Vec<ProviderModelResource>,
}

/// A provider/model route projected into the public protocol. Runtime model
/// selection, provider clients, and capability evaluation stay outside this
/// DTO; clients receive only serializable route metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderModelResource {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<String>,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default = "default_true")]
    pub native_compaction: bool,
    #[serde(default)]
    pub capabilities: ProviderModelCapabilitiesResource,
    #[serde(
        default,
        skip_serializing_if = "ProviderModelMetadataResource::is_empty"
    )]
    pub metadata: ProviderModelMetadataResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking_modes: Vec<ProviderModelThinkingModeResource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub speed_modes: BTreeMap<String, ProviderModelSpeedModeResource>,
}

impl ProviderModelResource {
    /// Construct the minimal public projection for a configured route when
    /// discovery has not supplied capability metadata yet.
    pub fn configured(adapter_id: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            provider_id: String::new(),
            adapter_id: Some(adapter_id.into()),
            id: id.into(),
            catalog_model_id: None,
            display_name: None,
            native_compaction: true,
            capabilities: ProviderModelCapabilitiesResource::default(),
            metadata: ProviderModelMetadataResource::default(),
            thinking_modes: Vec::new(),
            speed_modes: BTreeMap::new(),
        }
    }
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// Whether a model capability is supported.
pub enum CapabilitySupportResource {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Capabilities of a provider model (input modalities and features).
pub struct ProviderModelCapabilitiesResource {
    #[serde(default = "capability_supported")]
    pub text_input: CapabilitySupportResource,
    #[serde(default)]
    pub image_input: CapabilitySupportResource,
    #[serde(default)]
    pub document_input: CapabilitySupportResource,
    #[serde(default)]
    pub audio_input: CapabilitySupportResource,
    #[serde(default)]
    pub video_input: CapabilitySupportResource,
    #[serde(default)]
    pub file_input: CapabilitySupportResource,
    #[serde(default)]
    pub tool_calling: CapabilitySupportResource,
    #[serde(default)]
    pub streaming: CapabilitySupportResource,
    #[serde(default)]
    pub reasoning: CapabilitySupportResource,
    #[serde(default)]
    pub structured_output: CapabilitySupportResource,
    #[serde(default = "capability_supported")]
    pub temperature_supported: CapabilitySupportResource,
}

const fn capability_supported() -> CapabilitySupportResource {
    CapabilitySupportResource::Supported
}

impl Default for ProviderModelCapabilitiesResource {
    fn default() -> Self {
        Self {
            text_input: CapabilitySupportResource::Supported,
            image_input: CapabilitySupportResource::Unknown,
            document_input: CapabilitySupportResource::Unknown,
            audio_input: CapabilitySupportResource::Unknown,
            video_input: CapabilitySupportResource::Unknown,
            file_input: CapabilitySupportResource::Unknown,
            tool_calling: CapabilitySupportResource::Unknown,
            streaming: CapabilitySupportResource::Unknown,
            reasoning: CapabilitySupportResource::Unknown,
            structured_output: CapabilitySupportResource::Unknown,
            temperature_supported: CapabilitySupportResource::Supported,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// Metadata of a provider model (lifecycle, context window, pricing).
pub struct ProviderModelMetadataResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_weights: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_verbosity: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_temperature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_p: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_interleaved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
}

impl ProviderModelMetadataResource {
    pub fn is_empty(&self) -> bool {
        self.lifecycle.is_none()
            && self.context_window_tokens.is_none()
            && self.max_input_tokens.is_none()
            && self.max_output_tokens.is_none()
            && self.description.is_none()
            && self.knowledge_cutoff.is_none()
            && self.release_date.is_none()
            && self.last_updated.is_none()
            && self.open_weights.is_none()
            && self.supports_parallel_tool_calls.is_none()
            && self.supports_verbosity.is_none()
            && self.default_verbosity.is_none()
            && self.default_temperature.is_none()
            && self.default_top_p.is_none()
            && self.default_top_k.is_none()
            && self.assistant_reasoning_interleaved.is_none()
            && self.assistant_reasoning_field.is_none()
            && self.output_modalities.is_empty()
            && self.pricing.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// Per-model request overrides (headers and body patch).
pub struct ProviderModelRequestOverrideResource {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub body_patch: BTreeMap<String, serde_json::Value>,
}

impl ProviderModelRequestOverrideResource {
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty() && self.body_patch.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
/// How thinking is requested from a model.
pub enum ThinkingRequestResource {
    Budget {
        budget_tokens: u32,
    },
    Adaptive {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<ReasoningEffortResource>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<ThinkingDisplayResource>,
    },
    Effort {
        effort: ReasoningEffortResource,
    },
    Disabled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Reasoning effort level of a model.
pub enum ReasoningEffortResource {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// How model thinking is displayed.
pub enum ThinkingDisplayResource {
    Summarized,
    Omitted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A thinking mode offered by a model.
pub struct ProviderModelThinkingModeResource {
    #[serde(
        default,
        rename = "default",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub is_default: bool,
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingRequestResource>,
    #[serde(
        default,
        skip_serializing_if = "ProviderModelRequestOverrideResource::is_empty"
    )]
    pub request_override: ProviderModelRequestOverrideResource,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ProviderModelRequestOverrideResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A speed mode offered by a model.
pub struct ProviderModelSpeedModeResource {
    #[serde(
        default,
        rename = "default",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub is_default: bool,
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "ProviderModelRequestOverrideResource::is_empty"
    )]
    pub request_override: ProviderModelRequestOverrideResource,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ProviderModelRequestOverrideResource>,
}
