use super::{
    Deserialize, Message, ModelInputModality, OpenAiResponsesInputItem, chat_wire, prompt_cache,
    wire_message,
};

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiResponsesResponse {
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) output_text: Option<String>,
    #[serde(default)]
    pub(super) output: Option<Vec<OpenAiOutputItem>>,
    #[serde(default)]
    pub(super) stop_reason: Option<String>,
    #[serde(default)]
    pub(super) status: Option<String>,
    #[serde(default)]
    pub(super) error: Option<serde_json::Value>,
    #[serde(default)]
    pub(super) incomplete_details: Option<OpenAiIncompleteDetails>,
    #[serde(default)]
    pub(super) usage: Option<OpenAiUsage>,
}

impl OpenAiResponsesResponse {
    pub(super) fn failure_event(&self) -> Option<serde_json::Value> {
        let status = self.status.as_deref();
        if self.error.is_none() && !matches!(status, Some("failed" | "cancelled")) {
            return None;
        }
        Some(serde_json::json!({
            "type": "response.failed",
            "response": {
                "status": status.unwrap_or("failed"),
                "error": self.error.as_ref(),
            }
        }))
    }

    pub(super) fn terminal_reason(&self) -> Option<&str> {
        self.stop_reason
            .as_deref()
            .or_else(|| {
                self.incomplete_details
                    .as_ref()
                    .and_then(|details| details.reason.as_deref())
            })
            .or_else(|| {
                self.status
                    .as_deref()
                    .filter(|status| matches!(*status, "completed" | "incomplete"))
            })
    }

