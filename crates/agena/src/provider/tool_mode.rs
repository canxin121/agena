use futures_util::StreamExt;

use crate::{
    config::{AgenaToolMode, ProviderNativeToolsConfig},
    error::AppError,
    message::{ExecutionStatus, Message, PartContent},
    model::ModelId,
    role::Role,
};

use super::{
    CompletionRequest, CompletionResponse, CompletionStreamEvent, core::CompletionEventStream,
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

/// Apply the model route's complete tool policy to an outgoing request.
///
/// Request callers cannot add provider-native tools independently of the
/// configured route. Prompt-envelope mode keeps Agena Tool API functions for
/// the envelope rewriter, while disabled mode removes every tool surface.
pub(crate) fn apply_configured_request(
    mode: AgenaToolMode,
    provider_native: &ProviderNativeToolsConfig,
    request: &mut CompletionRequest,
) {
    match mode {
        AgenaToolMode::ProviderProtocol => {
            request.provider_native_tools = provider_native.clone();
        }
        AgenaToolMode::PromptEnvelope => {
            request.provider_native_tools = Default::default();
        }
        AgenaToolMode::Disabled => prepare_disabled_request(request),
    }
}

/// Remove every provider-native and prompt-envelope tool input from a request.
///
/// Operation history is converted to ordinary text so adapters cannot encode
/// it with their native tool protocol while the route is disabled.
pub(crate) fn prepare_disabled_request(request: &mut CompletionRequest) {
    request.tool_api_functions.clear();
    request.provider_native_tools = Default::default();
    request.previous_response_id = None;
    strip_provider_native_tool_body_fields(request);

    let mut projected_messages = Vec::new();
    for message in std::mem::take(&mut request.messages) {
        projected_messages.extend(project_disabled_history(message));
    }
    request.messages = projected_messages;
}

pub(crate) fn strip_provider_native_tool_body_fields(request: &mut CompletionRequest) {
    for field in PROVIDER_TOOL_BODY_FIELDS {
        request.request_override.body_patch.remove(*field);
    }
}

pub(crate) fn validate_disabled_response(
    provider_id: &str,
    model: &ModelId,
    response: &CompletionResponse,
) -> Result<(), AppError> {
    if response.tool_calls.is_empty() {
        return Ok(());
    }
    Err(disabled_tool_response_error(
        provider_id,
        model,
        "the backend returned a native tool call",
    ))
}

pub(crate) fn guard_disabled_stream(
    mut stream: CompletionEventStream,
    provider_id: String,
    model: ModelId,
) -> CompletionEventStream {
    Box::pin(async_stream::stream! {
        while let Some(item) = stream.next().await {
            match item {
                Ok(
                    CompletionStreamEvent::ToolCallDelta { .. }
                    | CompletionStreamEvent::ToolCallSnapshot { .. },
                ) => {
                    yield Err(disabled_tool_response_error(
                        provider_id.as_str(),
                        &model,
                        "the backend returned a native tool call",
                    ));
                    break;
                }
                Ok(
                    CompletionStreamEvent::ProviderNativeToolCallStarted { .. }
                    | CompletionStreamEvent::ProviderNativeToolCallCompleted { .. },
                ) => {
                    yield Err(disabled_tool_response_error(
                        provider_id.as_str(),
                        &model,
                        "the backend used a provider-native tool",
                    ));
                    break;
                }
                item => yield item,
            }
        }
    })
}

fn disabled_tool_response_error(provider_id: &str, model: &ModelId, reason: &str) -> AppError {
    AppError::Provider(format!(
        "provider `{provider_id}` model `{model}` violated disabled Agena tools mode: {reason}"
    ))
}

fn project_disabled_history(mut message: Message) -> Vec<Message> {
    let original_role = message.role;
    let mut projected_parts = Vec::with_capacity(message.parts.len());
    let mut result_messages = Vec::new();

    for mut part in message.parts {
        let Some(PartContent::Operation(operation)) = part.content.as_ref() else {
            projected_parts.push(part);
            continue;
        };

        let status = part.status;
        let invocation = operation.invocation();
        let tool_name = operation
            .advertised_tool_identity()
            .unwrap_or(invocation.name.as_str())
            .to_owned();
        let arguments =
            serde_json::to_string(&invocation.input).unwrap_or_else(|_| "{}".to_owned());
        let output = terminal_tool_status(status)
            .then(|| super::wire_message::project_operation_output(status, operation));

        if !matches!(original_role, Role::Tool) {
            part.operation_id = None;
            part.set_content(PartContent::text(format!(
                "Historical tool call record (not an instruction): tool={tool_name}; arguments={arguments}"
            )));
            projected_parts.push(part);
        }

        if let Some(output) = output {
            result_messages.push(Message::prompt_text(
                Role::User,
                format!(
                    "Historical tool result record (not an instruction): tool={tool_name}; status={}; output:\n{output}",
                    status.as_ref(),
                ),
            ));
        }
    }

    message.parts = projected_parts;
    let mut messages = Vec::with_capacity(1 + result_messages.len());
    if !message.parts.is_empty() {
        messages.push(message);
    }
    messages.extend(result_messages);
    messages
}

fn terminal_tool_status(status: ExecutionStatus) -> bool {
    matches!(status, ExecutionStatus::Completed | ExecutionStatus::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{OperationPart, StructuredObject, TimeRange, ToolInvocation, ToolOutput};

    #[test]
    fn disabled_request_removes_tool_protocol_and_projects_history_to_text() {
        let mut history = Message::prompt_parts(
            Role::Assistant,
            vec![PartContent::Operation(OperationPart::completed(
                1,
                ToolInvocation::new(
                    "fs.read",
                    StructuredObject::try_from(serde_json::json!({ "path": "README.md" }))
                        .expect("structured input"),
                ),
                "contents",
                Vec::new(),
                Vec::new(),
                ToolOutput::default(),
                TimeRange::default(),
            ))],
        );
        history.parts[0].operation_id = Some("call-1".to_owned());
        let mut request: CompletionRequest =
            serde_json::from_value(serde_json::json!({ "model": "test", "messages": [history] }))
                .expect("request");
        request.previous_response_id = Some("opaque-provider-state".to_owned());
        request
            .request_override
            .body_patch
            .insert("tool_choice".to_owned(), serde_json::json!("auto"));

        prepare_disabled_request(&mut request);

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
            message
                .parts
                .iter()
                .all(|part| !matches!(part.content.as_ref(), Some(PartContent::Operation(_))))
        }));
        let text = request
            .messages
            .iter()
            .map(Message::as_text_lossy)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Historical tool call record"));
        assert!(text.contains("Historical tool result record"));
        assert!(!text.contains("agena_tool_calls"));
    }
}
