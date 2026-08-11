use crate::provider::chat_wire;
use agena_provider::ResponseFormat;

use super::{
    BTreeSet, CompletionStreamEvent, Deserialize, HashMap, ModelId, ProviderError, ProviderId,
    Serialize, ToolStreamInput, ToolStreamInputKind, ToolStreamUpdate, prompt_cache, protocol_ids,
    utils,
};

#[derive(Debug, Serialize)]
pub(super) struct OpenAiResponsesRequest {
    pub(super) model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) instructions: Option<String>,
    pub(super) input: Vec<OpenAiResponsesInputItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) tools: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_choice: Option<String>,
    pub(super) parallel_tool_calls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reasoning: Option<OpenAiResponsesReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) text: Option<OpenAiResponsesTextConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) client_metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct OpenAiResponsesReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiResponsesCompactResponse {
    #[serde(default)]
    pub(super) output: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub(super) struct OpenAiResponsesTextConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) verbosity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) format: Option<OpenAiResponsesTextFormat>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum OpenAiResponsesTextFormat {
    JsonObject,
    JsonSchema {
        name: String,
        schema: serde_json::Value,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        strict: bool,
    },
}

impl OpenAiResponsesTextFormat {
    pub(super) fn from_response_format(format: Option<&ResponseFormat>) -> Option<Self> {
        match format? {
            ResponseFormat::Text => None,
            ResponseFormat::JsonObject => Some(Self::JsonObject),
            ResponseFormat::JsonSchema {
                name,
                schema,
                strict,
            } => Some(Self::JsonSchema {
                name: name.clone(),
                schema: schema.clone(),
                strict: *strict,
            }),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct OpenAiInputMessage {
    pub(super) role: String,
    pub(super) content: Vec<OpenAiInputContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) copilot_cache_control: Option<prompt_cache::PromptCacheControl>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub(super) enum OpenAiInputContent {
    #[serde(rename = "input_text")]
    InputText { text: String },
    #[serde(rename = "output_text")]
    OutputText { text: String },
    #[serde(rename = "input_image")]
    Image { image_url: String },
    #[serde(rename = "input_file")]
    File {
        #[serde(skip_serializing_if = "Option::is_none")]
        file_data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
}

impl OpenAiInputContent {
    pub(super) fn text_for_role(role: &str, text: String) -> Self {
        if role == "assistant" {
            Self::OutputText { text }
        } else {
            Self::InputText { text }
        }
    }
}

pub(super) fn validate_responses_input(
    input: &[OpenAiResponsesInputItem],
) -> Result<(), ProviderError> {
    let mut seen_tool_calls = BTreeSet::new();

    for (index, item) in input.iter().enumerate() {
        match item {
            OpenAiResponsesInputItem::Message(message) => {
                validate_responses_message(index, message)?;
            }
            OpenAiResponsesInputItem::Reasoning(item) => {
                // Reasoning items may carry either OpenAI's `encrypted_content`
                // or plain-text `content` (chat-style `reasoning_content`
                // models that must replay their reasoning). Accept either.
                let has_encrypted_content = item
                    .get("encrypted_content")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|content| !content.is_empty());
                let has_content = item
                    .get("content")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|content| !content.is_empty());
                if !has_encrypted_content && !has_content {
                    return Err(ProviderError::Internal(format!(
                        "OpenAI Responses reasoning item at input[{index}] has neither encrypted_content nor content"
                    )));
                }
            }
            OpenAiResponsesInputItem::FunctionCall(item) => {
                if !protocol_ids::valid_openai_responses_call_id(item.call_id.as_ref()) {
                    return Err(ProviderError::Internal(format!(
                        "invalid OpenAI Responses function_call call_id at input[{index}]"
                    )));
                }
                if !responses_simple_tool_identifier(item.name.as_str()) {
                    return Err(ProviderError::Internal(format!(
                        "invalid OpenAI Responses function_call name at input[{index}]: expected 1-64 ASCII letters, digits, underscores, or hyphens"
                    )));
                }
                if item
                    .namespace
                    .as_deref()
                    .is_some_and(|namespace| !responses_simple_tool_identifier(namespace))
                {
                    return Err(ProviderError::Internal(format!(
                        "invalid OpenAI Responses function_call namespace at input[{index}]"
                    )));
                }
                seen_tool_calls.insert(item.call_id.clone());
            }
            OpenAiResponsesInputItem::FunctionCallOutput(item) => {
                if !protocol_ids::valid_openai_responses_call_id(item.call_id.as_ref()) {
                    return Err(ProviderError::Internal(format!(
                        "invalid OpenAI Responses function_call_output call_id at input[{index}]"
                    )));
                }
                if !seen_tool_calls.contains::<str>(item.call_id.as_ref()) {
                    return Err(ProviderError::Internal(format!(
                        "OpenAI Responses function_call_output at input[{index}] references unknown call_id `{}`",
                        item.call_id
                    )));
                }
            }
            OpenAiResponsesInputItem::Raw(item) => {
                if !item.is_object() {
                    return Err(ProviderError::Internal(format!(
                        "OpenAI compacted Responses item at input[{index}] is not an object"
                    )));
                }
            }
        }
    }

    Ok(())
}

pub(super) fn validate_responses_message(
    index: usize,
    message: &OpenAiInputMessage,
) -> Result<(), ProviderError> {
    let role = message.role.trim();
    if role.is_empty() {
        return Err(ProviderError::Internal(format!(
            "OpenAI Responses message at input[{index}] has empty role"
        )));
    }
    if message.content.is_empty() {
        return Err(ProviderError::Internal(format!(
            "OpenAI Responses message at input[{index}] has empty content"
        )));
    }

    for content in &message.content {
        match (role, content) {
            ("assistant", OpenAiInputContent::InputText { .. }) => {
                return Err(ProviderError::Internal(format!(
                    "OpenAI Responses assistant message at input[{index}] used input_text; assistant history must use output_text"
                )));
            }
            (role, OpenAiInputContent::OutputText { .. }) if role != "assistant" => {
                return Err(ProviderError::Internal(format!(
                    "OpenAI Responses {role} message at input[{index}] used output_text"
                )));
            }
            _ => {}
        }
    }

    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum OpenAiResponsesInputItem {
    Message(OpenAiInputMessage),
    Reasoning(serde_json::Value),
    FunctionCall(OpenAiFunctionCallItem),
    FunctionCallOutput(OpenAiFunctionCallOutputItem),
    Raw(serde_json::Value),
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum OpenAiRealtimeConversationItem {
    Message(OpenAiRealtimeMessageItem),
    FunctionCall(OpenAiFunctionCallItem),
    FunctionCallOutput(OpenAiFunctionCallOutputItem),
}

impl OpenAiRealtimeConversationItem {
    pub(super) fn from_responses_input(value: OpenAiResponsesInputItem) -> Option<Self> {
        match value {
            OpenAiResponsesInputItem::Message(message) => {
                Some(Self::Message(OpenAiRealtimeMessageItem {
                    kind: "message",
                    role: message.role,
                    content: message.content,
                }))
            }
            OpenAiResponsesInputItem::Reasoning(_) => None,
            OpenAiResponsesInputItem::FunctionCall(item) => Some(Self::FunctionCall(item)),
            OpenAiResponsesInputItem::FunctionCallOutput(item) => {
                Some(Self::FunctionCallOutput(item))
            }
            OpenAiResponsesInputItem::Raw(_) => None,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct OpenAiRealtimeMessageItem {
    #[serde(rename = "type")]
    kind: &'static str,
    role: String,
    content: Vec<OpenAiInputContent>,
}

#[derive(Debug, Serialize)]
pub(super) struct OpenAiFunctionCallItem {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) namespace: Option<String>,
    pub(super) name: String,
    pub(super) arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) copilot_cache_control: Option<prompt_cache::PromptCacheControl>,
}

#[derive(Debug, Serialize)]
pub(super) struct OpenAiFunctionCallOutputItem {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) call_id: String,
    pub(super) output: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) copilot_cache_control: Option<prompt_cache::PromptCacheControl>,
}

pub(super) fn responses_wire_tool_name(name: &str) -> Result<String, ProviderError> {
    if !responses_simple_tool_identifier(name) {
        return Err(ProviderError::Internal(format!(
            "invalid OpenAI Responses tool definition name {name:?}: expected a provider-safe identifier"
        )));
    }
    Ok(name.to_owned())
}

pub(super) fn openai_chat_tool_name(name: &str) -> String {
    name.to_owned()
}

/// Normalize one OpenAI Chat tool-call chunk into the same alias-aware stream
/// representation used by the Responses API.  A call id is authoritative,
/// while an index remains an alias for providers that omit either field on
/// some chunks.  Keeping both candidates prevents a compatible gateway from
/// turning a single call into several operations when it changes the index.
pub(crate) fn chat_tool_stream_input(
    provider_id: &str,
    tool: chat_wire::ChatToolCallWire,
) -> Result<ToolStreamInput, ProviderError> {
    let call_id = utils::normalize_optional_text(tool.id);
    let mut candidate_keys = Vec::new();
    if let Some(call_id) = call_id.as_ref() {
        candidate_keys.push(format!("id:{call_id}"));
    }
    if let Some(index) = tool.index {
        candidate_keys.push(format!("idx:{index}"));
    }
    let stream_key_candidates = candidate_keys
        .into_iter()
        .filter_map(|key| key.parse().ok())
        .collect::<Vec<_>>();
    if stream_key_candidates.is_empty() {
        return Err(ProviderError::Provider(format!(
            "{provider_id} returned chat tool_call delta without index/id"
        )));
    }

    let (name, arguments) = tool
        .function
        .map(|function| {
            (
                utils::optional_non_empty(function.name),
                utils::optional_non_empty(function.arguments),
            )
        })
        .unwrap_or_default();

    Ok(ToolStreamInput {
        // Standard OpenAI Chat streams carry argument deltas. A parameterless
        // function needs a registration event so the session processor does
        // not drop it before the stream completes.
        kind: if arguments.as_deref().is_some_and(|value| !value.is_empty()) {
            ToolStreamInputKind::Delta
        } else {
            ToolStreamInputKind::Start
        },
        stream_key_candidates,
        provider_item_id: None,
        model_call_id: call_id.and_then(|id| id.parse().ok()),
        name,
        arguments,
    })
}

pub(super) fn responses_model_tool_name(namespace: Option<&str>, name: &str) -> String {
    match namespace.filter(|value| !value.is_empty()) {
        Some(namespace) => format!("{namespace}.{name}"),
        None => name.to_owned(),
    }
}

pub(super) fn responses_simple_tool_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub(super) fn responses_tool_stream_input(
    provider_id: &str,
    event: utils::ResponsesToolEvent,
) -> Result<ToolStreamInput, ProviderError> {
    let stream_key_candidates = event
        .stream_key_candidates()
        .map_err(|message| ProviderError::Provider(format!("{provider_id} {message}")))?
        .into_iter()
        .filter_map(|value| value.parse().ok())
        .collect::<Vec<_>>();
    if stream_key_candidates.is_empty() {
        return Err(ProviderError::Provider(format!(
            "{provider_id} returned tool event without usable stream key candidates"
        )));
    }

    // Responses item ids (`fc_*`) identify output items, while `call_id`
    // identifies the model tool call and pairs it with function_call_output.
    // Argument delta events commonly carry only item_id, so promoting that
    // field to ModelToolCallId splits one call into two protocol identities.
    let model_call_id = event
        .call_id
        .as_deref()
        .and_then(protocol_ids::openai_responses_call_id);

    let name = event
        .name
        .as_deref()
        .map(|name| responses_model_tool_name(event.namespace.as_deref(), name));

    Ok(ToolStreamInput {
        kind: match event.kind {
            utils::ResponsesToolEventKind::Added => ToolStreamInputKind::Start,
            utils::ResponsesToolEventKind::Delta => ToolStreamInputKind::Delta,
            utils::ResponsesToolEventKind::Done => ToolStreamInputKind::Finish,
        },
        stream_key_candidates,
        provider_item_id: event.item_id.and_then(|value| value.parse().ok()),
        model_call_id,
        name,
        arguments: event.arguments,
    })
}

pub(crate) fn completion_event_from_tool_stream_update(
    provider_id: &ProviderId,
    model: &ModelId,
    update: ToolStreamUpdate,
) -> CompletionStreamEvent {
    match update {
        ToolStreamUpdate::Registered {
            stream_key,
            id,
            name,
        } => CompletionStreamEvent::ToolCallDelta {
            provider_id: provider_id.clone(),
            model: model.clone(),
            stream_key,
            id,
            name,
            arguments_delta: String::new(),
        },
        ToolStreamUpdate::ArgumentsDelta {
            stream_key,
            id,
            name,
            arguments_delta,
        } => CompletionStreamEvent::ToolCallDelta {
            provider_id: provider_id.clone(),
            model: model.clone(),
            stream_key,
            id,
            name,
            arguments_delta,
        },
        ToolStreamUpdate::ArgumentsSnapshot {
            stream_key,
            id,
            name,
            arguments_json,
        } => CompletionStreamEvent::ToolCallSnapshot {
            provider_id: provider_id.clone(),
            model: model.clone(),
            stream_key,
            id,
            name,
            arguments_json,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        OpenAiFunctionCallItem, OpenAiInputContent, OpenAiInputMessage, OpenAiResponsesInputItem,
        OpenAiResponsesRequest, OpenAiResponsesTextConfig, OpenAiResponsesTextFormat,
        responses_wire_tool_name, validate_responses_input,
    };

    #[test]
    fn responses_wire_shape_never_uses_chat_completions_fields() {
        let request = OpenAiResponsesRequest {
            model: "gpt-test".to_owned(),
            instructions: Some("system".to_owned()),
            input: vec![OpenAiResponsesInputItem::Message(OpenAiInputMessage {
                role: "user".to_owned(),
                content: vec![OpenAiInputContent::InputText {
                    text: "hello".to_owned(),
                }],
                copilot_cache_control: None,
            })],
            tools: Vec::new(),
            tool_choice: Some("auto".to_owned()),
            parallel_tool_calls: false,
            include: None,
            max_output_tokens: Some(1024),
            temperature: None,
            prompt_cache_key: None,
            previous_response_id: Some("resp_previous".to_owned()),
            store: Some(false),
            stream: Some(true),
            top_p: None,
            reasoning: None,
            service_tier: None,
            text: Some(OpenAiResponsesTextConfig {
                verbosity: None,
                format: Some(OpenAiResponsesTextFormat::JsonObject),
            }),
            client_metadata: None,
        };
        let value = serde_json::to_value(request).expect("serialize Responses request");

        assert!(value.get("input").is_some());
        assert!(value.get("text").is_some());
        assert!(value.get("previous_response_id").is_some());
        assert!(value.get("messages").is_none());
        assert!(value.get("response_format").is_none());
        assert!(value.get("max_completion_tokens").is_none());
    }

    #[test]
    fn responses_input_rejects_dotted_function_names_locally() {
        let input = vec![OpenAiResponsesInputItem::FunctionCall(
            OpenAiFunctionCallItem {
                kind: "function_call",
                call_id: "call_1".to_owned(),
                namespace: None,
                name: "agena.tools.help".to_owned(),
                arguments: "{}".to_owned(),
                copilot_cache_control: None,
            },
        )];

        let error = validate_responses_input(input.as_slice())
            .expect_err("dotted function name must be rejected before HTTP");
        assert!(error.to_string().contains("input[0]"));
        assert!(error.to_string().contains("ASCII letters"));
    }

    #[test]
    fn responses_input_rejects_function_name_whitespace_locally() {
        for name in [" tools_help", "tools_help "] {
            let input = vec![OpenAiResponsesInputItem::FunctionCall(
                OpenAiFunctionCallItem {
                    kind: "function_call",
                    call_id: "call_1".to_owned(),
                    namespace: None,
                    name: name.to_owned(),
                    arguments: "{}".to_owned(),
                    copilot_cache_control: None,
                },
            )];

            validate_responses_input(input.as_slice())
                .expect_err("function name whitespace must be rejected before HTTP");
        }
    }

    #[test]
    fn responses_tool_definition_has_no_raw_invalid_name_fallback() {
        let error = responses_wire_tool_name("agena.tools.help")
            .expect_err("multi-segment internal key must not reach Responses");
        assert!(
            error
                .to_string()
                .contains("invalid OpenAI Responses tool definition name")
        );

        responses_wire_tool_name("tools.help")
            .expect_err("execution tool must not become a provider function definition");

        let valid = responses_wire_tool_name("tools_help").expect("safe Tool API function");
        assert_eq!(valid, "tools_help");
    }

    #[test]
    fn compacted_responses_item_round_trips_without_text_conversion() {
        let raw = serde_json::json!({
            "type": "compaction",
            "encrypted_content": "opaque-provider-state",
            "provider_extension": { "version": 3 }
        });
        let input = vec![OpenAiResponsesInputItem::Raw(raw.clone())];
        validate_responses_input(input.as_slice()).expect("opaque object is valid input");
        let serialized = serde_json::to_value(&input).expect("serialize input");
        assert_eq!(serialized, serde_json::json!([raw]));
    }
}
