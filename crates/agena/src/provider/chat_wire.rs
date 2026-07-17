/// Shared wire types and message-conversion helpers for OpenAI-compatible
/// Chat Completions API endpoints.
///
/// The explicit OpenAI Chat Completions adapter and compatible Chat
/// Completions backends share these structs. Responses and Realtime use their
/// own wire types and never serialize this schema.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::AppError,
    message::{AssistantReasoningField, Message, MessageUsage},
    model::{ModelId, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionToolCall,
        CompletionUsage, ResponseFormat, ThinkingRequest, utils, wire_message,
    },
    role::Role,
};

// ─── Request body ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ChatToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    /// Prompt-cache control field used by some OpenAI-compatible providers
    /// (e.g. OpenRouter, ZenMux). Not sent when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<crate::provider::prompt_cache::PromptCacheControl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    /// OpenAI-compatible switch controlling whether the model may return
    /// more than one function call in a turn. Omitted unless the caller
    /// explicitly selected a policy so existing provider defaults remain
    /// unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<ChatStreamOptions>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub stop: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ChatResponseFormat>,
    /// OpenAI o-series reasoning effort; `None` for non-reasoning models.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatStreamOptions {
    pub include_usage: bool,
}

// ─── Message types ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_opaque: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCallRequest>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_cache_control: Option<crate::provider::prompt_cache::PromptCacheControl>,
}

impl ChatMessage {
    pub fn system(content: String) -> Self {
        Self {
            role: "system".to_owned(),
            kind: None,
            content: Some(Value::String(content)),
            reasoning_content: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            tool_calls: None,
            tool_call_id: None,
            copilot_cache_control: None,
        }
    }

    pub fn user(content: Value) -> Self {
        Self {
            role: "user".to_owned(),
            kind: None,
            content: Some(content),
            reasoning_content: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            tool_calls: None,
            tool_call_id: None,
            copilot_cache_control: None,
        }
    }

    pub fn assistant(content: Option<Value>, tool_calls: Option<Vec<ChatToolCallRequest>>) -> Self {
        Self {
            role: "assistant".to_owned(),
            kind: None,
            content,
            reasoning_content: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            tool_calls,
            tool_call_id: None,
            copilot_cache_control: None,
        }
    }

