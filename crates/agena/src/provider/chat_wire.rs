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
    #[serde(rename = "promptCacheKey", skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key_camel_case: Option<String>,
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
    pub completion_tokens: Option<u64>,
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
) -> Option<String> {
    reasoning_content
        .or(reasoning_details)
        .map(extract_text_from_content)
        .and_then(|text| (!text.trim().is_empty()).then_some(text))
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
        ChatCompletionRequest, ChatUsage, ThinkingRequest, chat_usage_to_completion,
        reasoning_effort,
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
            prompt_cache_key_camel_case: None,
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
}

pub(crate) fn extract_reasoning_text_from_delta_or_message(
    value: &ChatDeltaOrMessage,
) -> Option<String> {
    extract_reasoning_text_from_fields(
        value.reasoning_content.as_ref(),
        value.reasoning_details.as_ref(),
    )
}

pub(crate) fn chat_usage_to_completion(usage: ChatUsage) -> CompletionUsage {
    let prompt_tokens = usage.prompt_tokens.unwrap_or_default();
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
    let reasoning_tokens = usage
        .completion_tokens_details
        .and_then(|d| d.reasoning_tokens)
        .or_else(|| {
            usage
                .output_tokens_details
                .and_then(|details| details.reasoning_tokens)
        })
        .unwrap_or_default();
    let output_tokens = usage
        .completion_tokens
        .unwrap_or_default()
        .saturating_sub(reasoning_tokens);
    MessageUsage {
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_write_tokens: 0,
        cache_read_tokens,
        total_cost: 0.0,
    }
    .into()
}

pub(crate) fn parse_chat_tool_calls(
    _provider_id: &str,
    calls: Option<&Vec<ChatToolCallWire>>,
) -> Result<Vec<CompletionToolCall>, AppError> {
    let Some(calls) = calls else {
        return Ok(Vec::new());
    };

    calls
        .iter()
        .filter_map(|call| {
            let func = call.function.as_ref()?;
            let name = func.name.clone().filter(|n| !n.is_empty())?;
            let id = call
                .id
                .clone()
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| name.clone());
            Some(Ok(CompletionToolCall::Function {
                id,
                name,
                arguments_json: func.arguments.clone().unwrap_or_default(),
            }))
        })
        .collect()
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

            let name = utils::normalize_optional_text(function.name.clone()).ok_or_else(|| {
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
    let reasoning_text = payload
        .choices
        .first()
        .and_then(|c| c.message.as_ref())
        .and_then(extract_reasoning_text_from_delta_or_message)
        .or_else(|| {
            payload
                .choices
                .first()
                .and_then(|c| c.delta.as_ref())
                .and_then(extract_reasoning_text_from_delta_or_message)
        });
    let text = payload
        .choices
        .first()
        .and_then(|c| c.message.as_ref())
        .and_then(|m| m.content.as_ref())
        .map(extract_text_from_content)
        .or_else(|| {
            payload
                .choices
                .first()
                .and_then(|c| c.delta.as_ref())
                .and_then(|d| d.content.as_ref())
                .map(extract_text_from_content)
        })
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
        payload
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.tool_calls.as_ref()),
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

    Ok(CompletionResponse {
        provider_id: ProviderId::new(provider_id),
        model: ModelId::new(payload.model.unwrap_or_else(|| default_model.to_owned())),
        text,
        reasoning_text,
        finish_reason,
        tool_calls,
        usage,
        provider_metadata: utils::response_id_metadata(response_id),
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
        parse_chat_tool_calls,
    )
}

pub(crate) fn parse_completion_response_with_required_tool_calls(
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
                name,
                arguments_json,
            } => {
                tool_calls.push(ChatToolCallRequest {
                    kind: "function".to_owned(),
                    id: id.clone(),
                    function: ChatFunctionCallRequest {
                        name: name.clone(),
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
