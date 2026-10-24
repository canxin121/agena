use agena_domain::*;
use agena_domain::{AssistantReasoningField, Role};
use agena_provider::{
    CompletionFinishReason, CompletionInputPart, CompletionInputRun, CompletionToolCall,
};
/// Shared wire types and message-conversion helpers for OpenAI-compatible
/// Chat Completions API endpoints.
///
/// The explicit OpenAI Chat Completions adapter and compatible Chat
/// Completions backends share these structs. Responses uses its
/// own wire types and never serialize this schema.
use serde_json::Value;

use crate::{
    ProviderError,
    provider::{CompletionResponse, utils, wire_message},
};
use agena_provider::CompletionRequest;

// ─── Request body ────────────────────────────────────────────────────────────

pub use agena_provider::ChatCompletionRequest;

pub use agena_provider::ChatStreamOptions;

// ─── Message types ────────────────────────────────────────────────────────────

pub use agena_provider::ChatMessage;

pub use agena_provider::{ChatFunctionCallRequest, ChatToolCallRequest};

// ─── Tool definition ──────────────────────────────────────────────────────────

pub use agena_provider::{ChatFunctionDefinition, ChatToolDefinition};

// ─── Response format ─────────────────────────────────────────────────────────

pub use agena_provider::openai_chat_response_format as map_response_format;

// ─── Reasoning effort mapping ─────────────────────────────────────────────────

/// Convert a `ThinkingRequest` to an OpenAI `reasoning_effort` string for
/// OpenAI reasoning models. Returns `None` for non-reasoning models or when thinking
/// is disabled / absent.
pub use agena_provider::openai_chat_reasoning_effort as reasoning_effort;

// ─── Response / decode types ──────────────────────────────────────────────────

pub use agena_provider::ChatFunctionCallWire;
pub use agena_provider::{ChatCompletionResponse, ChatDeltaOrMessage, ChatToolCallWire};

pub use agena_provider::{ChatUsage, chat_usage_to_completion};

// ─── Utility helpers ──────────────────────────────────────────────────────────

pub use agena_provider::openai_chat_extract_text as extract_text_from_content;

pub use agena_provider::openai_chat_extract_reasoning_text as extract_reasoning_text_from_fields;

pub fn extract_reasoning_text_from_delta_or_message(value: &ChatDeltaOrMessage) -> Option<String> {
    extract_reasoning_text_from_fields(
        value.reasoning_content.as_ref(),
        value.reasoning_details.as_ref(),
        value.reasoning_text.as_ref(),
    )
}