    pub fn tool_result(tool_call_id: String, content: Value) -> Self {
        Self {
            role: "tool".to_owned(),
            kind: None,
            content: Some(content),
            reasoning_content: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            copilot_cache_control: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ChatToolCallRequest {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub function: ChatFunctionCallRequest,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ChatFunctionCallRequest {
    pub name: String,
    pub arguments: String,
}

// ─── Tool definition ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct ChatToolDefinition {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ChatFunctionDefinition,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatFunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub strict: bool,
}

// ─── Response format ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ChatResponseFormat {
    Text,
    JsonObject,
    #[serde(rename = "json_schema")]
    JsonSchema {
        json_schema: ChatJsonSchemaSpec,
    },
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatJsonSchemaSpec {
    pub name: String,
    pub schema: Value,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub strict: bool,
}

pub(crate) fn map_response_format(fmt: Option<&ResponseFormat>) -> Option<ChatResponseFormat> {
    match fmt? {
        ResponseFormat::Text => Some(ChatResponseFormat::Text),
        ResponseFormat::JsonObject => Some(ChatResponseFormat::JsonObject),
        ResponseFormat::JsonSchema {
            name,
            schema,
            strict,
        } => Some(ChatResponseFormat::JsonSchema {
            json_schema: ChatJsonSchemaSpec {
                name: name.clone(),
                schema: schema.clone(),
                strict: *strict,
            },
        }),
    }
}

// ─── Reasoning effort mapping ─────────────────────────────────────────────────

/// Convert a `ThinkingRequest` to an OpenAI `reasoning_effort` string for
/// OpenAI reasoning models. Returns `None` for non-reasoning models or when thinking
/// is disabled / absent.
pub(crate) fn reasoning_effort(thinking: Option<&ThinkingRequest>, model: &str) -> Option<String> {
    if !supports_reasoning_effort(model) {
        return None;
    }
    match thinking {
        Some(ThinkingRequest::Effort { effort }) => {
            Some(openai_compatible_reasoning_effort(model, *effort).to_owned())
        }
        Some(ThinkingRequest::Adaptive { effort, .. }) => Some(
            openai_compatible_reasoning_effort(
                model,
                (*effort).unwrap_or(crate::provider::ReasoningEffort::High),
            )
            .to_owned(),
        ),
        Some(ThinkingRequest::Budget { budget_tokens }) => {
            let effort = if *budget_tokens > 10_000 {
                "high"
            } else if *budget_tokens > 3_000 {
                "medium"
            } else {
                "low"
            };
            Some(effort.to_owned())
        }
        Some(ThinkingRequest::Disabled) if supports_none_reasoning_effort(model) => {
            Some("none".to_owned())
        }
        _ => None,
    }
}

fn openai_compatible_reasoning_effort(
    model: &str,
    effort: crate::provider::ReasoningEffort,
) -> &'static str {
    use crate::provider::ReasoningEffort;

    match effort {
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::Xhigh => "xhigh",
        ReasoningEffort::Max if model.to_ascii_lowercase().contains("deepseek-v4") => "max",
        // OpenAI's strongest official wire value is `xhigh`; `max` is used
        // by Anthropic and a small number of OpenAI-compatible gateways.
        ReasoningEffort::Max => "xhigh",
    }
}

fn supports_none_reasoning_effort(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    let Some(version) = normalized
        .split(['/', ':'])
        .find_map(|segment| segment.strip_prefix("gpt-5."))
        .and_then(|suffix| suffix.split(['-', '.']).next())
        .and_then(|version| version.parse::<u32>().ok())
    else {
        return false;
    };
    version >= 1
}

pub(crate) fn supports_reasoning_effort(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.split('/').any(|segment| {
        segment.starts_with("o1")
            || segment.starts_with("o3")
            || segment.starts_with("o4")
            || segment.starts_with("gpt-5")
    }) || normalized.contains("codex")
        || normalized.contains("deepseek-v4")
}

// ─── Response / decode types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ChatCompletionResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<ChatCompletionChoice>,
    #[serde(default)]
    pub usage: Option<ChatUsage>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatCompletionChoice {
    #[serde(default)]
    pub message: Option<ChatDeltaOrMessage>,
    #[serde(default)]
    pub delta: Option<ChatDeltaOrMessage>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatDeltaOrMessage {
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default)]
    pub reasoning_content: Option<Value>,
    #[serde(default)]
    pub reasoning_details: Option<Value>,
    #[serde(default)]
    pub reasoning_text: Option<Value>,
    #[serde(default)]
    pub reasoning_opaque: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ChatToolCallWire>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatToolCallWire {
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<ChatFunctionCallWire>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatFunctionCallWire {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatUsage {
    #[serde(default)]
    pub prompt_tokens: Option<u64>,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
    /// xAI reports exact request cost in 10^-10 USD ticks.
    #[serde(default)]
    pub cost_in_usd_ticks: Option<u64>,
    /// GitHub Copilot Chat reports reasoning separately at the usage top level.
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    /// OpenAI Chat Completions uses this field name.
    #[serde(default)]
    pub completion_tokens_details: Option<ChatOutputTokensDetails>,
    /// Some compatible gateways additionally expose the Responses-style name.
    /// Keep it separate rather than as a serde alias: gateways such as xAI can
    /// send both names in one payload, which makes aliases fail as duplicates.
    #[serde(default)]
    pub output_tokens_details: Option<ChatOutputTokensDetails>,
    /// OpenAI Chat Completions uses this field name.
    #[serde(default)]
    pub prompt_tokens_details: Option<ChatInputTokensDetails>,
    /// Some compatible gateways additionally expose the Responses-style name.
    #[serde(default)]
    pub input_tokens_details: Option<ChatInputTokensDetails>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatOutputTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatInputTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

// ─── Utility helpers ──────────────────────────────────────────────────────────

pub(crate) fn extract_text_from_content(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

pub(crate) fn extract_reasoning_text_from_fields(
    reasoning_content: Option<&Value>,
    reasoning_details: Option<&Value>,
    reasoning_text: Option<&Value>,
) -> Option<String> {
    [
        reasoning_content.map(extract_text_from_content),
        reasoning_details.map(extract_reasoning_details_text),
        reasoning_text.map(extract_text_from_content),
    ]
    .into_iter()
    .flatten()
    .find(|text| !text.trim().is_empty())
}

fn extract_reasoning_details_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(extract_reasoning_details_text)
            .collect::<Vec<_>>()
            .join(""),
        Value::Object(item) => item
            .get("text")
            .or_else(|| item.get("summary"))
            .map(extract_reasoning_details_text)
            .unwrap_or_default(),
        _ => String::new(),
    }
}

pub(crate) fn assistant_reasoning_field_from_fields(
    reasoning_content: Option<&Value>,
    reasoning_details: Option<&Value>,
) -> Option<&'static str> {
    if reasoning_content.is_some() {
        Some("reasoning_content")
    } else if reasoning_details.is_some() {
        Some("reasoning_details")
    } else {
        None
    }
}

pub(crate) fn merge_reasoning_details(target: &mut Option<Value>, incoming: &Value) {
    let Some(current) = target.as_mut() else {
        *target = Some(incoming.clone());
        return;
    };
    let (Some(current_items), Some(incoming_items)) = (current.as_array_mut(), incoming.as_array())
    else {
        *current = incoming.clone();
        return;
    };

    for (position, incoming_item) in incoming_items.iter().enumerate() {
        // OpenRouter's reasoning_details is an ordered continuation token.
        // Only adjacent text deltas from consecutive SSE events may be
        // coalesced; summary/encrypted blocks and non-adjacent text must stay
        // in arrival order so replay sends the exact provider state back.
        let merge_with_tail = position == 0
            && current_items.last().is_some_and(|current| {
                reasoning_detail_key(current) == reasoning_detail_key(incoming_item)
                    && reasoning_detail_key(current)
                        .is_some_and(|(kind, _)| kind == "reasoning.text")
            });
        if merge_with_tail {
            let current = current_items
                .last_mut()
                .expect("merge candidate checked above");
            merge_reasoning_detail(current, incoming_item);
        } else {
            current_items.push(incoming_item.clone());
        }
    }
}

fn reasoning_detail_key(value: &Value) -> Option<(String, u64)> {
    let object = value.as_object()?;
    let kind = object.get("type")?.as_str()?.to_owned();
    let index = object
        .get("index")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    Some((kind, index))
}

fn merge_reasoning_detail(current: &mut Value, incoming: &Value) {
    if !current.is_object() || !incoming.is_object() {
        *current = incoming.clone();
        return;
    }
    let current = current.as_object_mut().expect("object checked above");
    let incoming = incoming.as_object().expect("object checked above");
    for (key, value) in incoming {
        if key == "text"
            && let Some(next_text) = value.as_str()
            && let Some(existing_text) = current.get(key).and_then(Value::as_str)
        {
            let merged = if next_text == existing_text || existing_text.ends_with(next_text) {
                existing_text.to_owned()
            } else if next_text.starts_with(existing_text) {
                next_text.to_owned()
            } else {
                format!("{existing_text}{next_text}")
            };
            current.insert(key.clone(), Value::String(merged));
        } else {
            current.insert(key.clone(), value.clone());
        }
    }
}

pub(crate) fn assistant_reasoning_field_from_delta_or_message(
    value: &ChatDeltaOrMessage,
) -> Option<&'static str> {
    assistant_reasoning_field_from_fields(
        value.reasoning_content.as_ref(),
        value.reasoning_details.as_ref(),
    )
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ChatUsage, ThinkingRequest,
        apply_raw_assistant_reasoning_state, chat_usage_to_completion, merge_reasoning_details,
        parse_completion_response, reasoning_effort,
    };
    use crate::{
        message::{Message, MessageProviderState, PartContent},
        role::Role,
    };

    fn request(parallel_tool_calls: Option<bool>) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: Vec::new(),
            tools: None,
            temperature: None,
            max_tokens: None,
            max_completion_tokens: None,
            cache_control: None,
            prompt_cache_key: None,
            parallel_tool_calls,
            stream: true,
            stream_options: None,
            stop: Vec::new(),
            top_p: None,
            seed: None,
            response_format: None,
            reasoning_effort: None,
            verbosity: None,
        }
    }

    #[test]
    fn malformed_tool_calls_are_rejected_instead_of_silently_dropped() {
        let malformed_calls = [
            serde_json::json!({ "id": "call-1" }),
            serde_json::json!({ "function": { "name": "tools_help", "arguments": "{}" } }),
            serde_json::json!({ "id": "call-1", "function": { "arguments": "{}" } }),
        ];

        for tool_call in malformed_calls {
            let payload: ChatCompletionResponse = serde_json::from_value(serde_json::json!({
                "model": "test-model",
                "choices": [{
                    "message": { "tool_calls": [tool_call] },
                    "finish_reason": "tool_calls"
                }]
            }))
            .expect("deserialize response");

            let error = parse_completion_response("test", "test-model", payload)
                .expect_err("malformed tool call must fail");
            assert!(error.to_string().contains("returned tool_call without"));
        }
    }

    #[test]
    fn serializes_explicit_parallel_tool_call_policy_without_forcing_a_default() {
        let disabled = serde_json::to_value(request(Some(false))).expect("serialize request");
        assert_eq!(
            disabled.get("parallel_tool_calls"),
            Some(&serde_json::Value::Bool(false))
        );

        let unspecified = serde_json::to_value(request(None)).expect("serialize request");
        assert!(unspecified.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn serializes_the_official_completion_token_field_independently() {
        let mut request = request(None);
        request.max_completion_tokens = Some(4096);

        let value = serde_json::to_value(request).expect("serialize request");
        assert_eq!(value.get("max_completion_tokens"), Some(&4096.into()));
        assert!(value.get("max_tokens").is_none());
    }

    #[test]
    fn chat_completions_wire_shape_never_uses_responses_fields() {
        let value = serde_json::to_value(request(None)).expect("serialize Chat Completions");

        assert!(value.get("messages").is_some());
        assert!(value.get("input").is_none());
        assert!(value.get("instructions").is_none());
        assert!(value.get("text").is_none());
        assert!(value.get("previous_response_id").is_none());
    }

    #[test]
    fn prompt_cache_key_uses_the_openai_wire_name_only() {
        let mut request = request(None);
        request.prompt_cache_key = Some("session-affinity".to_owned());

        let value = serde_json::to_value(request).expect("serialize Chat Completions");
        assert_eq!(
            value.get("prompt_cache_key"),
            Some(&"session-affinity".into())
        );
        assert!(value.get("promptCacheKey").is_none());
    }

    #[test]
    fn disabled_reasoning_uses_none_only_for_supported_gpt5_versions() {
        assert_eq!(
            reasoning_effort(Some(&ThinkingRequest::Disabled), "gpt-5.2"),
            Some("none".to_owned())
        );
        assert_eq!(
            reasoning_effort(Some(&ThinkingRequest::Disabled), "openai/gpt-5.4-codex"),
            Some("none".to_owned())
        );
        assert_eq!(
            reasoning_effort(Some(&ThinkingRequest::Disabled), "gpt-5"),
            None
        );
        assert_eq!(
            reasoning_effort(Some(&ThinkingRequest::Disabled), "o4-mini"),
            None
        );
    }

    #[test]
    fn max_reasoning_uses_each_protocols_strongest_wire_value() {
        let thinking = ThinkingRequest::Effort {
            effort: crate::provider::ReasoningEffort::Max,
        };
        assert_eq!(
            reasoning_effort(Some(&thinking), "gpt-5.4-codex"),
            Some("xhigh".to_owned())
        );
        assert_eq!(
            reasoning_effort(Some(&thinking), "deepseek-v4"),
            Some("max".to_owned())
        );
    }

    #[test]
    fn official_chat_usage_detail_fields_are_normalized() {
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 80,
            "prompt_tokens_details": { "cached_tokens": 25 },
            "completion_tokens_details": { "reasoning_tokens": 30 }
        }))
        .expect("deserialize Chat Completions usage");
        let usage = chat_usage_to_completion(usage);

        assert_eq!(usage.input_tokens, 75);
        assert_eq!(usage.cache_read_tokens, 25);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.reasoning_tokens, 30);
    }

    #[test]
    fn xai_chat_usage_keeps_reasoning_separate_from_visible_completion() {
        // xAI Chat Completions reports completion_tokens as visible output,
        // with reasoning added separately into total_tokens.
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 32,
            "completion_tokens": 9,
            "total_tokens": 135,
            "cost_in_usd_ticks": 37_756_000,
            "prompt_tokens_details": { "cached_tokens": 6 },
            "completion_tokens_details": { "reasoning_tokens": 94 }
        }))
        .expect("deserialize xAI Chat usage");
        let usage = chat_usage_to_completion(usage);

        assert_eq!(usage.input_tokens, 26);
        assert_eq!(usage.cache_read_tokens, 6);
        assert_eq!(usage.output_tokens, 9);
        assert_eq!(usage.reasoning_tokens, 94);
        assert!((usage.total_cost - 0.0037756).abs() < 1e-12);
    }

    #[test]
    fn responses_style_chat_usage_detail_fields_are_normalized() {
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 80,
            "input_tokens_details": { "cached_tokens": 25 },
            "output_tokens_details": { "reasoning_tokens": 30 }
        }))
        .expect("deserialize Responses-style usage fields");
        let usage = chat_usage_to_completion(usage);

        assert_eq!(usage.input_tokens, 75);
        assert_eq!(usage.cache_read_tokens, 25);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.reasoning_tokens, 30);
    }

    #[test]
    fn chat_usage_accepts_both_detail_field_names_in_one_payload() {
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 80,
            "prompt_tokens_details": { "cached_tokens": 25 },
            "input_tokens_details": { "cached_tokens": 20 },
            "completion_tokens_details": { "reasoning_tokens": 30 },
            "output_tokens_details": { "reasoning_tokens": 10 }
        }))
        .expect("deserialize usage containing both naming conventions");
        let usage = chat_usage_to_completion(usage);

        // Chat Completions names are authoritative when both are populated.
        assert_eq!(usage.input_tokens, 75);
        assert_eq!(usage.cache_read_tokens, 25);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.reasoning_tokens, 30);
    }

    #[test]
    fn copilot_chat_usage_keeps_separately_reported_reasoning_tokens() {
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 19_581,
            "completion_tokens": 53,
            "reasoning_tokens": 134,
            "total_tokens": 19_768,
            "prompt_tokens_details": { "cached_tokens": 17_068 }
        }))
        .expect("deserialize Copilot usage");
        let usage = chat_usage_to_completion(usage);

        assert_eq!(usage.input_tokens, 2_513);
        assert_eq!(usage.cache_read_tokens, 17_068);
        assert_eq!(usage.output_tokens, 53);
        assert_eq!(usage.reasoning_tokens, 134);
    }

    #[test]
    fn total_tokens_can_identify_separate_reasoning_without_a_named_field() {
        let usage: ChatUsage = serde_json::from_value(serde_json::json!({
            "input_tokens": 100,
            "output_tokens": 20,
            "total_tokens": 135
        }))
        .expect("deserialize compatible usage");
        let usage = chat_usage_to_completion(usage);

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.reasoning_tokens, 15);
    }

    #[test]
    fn streamed_reasoning_details_merge_text_and_preserve_provider_state() {
        let mut details = None;
        merge_reasoning_details(
            &mut details,
            &serde_json::json!([{
                "type": "reasoning.text",
                "index": 0,
                "text": "think",
                "format": "unknown"
            }]),
        );
        merge_reasoning_details(
            &mut details,
            &serde_json::json!([{
                "type": "reasoning.text",
                "index": 0,
                "text": "ing",
                "signature": "opaque-signature"
            }]),
        );

        let details = details.expect("merged reasoning details");
        assert_eq!(details[0]["text"], "thinking");
        assert_eq!(details[0]["format"], "unknown");
        assert_eq!(details[0]["signature"], "opaque-signature");
    }

    #[test]
    fn reasoning_details_preserve_summary_encrypted_and_non_adjacent_order() {
        let mut details = None;
        merge_reasoning_details(
            &mut details,
            &serde_json::json!([
                { "type": "reasoning.text", "index": 0, "text": "first" },
                { "type": "reasoning.summary", "index": 0, "summary": "summary one" }
            ]),
        );
        merge_reasoning_details(
            &mut details,
            &serde_json::json!([
                { "type": "reasoning.encrypted", "index": 0, "data": "cipher one" },
                { "type": "reasoning.text", "index": 0, "text": "second" }
            ]),
        );
        merge_reasoning_details(
            &mut details,
            &serde_json::json!([
                { "type": "reasoning.summary", "index": 0, "summary": "summary two" },
                { "type": "reasoning.encrypted", "index": 0, "data": "cipher two" }
            ]),
        );

        assert_eq!(
            details.expect("ordered reasoning details"),
            serde_json::json!([
                { "type": "reasoning.text", "index": 0, "text": "first" },
                { "type": "reasoning.summary", "index": 0, "summary": "summary one" },
                { "type": "reasoning.encrypted", "index": 0, "data": "cipher one" },
                { "type": "reasoning.text", "index": 0, "text": "second" },
                { "type": "reasoning.summary", "index": 0, "summary": "summary two" },
                { "type": "reasoning.encrypted", "index": 0, "data": "cipher two" }
            ])
        );
    }

    #[test]
    fn reasoning_summary_is_emitted_as_thinking_text() {
        let details = serde_json::json!([
            { "type": "reasoning.summary", "index": 0, "summary": "short summary" },
            { "type": "reasoning.encrypted", "index": 0, "data": "opaque" }
        ]);

        assert_eq!(
            super::extract_reasoning_text_from_fields(None, Some(&details), None).as_deref(),
            Some("short summary")
        );
    }

    #[test]
    fn assistant_replay_uses_exact_reasoning_details_and_copilot_opaque_state() {
        let raw_details = serde_json::json!([{
            "type": "reasoning.text",
            "index": 0,
            "text": "thinking",
            "signature": "provider-signature"
        }]);
        let mut source = Message::prompt_parts(
            Role::Assistant,
            vec![
                PartContent::reasoning_summary("thinking"),
                PartContent::text("answer"),
            ],
        );
        source.provider_state = Some(MessageProviderState {
            openai_chat_reasoning_details: Some(raw_details.clone()),
            copilot_reasoning_opaque: Some("opaque-state".to_owned()),
            ..MessageProviderState::default()
        });
        let mut target = ChatMessage::assistant(Some("answer".into()), None);

        apply_raw_assistant_reasoning_state(&source, &mut target, "thinking");

        assert_eq!(target.reasoning_details, Some(raw_details));
        assert_eq!(target.reasoning_text.as_deref(), Some("thinking"));
        assert_eq!(target.reasoning_opaque.as_deref(), Some("opaque-state"));
    }
}

