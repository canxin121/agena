use std::{
    collections::{BTreeMap, HashMap},
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::{
    config::ProviderToolsConfig,
    message::{Message, MessageUsage, OperationBlock, ToolInvocation, ToolOutput},
    model::{ModelId, ModelSpeedModeRequestOverride, ProviderId},
    plugin::registry::RegisteredTool,
};

/// Controls extended thinking / reasoning for providers that support it.
///
/// For Anthropic: maps to `thinking` plus provider-specific effort/output settings.
/// For OpenAI reasoning models: maps to Responses `reasoning.effort` or
/// Chat Completions `reasoning_effort`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThinkingRequest {
    #[serde(rename = "budget")]
    Budget {
        budget_tokens: u32,
    },
    Adaptive {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<ReasoningEffort>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<ThinkingDisplay>,
    },
    Effort {
        effort: ReasoningEffort,
    },
    Disabled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl AsRef<str> for ReasoningEffort {
    fn as_ref(&self) -> &str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingDisplay {
    Summarized,
    Omitted,
}

impl AsRef<str> for ThinkingDisplay {
    fn as_ref(&self) -> &str {
        match self {
            Self::Summarized => "summarized",
            Self::Omitted => "omitted",
        }
    }
}

impl fmt::Display for ThinkingDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

/// Instructs the provider to produce output in a specific format.
///
/// Not all providers support all thinking/speed modes; unsupported settings are silently ignored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        schema: serde_json::Value,
        #[serde(default)]
        strict: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponsesApiRequestMetadata {
    pub installation_id: String,
    pub session_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub window_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_started_at_unix_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

impl ResponsesApiRequestMetadata {
    pub fn client_metadata(&self) -> HashMap<String, String> {
        let mut metadata = HashMap::from([
            (
                "x-codex-installation-id".to_owned(),
                self.installation_id.clone(),
            ),
            ("session_id".to_owned(), self.session_id.clone()),
            ("thread_id".to_owned(), self.thread_id.clone()),
            ("turn_id".to_owned(), self.turn_id.clone()),
            ("x-codex-window-id".to_owned(), self.window_id.clone()),
            (
                "x-codex-turn-metadata".to_owned(),
                self.turn_metadata_json(),
            ),
        ]);
        if let Some(subagent_header) = self.subagent_header.as_ref() {
            metadata.insert("x-openai-subagent".to_owned(), subagent_header.clone());
        }
        if let Some(parent_thread_id) = self.parent_thread_id.as_ref() {
            metadata.insert(
                "x-codex-parent-thread-id".to_owned(),
                parent_thread_id.clone(),
            );
        }
        metadata
    }

    pub fn compatibility_headers(&self) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::from([
            ("x-codex-window-id".to_owned(), self.window_id.clone()),
            (
                "x-codex-turn-metadata".to_owned(),
                self.turn_metadata_json(),
            ),
        ]);
        if let Some(subagent_header) = self.subagent_header.as_ref() {
            headers.insert("x-openai-subagent".to_owned(), subagent_header.clone());
        }
        if let Some(parent_thread_id) = self.parent_thread_id.as_ref() {
            headers.insert(
                "x-codex-parent-thread-id".to_owned(),
                parent_thread_id.clone(),
            );
        }
        headers
    }

    pub fn session_headers(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("session-id".to_owned(), self.session_id.clone()),
            ("thread-id".to_owned(), self.thread_id.clone()),
        ])
    }

    pub fn turn_metadata_json(&self) -> String {
        let mut value = serde_json::Map::from_iter([
            (
                "installation_id".to_owned(),
                serde_json::Value::String(self.installation_id.clone()),
            ),
            (
                "session_id".to_owned(),
                serde_json::Value::String(self.session_id.clone()),
            ),
            (
                "thread_id".to_owned(),
                serde_json::Value::String(self.thread_id.clone()),
            ),
            (
                "turn_id".to_owned(),
                serde_json::Value::String(self.turn_id.clone()),
            ),
            (
                "window_id".to_owned(),
                serde_json::Value::String(self.window_id.clone()),
            ),
        ]);
        if let Some(parent_thread_id) = self.parent_thread_id.as_ref() {
            value.insert(
                "parent_thread_id".to_owned(),
                serde_json::Value::String(parent_thread_id.clone()),
            );
        }
        if let Some(subagent_kind) = self.subagent_kind.as_ref() {
            value.insert(
                "subagent_kind".to_owned(),
                serde_json::Value::String(subagent_kind.clone()),
            );
        }
        if let Some(request_kind) = self.request_kind.as_ref() {
            value.insert(
                "request_kind".to_owned(),
                serde_json::Value::String(request_kind.clone()),
            );
        }
        if let Some(turn_started_at_unix_ms) = self.turn_started_at_unix_ms {
            value.insert(
                "turn_started_at_unix_ms".to_owned(),
                serde_json::Value::from(turn_started_at_unix_ms),
            );
        }
        for (key, field_value) in &self.extra {
            if !key.trim().is_empty()
                && !field_value.trim().is_empty()
                && !reserved_responses_metadata_key(key)
            {
                value.insert(key.clone(), serde_json::Value::String(field_value.clone()));
            }
        }
        serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_owned())
    }
}

fn reserved_responses_metadata_key(key: &str) -> bool {
    matches!(
        key,
        "installation_id"
            | "x-codex-installation-id"
            | "session_id"
            | "session-id"
            | "thread_id"
            | "thread-id"
            | "turn_id"
            | "window_id"
            | "x-codex-window-id"
            | "x-codex-turn-metadata"
            | "x-codex-parent-thread-id"
            | "x-openai-subagent"
            | "request_kind"
            | "turn_started_at_unix_ms"
            | "parent_thread_id"
            | "subagent_kind"
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionRequest {
    pub model: ModelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<RegisteredTool>,
    #[serde(
        default,
        alias = "native_tools",
        skip_serializing_if = "ProviderToolsConfig::is_empty"
    )]
    pub provider_tools: ProviderToolsConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_window_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responses_api_metadata: Option<ResponsesApiRequestMetadata>,
    #[serde(
        default,
        skip_serializing_if = "ModelSpeedModeRequestOverride::is_empty"
    )]
    pub request_override: ModelSpeedModeRequestOverride,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionResponse {
    pub provider_id: ProviderId,
    pub model: ModelId,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_text: Option<String>,
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
    ThinkingDelta {
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
    ToolCallSnapshot {
        provider_id: ProviderId,
        model: ModelId,
        stream_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default)]
        arguments_json: String,
    },
    #[serde(alias = "native_tool_call_started")]
    ProviderToolCallStarted {
        provider_id: ProviderId,
        model: ModelId,
        stream_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        invocation: ToolInvocation,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<serde_json::Value>,
    },
    #[serde(alias = "native_tool_call_completed")]
    ProviderToolCallCompleted {
        provider_id: ProviderId,
        model: ModelId,
        stream_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        invocation: ToolInvocation,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        output_text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<OperationBlock>,
        #[serde(default, skip_serializing_if = "ToolOutput::is_empty")]
        details: ToolOutput,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<serde_json::Value>,
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
