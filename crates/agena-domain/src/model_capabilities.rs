use serde::{Deserialize, Serialize};

use crate::{CapabilitySupport, ModelInputModality};

/// Stable capability declarations for a provider model catalog entry.
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

#[cfg(test)]
mod tests {
    use super::ModelCapabilities;
    use crate::{CapabilitySupport, ModelInputModality};

    #[test]
    fn defaults_preserve_catalog_wire_contract() {
        let value = serde_json::to_value(ModelCapabilities::default()).unwrap();
        assert_eq!(value["text_input"], "supported");
        assert_eq!(value["temperature_supported"], "supported");
        assert_eq!(value["streaming"], "unknown");
    }

    #[test]
    fn unknown_capabilities_accept_fallbacks() {
        let primary = ModelCapabilities::default();
        let fallback = ModelCapabilities {
            image_input: CapabilitySupport::Supported,
            ..ModelCapabilities::default()
        };

        let merged = primary.merged_with_fallbacks_from(&fallback);
        assert_eq!(merged.image_input, CapabilitySupport::Supported);
        assert_eq!(
            merged.support_for_input_modality(ModelInputModality::Image),
            CapabilitySupport::Supported
        );
    }
}