pub(crate) fn extract_reasoning_text_from_delta_or_message(
    value: &ChatDeltaOrMessage,
) -> Option<String> {
    extract_reasoning_text_from_fields(
        value.reasoning_content.as_ref(),
        value.reasoning_details.as_ref(),
        value.reasoning_text.as_ref(),
    )
}

pub(crate) fn chat_usage_to_completion(usage: ChatUsage) -> CompletionUsage {
    let prompt_tokens = usage
        .prompt_tokens
        .or(usage.input_tokens)
        .unwrap_or_default();
    let cache_read_tokens = usage
        .prompt_tokens_details
        .and_then(|d| d.cached_tokens)
        .or_else(|| {
            usage
                .input_tokens_details
                .and_then(|details| details.cached_tokens)
        })
        .unwrap_or_default();
    // OpenAI's `prompt_tokens` is inclusive of cached tokens; the rest of
    // the codebase follows Anthropic's convention where `input_tokens`
    // names only the uncached portion. Subtract to match.
    let input_tokens = prompt_tokens.saturating_sub(cache_read_tokens);
    let detailed_reasoning_tokens = usage
        .completion_tokens_details
        .and_then(|d| d.reasoning_tokens)
        .or_else(|| {
            usage
                .output_tokens_details
                .and_then(|details| details.reasoning_tokens)
        });
    let raw_output_tokens = usage
        .completion_tokens
        .or(usage.output_tokens)
        .unwrap_or_default();
    let inferred_separate_reasoning = usage.total_tokens.and_then(|total| {
        total
            .checked_sub(prompt_tokens.saturating_add(raw_output_tokens))
            .filter(|tokens| *tokens > 0)
    });
    let reasoning_tokens = usage
        .reasoning_tokens
        .or(detailed_reasoning_tokens)
        .or(inferred_separate_reasoning)
        .unwrap_or_default();
    let total_without_separate_reasoning = prompt_tokens.saturating_add(raw_output_tokens);
    let total_with_separate_reasoning =
        total_without_separate_reasoning.saturating_add(reasoning_tokens);
    let output_includes_reasoning = match usage.total_tokens {
        Some(total)
            if reasoning_tokens > 0
                && total == total_with_separate_reasoning
                && total != total_without_separate_reasoning =>
        {
            false
        }
        Some(total) if total == total_without_separate_reasoning => reasoning_tokens > 0,
        // OpenAI includes nested reasoning tokens in completion_tokens. A
        // top-level reasoning_tokens field is used by Copilot/xAI-compatible
        // variants that commonly report it separately.
        _ => usage.reasoning_tokens.is_none() && detailed_reasoning_tokens.is_some(),
    };
    let output_tokens = if output_includes_reasoning {
        raw_output_tokens.saturating_sub(reasoning_tokens)
    } else {
        raw_output_tokens
    };
    MessageUsage {
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_write_tokens: 0,
        cache_read_tokens,
        total_cost: usage
            .cost_in_usd_ticks
            .map(|ticks| ticks as f64 / 10_000_000_000.0)
            .unwrap_or_default(),
    }
    .into()
}