    pub(super) fn unexpected_nonstream_status(&self) -> Option<&str> {
        self.status.as_deref().filter(|status| {
            !matches!(*status, "completed" | "incomplete" | "failed" | "cancelled")
        })
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiIncompleteDetails {
    #[serde(default)]
    pub(super) reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiOutputItem {
    #[serde(default, rename = "type")]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) call_id: Option<String>,
    #[serde(default)]
    pub(super) namespace: Option<String>,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) arguments: Option<String>,
    #[serde(default)]
    pub(super) content: Option<Vec<OpenAiOutputContent>>,
    #[serde(default)]
    pub(super) summary: Option<Vec<OpenAiReasoningSummaryContent>>,
    #[serde(default)]
    pub(super) encrypted_content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiOutputContent {
    #[serde(default, rename = "type")]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiReasoningSummaryContent {
    #[serde(default)]
    pub(super) text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiUsage {
    #[serde(default)]
    pub(super) input_tokens: Option<u64>,
    #[serde(default)]
    pub(super) total_tokens: Option<u64>,
    #[serde(default)]
    pub(super) output_tokens: Option<u64>,
    #[serde(default)]
    pub(super) output_tokens_details: Option<OpenAiOutputTokenDetails>,
    #[serde(default)]
    pub(super) input_tokens_details: Option<OpenAiInputTokenDetails>,
    /// xAI reports exact request cost in 10^-10 USD ticks.
    #[serde(default)]
    pub(super) cost_in_usd_ticks: Option<u64>,
}

pub(super) fn collect_compact_content_text(
    value: Option<&serde_json::Value>,
    chunks: &mut Vec<String>,
) {
    match value {
        Some(serde_json::Value::String(text)) => chunks.push(text.clone()),
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                collect_compact_string_field(item, "text", chunks);
                collect_compact_string_field(item, "summary", chunks);
            }
        }
        Some(serde_json::Value::Object(_)) => {
            if let Some(value) = value {
                collect_compact_string_field(value, "text", chunks);
                collect_compact_string_field(value, "summary", chunks);
            }
        }
        _ => {}
    }
}

pub(super) fn collect_compact_string_field(
    value: &serde_json::Value,
    field: &str,
    chunks: &mut Vec<String>,
) {
    if let Some(text) = value.get(field).and_then(serde_json::Value::as_str)
        && !text.trim().is_empty()
    {
        chunks.push(text.to_string());
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiOutputTokenDetails {
    #[serde(default)]
    pub(super) reasoning_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiInputTokenDetails {
    #[serde(default)]
    pub(super) cached_tokens: Option<u64>,
}

pub(super) fn responses_reasoning_delta(event: &serde_json::Value) -> Option<String> {
    let event_type = event.get("type")?.as_str()?;
    if event_type == "response.reasoning_summary_text.delta"
        || event_type == "response.reasoning_text.delta"
        || event_type == "response.reasoning.delta"
    {
        return event
            .get("delta")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
    }
    None
}

pub(super) fn apply_chat_prompt_cache_hints(messages: &mut [chat_wire::ChatMessage]) {
    let flags = messages
        .iter()
        .map(|message| message.role == "system")
        .collect::<Vec<_>>();
    for index in prompt_cache::select_cache_target_indices(flags.as_slice()) {
        if let Some(message) = messages.get_mut(index) {
            message.copilot_cache_control = Some(prompt_cache::PromptCacheControl::ephemeral());
        }
    }
}

pub(super) fn clear_responses_prompt_cache_hints(input: &mut [OpenAiResponsesInputItem]) {
    for item in input {
        match item {
            OpenAiResponsesInputItem::Message(message) => message.copilot_cache_control = None,
            OpenAiResponsesInputItem::Reasoning(_) => {}
            OpenAiResponsesInputItem::FunctionCall(item) => item.copilot_cache_control = None,
            OpenAiResponsesInputItem::FunctionCallOutput(item) => item.copilot_cache_control = None,
        }
    }
}

pub(super) fn session_text_lossy(
    message: &Message,
    projected_parts: &[wire_message::WirePart],
) -> String {
    if projected_parts.is_empty() {
        message.as_text_lossy()
    } else {
        wire_message::parts_text_lossy(projected_parts)
    }
}

pub(super) fn normalize_domain(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

pub(super) fn openai_client_version() -> String {
    crate::provider::codex_package_version()
}

pub(super) fn append_query_param(endpoint: &str, key: &str, value: &str) -> String {
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    format!("{endpoint}{separator}{key}={value}")
}

pub(super) fn model_supports_input_modality(
    input_modality: &str,
    modality: ModelInputModality,
) -> bool {
    let normalized = input_modality.trim().to_ascii_lowercase();
    match modality {
        ModelInputModality::Text => normalized == "text",
        ModelInputModality::Image => normalized == "image",
        ModelInputModality::Document => normalized == "document",
        ModelInputModality::Audio => normalized == "audio",
        ModelInputModality::Video => normalized == "video",
        ModelInputModality::File => normalized == "file",
    }
}

pub(super) fn clamp_u64_to_u32(value: u64) -> u32 {
    value.min(u32::MAX as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::OpenAiResponsesResponse;

    #[test]
    fn failed_nonstream_response_preserves_nested_error_payload() {
        let response: OpenAiResponsesResponse = serde_json::from_value(serde_json::json!({
            "id": "resp_failed",
            "status": "failed",
            "error": {
                "code": "rate_limit_exceeded",
                "message": "slow down"
            }
        }))
        .expect("deserialize failed response");

        let event = response.failure_event().expect("failure event");
        assert_eq!(event["type"], "response.failed");
        assert_eq!(event["response"]["error"]["code"], "rate_limit_exceeded");
        assert_eq!(event["response"]["error"]["message"], "slow down");
    }

    #[test]
    fn incomplete_nonstream_response_is_terminal_without_output() {
        let response: OpenAiResponsesResponse = serde_json::from_value(serde_json::json!({
            "id": "resp_incomplete",
            "status": "incomplete",
            "incomplete_details": { "reason": "max_output_tokens" },
            "output": []
        }))
        .expect("deserialize incomplete response");

        assert!(response.failure_event().is_none());
        assert_eq!(response.terminal_reason(), Some("max_output_tokens"));
        assert!(response.unexpected_nonstream_status().is_none());
    }

    #[test]
    fn in_progress_nonstream_response_is_not_treated_as_an_empty_legacy_response() {
        let response: OpenAiResponsesResponse = serde_json::from_value(serde_json::json!({
            "status": "in_progress",
            "output": []
        }))
        .expect("deserialize in-progress response");

        assert_eq!(response.unexpected_nonstream_status(), Some("in_progress"));
        assert!(response.terminal_reason().is_none());
    }
}
