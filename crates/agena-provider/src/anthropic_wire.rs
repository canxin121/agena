//! Complete protocol records for Anthropic's Messages API.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AnthropicTextBlock, CopilotModelExtension};

#[derive(Debug, Serialize)]
/// Wire shape of an Anthropic Messages API request.
pub struct AnthropicMessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Vec<AnthropicTextBlock>>,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<AnthropicOutputConfig>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub stop_sequences: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

#[derive(Debug, Serialize)]
/// Wire shape of an Anthropic output config (effort).
pub struct AnthropicOutputConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<&'static str>,
}

#[derive(Debug, Serialize)]
/// Wire shape of a message inside an Anthropic request.
pub struct AnthropicMessage {
    pub role: String,
    pub content: Vec<AnthropicTextBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
/// Wire shape of an Anthropic model list response.
pub enum AnthropicModelListResponse {
    Wrapped { data: Vec<AnthropicModel> },
    Bare(Vec<AnthropicModel>),
}

impl AnthropicModelListResponse {
    pub fn into_items(self) -> Vec<AnthropicModel> {
        match self {
            Self::Wrapped { data } | Self::Bare(data) => data,
        }
    }
}

#[derive(Debug, Deserialize)]
/// Wire shape of an Anthropic model entry.
pub struct AnthropicModel {
    pub id: String,
    #[serde(default, flatten)]
    pub copilot: CopilotModelExtension,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of an Anthropic Messages API response.
pub struct AnthropicMessagesResponse {
    pub model: String,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub content: Vec<AnthropicTextBlock>,
    #[serde(default)]
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of Anthropic usage counters.
pub struct AnthropicUsage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens_details: Option<AnthropicOutputTokensDetails>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation: Option<AnthropicCacheCreationUsage>,
}

#[derive(Debug, Deserialize, Default)]
/// Wire shape of Anthropic output token details.
pub struct AnthropicOutputTokensDetails {
    #[serde(default)]
    pub thinking_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
/// Wire shape of Anthropic cache creation counters.
pub struct AnthropicCacheCreationUsage {
    #[serde(default)]
    pub ephemeral_1h_input_tokens: Option<u64>,
    #[serde(default)]
    pub ephemeral_5m_input_tokens: Option<u64>,
}

impl AnthropicCacheCreationUsage {
    pub fn total_input_tokens(&self) -> u64 {
        self.ephemeral_1h_input_tokens.unwrap_or_default()
            + self.ephemeral_5m_input_tokens.unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Wire shape of an Anthropic SSE event.
pub enum AnthropicSseEvent {
    MessageStart {
        #[serde(default)]
        message: AnthropicSseMessage,
    },
    ContentBlockStart {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        content_block: AnthropicSseContentBlock,
    },
    ContentBlockDelta {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        delta: AnthropicSseDelta,
    },
    ContentBlockStop {
        #[serde(default)]
        index: Option<usize>,
    },
    MessageDelta {
        #[serde(default)]
        delta: AnthropicSseMessageDelta,
        #[serde(default)]
        usage: Option<AnthropicUsage>,
        #[serde(default)]
        message: Option<AnthropicSseMessage>,
    },
    MessageStop {
        #[serde(default)]
        usage: Option<AnthropicUsage>,
        #[serde(default)]
        message: Option<AnthropicSseMessage>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize, Default)]
/// Wire shape of an Anthropic SSE content block.
pub struct AnthropicSseContentBlock {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub input: Option<Value>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
/// Wire shape of an Anthropic SSE delta.
pub struct AnthropicSseDelta {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub partial_json: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
/// Wire shape of an Anthropic SSE message delta.
pub struct AnthropicSseMessageDelta {
    #[serde(default)]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
/// Wire shape of an Anthropic SSE message.
pub struct AnthropicSseMessage {
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Default)]
/// Accumulated state of an Anthropic tool call.
pub struct AnthropicToolCallState {
    pub id: String,
    pub name: String,
    /// Accumulated JSON arguments for one tool_use block. Structured-output
    /// requests surface this as the completion text, so it is tracked even
    /// though the stream only yields deltas.
    pub arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(tool_choice: Option<Value>) -> AnthropicMessagesRequest {
        AnthropicMessagesRequest {
            model: "claude-test".to_owned(),
            max_tokens: 256,
            system: None,
            messages: Vec::new(),
            tools: Some(vec![serde_json::json!({
                "name": "permission_verdict",
                "input_schema": { "type": "object" }
            })]),
            tool_choice,
            temperature: None,
            stream: None,
            thinking: None,
            output_config: None,
            stop_sequences: Vec::new(),
            top_p: None,
            top_k: None,
        }
    }

    #[test]
    fn tool_choice_serializes_when_present_and_is_omitted_when_absent() {
        let json = serde_json::to_value(request(Some(serde_json::json!({
            "type": "tool",
            "name": "permission_verdict"
        }))))
        .expect("request should serialize");
        assert_eq!(
            json["tool_choice"],
            serde_json::json!({ "type": "tool", "name": "permission_verdict" })
        );

        let json = serde_json::to_value(request(None)).expect("request should serialize");
        assert!(json.get("tool_choice").is_none());
    }
}