pub(crate) fn parse_required_chat_tool_calls(
    provider_id: &str,
    calls: Option<&Vec<ChatToolCallWire>>,
) -> Result<Vec<CompletionToolCall>, AppError> {
    calls
        .into_iter()
        .flatten()
        .map(|call| {
            let id = utils::normalize_optional_text(call.id.clone()).ok_or_else(|| {
                AppError::Provider(format!(
                    "{provider_id} returned tool_call without id in completion response"
                ))
            })?;

            let function = call.function.as_ref().ok_or_else(|| {
                AppError::Provider(format!(
                    "{provider_id} returned tool_call without function payload"
                ))
            })?;

            let name = utils::optional_non_empty(function.name.clone()).ok_or_else(|| {
                AppError::Provider(format!(
                    "{provider_id} returned tool_call without function.name"
                ))
            })?;

            Ok(CompletionToolCall::Function {
                id,
                name,
                arguments_json: function.arguments.clone().unwrap_or_default(),
            })
        })
        .collect()
}

fn parse_completion_response_with_tool_parser(
    provider_id: &str,
    default_model: &str,
    payload: ChatCompletionResponse,
    parse_tool_calls: impl FnOnce(
        &str,
        Option<&Vec<ChatToolCallWire>>,
    ) -> Result<Vec<CompletionToolCall>, AppError>,
) -> Result<CompletionResponse, AppError> {
    if let Some(error) = payload.error.as_ref() {
        let envelope = serde_json::json!({ "error": error });
        return Err(
            utils::chat_stream_error(provider_id, &envelope).unwrap_or_else(|| {
                AppError::Provider(format!(
                    "{provider_id} returned an empty chat error envelope"
                ))
            }),
        );
    }
    let response_message = payload
        .choices
        .first()
        .and_then(|c| c.message.as_ref())
        .or_else(|| payload.choices.first().and_then(|c| c.delta.as_ref()));
    let reasoning_text = response_message.and_then(extract_reasoning_text_from_delta_or_message);
    let text = response_message
        .and_then(|m| m.content.as_ref())
        .map(extract_text_from_content)
        .or_else(|| payload.choices.first().and_then(|c| c.text.clone()))
        .unwrap_or_default();

    let finish_reason = CompletionFinishReason::from_provider(
        payload
            .choices
            .first()
            .and_then(|c| c.finish_reason.as_deref()),
    );

    let tool_calls = parse_tool_calls(
        provider_id,
        response_message.and_then(|m| m.tool_calls.as_ref()),
    )?;

    if text.is_empty()
        && reasoning_text.is_none()
        && tool_calls.is_empty()
        && finish_reason.is_none()
    {
        return Err(AppError::Provider(format!(
            "{provider_id} returned empty completion payload without finish reason"
        )));
    }

    let usage = payload.usage.map(chat_usage_to_completion);
    let response_id = payload.id;
    let provider_metadata = utils::provider_metadata_with_chat_reasoning_state(
        utils::response_id_metadata(response_id),
        response_message.and_then(assistant_reasoning_field_from_delta_or_message),
        response_message.and_then(|message| message.reasoning_details.clone()),
        response_message.and_then(|message| message.reasoning_opaque.clone()),
    );

    Ok(CompletionResponse {
        provider_id: ProviderId::new(provider_id),
        model: ModelId::new(payload.model.unwrap_or_else(|| default_model.to_owned())),
        text,
        reasoning_text,
        finish_reason,
        tool_calls,
        usage,
        provider_metadata,
    })
}

