use std::{
    borrow::{Borrow, Cow},
    cmp::Ordering,
    collections::BTreeMap,
    fmt,
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    message::{AttachmentItem, AttachmentKind},
    provider::ThinkingRequest,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{field} cannot be empty")]
pub struct IdentifierError {
    field: &'static str,
}

impl IdentifierError {
    const fn new(field: &'static str) -> Self {
        Self { field }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ModelRefParseError {
    #[error("model reference must be in `provider/model` format")]
    MissingSeparator,
    #[error(transparent)]
    InvalidProviderId(#[from] IdentifierError),
    #[error("{0}")]
    InvalidModelId(String),
}

fn normalize_non_empty(
    value: impl Into<String>,
    field: &'static str,
) -> Result<String, IdentifierError> {
    let value = value.into();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(IdentifierError::new(field));
    }
    Ok(trimmed.to_owned())
}

macro_rules! define_string_identifier {
    ($name:ident, $field:literal, $expect_message:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self::try_new(value).expect($expect_message)
            }

            pub fn try_new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                Ok(Self(normalize_non_empty(value, $field)?))
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                self.0.as_str()
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.0.as_str())
            }
        }

        impl FromStr for $name {
            type Err = IdentifierError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::try_new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentifierError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::try_new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

define_string_identifier!(ProviderId, "provider id", "provider id cannot be empty");
define_string_identifier!(AdapterId, "adapter id", "adapter id cannot be empty");
define_string_identifier!(ModelId, "model id", "model id cannot be empty");

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider_id: ProviderId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<AdapterId>,
    pub model_id: ModelId,
}

impl ModelRef {
    pub fn new(provider_id: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self::try_new(provider_id, model_id).expect("model reference must be valid")
    }

    pub fn try_new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, IdentifierError> {
        Ok(Self {
            provider_id: ProviderId::try_new(provider_id)?,
            adapter_id: None,
            model_id: ModelId::try_new(model_id)?,
        })
    }

    pub fn new_with_adapter(
        provider_id: impl Into<String>,
        adapter_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Self {
        Self::try_new_with_adapter(provider_id, adapter_id, model_id)
            .expect("model reference must be valid")
    }

    pub fn try_new_with_adapter(
        provider_id: impl Into<String>,
        adapter_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, IdentifierError> {
        Ok(Self {
            provider_id: ProviderId::try_new(provider_id)?,
            adapter_id: Some(AdapterId::try_new(adapter_id)?),
            model_id: ModelId::try_new(model_id)?,
        })
    }
}

impl fmt::Display for ModelRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(adapter_id) = &self.adapter_id {
            write!(
                f,
                "provider={} adapter={} model={}",
                self.provider_id, adapter_id, self.model_id
            )
        } else {
            write!(f, "{}/{}", self.provider_id, self.model_id)
        }
    }
}

impl FromStr for ModelRef {
    type Err = ModelRefParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((provider_id, model_id)) = value.split_once('/') else {
            return Err(ModelRefParseError::MissingSeparator);
        };
        let provider_id = ProviderId::try_new(provider_id)?;
        let model_id = ModelId::try_new(model_id)
            .map_err(|err| ModelRefParseError::InvalidModelId(err.to_string()))?;
        Ok(Self {
            provider_id,
            adapter_id: None,
            model_id,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySupport {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

impl CapabilitySupport {
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::Supported)
    }

    pub const fn is_unsupported(self) -> bool {
        matches!(self, Self::Unsupported)
    }

    pub const fn supported() -> Self {
        Self::Supported
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelInputModality {
    Text,
    Image,
    Document,
    Audio,
    Video,
    File,
}

impl AsRef<str> for ModelInputModality {
    fn as_ref(&self) -> &str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Document => "document",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::File => "file",
        }
    }
}

impl fmt::Display for ModelInputModality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelLifecycle {
    Active,
    Preview,
    Beta,
    Alpha,
    Experimental,
    Deprecated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelTokenLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

impl ModelTokenLimits {
    pub fn is_empty(&self) -> bool {
        self.context_window_tokens.is_none()
            && self.max_input_tokens.is_none()
            && self.max_output_tokens.is_none()
    }

    pub fn merged_with_fallbacks_from(self, fallback: &Self) -> Self {
        Self {
            context_window_tokens: self
                .context_window_tokens
                .or(fallback.context_window_tokens),
            max_input_tokens: self.max_input_tokens.or(fallback.max_input_tokens),
            max_output_tokens: self.max_output_tokens.or(fallback.max_output_tokens),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelPricing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<ModelPricingTier>,
}

impl ModelPricing {
    pub fn is_empty(&self) -> bool {
        self.input_usd_per_million_tokens.is_none()
            && self.output_usd_per_million_tokens.is_none()
            && self.cache_read_usd_per_million_tokens.is_none()
            && self.cache_write_usd_per_million_tokens.is_none()
            && self.tiers.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelPricingTier {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_usd_per_million_tokens: Option<String>,
}

impl ModelPricingTier {
    pub fn is_empty(&self) -> bool {
        self.tier_type.is_none()
            && self.size_tokens.is_none()
            && self.input_usd_per_million_tokens.is_none()
            && self.output_usd_per_million_tokens.is_none()
            && self.cache_read_usd_per_million_tokens.is_none()
            && self.cache_write_usd_per_million_tokens.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(default, skip_serializing_if = "ModelTokenLimits::is_empty")]
    pub limits: ModelTokenLimits,
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
    pub default_thinking_mode: Option<String>,
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

impl ModelMetadata {
    pub fn is_empty(&self) -> bool {
        self.lifecycle.is_none()
            && self.limits.is_empty()
            && self.description.is_none()
            && self.knowledge_cutoff.is_none()
            && self.release_date.is_none()
            && self.last_updated.is_none()
            && self.open_weights.is_none()
            && self.default_thinking_mode.is_none()
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

    pub fn supported_verbosity_levels_for_model(&self, model_id: &ModelId) -> Vec<String> {
        let default_verbosity = self
            .default_verbosity
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);
        if !self.supports_verbosity.unwrap_or(false) && default_verbosity.is_none() {
            return Vec::new();
        }

        let mut levels = if model_only_supports_medium_verbosity(model_id.as_ref()) {
            vec!["medium".to_owned()]
        } else {
            vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()]
        };
        if let Some(default_verbosity) = default_verbosity
            && !levels.iter().any(|value| value == &default_verbosity)
        {
            levels.push(default_verbosity);
        }
        levels
    }

    pub fn supports_verbosity_level_for_model(&self, model_id: &ModelId, verbosity: &str) -> bool {
        let normalized = verbosity.trim().to_ascii_lowercase();
        self.supported_verbosity_levels_for_model(model_id)
            .into_iter()
            .any(|candidate| candidate == normalized)
    }

    pub fn supports_parallel_tool_calls_for_model(&self) -> bool {
        self.supports_parallel_tool_calls.unwrap_or(false)
    }

    pub fn parsed_default_temperature(&self) -> Option<f32> {
        parse_optional_f32(self.default_temperature.as_deref(), |parsed| {
            parsed.is_finite() && parsed >= 0.0
        })
    }

    pub fn parsed_default_top_p(&self) -> Option<f32> {
        parse_optional_f32(self.default_top_p.as_deref(), |parsed| {
            parsed.is_finite() && parsed > 0.0 && parsed <= 1.0
        })
    }

    pub fn merged_with_fallbacks_from(self, fallback: &Self) -> Self {
        Self {
            lifecycle: self.lifecycle.or(fallback.lifecycle),
            limits: self.limits.merged_with_fallbacks_from(&fallback.limits),
            description: self.description.or_else(|| fallback.description.clone()),
            knowledge_cutoff: self
                .knowledge_cutoff
                .or_else(|| fallback.knowledge_cutoff.clone()),
            release_date: self.release_date.or_else(|| fallback.release_date.clone()),
            last_updated: self.last_updated.or_else(|| fallback.last_updated.clone()),
            open_weights: self.open_weights.or(fallback.open_weights),
            default_thinking_mode: self
                .default_thinking_mode
                .or_else(|| fallback.default_thinking_mode.clone()),
            supports_parallel_tool_calls: self
                .supports_parallel_tool_calls
                .or(fallback.supports_parallel_tool_calls),
            supports_verbosity: self.supports_verbosity.or(fallback.supports_verbosity),
            default_verbosity: self
                .default_verbosity
                .or_else(|| fallback.default_verbosity.clone()),
            default_temperature: self
                .default_temperature
                .or_else(|| fallback.default_temperature.clone()),
            default_top_p: self
                .default_top_p
                .or_else(|| fallback.default_top_p.clone()),
            default_top_k: self.default_top_k.or(fallback.default_top_k),
            assistant_reasoning_interleaved: self
                .assistant_reasoning_interleaved
                .or(fallback.assistant_reasoning_interleaved),
            assistant_reasoning_field: self
                .assistant_reasoning_field
                .or_else(|| fallback.assistant_reasoning_field.clone()),
            output_modalities: if self.output_modalities.is_empty() {
                fallback.output_modalities.clone()
            } else {
                self.output_modalities
            },
            pricing: self.pricing.or_else(|| fallback.pricing.clone()),
        }
    }
}

fn model_only_supports_medium_verbosity(model_id: &str) -> bool {
    let lowered = model_id.trim().to_ascii_lowercase();
    lowered.contains("gpt-5") && lowered.contains("-chat")
}

fn normalize_optional_decimal(
    value: Option<String>,
    predicate: impl Fn(f32) -> bool,
) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        trimmed
            .parse::<f32>()
            .ok()
            .filter(|parsed| predicate(*parsed))
            .map(|_| trimmed.to_owned())
    })
}

pub(crate) fn normalize_model_default_temperature(value: Option<String>) -> Option<String> {
    normalize_optional_decimal(value, |parsed| parsed.is_finite() && parsed >= 0.0)
}

pub(crate) fn normalize_model_default_top_p(value: Option<String>) -> Option<String> {
    normalize_optional_decimal(value, |parsed| {
        parsed.is_finite() && parsed > 0.0 && parsed <= 1.0
    })
}

pub(crate) fn normalize_model_default_top_k(value: Option<u32>) -> Option<u32> {
    value.filter(|value| *value > 0)
}

pub(crate) fn normalize_model_assistant_reasoning_field(value: Option<String>) -> Option<String> {
    normalize_assistant_reasoning_field(value)
}

pub(crate) fn normalize_model_output_modalities(
    output_modalities: impl IntoIterator<Item = String>,
) -> Vec<String> {
    output_modalities
        .into_iter()
        .filter_map(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
        .collect()
}

pub(crate) fn non_empty_model_pricing(pricing: Option<ModelPricing>) -> Option<ModelPricing> {
    pricing.filter(|pricing| !pricing.is_empty())
}

fn parse_optional_f32(value: Option<&str>, predicate: impl Fn(f32) -> bool) -> Option<f32> {
    value
        .and_then(|raw| raw.trim().parse::<f32>().ok())
        .filter(|parsed| predicate(*parsed))
}

fn normalize_assistant_reasoning_field(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let normalized = raw.trim().to_ascii_lowercase();
        matches!(
            normalized.as_str(),
            "reasoning_content" | "reasoning_details"
        )
        .then_some(normalized)
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCapabilities {
    #[serde(default = "CapabilitySupport::supported")]
    pub text_input: CapabilitySupport,
    #[serde(default)]
    pub image_input: CapabilitySupport,
    #[serde(default)]
    pub document_input: CapabilitySupport,
    #[serde(default)]
    pub audio_input: CapabilitySupport,
    #[serde(default)]
    pub video_input: CapabilitySupport,
    #[serde(default)]
    pub file_input: CapabilitySupport,
    #[serde(default)]
    pub tool_calling: CapabilitySupport,
    #[serde(default)]
    pub streaming: CapabilitySupport,
    /// Whether the model supports extended thinking / reasoning output.
    #[serde(default)]
    pub reasoning: CapabilitySupport,
    /// Whether the model supports JSON schema / structured output constraints.
    #[serde(default)]
    pub structured_output: CapabilitySupport,
    /// Whether the model accepts a `temperature` parameter.
    /// Some reasoning models (e.g. o1/o3) reject temperature and must receive 1.0 or omit it.
    #[serde(default = "CapabilitySupport::supported")]
    pub temperature_supported: CapabilitySupport,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            text_input: CapabilitySupport::Supported,
            image_input: CapabilitySupport::Unknown,
            document_input: CapabilitySupport::Unknown,
            audio_input: CapabilitySupport::Unknown,
            video_input: CapabilitySupport::Unknown,
            file_input: CapabilitySupport::Unknown,
            tool_calling: CapabilitySupport::Unknown,
            streaming: CapabilitySupport::Unknown,
            reasoning: CapabilitySupport::Unknown,
            structured_output: CapabilitySupport::Unknown,
            temperature_supported: CapabilitySupport::Supported,
        }
    }
}

impl ModelCapabilities {
    pub fn is_default_placeholder(&self) -> bool {
        self == &Self::default()
    }

    pub fn text_only() -> Self {
        Self {
            tool_calling: CapabilitySupport::Unsupported,
            streaming: CapabilitySupport::Unsupported,
            ..Self::default()
        }
    }

    pub fn support_for_input_modality(&self, modality: ModelInputModality) -> CapabilitySupport {
        match modality {
            ModelInputModality::Text => self.text_input,
            ModelInputModality::Image => self.image_input,
            ModelInputModality::Document => self.document_input,
            ModelInputModality::Audio => self.audio_input,
            ModelInputModality::Video => self.video_input,
            ModelInputModality::File => self.file_input,
        }
    }

    pub fn unsupported_attachment_modality(
        &self,
        attachment: &AttachmentItem,
    ) -> Option<ModelInputModality> {
        let required = required_attachment_modality(attachment)?;
        self.support_for_input_modality(required)
            .is_unsupported()
            .then_some(required)
    }

    pub fn merged_with_fallbacks_from(self, fallback: &Self) -> Self {
        Self {
            text_input: capability_with_fallback(self.text_input, fallback.text_input),
            image_input: capability_with_fallback(self.image_input, fallback.image_input),
            document_input: capability_with_fallback(self.document_input, fallback.document_input),
            audio_input: capability_with_fallback(self.audio_input, fallback.audio_input),
            video_input: capability_with_fallback(self.video_input, fallback.video_input),
            file_input: capability_with_fallback(self.file_input, fallback.file_input),
            tool_calling: capability_with_fallback(self.tool_calling, fallback.tool_calling),
            streaming: capability_with_fallback(self.streaming, fallback.streaming),
            reasoning: capability_with_fallback(self.reasoning, fallback.reasoning),
            structured_output: capability_with_fallback(
                self.structured_output,
                fallback.structured_output,
            ),
            temperature_supported: capability_with_fallback(
                self.temperature_supported,
                fallback.temperature_supported,
            ),
        }
    }
}

fn capability_with_fallback(
    primary: CapabilitySupport,
    fallback: CapabilitySupport,
) -> CapabilitySupport {
    if matches!(primary, CapabilitySupport::Unknown) {
        fallback
    } else {
        primary
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelSpeedModeRequestOverride {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub body_patch: BTreeMap<String, serde_json::Value>,
}

impl ModelSpeedModeRequestOverride {
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty() && self.body_patch.is_empty()
    }

    pub fn parallel_tool_calls(&self) -> Option<bool> {
        self.body_patch
            .get("parallel_tool_calls")
            .and_then(serde_json::Value::as_bool)
    }

    pub fn set_parallel_tool_calls(&mut self, enabled: Option<bool>) {
        match enabled {
            Some(enabled) => {
                self.body_patch.insert(
                    "parallel_tool_calls".to_owned(),
                    serde_json::Value::Bool(enabled),
                );
            }
            None => {
                self.body_patch.remove("parallel_tool_calls");
            }
        }
    }

    pub fn merged_with(&self, other: &Self) -> Self {
        let mut merged = self.clone();
        for (key, value) in &other.headers {
            merged.headers.insert(key.clone(), value.clone());
        }
        merge_json_patch_maps(&mut merged.body_patch, &other.body_patch);
        merged
    }
}

macro_rules! define_model_mode {
    (
        $name:ident,
        fields { $($extra_fields:tt)* },
        init { $($extra_init:tt)* },
        methods { $($extra_methods:tt)* }
    ) => {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
        pub struct $name {
            pub display_name: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub description: Option<String>,
            $($extra_fields)*
            #[serde(
                default,
                skip_serializing_if = "ModelSpeedModeRequestOverride::is_empty"
            )]
            pub request_override: ModelSpeedModeRequestOverride,
            #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
            pub adapter_overrides: BTreeMap<String, ModelSpeedModeRequestOverride>,
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    display_name: None,
                    description: None,
                    $($extra_init)*
                    request_override: ModelSpeedModeRequestOverride::default(),
                    adapter_overrides: BTreeMap::new(),
                }
            }
        }
    };
}

define_model_mode!(
    ModelThinkingMode,
    fields {
        /// Stable selector for modes whose identity cannot be derived from
        /// the request itself. Effort, adaptive-effort, and disabled modes
        /// must leave this unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub preset: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub thinking: Option<ThinkingRequest>,
    },
    init {
        preset: None,
        thinking: None,
    },
    methods {}
);

define_model_mode!(ModelSpeedMode, fields {}, init {}, methods {});

impl ModelThinkingMode {
    /// Returns the selector exposed to users and persisted in execution
    /// preferences. Standard selectors are derived from the request, so an
    /// effort can never be renamed independently from its semantic value.
    pub fn selector(&self) -> Option<Cow<'_, str>> {
        match self.thinking.as_ref() {
            Some(ThinkingRequest::Disabled) => Some(Cow::Borrowed("off")),
            Some(ThinkingRequest::Effort { effort })
            | Some(ThinkingRequest::Adaptive {
                effort: Some(effort),
                ..
            }) => Some(Cow::Borrowed(effort.as_ref())),
            Some(ThinkingRequest::Budget { .. })
            | Some(ThinkingRequest::Adaptive { effort: None, .. })
            | None => self
                .preset
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(Cow::Borrowed),
        }
    }

    pub fn has_invalid_custom_preset(&self) -> bool {
        self.preset.is_some()
            && matches!(
                self.thinking,
                Some(ThinkingRequest::Disabled)
                    | Some(ThinkingRequest::Effort { .. })
                    | Some(ThinkingRequest::Adaptive {
                        effort: Some(_),
                        ..
                    })
            )
    }
}

/// Orders think modes by reasoning strength instead of their spelling
/// and always puts an explicit disabled mode first.
pub fn compare_thinking_mode_strength(
    left: &ModelThinkingMode,
    right: &ModelThinkingMode,
) -> Ordering {
    thinking_mode_strength(left)
        .cmp(&thinking_mode_strength(right))
        .then_with(|| left.selector().cmp(&right.selector()))
}

fn thinking_mode_strength(mode: &ModelThinkingMode) -> (u8, u32) {
    match mode.thinking.as_ref() {
        Some(ThinkingRequest::Disabled) => (0, 0),
        Some(ThinkingRequest::Effort { effort })
        | Some(ThinkingRequest::Adaptive {
            effort: Some(effort),
            ..
        }) => (reasoning_effort_tier(*effort), 0),
        Some(ThinkingRequest::Budget { budget_tokens }) => (
            mode.selector()
                .as_deref()
                .and_then(thinking_mode_name_tier)
                .unwrap_or(3),
            *budget_tokens,
        ),
        Some(ThinkingRequest::Adaptive { effort: None, .. }) | None => (
            mode.selector()
                .as_deref()
                .and_then(thinking_mode_name_tier)
                .unwrap_or(3),
            0,
        ),
    }
}

fn thinking_mode_name_tier(name: &str) -> Option<u8> {
    name.to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .find_map(|token| match token {
            "no" | "none" | "off" | "disabled" => Some(0),
            "minimal" => Some(1),
            "low" => Some(2),
            "medium" => Some(3),
            "high" => Some(4),
            "xhigh" => Some(5),
            "max" | "maximum" => Some(6),
            _ => None,
        })
}

fn reasoning_effort_tier(effort: crate::provider::ReasoningEffort) -> u8 {
    match effort {
        crate::provider::ReasoningEffort::Minimal => 1,
        crate::provider::ReasoningEffort::Low => 2,
        crate::provider::ReasoningEffort::Medium => 3,
        crate::provider::ReasoningEffort::High => 4,
        crate::provider::ReasoningEffort::Xhigh => 5,
        crate::provider::ReasoningEffort::Max => 6,
    }
}

fn merge_json_patch_maps(
    target: &mut BTreeMap<String, serde_json::Value>,
    patch: &BTreeMap<String, serde_json::Value>,
) {
    for (key, value) in patch {
        match target.get_mut(key) {
            Some(current) => merge_json_value(current, value),
            None => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

fn merge_json_value(current: &mut serde_json::Value, patch: &serde_json::Value) {
    match (current, patch) {
        (serde_json::Value::Object(current), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                match current.get_mut(key) {
                    Some(existing) => merge_json_value(existing, value),
                    None => {
                        current.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (current, patch) => *current = patch.clone(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Model {
    pub provider_id: ProviderId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<AdapterId>,
    pub id: ModelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_model_id: Option<ModelId>,
    pub display_name: Option<String>,
    #[serde(default)]
    pub capabilities: ModelCapabilities,
    #[serde(default, skip_serializing_if = "ModelMetadata::is_empty")]
    pub metadata: ModelMetadata,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking_modes: Vec<ModelThinkingMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub speed_modes: BTreeMap<String, ModelSpeedMode>,
}

impl Model {
    pub fn new(provider_id: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            provider_id: ProviderId::new(provider_id),
            adapter_id: None,
            id: ModelId::new(id),
            catalog_model_id: None,
            display_name: None,
            capabilities: ModelCapabilities::default(),
            metadata: ModelMetadata::default(),
            thinking_modes: Vec::new(),
            speed_modes: BTreeMap::new(),
        }
    }

    pub fn reference(&self) -> ModelRef {
        ModelRef {
            provider_id: self.provider_id.clone(),
            adapter_id: self.adapter_id.clone(),
            model_id: self.id.clone(),
        }
    }

    pub fn using_thinking_modes(mut self, thinking_modes: Vec<ModelThinkingMode>) -> Self {
        self.thinking_modes = thinking_modes;
        self
    }

    pub fn using_thinking_mode(mut self, thinking_mode: ModelThinkingMode) -> Self {
        self.thinking_modes.push(thinking_mode);
        self
    }
}

fn required_attachment_modality(attachment: &AttachmentItem) -> Option<ModelInputModality> {
    match attachment.kind {
        AttachmentKind::Image => Some(ModelInputModality::Image),
        AttachmentKind::Pdf => Some(ModelInputModality::Document),
        AttachmentKind::Audio => Some(ModelInputModality::Audio),
        AttachmentKind::Video => Some(ModelInputModality::Video),
        AttachmentKind::File => {
            let mime = attachment.mime.trim().to_ascii_lowercase();
            let text_like = mime.starts_with("text/")
                || matches!(
                    mime.as_str(),
                    "application/json"
                        | "application/xml"
                        | "application/yaml"
                        | "application/x-yaml"
                        | "application/javascript"
                );
            (!text_like).then_some(ModelInputModality::File)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ModelThinkingMode, compare_thinking_mode_strength};
    use crate::provider::{ReasoningEffort, ThinkingRequest};

    #[test]
    fn effort_and_off_selectors_are_derived_from_the_request() {
        let high = effort_mode(ReasoningEffort::High);
        let off = ModelThinkingMode {
            thinking: Some(ThinkingRequest::Disabled),
            ..Default::default()
        };

        assert_eq!(high.selector().as_deref(), Some("high"));
        assert_eq!(off.selector().as_deref(), Some("off"));
    }

    #[test]
    fn thinking_modes_sort_from_disabled_through_max() {
        let mut modes = [
            effort_mode(ReasoningEffort::Xhigh),
            effort_mode(ReasoningEffort::Low),
            effort_mode(ReasoningEffort::Max),
            ModelThinkingMode {
                thinking: Some(ThinkingRequest::Disabled),
                ..Default::default()
            },
            effort_mode(ReasoningEffort::High),
            effort_mode(ReasoningEffort::Minimal),
            effort_mode(ReasoningEffort::Medium),
        ];

        modes.sort_by(compare_thinking_mode_strength);

        assert_eq!(
            modes
                .iter()
                .filter_map(|mode| mode.selector().map(|selector| selector.into_owned()))
                .collect::<Vec<_>>(),
            vec!["off", "minimal", "low", "medium", "high", "xhigh", "max",]
        );
    }

    #[test]
    fn thinking_mode_payload_breaks_ambiguous_or_misleading_name_order() {
        let disabled = ModelThinkingMode {
            thinking: Some(ThinkingRequest::Disabled),
            ..Default::default()
        };
        let low = effort_mode(ReasoningEffort::Low);
        let larger_budget = ModelThinkingMode {
            preset: Some("custom-plus".to_owned()),
            thinking: Some(ThinkingRequest::Budget {
                budget_tokens: 16_000,
            }),
            ..Default::default()
        };
        let smaller_budget = ModelThinkingMode {
            preset: Some("custom".to_owned()),
            thinking: Some(ThinkingRequest::Budget {
                budget_tokens: 4_000,
            }),
            ..Default::default()
        };

        assert!(compare_thinking_mode_strength(&disabled, &low).is_lt());
        assert!(compare_thinking_mode_strength(&smaller_budget, &larger_budget).is_lt());
    }

    fn effort_mode(effort: ReasoningEffort) -> ModelThinkingMode {
        ModelThinkingMode {
            thinking: Some(ThinkingRequest::Effort { effort }),
            ..Default::default()
        }
    }
}
