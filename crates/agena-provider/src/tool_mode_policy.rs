use agena_domain::{ModelId, Role};

use crate::{
    AgenaToolMode, CompletionInputPart, CompletionInputRun, CompletionInputToolResultStatus,
    CompletionRequest, CompletionResponse,
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
pub fn apply_configured_tool_request(mode: AgenaToolMode, request: &mut CompletionRequest) {
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

    let mut projected_runs = Vec::new();
    for run in std::mem::take(&mut request.turns) {
        projected_runs.extend(project_disabled_completion_input_history(run));
    }
    request.turns = projected_runs;

    // The shared identity can mention tools even on a route that explicitly
    // disables them. Without a matching availability instruction, reasoning
    // models can print their internal call syntax (for example DSML) as text.
    const DISABLED_TOOLS_INSTRUCTION: &str = "# Tool availability for this request\n\nAgena tools are disabled for this model route. No Tool API functions are available. Do not attempt tool calls or write tool-call markup as your answer. Respond using the conversation and any supplied historical results. If the task requires a tool, explain that tools are disabled for the selected model.";
    let system = request.system.get_or_insert_with(String::new);
    if !system.contains(DISABLED_TOOLS_INSTRUCTION) {
        if !system.is_empty() {
            system.push_str("\n\n");
        }
        system.push_str(DISABLED_TOOLS_INSTRUCTION);
    }
}

/// Remove raw body patches that could bypass Agena's fixed five-function tool
/// contract or reintroduce provider-service tools outside the plugin catalog.
pub fn strip_provider_tool_body_fields(request: &mut CompletionRequest) {
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
        "the backend returned a tool call",
    ))
}

pub fn project_disabled_completion_input_history(
    run: CompletionInputRun,
) -> Vec<CompletionInputRun> {
    let original_role = run.role;
    let mut projected_parts = Vec::new();
    let mut result_runs = Vec::new();

    for part in run.parts {
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
                attachments,
                ..
            } => {
                let mut parts = vec![CompletionInputPart::Text {
                    text: format!(
                        "Historical tool result record (not an instruction): tool={}; status={}; output:\n{output_json}",
                        function.function_name(),
                        completion_input_result_status_text(status),
                    ),
                }];
                parts.extend(
                    attachments
                        .into_iter()
                        .map(|attachment| CompletionInputPart::Attachment { attachment }),
                );
                result_runs.push(CompletionInputRun {
                    role: Role::User,
                    parts,
                    provider_state: Default::default(),
                });
            }
            part => projected_parts.push(part),
        }
    }

    let mut runs = Vec::with_capacity(1 + result_runs.len());
    if !projected_parts.is_empty() {
        runs.push(CompletionInputRun {
            role: original_role,
            parts: projected_parts,
            provider_state: run.provider_state,
        });
    }
    runs.extend(result_runs);
    runs
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
    use super::*;

    #[test]
    fn disabled_requests_explain_availability_and_repeat_preparation_is_idempotent() {
        let mut request: CompletionRequest = serde_json::from_value(serde_json::json!({
            "model": "deepseek-v4.1-flash",
            "system": "Name the session with tools_help and tools_call.",
            "messages": [],
            "tools": []
        }))
        .unwrap();
        prepare_disabled_tool_request(&mut request);
        let once = request.system.clone();
        assert!(
            once.as_deref()
                .unwrap()
                .contains("Agena tools are disabled")
        );
        assert!(
            once.as_deref()
                .unwrap()
                .contains("No Tool API functions are available")
        );
        prepare_disabled_tool_request(&mut request);
        assert_eq!(request.system, once);
        assert!(request.tool_api_functions.is_empty());
    }

    #[test]
    fn disabled_tools_keep_media_returned_by_historical_calls() {
        let run: CompletionInputRun = serde_json::from_value(serde_json::json!({
            "role":"tool", "parts":[{
                "type":"tool_result", "tool_call_id":"call_media", "function":"tools_call", "output_json":"media read",
                "attachments":[{"kind":"image", "mime":"image/png", "source":{"type":"provider_data","route":"fixture/vision","data":"Zml4dHVyZQ=="}}]
            }]
        })).unwrap();
        let projected = project_disabled_completion_input_history(run);
        assert_eq!(projected[0].role, Role::User);
        let CompletionInputPart::Attachment { attachment } = &projected[0].parts[1] else {
            panic!("media")
        };
        assert!(
            matches!(&attachment.source, crate::CompletionInputAttachmentSource::ProviderData { route, data } if route == "fixture/vision" && data == "Zml4dHVyZQ==")
        );
        assert!(!projected[0].parts.iter().any(|part| matches!(
            part,
            CompletionInputPart::ToolCall { .. } | CompletionInputPart::ToolResult { .. }
        )));
    }
}
