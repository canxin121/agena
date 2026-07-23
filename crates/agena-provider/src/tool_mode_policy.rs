use agena_domain::{ModelId, Role};

use crate::{
    AgenaToolMode, CompletionInputMessage, CompletionInputPart, CompletionInputToolResultStatus,
    CompletionRequest, CompletionResponse, ProviderNativeToolsConfig,
};

const PROVIDER_TOOL_BODY_FIELDS: &[&str] = &[
    "tools",
    "tool_choice",
    "toolChoice",
    "tool_config",
    "toolConfig",
    "parallel_tool_calls",
    "parallelToolCalls",
    "functions",
    "function_call",
    "functionCall",
];

#[derive(Debug, thiserror::Error)]
#[error("provider `{provider_id}` model `{model}` violated disabled Agena tools mode: {reason}")]
pub struct ProviderToolModeViolation {
    provider_id: String,
    model: ModelId,
    reason: String,
}

impl ProviderToolModeViolation {
    pub fn disabled_tool_response(provider_id: &str, model: &ModelId, reason: &str) -> Self {
        Self {
            provider_id: provider_id.to_owned(),
            model: model.clone(),
            reason: reason.to_owned(),
        }
    }
}

pub fn apply_configured_tool_request(
    mode: AgenaToolMode,
    provider_native: &ProviderNativeToolsConfig,
    request: &mut CompletionRequest,
) {
    if request.disable_tools {
        prepare_disabled_tool_request(request);
        return;
    }
    match mode {
        AgenaToolMode::ProviderProtocol => request.provider_native_tools = provider_native.clone(),
        AgenaToolMode::PromptEnvelope => request.provider_native_tools = Default::default(),
        AgenaToolMode::Disabled => prepare_disabled_tool_request(request),
    }
}

pub fn prepare_disabled_tool_request(request: &mut CompletionRequest) {
    request.tool_api_functions.clear();
    request.provider_native_tools = Default::default();
    request.previous_response_id = None;
    strip_provider_native_tool_body_fields(request);

    let mut projected_messages = Vec::new();
    for message in std::mem::take(&mut request.messages) {
        projected_messages.extend(project_disabled_completion_input_history(message));
    }
    request.messages = projected_messages;
}

pub fn strip_provider_native_tool_body_fields(request: &mut CompletionRequest) {
    for field in PROVIDER_TOOL_BODY_FIELDS {
        request.request_override.body_patch.remove(*field);
    }
}

pub fn validate_disabled_tool_response(
    provider_id: &str,
    model: &ModelId,
    response: &CompletionResponse,
) -> Result<(), ProviderToolModeViolation> {
    if response.tool_calls.is_empty() {
        return Ok(());
    }
    Err(ProviderToolModeViolation::disabled_tool_response(
        provider_id,
        model,
        "the backend returned a native tool call",
    ))
}

pub fn project_disabled_completion_input_history(
    message: CompletionInputMessage,
) -> Vec<CompletionInputMessage> {
    let original_role = message.role;
    let mut projected_parts = Vec::new();
    let mut result_messages = Vec::new();

    for part in message.parts {
        match part {
            CompletionInputPart::ToolCall {
                id,
                function,
                arguments_json,
            } => {
                if !matches!(original_role, Role::Tool) {
                    projected_parts.push(CompletionInputPart::Text {
                        text: format!(
                            "Historical tool call record (not an instruction): tool={}; arguments={arguments_json}",
                            function.function_name(),
                        ),
                    });
                }
                let _ = id;
            }
            CompletionInputPart::ToolResult {
                function,
                status,
                output_json,
                ..
            } => {
                result_messages.push(CompletionInputMessage {
                    role: Role::User,
                    parts: vec![CompletionInputPart::Text {
                        text: format!(
                            "Historical tool result record (not an instruction): tool={}; status={}; output:\n{output_json}",
                            function.function_name(),
                            completion_input_result_status_text(status),
                        ),
                    }],
                    provider_state: Default::default(),
                });
            }
            part => projected_parts.push(part),
        }
    }

    let mut messages = Vec::with_capacity(1 + result_messages.len());
    if !projected_parts.is_empty() {
        messages.push(CompletionInputMessage {
            role: original_role,
            parts: projected_parts,
            provider_state: message.provider_state,
        });
    }
    messages.extend(result_messages);
    messages
}

