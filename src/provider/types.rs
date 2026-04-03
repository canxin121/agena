use serde::{Deserialize, Serialize};

use crate::{
    message::{AttachmentItem, AttachmentKind, Message, MessageUsage},
    tool::ToolDefinition,
};

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
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderModel {
    pub provider_id: String,
    pub id: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub capabilities: ModelCapabilities,
}

impl ProviderModel {
    pub fn new(provider_id: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            id: id.into(),
            display_name: None,
            capabilities: ModelCapabilities::default(),
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_capabilities(mut self, capabilities: ModelCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_capability_fallbacks(mut self, fallback: &ModelCapabilities) -> Self {
        self.capabilities = self.capabilities.with_fallbacks_from(fallback);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionRequest {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionResponse {
    pub provider_id: String,
    pub model: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<CompletionFinishReason>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<CompletionToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<CompletionUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "raw", rename_all = "snake_case")]
pub enum CompletionFinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionToolCall {
    Function {
        id: String,
        name: String,
        #[serde(default)]
        arguments_json: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost: f64,
}

impl From<MessageUsage> for CompletionUsage {
    fn from(value: MessageUsage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            reasoning_tokens: value.reasoning_tokens,
            cache_write_tokens: value.cache_write_tokens,
            cache_read_tokens: value.cache_read_tokens,
            total_cost: value.total_cost,
        }
    }
}

impl From<CompletionUsage> for MessageUsage {
    fn from(value: CompletionUsage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            reasoning_tokens: value.reasoning_tokens,
            cache_write_tokens: value.cache_write_tokens,
            cache_read_tokens: value.cache_read_tokens,
            total_cost: value.total_cost,
        }
    }
}

impl CompletionFinishReason {
    pub fn from_provider(value: Option<impl AsRef<str>>) -> Option<Self> {
        let value = value?;
        let raw = value.as_ref().trim();
        if raw.is_empty() {
            return None;
        }

        let normalized = raw.to_ascii_lowercase().replace('-', "_");
        let reason = match normalized.as_str() {
            "stop" | "end_turn" | "message_stop" | "completed" => Self::Stop,
            "length" | "max_tokens" => Self::Length,
            "tool_calls" | "tool_use" | "function_call" => Self::ToolCalls,
            "content_filter" => Self::ContentFilter,
            _ => Self::Other(raw.to_owned()),
        };
        Some(reason)
    }
}

impl CapabilitySupport {
    const fn supported() -> Self {
        Self::Supported
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionStreamEvent {
    TextDelta {
        provider_id: String,
        model: String,
        delta: String,
    },
    ToolCallDelta {
        provider_id: String,
        model: String,
        stream_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default)]
        arguments_delta: String,
    },
    Completed {
        provider_id: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish_reason: Option<CompletionFinishReason>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<CompletionUsage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<serde_json::Value>,
    },
}
