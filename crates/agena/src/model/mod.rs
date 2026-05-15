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
    #[error("model reference must be in `provider/adapter/model` format")]
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
            model_id: ModelId::try_new(model_id)?,
        })
    }
}

impl fmt::Display for ModelRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider_id, self.model_id)
    }
}

impl FromStr for ModelRef {
    type Err = ModelRefParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((provider_id, model_id)) = value.split_once('/') else {
            return Err(ModelRefParseError::MissingSeparator);
        };
        if !model_id.contains('/') {
            return Err(ModelRefParseError::MissingSeparator);
        }
        let provider_id = ProviderId::try_new(provider_id)?;
        let model_id = ModelId::try_new(model_id)
            .map_err(|err| ModelRefParseError::InvalidModelId(err.to_string()))?;
        Ok(Self {
            provider_id,
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
pub enum ModelFamily {
    Gpt,
    Codex,
    Claude,
    Gemini,
    Llama,
    Mistral,
    Deepseek,
    Qwen,
    Nova,
    Grok,
    Phi,
    Command,
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
    pub max_output_tokens: Option<u32>,
}

impl ModelTokenLimits {
    pub fn is_empty(&self) -> bool {
        self.context_window_tokens.is_none() && self.max_output_tokens.is_none()
    }

    pub fn with_context_window_tokens(mut self, context_window_tokens: u32) -> Self {
        self.context_window_tokens = Some(context_window_tokens);
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
        if self.max_output_tokens.is_none() {
            self.max_output_tokens = fallback.max_output_tokens;
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<ModelFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(default, skip_serializing_if = "ModelTokenLimits::is_empty")]
    pub limits: ModelTokenLimits,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ModelMetadata {
    pub fn is_empty(&self) -> bool {
        self.family.is_none()
            && self.lifecycle.is_none()
            && self.limits.is_empty()
            && self.description.is_none()
    }

    pub fn with_family(mut self, family: ModelFamily) -> Self {
        self.family = Some(family);
        self
    }

    pub fn with_lifecycle(mut self, lifecycle: ModelLifecycle) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_context_window_tokens(mut self, context_window_tokens: u32) -> Self {
        self.limits = self
            .limits
            .with_context_window_tokens(context_window_tokens);
        self
    }

    pub fn with_max_output_tokens(mut self, max_output_tokens: u32) -> Self {
        self.limits = self.limits.with_max_output_tokens(max_output_tokens);
        self
    }

    pub fn with_fallbacks_from(mut self, fallback: &Self) -> Self {
        if self.family.is_none() {
            self.family = fallback.family;
        }
        if self.lifecycle.is_none() {
            self.lifecycle = fallback.lifecycle;
        }
        self.limits = self.limits.with_fallbacks_from(&fallback.limits);
        if self.description.is_none() {
            self.description = fallback.description.clone();
        }
        self
    }
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
pub struct ModelVariant {
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingRequest>,
}

impl ModelVariant {
    pub fn new() -> Self {
        Self {
            display_name: None,
            description: None,
            thinking: None,
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
}

impl Default for ModelVariant {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Model {
    pub provider_id: ProviderId,
    pub id: ModelId,
    pub display_name: Option<String>,
    #[serde(default)]
    pub capabilities: ModelCapabilities,
    #[serde(default, skip_serializing_if = "ModelMetadata::is_empty")]
    pub metadata: ModelMetadata,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variants: BTreeMap<String, ModelVariant>,
}

impl Model {
    pub fn new(provider_id: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            provider_id: ProviderId::new(provider_id),
            id: ModelId::new(id),
            display_name: None,
            capabilities: ModelCapabilities::default(),
            metadata: ModelMetadata::default(),
            variants: BTreeMap::new(),
        }
    }

    pub fn reference(&self) -> ModelRef {
        ModelRef {
            provider_id: self.provider_id.clone(),
            model_id: self.id.clone(),
        }
    }

    pub fn with_provider_id(mut self, provider_id: impl Into<String>) -> Self {
        self.provider_id = ProviderId::new(provider_id);
        self
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
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

    pub fn with_variants(mut self, variants: BTreeMap<String, ModelVariant>) -> Self {
        self.variants = variants;
        self
    }

    pub fn with_variant(mut self, name: impl Into<String>, variant: ModelVariant) -> Self {
        self.variants.insert(name.into(), variant);
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.metadata = self.metadata.with_description(description);
        self
    }

    pub fn with_family(mut self, family: ModelFamily) -> Self {
        self.metadata = self.metadata.with_family(family);
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

    pub fn with_max_output_tokens(mut self, max_output_tokens: u32) -> Self {
        self.metadata = self.metadata.with_max_output_tokens(max_output_tokens);
        self
    }

    pub fn with_capability_fallbacks(mut self, fallback: &ModelCapabilities) -> Self {
        self.capabilities = self.capabilities.with_fallbacks_from(fallback);
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
        let parsed: ModelRef = "openai/openai/gpt-5"
            .parse()
            .expect("model ref should parse");
        assert_eq!(parsed.provider_id.as_str(), "openai");
        assert_eq!(parsed.model_id.as_str(), "openai/gpt-5");
    }

    #[test]
    fn model_ref_rejects_missing_separator() {
        let err = "openai/gpt-5"
            .parse::<ModelRef>()
            .expect_err("missing provider should fail");
        assert!(matches!(err, ModelRefParseError::MissingSeparator));
    }

    #[test]
    fn model_metadata_fallbacks_fill_missing_fields() {
        let base = ModelMetadata::default().with_family(ModelFamily::Gpt);
        let fallback = ModelMetadata::default()
            .with_lifecycle(ModelLifecycle::Preview)
            .with_context_window_tokens(128_000)
            .with_max_output_tokens(16_384);

        let merged = base.with_fallbacks_from(&fallback);
        assert_eq!(merged.family, Some(ModelFamily::Gpt));
        assert_eq!(merged.lifecycle, Some(ModelLifecycle::Preview));
        assert_eq!(merged.limits.context_window_tokens, Some(128_000));
        assert_eq!(merged.limits.max_output_tokens, Some(16_384));
    }
}
