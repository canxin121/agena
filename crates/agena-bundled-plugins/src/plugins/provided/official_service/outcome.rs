//! Transport success, provider outcome and local persistence are separate facts.
//! This classifier recognizes tested envelope fields; unknown new statuses
//! remain unknown rather than being silently interpreted as completion.
use serde_json::Value;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    Completed,
    Pending,
    Refused,
    Incomplete,
    Failed,
    Unknown,
}
impl Outcome {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Pending => "pending_calls",
            Self::Refused => "refused",
            Self::Incomplete => "incomplete",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}
pub(super) fn classify(provider: &str, value: &Value, pending: bool) -> Outcome {
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return Outcome::Failed;
    }
    if matches!(
        value.get("status").and_then(Value::as_str),
        Some("failed" | "cancelled" | "expired")
    ) {
        return Outcome::Failed;
    }
    let content = value
        .get("content")
        .or_else(|| value.get("output"))
        .and_then(Value::as_array);
    if content.is_some_and(|items| {
        items.iter().any(|item| {
            item.get("is_error").and_then(Value::as_bool) == Some(true)
                || item.get("status").and_then(Value::as_str) == Some("failed")
        })
    }) {
        return Outcome::Failed;
    }
    if content.is_some_and(|items| {
        items.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("refusal")
                || item
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|parts| {
                        parts
                            .iter()
                            .any(|part| part.get("type").and_then(Value::as_str) == Some("refusal"))
                    })
        })
    }) {
        return Outcome::Refused;
    }
    if pending {
        return Outcome::Pending;
    }
    if let Some(status) = value.get("status").and_then(Value::as_str) {
        return match status {
            "completed" => Outcome::Completed,
            "incomplete" => Outcome::Incomplete,
            "queued" | "in_progress" | "requires_action" => Outcome::Pending,
            _ => Outcome::Unknown,
        };
    }
    if provider == "claude" {
        return match value.get("stop_reason").and_then(Value::as_str) {
            Some("end_turn" | "stop_sequence") => Outcome::Completed,
            Some("tool_use") => Outcome::Pending,
            Some("max_tokens" | "pause_turn") => Outcome::Incomplete,
            Some("refusal") => Outcome::Refused,
            _ => Outcome::Unknown,
        };
    }
    if provider == "gemini" {
        if value
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
            .is_some_and(|s| s != "BLOCK_REASON_UNSPECIFIED")
        {
            return Outcome::Refused;
        }
        if let Some(candidates) = value.get("candidates").and_then(Value::as_array) {
            let reasons: Vec<_> = candidates
                .iter()
                .filter_map(|c| c.get("finishReason").and_then(Value::as_str))
                .collect();
            if reasons.iter().any(|r| {
                matches!(
                    *r,
                    "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII"
                )
            }) {
                return Outcome::Refused;
            }
            if reasons.contains(&"MAX_TOKENS") {
                return Outcome::Incomplete;
            }
            if !candidates.is_empty()
                && reasons.len() == candidates.len()
                && reasons.iter().all(|r| *r == "STOP")
            {
                return Outcome::Completed;
            }
        }
    }
    // The OpenAI Images endpoint has no completion status. Nonempty image data
    // is evidence of a returned image response, not of local file persistence.
    if value
        .get("data")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            !items.is_empty()
                && items
                    .iter()
                    .all(|item| item.get("b64_json").is_some() || item.get("url").is_some())
        })
    {
        return Outcome::Completed;
    }
    Outcome::Unknown
}