pub(crate) fn parse_completion_response(
    provider_id: &str,
    default_model: &str,
    payload: ChatCompletionResponse,
) -> Result<CompletionResponse, AppError> {
    parse_completion_response_with_tool_parser(
        provider_id,
        default_model,
        payload,
        parse_required_chat_tool_calls,
    )
}

// ─── Message conversion ───────────────────────────────────────────────────────

/// Convert a `CompletionRequest` into the flat `Vec<ChatMessage>` wire format
/// used by Chat Completions endpoints.
pub(crate) fn request_to_chat_messages_with_assistant_reasoning_field(
    request: &CompletionRequest,
    assistant_reasoning_field: Option<&str>,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
        messages.push(ChatMessage::system(system.clone()));
    }

    for message in &request.messages {
        let parts = wire_message::project(message);
        match message.role {
            Role::System => messages.push(ChatMessage::system(session_text_lossy(
                message,
                parts.as_slice(),
            ))),
            Role::User => messages.push(ChatMessage::user(message_content_value(
                message,
                parts.as_slice(),
            ))),
            Role::Assistant => {
                messages.extend(assistant_messages_from_parts(
                    message,
                    &parts,
                    assistant_reasoning_field,
                ));
            }
            Role::Tool => messages.extend(tool_messages_from_parts(&parts)),
        }
    }

    messages
}

