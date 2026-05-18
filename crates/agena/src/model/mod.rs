use std::{borrow::Borrow, collections::BTreeMap, fmt, str::FromStr};

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProviderId(String);

impl ProviderId {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("provider id cannot be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        Ok(Self(normalize_non_empty(value, "provider id")?))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Borrow<str> for ProviderId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for ProviderId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProviderId {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_new(value)
    }
}

impl TryFrom<String> for ProviderId {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<ProviderId> for String {
    fn from(value: ProviderId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AdapterId(String);

impl AdapterId {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("adapter id cannot be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        Ok(Self(normalize_non_empty(value, "adapter id")?))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Borrow<str> for AdapterId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for AdapterId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for AdapterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AdapterId {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_new(value)
    }
}

impl TryFrom<String> for AdapterId {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<AdapterId> for String {
    fn from(value: AdapterId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ModelId(String);

impl ModelId {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("model id cannot be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        Ok(Self(normalize_non_empty(value, "model id")?))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Borrow<str> for ModelId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for ModelId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ModelId {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_new(value)
    }
}

impl TryFrom<String> for ModelId {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<ModelId> for String {
    fn from(value: ModelId) -> Self {
        value.0
    }
}

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

    pub fn with_adapter_id(mut self, adapter_id: impl Into<String>) -> Self {
        self.adapter_id = Some(AdapterId::new(adapter_id));
        self
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

impl ModelInputModality {
    pub const fn as_str(self) -> &'static str {
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

    pub fn with_context_window_tokens(mut self, context_window_tokens: u32) -> Self {
        self.context_window_tokens = Some(context_window_tokens);
        self
    }

    pub fn with_max_input_tokens(mut self, max_input_tokens: u32) -> Self {
        self.max_input_tokens = Some(max_input_tokens);
        self
    }

    pub fn with_max_output_tokens(mut self, max_output_tokens: u32) -> Self {
        self.max_output_tokens = Some(max_output_tokens);
        self
    }

    pub fn with_fallbacks_from(mut self, fallback: &Self) -> Self {
        if self.context_window_tokens.is_none() {
            self.context_window_tokens = fallback.context_window_tokens;
        }
        if self.max_input_tokens.is_none() {
            self.max_input_tokens = fallback.max_input_tokens;
        }
        if self.max_output_tokens.is_none() {
            self.max_output_tokens = fallback.max_output_tokens;
        }
        self
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
            && self.output_modalities.is_empty()
            && self.pricing.is_none()
    }

    pub fn with_lifecycle(mut self, lifecycle: ModelLifecycle) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_knowledge_cutoff(mut self, knowledge_cutoff: impl Into<String>) -> Self {
        self.knowledge_cutoff = Some(knowledge_cutoff.into());
        self
    }

    pub fn with_release_date(mut self, release_date: impl Into<String>) -> Self {
        self.release_date = Some(release_date.into());
        self
    }

    pub fn with_last_updated(mut self, last_updated: impl Into<String>) -> Self {
        self.last_updated = Some(last_updated.into());
        self
    }

    pub fn with_open_weights(mut self, open_weights: bool) -> Self {
        self.open_weights = Some(open_weights);
        self
    }

    pub fn with_default_thinking_mode(mut self, default_thinking_mode: impl Into<String>) -> Self {
        self.default_thinking_mode = Some(default_thinking_mode.into());
        self
    }

    pub fn with_supports_parallel_tool_calls(mut self, supports_parallel_tool_calls: bool) -> Self {
        self.supports_parallel_tool_calls = Some(supports_parallel_tool_calls);
        self
    }

    pub fn with_supports_verbosity(mut self, supports_verbosity: bool) -> Self {
        self.supports_verbosity = Some(supports_verbosity);
        self
    }

    pub fn with_default_verbosity(mut self, default_verbosity: impl Into<String>) -> Self {
        self.default_verbosity = Some(default_verbosity.into());
        self
    }

    pub fn with_output_modalities(
        mut self,
        output_modalities: impl IntoIterator<Item = String>,
    ) -> Self {
        self.output_modalities = output_modalities
            .into_iter()
            .filter_map(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            })
            .collect();
        self
    }

    pub fn with_pricing(mut self, pricing: ModelPricing) -> Self {
        if pricing.is_empty() {
            self.pricing = None;
        } else {
            self.pricing = Some(pricing);
        }
        self
    }

    pub fn with_context_window_tokens(mut self, context_window_tokens: u32) -> Self {
        self.limits = self
            .limits
            .with_context_window_tokens(context_window_tokens);
        self
    }

    pub fn with_max_input_tokens(mut self, max_input_tokens: u32) -> Self {
        self.limits = self.limits.with_max_input_tokens(max_input_tokens);
        self
    }

    pub fn with_max_output_tokens(mut self, max_output_tokens: u32) -> Self {
        self.limits = self.limits.with_max_output_tokens(max_output_tokens);
        self
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

        let mut levels = if model_only_supports_medium_verbosity(model_id.as_str()) {
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

    pub fn with_fallbacks_from(mut self, fallback: &Self) -> Self {
        if self.lifecycle.is_none() {
            self.lifecycle = fallback.lifecycle;
        }
        self.limits = self.limits.with_fallbacks_from(&fallback.limits);
        if self.description.is_none() {
            self.description = fallback.description.clone();
        }
        if self.knowledge_cutoff.is_none() {
            self.knowledge_cutoff = fallback.knowledge_cutoff.clone();
        }
        if self.release_date.is_none() {
            self.release_date = fallback.release_date.clone();
        }
        if self.last_updated.is_none() {
            self.last_updated = fallback.last_updated.clone();
        }
        if self.open_weights.is_none() {
            self.open_weights = fallback.open_weights;
        }
        if self.default_thinking_mode.is_none() {
            self.default_thinking_mode = fallback.default_thinking_mode.clone();
        }
        if self.supports_parallel_tool_calls.is_none() {
            self.supports_parallel_tool_calls = fallback.supports_parallel_tool_calls;
        }
        if self.supports_verbosity.is_none() {
            self.supports_verbosity = fallback.supports_verbosity;
        }
        if self.default_verbosity.is_none() {
            self.default_verbosity = fallback.default_verbosity.clone();
        }
        if self.output_modalities.is_empty() {
            self.output_modalities = fallback.output_modalities.clone();
        }
        if self.pricing.is_none() {
            self.pricing = fallback.pricing.clone();
        }
        self
    }
}

fn model_only_supports_medium_verbosity(model_id: &str) -> bool {
    let lowered = model_id.trim().to_ascii_lowercase();
    lowered.contains("gpt-5") && lowered.contains("-chat")
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
        Self::default()
            .with_tool_calling(CapabilitySupport::Unsupported)
            .with_streaming(CapabilitySupport::Unsupported)
    }

    pub fn with_image_input(mut self, support: CapabilitySupport) -> Self {
        self.image_input = support;
        self
    }

    pub fn with_document_input(mut self, support: CapabilitySupport) -> Self {
        self.document_input = support;
        self
    }

    pub fn with_audio_input(mut self, support: CapabilitySupport) -> Self {
        self.audio_input = support;
        self
    }

    pub fn with_video_input(mut self, support: CapabilitySupport) -> Self {
        self.video_input = support;
        self
    }

    pub fn with_file_input(mut self, support: CapabilitySupport) -> Self {
        self.file_input = support;
        self
    }

    pub fn with_tool_calling(mut self, support: CapabilitySupport) -> Self {
        self.tool_calling = support;
        self
    }

    pub fn with_streaming(mut self, support: CapabilitySupport) -> Self {
        self.streaming = support;
        self
    }

    pub fn with_reasoning(mut self, support: CapabilitySupport) -> Self {
        self.reasoning = support;
        self
    }

    pub fn with_structured_output(mut self, support: CapabilitySupport) -> Self {
        self.structured_output = support;
        self
    }

    pub fn with_temperature_supported(mut self, support: CapabilitySupport) -> Self {
        self.temperature_supported = support;
        self
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

    pub fn with_fallbacks_from(mut self, fallback: &Self) -> Self {
        if matches!(self.text_input, CapabilitySupport::Unknown) {
            self.text_input = fallback.text_input;
        }
        if matches!(self.image_input, CapabilitySupport::Unknown) {
            self.image_input = fallback.image_input;
        }
        if matches!(self.document_input, CapabilitySupport::Unknown) {
            self.document_input = fallback.document_input;
        }
        if matches!(self.audio_input, CapabilitySupport::Unknown) {
            self.audio_input = fallback.audio_input;
        }
        if matches!(self.video_input, CapabilitySupport::Unknown) {
            self.video_input = fallback.video_input;
        }
        if matches!(self.file_input, CapabilitySupport::Unknown) {
            self.file_input = fallback.file_input;
        }
        if matches!(self.tool_calling, CapabilitySupport::Unknown) {
            self.tool_calling = fallback.tool_calling;
        }
        if matches!(self.streaming, CapabilitySupport::Unknown) {
            self.streaming = fallback.streaming;
        }
        if matches!(self.reasoning, CapabilitySupport::Unknown) {
            self.reasoning = fallback.reasoning;
        }
        if matches!(self.structured_output, CapabilitySupport::Unknown) {
            self.structured_output = fallback.structured_output;
        }
        if matches!(self.temperature_supported, CapabilitySupport::Unknown) {
            self.temperature_supported = fallback.temperature_supported;
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

impl Default for ModelSpeedModeRequestOverride {
    fn default() -> Self {
        Self {
            headers: BTreeMap::new(),
            body_patch: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelThinkingMode {
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingRequest>,
    #[serde(
        default,
        skip_serializing_if = "ModelSpeedModeRequestOverride::is_empty"
    )]
    pub request_override: ModelSpeedModeRequestOverride,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ModelSpeedModeRequestOverride>,
}

impl ModelThinkingMode {
    pub fn new() -> Self {
        Self {
            display_name: None,
            description: None,
            thinking: None,
            request_override: ModelSpeedModeRequestOverride::default(),
            adapter_overrides: BTreeMap::new(),
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_thinking(mut self, thinking: ThinkingRequest) -> Self {
        self.thinking = Some(thinking);
        self
    }

    pub fn with_request_override(
        mut self,
        request_override: ModelSpeedModeRequestOverride,
    ) -> Self {
        self.request_override = request_override;
        self
    }

    pub fn with_adapter_override(
        mut self,
        adapter_id: impl Into<String>,
        request_override: ModelSpeedModeRequestOverride,
    ) -> Self {
        self.adapter_overrides
            .insert(adapter_id.into(), request_override);
        self
    }
}

impl Default for ModelThinkingMode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelSpeedMode {
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "ModelSpeedModeRequestOverride::is_empty"
    )]
    pub request_override: ModelSpeedModeRequestOverride,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ModelSpeedModeRequestOverride>,
}

impl ModelSpeedMode {
    pub fn new() -> Self {
        Self {
            display_name: None,
            description: None,
            request_override: ModelSpeedModeRequestOverride::default(),
            adapter_overrides: BTreeMap::new(),
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_request_override(
        mut self,
        request_override: ModelSpeedModeRequestOverride,
    ) -> Self {
        self.request_override = request_override;
        self
    }

    pub fn with_adapter_override(
        mut self,
        adapter_id: impl Into<String>,
        request_override: ModelSpeedModeRequestOverride,
    ) -> Self {
        self.adapter_overrides
            .insert(adapter_id.into(), request_override);
        self
    }
}

impl Default for ModelSpeedMode {
    fn default() -> Self {
        Self::new()
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub thinking_modes: BTreeMap<String, ModelThinkingMode>,
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
            thinking_modes: BTreeMap::new(),
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

    pub fn with_provider_id(mut self, provider_id: impl Into<String>) -> Self {
        self.provider_id = ProviderId::new(provider_id);
        self
    }

    pub fn with_adapter_id(mut self, adapter_id: impl Into<String>) -> Self {
        self.adapter_id = Some(AdapterId::new(adapter_id));
        self
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_catalog_model_id(mut self, catalog_model_id: impl Into<String>) -> Self {
        self.catalog_model_id = Some(ModelId::new(catalog_model_id));
        self
    }

    pub fn with_capabilities(mut self, capabilities: ModelCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_metadata(mut self, metadata: ModelMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_thinking_modes(
        mut self,
        thinking_modes: BTreeMap<String, ModelThinkingMode>,
    ) -> Self {
        self.thinking_modes = thinking_modes;
        self
    }

    pub fn with_thinking_mode(
        mut self,
        name: impl Into<String>,
        thinking_mode: ModelThinkingMode,
    ) -> Self {
        self.thinking_modes.insert(name.into(), thinking_mode);
        self
    }

    pub fn with_speed_modes(mut self, speed_modes: BTreeMap<String, ModelSpeedMode>) -> Self {
        self.speed_modes = speed_modes;
        self
    }

    pub fn with_speed_mode(mut self, name: impl Into<String>, speed_mode: ModelSpeedMode) -> Self {
        self.speed_modes.insert(name.into(), speed_mode);
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.metadata = self.metadata.with_description(description);
        self
    }

    pub fn with_lifecycle(mut self, lifecycle: ModelLifecycle) -> Self {
        self.metadata = self.metadata.with_lifecycle(lifecycle);
        self
    }

    pub fn with_context_window_tokens(mut self, context_window_tokens: u32) -> Self {
        self.metadata = self
            .metadata
            .with_context_window_tokens(context_window_tokens);
        self
    }

    pub fn with_max_input_tokens(mut self, max_input_tokens: u32) -> Self {
        self.metadata = self.metadata.with_max_input_tokens(max_input_tokens);
        self
    }

    pub fn with_max_output_tokens(mut self, max_output_tokens: u32) -> Self {
        self.metadata = self.metadata.with_max_output_tokens(max_output_tokens);
        self
    }

    pub fn with_capability_fallbacks(mut self, fallback: &ModelCapabilities) -> Self {
        self.capabilities = if self.capabilities.is_default_placeholder() {
            fallback.clone()
        } else {
            self.capabilities.with_fallbacks_from(fallback)
        };
        self
    }

    pub fn with_metadata_fallbacks(mut self, fallback: &ModelMetadata) -> Self {
        self.metadata = self.metadata.with_fallbacks_from(fallback);
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
    use super::*;

    #[test]
    fn model_ref_parses_provider_and_model() {
        let parsed: ModelRef = "openai/gpt-5".parse().expect("model ref should parse");
        assert_eq!(parsed.provider_id.as_str(), "openai");
        assert_eq!(parsed.adapter_id, None);
        assert_eq!(parsed.model_id.as_str(), "gpt-5");
    }

    #[test]
    fn model_ref_preserves_slashful_model_ids_without_adapter_parsing() {
        let parsed: ModelRef = "openai/openai/gpt-5"
            .parse()
            .expect("model ref should parse");
        assert_eq!(parsed.provider_id.as_str(), "openai");
        assert_eq!(parsed.adapter_id, None);
        assert_eq!(parsed.model_id.as_str(), "openai/gpt-5");
    }

    #[test]
    fn model_ref_rejects_missing_separator() {
        let err = "gpt-5"
            .parse::<ModelRef>()
            .expect_err("missing provider should fail");
        assert!(matches!(err, ModelRefParseError::MissingSeparator));
    }

    #[test]
    fn model_metadata_fallbacks_fill_missing_fields() {
        let base = ModelMetadata::default().with_description("GPT model");
        let fallback = ModelMetadata::default()
            .with_lifecycle(ModelLifecycle::Preview)
            .with_context_window_tokens(128_000)
            .with_max_input_tokens(96_000)
            .with_max_output_tokens(16_384)
            .with_knowledge_cutoff("2025-04")
            .with_release_date("2026-04-23")
            .with_last_updated("2026-04-24")
            .with_open_weights(false)
            .with_default_thinking_mode("thinking-medium")
            .with_supports_parallel_tool_calls(true)
            .with_supports_verbosity(true)
            .with_default_verbosity("low")
            .with_output_modalities(["text".to_owned(), "image".to_owned()])
            .with_pricing(ModelPricing {
                input_usd_per_million_tokens: Some("1.25".to_owned()),
                output_usd_per_million_tokens: Some("10".to_owned()),
                cache_read_usd_per_million_tokens: None,
                cache_write_usd_per_million_tokens: None,
                tiers: vec![ModelPricingTier {
                    tier_type: Some("context".to_owned()),
                    size_tokens: Some(200_000),
                    input_usd_per_million_tokens: Some("2.5".to_owned()),
                    output_usd_per_million_tokens: Some("15".to_owned()),
                    cache_read_usd_per_million_tokens: None,
                    cache_write_usd_per_million_tokens: None,
                }],
            });

        let merged = base.with_fallbacks_from(&fallback);
        assert_eq!(merged.description.as_deref(), Some("GPT model"));
        assert_eq!(merged.lifecycle, Some(ModelLifecycle::Preview));
        assert_eq!(merged.limits.context_window_tokens, Some(128_000));
        assert_eq!(merged.limits.max_input_tokens, Some(96_000));
        assert_eq!(merged.limits.max_output_tokens, Some(16_384));
        assert_eq!(merged.knowledge_cutoff.as_deref(), Some("2025-04"));
        assert_eq!(merged.release_date.as_deref(), Some("2026-04-23"));
        assert_eq!(merged.last_updated.as_deref(), Some("2026-04-24"));
        assert_eq!(merged.open_weights, Some(false));
        assert_eq!(
            merged.default_thinking_mode.as_deref(),
            Some("thinking-medium")
        );
        assert_eq!(merged.supports_parallel_tool_calls, Some(true));
        assert_eq!(merged.supports_verbosity, Some(true));
        assert_eq!(merged.default_verbosity.as_deref(), Some("low"));
        assert_eq!(merged.output_modalities, vec!["text", "image"]);
        assert_eq!(
            merged
                .pricing
                .as_ref()
                .and_then(|pricing| pricing.input_usd_per_million_tokens.as_deref()),
            Some("1.25")
        );
        assert_eq!(
            merged.pricing.as_ref().map(|pricing| pricing.tiers.len()),
            Some(1)
        );
    }

    #[test]
    fn model_metadata_supported_verbosity_levels_default_to_three_tiers() {
        let metadata = ModelMetadata::default()
            .with_supports_verbosity(true)
            .with_default_verbosity("low");
        let levels = metadata.supported_verbosity_levels_for_model(&ModelId::new("gpt-5.4"));
        assert_eq!(levels, vec!["low", "medium", "high"]);
        assert!(metadata.supports_verbosity_level_for_model(&ModelId::new("gpt-5.4"), "HIGH"));
    }

    #[test]
    fn model_metadata_supported_verbosity_levels_restrict_chat_models_to_medium() {
        let metadata = ModelMetadata::default()
            .with_supports_verbosity(true)
            .with_default_verbosity("medium");
        let levels =
            metadata.supported_verbosity_levels_for_model(&ModelId::new("gpt-5.2-chat-latest"));
        assert_eq!(levels, vec!["medium"]);
        assert!(
            metadata
                .supports_verbosity_level_for_model(&ModelId::new("gpt-5.2-chat-latest"), "medium")
        );
        assert!(
            !metadata
                .supports_verbosity_level_for_model(&ModelId::new("gpt-5.2-chat-latest"), "low")
        );
    }

    #[test]
    fn model_capability_fallbacks_replace_default_placeholder_capabilities() {
        let model = Model::new("bedrock", "anthropic.claude-opus-4-7");
        let fallback = ModelCapabilities::default()
            .with_reasoning(CapabilitySupport::Supported)
            .with_temperature_supported(CapabilitySupport::Unsupported);

        let merged = model.with_capability_fallbacks(&fallback);

        assert_eq!(merged.capabilities.reasoning, CapabilitySupport::Supported);
        assert_eq!(
            merged.capabilities.temperature_supported,
            CapabilitySupport::Unsupported
        );
    }
}
