use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// Whether a model capability is supported.
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
/// Input modality a model accepts (text, image, document, ...).
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
/// Lifecycle stage of a model (active, preview, beta, deprecated, ...).
pub enum ModelLifecycle {
    Active,
    Preview,
    Beta,
    Alpha,
    Experimental,
    Deprecated,
}

#[cfg(test)]
mod tests {
    use super::{CapabilitySupport, ModelInputModality, ModelLifecycle};

    #[test]
    fn capability_support_has_expected_default_and_helpers() {
        assert_eq!(CapabilitySupport::default(), CapabilitySupport::Unknown);
        assert!(CapabilitySupport::Supported.is_supported());
        assert!(CapabilitySupport::Unsupported.is_unsupported());
        assert_eq!(CapabilitySupport::supported(), CapabilitySupport::Supported);
    }

    #[test]
    fn model_input_modality_uses_stable_wire_and_display_values() {
        assert_eq!(ModelInputModality::Document.as_ref(), "document");
        assert_eq!(ModelInputModality::Video.to_string(), "video");
        assert_eq!(
            serde_json::to_string(&ModelInputModality::File).unwrap(),
            "\"file\""
        );
    }

    #[test]
    fn model_lifecycle_uses_snake_case_wire_values() {
        assert_eq!(
            serde_json::to_string(&ModelLifecycle::Experimental).unwrap(),
            "\"experimental\""
        );
        assert_eq!(
            serde_json::from_str::<ModelLifecycle>("\"deprecated\"").unwrap(),
            ModelLifecycle::Deprecated
        );
    }
}