pub(crate) fn backfill_assistant_reasoning_field_on_request(
    request: &mut CompletionRequest,
    assistant_reasoning_field: Option<&str>,
    assistant_reasoning_interleaved: bool,
) {
    let Some(field) = assistant_reasoning_field else {
        return;
    };

    for message in &mut request.messages {
        if !matches!(message.role, Role::Assistant) {
            continue;
        }
        if assistant_reasoning_field_from_message_metadata(message).is_some() {
            continue;
        }
        if !assistant_reasoning_interleaved && assistant_reasoning_text(message).trim().is_empty() {
            continue;
        }
        let assistant_reasoning_field = match field {
            "reasoning_content" => AssistantReasoningField::ReasoningContent,
            "reasoning_details" => AssistantReasoningField::ReasoningDetails,
            _ => continue,
        };
        let mut provider_state = message.provider_state.take().unwrap_or_default();
        provider_state.assistant_reasoning_field = Some(assistant_reasoning_field);
        message.provider_state = Some(provider_state);
    }
}

fn session_text_lossy(message: &Message, parts: &[wire_message::WirePart]) -> String {
    if parts.is_empty() {
        message.as_text_lossy()
    } else {
        wire_message::parts_text_lossy(parts)
    }
}

fn message_content_value(message: &Message, parts: &[wire_message::WirePart]) -> Value {
    if parts.is_empty() {
        return Value::String(message.as_text_lossy());
    }
    wire_message::parts_to_openai_content_array(parts)
}

