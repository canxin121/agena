//! Compatibility names for product-native protocol fixtures.
//!
//! New tests can select ModelProtocol with TestAgena::model_protocol and use the
//! same Reply / RequestMatch API for every provider.

use crate::{ModelProtocol, ModelRoute, Reply};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeProtocol {
    Anthropic,
    Gemini,
    Ollama,
    OpenAiChat,
    BedrockAnthropic,
}

impl From<NativeProtocol> for ModelProtocol {
    fn from(protocol: NativeProtocol) -> Self {
        match protocol {
            NativeProtocol::Anthropic => Self::Anthropic,
            NativeProtocol::Gemini => Self::Gemini,
            NativeProtocol::Ollama => Self::Ollama,
            NativeProtocol::OpenAiChat => Self::OpenAiChat,
            NativeProtocol::BedrockAnthropic => Self::BedrockAnthropic,
        }
    }
}

impl NativeProtocol {
    pub fn adapter(self) -> &'static str {
        ModelProtocol::from(self)
            .adapter()
            .expect("native protocol has an adapter")
    }

    pub fn routes(self) -> Vec<ModelRoute> {
        ModelProtocol::from(self).routes()
    }

    pub fn text(self, text: &str) -> Reply {
        Reply::text(text).for_protocol(self.into())
    }

    pub fn tool(self, call_id: &str, tool: &str, input: Value) -> Reply {
        Reply::tool_call(call_id, tool, input).for_protocol(self.into())
    }

    pub fn request_path(self) -> &'static str {
        match self {
            Self::Anthropic => "/v1/messages",
            Self::Gemini => "/v1beta/models/fake-model:streamGenerateContent",
            Self::Ollama => "/api/chat",
            Self::OpenAiChat => "/v1/chat/completions",
            Self::BedrockAnthropic => {
                "/model/us.anthropic.claude-test-v1:0/invoke-with-response-stream"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_helpers_are_not_openai_responses_and_routes_are_explicit() {
        for protocol in [
            NativeProtocol::Anthropic,
            NativeProtocol::Gemini,
            NativeProtocol::Ollama,
            NativeProtocol::OpenAiChat,
            NativeProtocol::BedrockAnthropic,
        ] {
            assert!(!protocol.routes().is_empty());
            assert_ne!(protocol.request_path(), "/v1/responses");
        }
    }
}
