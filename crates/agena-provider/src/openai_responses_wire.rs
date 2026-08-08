use serde::Deserialize;

#[derive(Debug, Deserialize)]
/// Wire shape of an OpenAI Responses API response.
pub struct OpenAiResponsesResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub output_text: Option<String>,
    #[serde(default)]
    pub output: Option<Vec<OpenAiOutputItem>>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
    #[serde(default)]
    pub incomplete_details: Option<OpenAiIncompleteDetails>,
    #[serde(default)]
    pub usage: Option<OpenAiUsage>,
}

impl OpenAiResponsesResponse {
    pub fn failure_event(&self) -> Option<serde_json::Value> {
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

    pub fn terminal_reason(&self) -> Option<&str> {
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

    pub fn unexpected_nonstream_status(&self) -> Option<&str> {
        self.status.as_deref().filter(|status| {
            !matches!(*status, "completed" | "incomplete" | "failed" | "cancelled")
        })
    }
}

#[derive(Debug, Deserialize)]
/// Wire shape of OpenAI incomplete details.
pub struct OpenAiIncompleteDetails {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of an OpenAI output item.
pub struct OpenAiOutputItem {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
    #[serde(default)]
    pub content: Option<Vec<OpenAiOutputContent>>,
    #[serde(default)]
    pub summary: Option<Vec<OpenAiReasoningSummaryContent>>,
    #[serde(default)]
    pub encrypted_content: Option<String>,
    /// Terminal base64 payload returned by an `image_generation_call` item.
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub revised_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of OpenAI output content.
pub struct OpenAiOutputContent {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of OpenAI reasoning summary content.
pub struct OpenAiReasoningSummaryContent {
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of OpenAI usage counters.
pub struct OpenAiUsage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens_details: Option<OpenAiOutputTokenDetails>,
    #[serde(default)]
    pub input_tokens_details: Option<OpenAiInputTokenDetails>,
    /// xAI reports exact request cost in 10^-10 USD ticks.
    #[serde(default)]
    pub cost_in_usd_ticks: Option<u64>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of OpenAI output token details.
pub struct OpenAiOutputTokenDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of OpenAI input token details.
pub struct OpenAiInputTokenDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
}

pub fn openai_responses_reasoning_delta(event: &serde_json::Value) -> Option<String> {
    let event_type = event.get("type")?.as_str()?;
    if event_type == "response.reasoning_summary_text.delta"
        || event_type == "response.reasoning_text.delta"
        || event_type == "response.reasoning.delta"
    {
        return event
            .get("delta")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::OpenAiResponsesResponse;

    #[test]
    fn failed_nonstream_response_preserves_nested_error_payload() {
        let response: OpenAiResponsesResponse = serde_json::from_value(serde_json::json!({
            "id": "resp_failed",
            "status": "failed",
            "error": { "code": "rate_limit_exceeded", "message": "slow down" }
        }))
        .expect("deserialize failed response");

        let event = response.failure_event().expect("failure event");
        assert_eq!(event["type"], "response.failed");
        assert_eq!(event["response"]["error"]["code"], "rate_limit_exceeded");
    }

    #[test]
    fn incomplete_nonstream_response_is_terminal_without_output() {
        let response: OpenAiResponsesResponse = serde_json::from_value(serde_json::json!({
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
    fn in_progress_nonstream_response_is_not_treated_as_terminal() {
        let response: OpenAiResponsesResponse = serde_json::from_value(serde_json::json!({
            "status": "in_progress",
            "output": []
        }))
        .expect("deserialize in-progress response");

        assert_eq!(response.unexpected_nonstream_status(), Some("in_progress"));
        assert!(response.terminal_reason().is_none());
    }
}