fn assistant_messages_from_parts(
    message: &Message,
    parts: &[wire_message::WirePart],
    assistant_reasoning_field: Option<&str>,
) -> Vec<ChatMessage> {
    let assistant_reasoning_field =
        assistant_reasoning_field_from_message_metadata(message).or(assistant_reasoning_field);
    let assistant_reasoning_text = assistant_reasoning_text(message);
    let has_tool_result = parts
        .iter()
        .any(|part| matches!(part, wire_message::WirePart::ToolResult { .. }));
    if !has_tool_result {
        let (content, tool_calls) = assistant_content_and_tool_calls(message, parts);
        let mut chat_message =
            ChatMessage::assistant(content, (!tool_calls.is_empty()).then_some(tool_calls));
        apply_assistant_reasoning_field(
            &mut chat_message,
            assistant_reasoning_field,
            assistant_reasoning_text.as_str(),
        );
        apply_raw_assistant_reasoning_state(message, &mut chat_message, &assistant_reasoning_text);
        return vec![chat_message];
    }

    let mut messages = Vec::new();
    let mut buffered = Vec::new();
    for part in parts {
        match part {
            wire_message::WirePart::ToolResult {
                tool_call_id,
                output_json,
                ..
            } if !tool_call_id.trim().is_empty() => {
                if !buffered.is_empty() {
                    let (content, tool_calls) =
                        assistant_content_and_tool_calls(message, &buffered);
                    let mut chat_message = ChatMessage::assistant(
                        content,
                        (!tool_calls.is_empty()).then_some(tool_calls),
                    );
                    apply_assistant_reasoning_field(
                        &mut chat_message,
                        assistant_reasoning_field,
                        assistant_reasoning_text.as_str(),
                    );
                    apply_raw_assistant_reasoning_state(
                        message,
                        &mut chat_message,
                        &assistant_reasoning_text,
                    );
                    messages.push(chat_message);
                    buffered.clear();
                }
                messages.push(ChatMessage::tool_result(
                    tool_call_id.clone(),
                    Value::String(output_json.clone()),
                ));
            }
            wire_message::WirePart::ToolResult { output_json, .. } => {
                buffered.push(wire_message::WirePart::Text {
                    text: output_json.clone(),
                });
            }
            other => buffered.push(other.clone()),
        }
    }

    if !buffered.is_empty() {
        let (content, tool_calls) = assistant_content_and_tool_calls(message, &buffered);
        let mut chat_message =
            ChatMessage::assistant(content, (!tool_calls.is_empty()).then_some(tool_calls));
        apply_assistant_reasoning_field(
            &mut chat_message,
            assistant_reasoning_field,
            assistant_reasoning_text.as_str(),
        );
        apply_raw_assistant_reasoning_state(message, &mut chat_message, &assistant_reasoning_text);
        messages.push(chat_message);
    }

    messages
}