pub fn parse_required_chat_tool_calls(
    provider_id: &str,
    calls: Option<&Vec<ChatToolCallWire>>,
) -> Result<Vec<CompletionToolCall>, ProviderError> {
    calls
        .into_iter()
        .flatten()
        .map(|call| {
            let id = utils::normalize_optional_text(call.id.clone()).ok_or_else(|| {
                ProviderError::Provider(format!(
                    "{provider_id} returned tool_call without id in completion response"
                ))
            })?;

            let function = call.function.as_ref().ok_or_else(|| {
                ProviderError::Provider(format!(
                    "{provider_id} returned tool_call without function payload"
                ))
            })?;

            let name = utils::optional_non_empty(function.name.clone()).ok_or_else(|| {
                ProviderError::Provider(format!(
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
    ) -> Result<Vec<CompletionToolCall>, ProviderError>,
) -> Result<CompletionResponse, ProviderError> {
    if let Some(error) = payload.error.as_ref() {
        let envelope = serde_json::json!({ "error": error });
        return Err(
            utils::chat_stream_error(provider_id, &envelope).unwrap_or_else(|| {
                ProviderError::Provider(format!(
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
    let finish_reason =
        CompletionFinishReason::normalize_with_tool_calls(finish_reason, !tool_calls.is_empty());

    if text.is_empty()
        && reasoning_text.is_none()
        && tool_calls.is_empty()
        && finish_reason.is_none()
    {
        return Err(ProviderError::Provider(format!(
            "{provider_id} returned empty completion payload without finish reason"
        )));
    }

    let usage = payload.usage.map(chat_usage_to_completion);
    let response_id = payload.id;
    let provider_metadata = utils::provider_metadata_with_chat_reasoning_state(
        utils::response_id_metadata(response_id),
        response_message.and_then(agena_provider::openai_chat_reasoning_field_from_delta),
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

pub fn parse_completion_response(
    provider_id: &str,
    default_model: &str,
    payload: ChatCompletionResponse,
) -> Result<CompletionResponse, ProviderError> {
    parse_completion_response_with_tool_parser(
        provider_id,
        default_model,
        payload,
        parse_required_chat_tool_calls,
    )
}

pub fn parse_completion_response_with_required_tool_calls(
    provider_id: &str,
    default_model: &str,
    payload: ChatCompletionResponse,
) -> Result<CompletionResponse, ProviderError> {
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
pub fn request_to_chat_messages_with_assistant_reasoning_field(
    request: &CompletionRequest,
    assistant_reasoning_field: Option<&str>,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    if let Some(system) = request.system.as_ref().filter(|s| !s.trim().is_empty()) {
        messages.push(ChatMessage::system(system.clone()));
    }

    for run in &request.turns {
        let parts = wire_message::project(run);
        match run.role {
            Role::System => messages.push(ChatMessage::system(session_text_lossy(
                run,
                parts.as_slice(),
            ))),
            Role::User => messages.push(ChatMessage::user(message_content_value(
                run,
                parts.as_slice(),
            ))),
            Role::Assistant => {
                messages.extend(assistant_messages_from_parts(
                    run,
                    &parts,
                    assistant_reasoning_field,
                ));
            }
            Role::Tool => messages.extend(tool_messages_from_parts(&parts)),
        }
    }

    append_tool_result_media(request, messages)
}

fn append_tool_result_media(
    request: &CompletionRequest,
    messages: Vec<ChatMessage>,
) -> Vec<ChatMessage> {
    // Chat only allows text in tool messages. Keep all parallel tool replies
    // together, then supply ordinary user media before the next assistant
    // reply, including when calls/results/replies share one persisted run.
    let mut results =
        std::collections::HashMap::<String, std::collections::VecDeque<Vec<AttachmentItem>>>::new();
    for part in request
        .turns
        .iter()
        .filter(|run| matches!(run.role, Role::Assistant | Role::Tool))
        .flat_map(wire_message::project)
    {
        if let wire_message::WirePart::ToolResult {
            tool_call_id,
            attachments,
            ..
        } = part
        {
            results
                .entry(tool_call_id)
                .or_default()
                .push_back(attachments);
        }
    }
    let mut output = Vec::with_capacity(messages.len());
    let mut pending_media = Vec::new();
    for message in messages {
        if message.role != "tool" && !pending_media.is_empty() {
            output.push(ChatMessage::user(
                wire_message::parts_to_openai_content_array(&pending_media),
            ));
            pending_media.clear();
        }
        if message.role == "tool"
            && let Some(id) = &message.tool_call_id
            && let Some(attachments) = results.get_mut(id).and_then(|items| items.pop_front())
            && !attachments.is_empty()
        {
            pending_media.push(wire_message::WirePart::Text {
                text: format!("Media read by tool call {id}; analyze the following file contents."),
            });
            pending_media.extend(
                attachments
                    .into_iter()
                    .map(|item| wire_message::WirePart::Attachment { item }),
            );
        }
        output.push(message);
    }
    if !pending_media.is_empty() {
        output.push(ChatMessage::user(
            wire_message::parts_to_openai_content_array(&pending_media),
        ));
    }
    output
}

pub fn backfill_assistant_reasoning_field_on_request(
    request: &mut CompletionRequest,
    assistant_reasoning_field: Option<&str>,
    assistant_reasoning_interleaved: bool,
) {
    let Some(field) = assistant_reasoning_field else {
        return;
    };

    for run in &mut request.turns {
        if !matches!(run.role, Role::Assistant) {
            continue;
        }
        if assistant_reasoning_field_from_message_metadata(run).is_some() {
            continue;
        }
        if !assistant_reasoning_interleaved && assistant_reasoning_text(run).trim().is_empty() {
            continue;
        }
        let assistant_reasoning_field = match field {
            "reasoning_content" => AssistantReasoningField::ReasoningContent,
            "reasoning_details" => AssistantReasoningField::ReasoningDetails,
            _ => continue,
        };
        run.provider_state.assistant_reasoning_field = Some(assistant_reasoning_field);
    }
}

fn session_text_lossy(run: &CompletionInputRun, parts: &[wire_message::WirePart]) -> String {
    if parts.is_empty() {
        run.as_text_lossy()
    } else {
        wire_message::parts_text_lossy(parts)
    }
}

fn message_content_value(run: &CompletionInputRun, parts: &[wire_message::WirePart]) -> Value {
    if parts.is_empty() {
        return Value::String(run.as_text_lossy());
    }
    wire_message::parts_to_openai_content_array(parts)
}

fn assistant_messages_from_parts(
    run: &CompletionInputRun,
    parts: &[wire_message::WirePart],
    assistant_reasoning_field: Option<&str>,
) -> Vec<ChatMessage> {
    let assistant_reasoning_field =
        assistant_reasoning_field_from_message_metadata(run).or(assistant_reasoning_field);
    let assistant_reasoning_text = assistant_reasoning_text(run);

    // Accumulated assistant content, flushed as an assistant ChatMessage each
    // time the stream is interrupted by a tool result or a system notice.
    let mut messages = Vec::new();
    let mut buffered = Vec::new();
    let flush = |messages: &mut Vec<ChatMessage>, buffered: &mut Vec<wire_message::WirePart>| {
        if buffered.is_empty() {
            return;
        }
        let (content, tool_calls) = assistant_content_and_tool_calls(run, buffered);
        let mut chat_message =
            ChatMessage::assistant(content, (!tool_calls.is_empty()).then_some(tool_calls));
        apply_assistant_reasoning_field(
            &mut chat_message,
            assistant_reasoning_field,
            assistant_reasoning_text.as_str(),
        );
        apply_raw_assistant_reasoning_state(run, &mut chat_message, &assistant_reasoning_text);
        messages.push(chat_message);
        buffered.clear();
    };

    for part in parts {
        match part {
            // A system notice (background-operation notification) is a genuine
            // mid-conversation `system` message, never the assistant's reply.
            // Split the stream so the notice lands as its own system message,
            // keeping the surrounding assistant content in role.
            wire_message::WirePart::SystemMessage { text } => {
                flush(&mut messages, &mut buffered);
                if !text.trim().is_empty() {
                    messages.push(ChatMessage::system(text.clone()));
                }
            }
            wire_message::WirePart::ToolResult {
                tool_call_id,
                output_json,
                ..
            } if !tool_call_id.trim().is_empty() => {
                flush(&mut messages, &mut buffered);
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
    flush(&mut messages, &mut buffered);

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

fn assistant_reasoning_text(run: &CompletionInputRun) -> String {
    run.parts
        .iter()
        .filter_map(|part| match part {
            CompletionInputPart::Reasoning { text } if !text.is_empty() => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn assistant_reasoning_field_from_message_metadata(
    run: &CompletionInputRun,
) -> Option<&'static str> {
    match run.provider_state.assistant_reasoning_field {
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
            // Never send an empty `reasoning_content` to a provider that
            // requires prior reasoning be passed back: `""` can be rejected
            // just like a missing field, and omitting it lets providers that
            // tolerate a missing field proceed.
            if !reasoning_text.trim().is_empty() {
                message.reasoning_content = Some(Value::String(reasoning_text.to_owned()));
            }
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
    source: &CompletionInputRun,
    target: &mut ChatMessage,
    reasoning_text: &str,
) {
    let state = &source.provider_state;
    if let Some(details) = state.openai_chat_reasoning_details.as_ref() {
        target.reasoning_details = Some(details.clone());
    }
    if let Some(opaque) = state
        .copilot_reasoning_opaque
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        target.reasoning_text = (!reasoning_text.is_empty()).then(|| reasoning_text.to_owned());
        target.reasoning_opaque = Some(opaque.to_owned());
    }
}

fn assistant_content_and_tool_calls(
    run: &CompletionInputRun,
    parts: &[wire_message::WirePart],
) -> (Option<Value>, Vec<ChatToolCallRequest>) {
    if parts.is_empty() {
        return (Some(Value::String(run.as_text_lossy())), Vec::new());
    }

    let mut text_chunks = Vec::new();
    let mut tool_calls = Vec::new();
    for part in parts {
        match part {
            wire_message::WirePart::Text { text } => text_chunks.push(text.clone()),
            wire_message::WirePart::Reasoning { .. } => {
                // Reasoning is replayed through the dedicated reasoning field
                // (see `assistant_reasoning_text`), never as visible content.
            }
            // Defensive fallback: `assistant_content_and_tool_calls` only ever
            // sees assistant-content parts (SystemMessage is intercepted and
            // split out in `assistant_messages_from_parts`), but keep the match
            // total so a notice can never be silently dropped.
            wire_message::WirePart::SystemMessage { text } => text_chunks.push(text.clone()),
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
