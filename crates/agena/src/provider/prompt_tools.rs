use serde::Deserialize;

use crate::{
    message::{AttachmentKind, Message},
    provider::{CompletionRequest, CompletionToolCall, wire_message},
};

const TOOL_CALL_START: &str = "<agena_tool_call>";
const TOOL_CALL_END: &str = "</agena_tool_call>";
const TOOL_RESULT_START: &str = "<agena_tool_result>";
const TOOL_RESULT_END: &str = "</agena_tool_result>";

#[derive(Debug, Deserialize)]
struct PromptToolCall {
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct PromptToolCalls {
    #[serde(default)]
    tool_calls: Vec<PromptToolCall>,
}

pub(crate) fn system_prompt(tools: &[crate::plugin::registry::RegisteredTool]) -> String {
    let mut out = String::from(
        "Agena tools are available through a text tool-call protocol. Tool names are exact and may contain dots. When you need a tool, output only one or more tool-call blocks and no explanatory text.\n\nFormat:\n<agena_tool_call>{\"id\":\"call_1\",\"name\":\"tool.subcommand\",\"arguments\":{}}</agena_tool_call>\n\nRules:\n- Use only names from the catalog below.\n- Put the complete JSON object on the inside of the tag.\n- `arguments` must be a JSON object matching the tool schema.\n- After tool results are provided, continue normally.\n\nAvailable tools:\n",
    );

    for tool in tools {
        out.push_str("- `");
        out.push_str(tool.exposed_name.as_str());
        out.push_str("`: ");
        out.push_str(tool.description_text().trim());
        out.push_str("\n  input_schema: ");
        out.push_str(
            serde_json::to_string(&crate::tool::model_safe_tool_schema(
                &tool.sanitized_input_schema(),
            ))
            .unwrap_or_else(|_| "{}".to_owned())
            .as_str(),
        );
        out.push('\n');
    }

    out
}

pub(crate) fn request_needs_text_protocol(request: &CompletionRequest) -> bool {
    !request.tools.is_empty() || request_has_tool_history(request)
}

pub(crate) fn request_has_tool_history(request: &CompletionRequest) -> bool {
    request.messages.iter().any(|message| {
        wire_message::project(message).iter().any(|part| {
            matches!(
                part,
                wire_message::WirePart::ToolCall { .. } | wire_message::WirePart::ToolResult { .. }
            )
        })
    })
}

pub(crate) fn message_text(message: &Message) -> String {
    let projected = wire_message::project(message);
    if projected.is_empty() {
        return message.as_text_lossy();
    }

    let mut out = String::new();
    for part in projected {
        match part {
            wire_message::WirePart::Text { text } => out.push_str(text.as_str()),
            wire_message::WirePart::Attachment { item } => {
                out.push_str(&attachment_prompt_text(&item));
            }
            wire_message::WirePart::ToolCall {
                id,
                name,
                arguments_json,
            } => {
                let arguments = serde_json::from_str::<serde_json::Value>(&arguments_json)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                let payload = serde_json::json!({
                    "id": id,
                    "name": name,
                    "arguments": arguments,
                });
                out.push_str(TOOL_CALL_START);
                out.push_str(
                    serde_json::to_string(&payload)
                        .unwrap_or_else(|_| "{}".to_owned())
                        .as_str(),
                );
                out.push_str(TOOL_CALL_END);
            }
            wire_message::WirePart::ToolResult {
                tool_call_id,
                tool_name,
                output_json,
            } => {
                let output = serde_json::from_str::<serde_json::Value>(&output_json)
                    .unwrap_or_else(|_| serde_json::Value::String(output_json));
                let payload = serde_json::json!({
                    "id": tool_call_id,
                    "name": tool_name,
                    "output": output,
                });
                out.push_str(TOOL_RESULT_START);
                out.push_str(
                    serde_json::to_string(&payload)
                        .unwrap_or_else(|_| "{}".to_owned())
                        .as_str(),
                );
                out.push_str(TOOL_RESULT_END);
            }
        }
    }
    out
}

pub(crate) fn parse_tool_calls(text: &str) -> (String, Vec<CompletionToolCall>) {
    let mut clean = String::new();
    let mut calls = Vec::new();
    let mut remainder = text;

    while let Some(start) = remainder.find(TOOL_CALL_START) {
        clean.push_str(&remainder[..start]);
        let after_start = &remainder[start + TOOL_CALL_START.len()..];
        let Some(end) = after_start.find(TOOL_CALL_END) else {
            clean.push_str(&remainder[start..]);
            return (clean, calls);
        };
        let raw = strip_json_fence(&after_start[..end]);
        push_parsed_tool_calls(raw, &mut calls);
        remainder = &after_start[end + TOOL_CALL_END.len()..];
    }

    clean.push_str(remainder);
    (clean, calls)
}

fn push_parsed_tool_calls(raw: &str, calls: &mut Vec<CompletionToolCall>) {
    if let Ok(call) = serde_json::from_str::<PromptToolCall>(raw) {
        push_tool_call(call, calls);
        return;
    }

    if let Ok(batch) = serde_json::from_str::<PromptToolCalls>(raw) {
        for call in batch.tool_calls {
            push_tool_call(call, calls);
        }
        return;
    }

    if let Ok(items) = serde_json::from_str::<Vec<PromptToolCall>>(raw) {
        for call in items {
            push_tool_call(call, calls);
        }
    }
}

fn push_tool_call(call: PromptToolCall, calls: &mut Vec<CompletionToolCall>) {
    let name = call.name.trim();
    if name.is_empty() {
        return;
    }
    let id = call
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("call_{}", calls.len() + 1));
    let arguments = match call.arguments {
        serde_json::Value::Null => serde_json::Value::Object(serde_json::Map::new()),
        other => other,
    };
    calls.push(CompletionToolCall::Function {
        id,
        name: name.to_owned(),
        arguments_json: serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_owned()),
    });
}