fn tool_messages_from_parts(parts: &[wire_message::WirePart]) -> Vec<ChatMessage> {
    parts
        .iter()
        .filter_map(|part| match part {
            wire_message::WirePart::ToolResult {
                tool_call_id,
                output_json,
                ..
            } if !tool_call_id.trim().is_empty() => Some(ChatMessage::tool_result(
                tool_call_id.clone(),
                Value::String(output_json.clone()),
            )),
            _ => None,
        })
        .collect()
}

fn assistant_reasoning_text(message: &Message) -> String {
    let mut chunks = Vec::new();
    for part in &message.parts {
        let Some(content) = part.content.as_ref() else {
            continue;
        };
        if let crate::message::PartContent::Reasoning(reasoning) = content {
            let text = reasoning.preferred_text();
            if !text.is_empty() {
                chunks.push(text);
            }
        }
    }
    chunks.join("")
}

fn assistant_reasoning_field_from_message_metadata(message: &Message) -> Option<&'static str> {
    match message
        .provider_state
        .as_ref()
        .and_then(|state| state.assistant_reasoning_field)
    {
        Some(AssistantReasoningField::ReasoningContent) => Some("reasoning_content"),
        Some(AssistantReasoningField::ReasoningDetails) => Some("reasoning_details"),
        None => None,
    }
}

fn apply_assistant_reasoning_field(
    message: &mut ChatMessage,
    field: Option<&str>,
    reasoning_text: &str,
) {
    match field {
        Some("reasoning_content") => {
            message.reasoning_content = Some(Value::String(reasoning_text.to_owned()));
        }
        Some("reasoning_details") => {
            let details = if reasoning_text.trim().is_empty() {
                Vec::new()
            } else {
                vec![serde_json::json!({
                    "type": "reasoning.text",
                    "text": reasoning_text,
                    "index": 0
                })]
            };
            message.reasoning_details = Some(Value::Array(details));
        }
        _ => {}
    }
}

fn apply_raw_assistant_reasoning_state(
    source: &Message,
    target: &mut ChatMessage,
    reasoning_text: &str,
) {
    let Some(state) = source.provider_state.as_ref() else {
        return;
    };
    if let Some(details) = state.openai_chat_reasoning_details.as_ref() {
        target.reasoning_details = Some(details.clone());
    }
    if let Some(opaque) = state
        .copilot_reasoning_opaque
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        target.reasoning_text = (!reasoning_text.is_empty()).then(|| reasoning_text.to_owned());
        target.reasoning_opaque = Some(opaque.to_owned());
    }
}

fn assistant_content_and_tool_calls(
    message: &Message,
    parts: &[wire_message::WirePart],
) -> (Option<Value>, Vec<ChatToolCallRequest>) {
    if parts.is_empty() {
        return (Some(Value::String(message.as_text_lossy())), Vec::new());
    }

    let mut text_chunks = Vec::new();
    let mut tool_calls = Vec::new();
    for part in parts {
        match part {
            wire_message::WirePart::Text { text } => text_chunks.push(text.clone()),
            wire_message::WirePart::ToolCall {
                id,
                function,
                arguments_json,
            } => {
                tool_calls.push(ChatToolCallRequest {
                    kind: "function".to_owned(),
                    id: id.clone(),
                    function: ChatFunctionCallRequest {
                        name: function.function_name().to_owned(),
                        arguments: arguments_json.clone(),
                    },
                });
            }
            wire_message::WirePart::Attachment { item } => {
                text_chunks.push(wire_message::hint_text(item));
            }
            wire_message::WirePart::ToolResult { tool_call_id, .. } => {
                text_chunks.push(format!("[tool_result:{tool_call_id}]"));
            }
        }
    }
    let content = (!text_chunks.is_empty()).then(|| Value::String(text_chunks.join("")));
    (content, tool_calls)
}
