use serde::{Deserialize, Serialize};

use crate::{
    message::{Message, MessageUsage},
    model::{ModelId, ProviderId},
    tool::ToolDefinition,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionRequest {
    pub model: ModelId,
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
    pub provider_id: ProviderId,
    pub model: ModelId,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionStreamEvent {
    TextDelta {
        provider_id: ProviderId,
        model: ModelId,
        delta: String,
    },
    ToolCallDelta {
        provider_id: ProviderId,
        model: ModelId,
        stream_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default)]
        arguments_delta: String,
    },
    Completed {
        provider_id: ProviderId,
        model: ModelId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish_reason: Option<CompletionFinishReason>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<CompletionUsage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<serde_json::Value>,
    },
}
