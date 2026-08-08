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
/// Record of a tool-mode policy violation.
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

/// Apply the transport mode for Agena's five fixed Tool API functions.
///
/// `provider_native` is retained only in the transition signature for callers
/// compiled against the old provider contract. It is intentionally ignored:
/// provider service capabilities are ordinary execution tools and never become
/// conversation-level provider declarations.
pub fn apply_configured_tool_request(
    mode: AgenaToolMode,
    _provider_native: &ProviderNativeToolsConfig,
    request: &mut CompletionRequest,
) {
    request.provider_native_tools = Default::default();
    // Saved request patches must never inject an alternate tool surface. Each
    // adapter rebuilds its protocol declarations from tool_api_functions only.
    strip_provider_tool_body_fields(request);
    if request.disable_tools || mode.is_disabled() {
        prepare_disabled_tool_request(request);
    }
}

pub fn prepare_disabled_tool_request(request: &mut CompletionRequest) {
    request.tool_api_functions.clear();
    request.provider_native_tools = Default::default();
    request.previous_response_id = None;
    strip_provider_tool_body_fields(request);

    let mut projected_messages = Vec::new();
    for message in std::mem::take(&mut request.messages) {
        projected_messages.extend(project_disabled_completion_input_history(message));
    }
    request.messages = projected_messages;
}

/// Remove raw body patches that could bypass Agena's fixed five-function tool
/// contract or reintroduce provider-service tools outside the plugin catalog.
pub fn strip_provider_tool_body_fields(request: &mut CompletionRequest) {
    for field in PROVIDER_TOOL_BODY_FIELDS {
        request.request_override.body_patch.remove(*field);
    }
}

/// Backward-compatible name retained for downstream callers.
pub fn strip_provider_native_tool_body_fields(request: &mut CompletionRequest) {
    strip_provider_tool_body_fields(request);
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
        "the backend returned a tool call",
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
