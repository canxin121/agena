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