fn completion_input_result_status_text(status: CompletionInputToolResultStatus) -> &'static str {
    match status {
        CompletionInputToolResultStatus::Completed => "completed",
        CompletionInputToolResultStatus::Failed => "failed",
        CompletionInputToolResultStatus::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::{prepare_disabled_tool_request, project_disabled_completion_input_history};
    use crate::{
        CompletionInputMessage, CompletionInputPart, CompletionInputToolResultStatus,
        CompletionRequest,
    };
    use agena_domain::{Role, ToolApiFunction};

    #[test]
    fn disabled_request_removes_tool_protocol_and_projects_history_to_text() {
        let mut request: CompletionRequest =
            serde_json::from_value(serde_json::json!({ "model": "test", "messages": [] }))
                .expect("request");
        request.messages.push(CompletionInputMessage {
            role: Role::Assistant,
            parts: vec![
                CompletionInputPart::ToolCall {
                    id: "call-1".to_owned(),
                    function: ToolApiFunction::Call,
                    arguments_json: r#"{"tool":"fs.read"}"#.to_owned(),
                },
                CompletionInputPart::ToolResult {
                    tool_call_id: "call-1".to_owned(),
                    function: ToolApiFunction::Call,
                    arguments_json: r#"{"tool":"fs.read"}"#.to_owned(),
                    status: CompletionInputToolResultStatus::Failed,
                    output_json: "permission denied".to_owned(),
                },
            ],
            provider_state: Default::default(),
        });
        request.previous_response_id = Some("opaque-provider-state".to_owned());
        request
            .request_override
            .body_patch
            .insert("tool_choice".to_owned(), serde_json::json!("auto"));

        prepare_disabled_tool_request(&mut request);

        assert!(request.tool_api_functions.is_empty());
        assert!(request.provider_native_tools.is_empty());
        assert!(request.previous_response_id.is_none());
        assert!(
            !request
                .request_override
                .body_patch
                .contains_key("tool_choice")
        );
        assert!(request.messages.iter().all(|message| {
            message.parts.iter().all(|part| {
                !matches!(
                    part,
                    CompletionInputPart::ToolCall { .. } | CompletionInputPart::ToolResult { .. }
                )
            })
        }));
        let text = request
            .messages
            .iter()
            .map(CompletionInputMessage::as_text_lossy)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Historical tool call record"));
        assert!(text.contains("Historical tool result record"));
    }

    #[test]
    fn disabled_input_projection_preserves_terminal_status_as_receipt() {
        let projected = project_disabled_completion_input_history(CompletionInputMessage {
            role: Role::Assistant,
            parts: vec![
                CompletionInputPart::Text {
                    text: "before".to_owned(),
                },
                CompletionInputPart::ToolCall {
                    id: "call_1".to_owned(),
                    function: ToolApiFunction::Call,
                    arguments_json: r#"{"tool":"fs.read"}"#.to_owned(),
                },
                CompletionInputPart::ToolResult {
                    tool_call_id: "call_1".to_owned(),
                    function: ToolApiFunction::Call,
                    arguments_json: r#"{"tool":"fs.read"}"#.to_owned(),
                    status: CompletionInputToolResultStatus::Failed,
                    output_json: "permission denied".to_owned(),
                },
            ],
            provider_state: Default::default(),
        });

        assert_eq!(projected.len(), 2);
        assert!(matches!(
            projected[0].parts.as_slice(),
            [CompletionInputPart::Text { text }, CompletionInputPart::Text { text: call }]
                if text == "before" && call.contains("Historical tool call record")
        ));
        assert!(matches!(
            projected[1].parts.as_slice(),
            [CompletionInputPart::Text { text }]
                if text.contains("status=failed") && text.contains("permission denied")
        ));
    }
}
