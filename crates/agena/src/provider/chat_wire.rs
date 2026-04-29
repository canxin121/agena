/// Shared wire types and message-conversion helpers for OpenAI-compatible
/// Chat Completions API endpoints.
///
/// Both `openai.rs` (Chat API path) and `openai_compatible.rs` use the same
/// JSON wire format; the structs defined here are reused by both rather than
/// duplicating them across files.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::AppError,
    message::{Message, MessageUsage},
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
    /// Prompt-cache control field used by some OpenAI-compatible providers
    /// (e.g. OpenRouter, ZenMux). Not sent when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<crate::provider::prompt_cache::PromptCacheControl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(rename = "promptCacheKey", skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key_camel_case: Option<String>,
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
    pub tool_calls: Option<Vec<ChatToolCallRequest>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: String) -> Self {
        Self {
            role: "system".to_owned(),
            kind: None,
            content: Some(Value::String(content)),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: Value) -> Self {
        Self {
            role: "user".to_owned(),
            kind: None,
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: Option<Value>, tool_calls: Option<Vec<ChatToolCallRequest>>) -> Self {
        Self {
            role: "assistant".to_owned(),
            kind: None,
            content,
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: String, content: Value) -> Self {
        Self {
            role: "tool".to_owned(),
            kind: None,
            content: Some(content),
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
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

pub(crate) fn tools_to_chat_definitions(
    tools: &[crate::tool::ToolDefinition],
) -> Vec<ChatToolDefinition> {
    tools
        .iter()
        .map(|tool| ChatToolDefinition {
            kind: "function".to_owned(),
            function: ChatFunctionDefinition {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
                strict: tool.strict,
            },
        })
        .collect()
}

// ─── Response format ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ChatResponseFormat {
    Text,
    JsonObject,
    #[serde(rename = "json_schema")]
    JsonSchema { json_schema: ChatJsonSchemaSpec },
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
        ResponseFormat::JsonSchema { name, schema, strict } => {
            Some(ChatResponseFormat::JsonSchema {
                json_schema: ChatJsonSchemaSpec {
                    name: name.clone(),
                    schema: schema.clone(),
                    strict: *strict,
                },
            })
        }
    }
}

// ─── Reasoning effort mapping ─────────────────────────────────────────────────

/// Convert a `ThinkingRequest` to an OpenAI `reasoning_effort` string for
/// o-series models.  Returns `None` for non-o-series models or when thinking
/// is disabled / absent.
pub(crate) fn reasoning_effort(thinking: Option<&ThinkingRequest>, model: &str) -> Option<String> {
    let is_o_series = model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4");
    if !is_o_series {
        return None;
    }
    match thinking {
        Some(ThinkingRequest::Enabled { budget_tokens }) => {
            let effort = if *budget_tokens > 10_000 {
                "high"
            } else if *budget_tokens > 3_000 {
                "medium"
            } else {
                "low"
            };
            Some(effort.to_owned())
        }
        _ => None,
    }
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
    #[serde(default, alias = "input_tokens")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, alias = "output_tokens")]
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens_details: Option<ChatOutputTokensDetails>,
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

// ─── Stream accumulator ───────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub(crate) struct ChatToolCallStreamState {
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: String,
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

pub(crate) fn chat_usage_to_completion(usage: ChatUsage) -> CompletionUsage {
    MessageUsage {
        input_tokens: usage.prompt_tokens.unwrap_or_default(),
        output_tokens: usage.completion_tokens.unwrap_or_default(),
        reasoning_tokens: usage
            .output_tokens_details
            .and_then(|d| d.reasoning_tokens)
            .unwrap_or_default(),
        cache_write_tokens: 0,
        cache_read_tokens: usage
            .input_tokens_details
            .and_then(|d| d.cached_tokens)
            .unwrap_or_default(),
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

pub(crate) fn parse_completion_response(
    provider_id: &str,
    default_model: &str,
    payload: ChatCompletionResponse,
) -> Result<CompletionResponse, AppError> {
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

    let tool_calls = parse_chat_tool_calls(
        provider_id,
        payload
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.tool_calls.as_ref()),
    )?;

    if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
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
        reasoning_text: None,
        finish_reason,
        tool_calls,
        usage,
        provider_metadata: utils::response_id_metadata(response_id),
    })
}

// ─── Message conversion ───────────────────────────────────────────────────────

/// Convert a `CompletionRequest` into the flat `Vec<ChatMessage>` wire format
/// used by Chat Completions endpoints.
pub(crate) fn request_to_chat_messages(request: &CompletionRequest) -> Vec<ChatMessage> {
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
                let (content, tool_calls) = assistant_content_and_tool_calls(message, &parts);
                messages.push(ChatMessage::assistant(
                    content,
                    (!tool_calls.is_empty()).then_some(tool_calls),
                ));
            }
            Role::Tool => {
                let ordered = ordered_tool_and_user_messages(&parts);
                if ordered.is_empty() {
                    // Fallback: no recognised tool-result parts — treat as user text.
                    messages.push(ChatMessage::user(Value::String(session_text_lossy(
                        message,
                        parts.as_slice(),
                    ))));
                } else {
                    messages.extend(ordered);
                }
            }
        }
    }

    messages
}

fn session_text_lossy(message: &Message, parts: &[wire_message::WirePart]) -> String {
    if parts.is_empty() {
        message.as_text_lossy()
    } else {
        wire_message::parts_text_lossy(parts)
    }
}

fn message_content_value(
    message: &Message,
    parts: &[wire_message::WirePart],
) -> Value {
    if parts.is_empty() {
        return Value::String(message.as_text_lossy());
    }
    wire_message::parts_to_openai_content_array(parts)
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

/// Expand a `Role::Tool` message's parts into individual `ChatMessage`s,
/// interleaving tool-result messages with any follow-up user content.
///
/// Returns an empty `Vec` when no tool-result parts with a valid ID are found
/// (the caller falls back to sending the message as a plain user message).
fn ordered_tool_and_user_messages(
    parts: &[wire_message::WirePart],
) -> Vec<ChatMessage> {
    let has_identified_result = parts.iter().any(|part| {
        matches!(
            part,
            wire_message::WirePart::ToolResult { tool_call_id, .. }
                if !tool_call_id.trim().is_empty()
        )
    });
    if !has_identified_result {
        return Vec::new();
    }

    let mut messages = Vec::new();
    let mut buffered: Vec<wire_message::WirePart> = Vec::new();

    for part in parts {
        match part {
            wire_message::WirePart::ToolResult {
                tool_call_id,
                output_json,
            } if !tool_call_id.trim().is_empty() => {
                if !buffered.is_empty() {
                    messages.push(ChatMessage::user(wire_message::parts_to_openai_content_array(
                        buffered.as_slice(),
                    )));
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
        messages.push(ChatMessage::user(wire_message::parts_to_openai_content_array(
            buffered.as_slice(),
        )));
    }

    messages
}