fn strip_json_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(without_start) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let without_lang = without_start
        .strip_prefix("json")
        .or_else(|| without_start.strip_prefix("JSON"))
        .unwrap_or(without_start)
        .trim_start_matches(['\r', '\n']);
    without_lang
        .strip_suffix("```")
        .map(str::trim)
        .unwrap_or(trimmed)
}

fn attachment_prompt_text(item: &crate::message::AttachmentItem) -> String {
    match item.kind {
        AttachmentKind::File => {
            wire_message::attachment_text(item).unwrap_or_else(|| wire_message::hint_text(item))
        }
        AttachmentKind::Image
        | AttachmentKind::Audio
        | AttachmentKind::Video
        | AttachmentKind::Pdf => wire_message::hint_text(item),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        message::{
            ExecutionStatus, MessagePart, OperationPart, PartContent, StructuredObject, TimeRange,
            ToolInvocation, ToolOutput,
        },
        model::ModelId,
        role::Role,
    };

    #[test]
    fn parses_tagged_dotted_tool_calls_and_removes_protocol_text() {
        let (text, calls) = parse_tool_calls(
            "before <agena_tool_call>{\"id\":\"call_1\",\"name\":\"fs.read\",\"arguments\":{\"path\":\"Cargo.toml\"}}</agena_tool_call> after",
        );

        assert_eq!(text, "before  after");
        assert_eq!(
            calls,
            vec![CompletionToolCall::Function {
                id: "call_1".to_owned(),
                name: "fs.read".to_owned(),
                arguments_json: "{\"path\":\"Cargo.toml\"}".to_owned(),
            }]
        );
    }

    #[test]
    fn request_needs_text_protocol_for_exact_dotted_tool_history() {
        let created_at = chrono::Utc::now();
        let invocation = ToolInvocation::new(
            "web.search",
            StructuredObject::try_from(serde_json::json!({ "query": "weather" }))
                .expect("tool input"),
        );
        let mut tool_part = MessagePart::with_content(
            1,
            0,
            created_at,
            ExecutionStatus::Completed,
            PartContent::Operation(OperationPart::completed(
                1,
                invocation,
                r#"{"forecast":"sunny"}"#,
                Vec::new(),
                Vec::new(),
                ToolOutput::default(),
                TimeRange::default(),
            )),
        );
        tool_part.operation_id = Some("call_1".to_owned());
        let request = CompletionRequest {
            model: ModelId::new("test-model"),
            system: None,
            messages: vec![Message {
                id: 1,
                role: Role::Assistant,
                state: ExecutionStatus::Completed,
                parts: vec![tool_part],
                created_at,
                metadata: Default::default(),
                provider_state: None,
                usage: None,
            }],
            tools: Vec::new(),
            native_tools: crate::config::ProviderNativeToolsConfig::default(),
            temperature: None,
            max_output_tokens: None,
            prompt_cache_key: None,
            previous_response_id: None,
            prompt_window_generation: None,
            stop_sequences: Vec::new(),
            top_p: None,
            top_k: None,
            seed: None,
            thinking: None,
            verbosity: None,
            response_format: None,
            responses_api_metadata: None,
            request_override: crate::model::ModelSpeedModeRequestOverride::default(),
        };

        assert!(request_has_tool_history(&request));
        assert!(request_needs_text_protocol(&request));

        let text = message_text(&request.messages[0]);
        assert!(text.contains("\"name\":\"web.search\""));
        assert!(!text.contains("web_search"));
    }
}
