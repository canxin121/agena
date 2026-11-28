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
    pub reasoning_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens_details: Option<OpenAiOutputTokenDetails>,
    #[serde(default)]
    pub input_tokens_details: Option<OpenAiInputTokenDetails>,
    /// xAI reports exact request cost in 10^-10 USD ticks.
    #[serde(default)]
    pub cost_in_usd_ticks: Option<u64>,
    /// Weighted rollout units reported by the Responses provider.
    #[serde(default, deserialize_with = "rollout_units")]
    pub codex_rollout_budget_units: Option<f64>,
}

fn rollout_units<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<f64>, D::Error> {
    let units = Option::<f64>::deserialize(input)?;
    if units.is_some_and(|value| !value.is_finite() || value < 0.0) {
        return Err(serde::de::Error::custom(
            "usage.codex_rollout_budget_units must be finite and non-negative",
        ));
    }
    Ok(units)
}

pub type OpenAiOutputTokenDetails = crate::ChatOutputTokensDetails;

pub type OpenAiInputTokenDetails = crate::ChatInputTokensDetails;

pub fn openai_responses_usage_to_completion(usage: OpenAiUsage) -> crate::CompletionUsage {
    let mut normalized = crate::chat_usage_to_completion(crate::ChatUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        input_tokens_details: usage.input_tokens_details,
        output_tokens_details: usage.output_tokens_details,
        cost_in_usd_ticks: usage.cost_in_usd_ticks,
        ..Default::default()
    });
    if let Some(quantity) = usage.codex_rollout_budget_units {
        normalized.billable_items.push(crate::BillableUsageItem {
            kind: "codex_rollout_budget_units".into(),
            unit: "codex_rollout_budget_unit".into(),
            quantity,
            pricing_source: Some("provider_reported".into()),
            ..Default::default()
        });
    }
    normalized
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
    use super::*;
    use serde_json::json;

    #[test]
    fn imported_codex_rollout_units_are_optional_and_nonnegative() {
        for value in [json!(-0.5), json!("2.5"), json!(true)] {
            assert!(
                serde_json::from_value::<OpenAiUsage>(json!({"codex_rollout_budget_units": value}))
                    .is_err()
            );
        }
        let observed = |wire| {
            openai_responses_usage_to_completion(serde_json::from_value(wire).unwrap())
                .billable_items
        };
        assert!(observed(json!({"input_tokens": 2})).is_empty());
        let zero = observed(json!({"input_tokens": 2, "codex_rollout_budget_units": 0}));
        assert_eq!(zero[0].quantity, 0.0);
        assert_eq!(zero[0].kind, "codex_rollout_budget_units");
    }
}
