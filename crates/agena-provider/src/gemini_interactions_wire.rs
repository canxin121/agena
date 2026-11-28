//! Google Gemini Interactions API wire types, usage normalization, and error classification.
//!
//! Used by the `gemini_interactions` adapter (`POST <baseUrl>/interactions`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CompletionFinishReason, CompletionUsage, ProviderErrorKind};

/// Signature sentinel documented by Google for replaying function calls that
/// were not produced with a signed thought step by the current Gemini model.
pub const GEMINI_SKIP_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

// Matches Google's standard RPC detail `@type` suffixes for structured status errors.
const GOOGLE_RPC_ERROR_DETAIL_SUFFIX: &str = concat!("google.rpc.Error", "Info");
const GOOGLE_RPC_QUOTA_FAILURE_SUFFIX: &str = "google.rpc.QuotaFailure";

#[derive(Debug, Clone, Serialize)]
/// Request body for `POST /interactions`.
pub struct GeminiInteractionsRequest {
    pub model: String,
    pub store: bool,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<GeminiInteractionsTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GeminiInteractionsGenerationConfig>,
    pub input: Vec<GeminiInteractionsStep>,
}

#[derive(Debug, Clone, Serialize)]
/// Function tool declaration for the Google Interactions API.
pub struct GeminiInteractionsTool {
    #[serde(rename = "type")]
    pub tool_type: &'static str,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
/// Generation configuration for the Google Interactions API.
pub struct GeminiInteractionsGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_summaries: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
}

impl GeminiInteractionsGenerationConfig {
    pub fn is_empty(&self) -> bool {
        self.thinking_level.is_none()
            && self.thinking_summaries.is_none()
            && self.max_output_tokens.is_none()
            && self.tool_choice.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Content item inside an Interactions API step.
pub enum GeminiInteractionsContent {
    Text { text: String },
    Image { mime_type: String, data: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
/// Result payload for a `function_result` step (plain string or multimodal items).
pub enum GeminiInteractionsFunctionResultPayload {
    Text(String),
    Parts(Vec<GeminiInteractionsContent>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Typed stateless transcript step sent in `input` of an Interactions request.
pub enum GeminiInteractionsStep {
    UserInput {
        content: Vec<GeminiInteractionsContent>,
    },
    Thought {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<Vec<GeminiInteractionsContent>>,
    },
    ModelOutput {
        content: Vec<GeminiInteractionsContent>,
    },
    FunctionCall {
        id: String,
        name: String,
        arguments: Value,
    },
    FunctionResult {
        call_id: String,
        name: String,
        result: GeminiInteractionsFunctionResultPayload,
        #[serde(default, skip_serializing_if = "is_false")]
        is_error: bool,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Deserialize)]
/// Wire usage summary attached to `interaction.completed`.
pub struct GeminiInteractionsUsage {
    #[serde(default)]
    pub total_input_tokens: Option<u64>,
    #[serde(default)]
    pub total_cached_tokens: Option<u64>,
    #[serde(default)]
    pub total_output_tokens: Option<u64>,
    #[serde(default)]
    pub total_thought_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
}

/// Normalize Interactions API usage into Agena's provider-wide token accounting.
pub fn gemini_interactions_usage_to_completion(usage: GeminiInteractionsUsage) -> CompletionUsage {
    let total_input = usage.total_input_tokens.unwrap_or_default();
    let cache_read_tokens = usage.total_cached_tokens.unwrap_or_default();
    let output_tokens = usage.total_output_tokens.unwrap_or_default();
    let reasoning_tokens = usage.total_thought_tokens.unwrap_or_default();
    let known = total_input
        .saturating_add(output_tokens)
        .saturating_add(reasoning_tokens);
    CompletionUsage {
        requests: 1,
        input_tokens: total_input.saturating_sub(cache_read_tokens),
        output_tokens,
        reasoning_tokens,
        cache_read_tokens,
        other_tokens: usage.total_tokens.unwrap_or_default().saturating_sub(known),
        ..CompletionUsage::default()
    }
}

/// Map Interactions API final status to Agena's finish reason.
/// Returns `None` for terminal failure statuses (e.g. `failed`, `cancelled`).
pub fn gemini_interactions_finish_reason(status: &str) -> Option<CompletionFinishReason> {
    match status {
        "completed" => Some(CompletionFinishReason::Stop),
        "requires_action" => Some(CompletionFinishReason::ToolCalls),
        "incomplete" => Some(CompletionFinishReason::Length),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Classified HTTP failure from the Google Interactions API.
pub struct GoogleInteractionsErrorClassification {
    pub message: String,
    pub kind: ProviderErrorKind,
    pub retryable: bool,
}

#[derive(Debug, Default)]
struct ParsedGoogleErrorBody {
    message: String,
    reason: Option<String>,
    per_day_quota: Option<Option<u64>>,
}

enum RateLimitClass {
    PerMinute,
    ZeroDailyQuota,
    Daily { reset_in_message: bool },
}

/// Classify a non-2xx response from Google's Interactions API (`/interactions`)
/// into user-facing text, error kind, and retry decision.
///
/// Only per-minute 429s and 5xx server failures are retryable. Daily quota
/// exhaustion and zero-quota models fail immediately.
pub fn classify_google_interactions_error(
    status: u16,
    retry_after_header: Option<&str>,
    body: &str,
    model_id: &str,
) -> GoogleInteractionsErrorClassification {
    let parsed = parse_google_error_body(body);
    let compose = |hint: Option<String>| -> String {
        let mut parts = vec![status.to_string()];
        if !parsed.message.is_empty() {
            parts.push(parsed.message.clone());
        }
        if let Some(hint) = hint
            && !hint.is_empty()
        {
            parts.push(hint);
        }
        parts.join(" ")
    };

    if status == 429 {
        match classify_rate_limit(&parsed) {
            RateLimitClass::PerMinute => {
                return GoogleInteractionsErrorClassification {
                    message: compose(None),
                    kind: ProviderErrorKind::RateLimited,
                    retryable: true,
                };
            }
            RateLimitClass::ZeroDailyQuota => {
                let hint = format!(
                    "{model_id} has zero daily quota on this API key or project. Choose another model or check your Gemini API quota."
                );
                return GoogleInteractionsErrorClassification {
                    message: compose(Some(hint)),
                    kind: ProviderErrorKind::QuotaExceeded,
                    retryable: false,
                };
            }
            RateLimitClass::Daily { reset_in_message } => {
                let reset = if reset_in_message {
                    String::new()
                } else {
                    format_reset_duration(retry_after_header)
                };
                let hint = format!(
                    "Daily limit reached for {model_id}{reset}: switch to another model or wait for the quota to reset."
                );
                return GoogleInteractionsErrorClassification {
                    message: compose(Some(hint)),
                    kind: ProviderErrorKind::QuotaExceeded,
                    retryable: false,
                };
            }
        }
    }

    let kind = classify_non_429_kind(status, &parsed);
    let retryable = (500..600).contains(&status);
    GoogleInteractionsErrorClassification {
        message: compose(None),
        kind,
        retryable,
    }
}

fn classify_rate_limit(error: &ParsedGoogleErrorBody) -> RateLimitClass {
    let lower = error.message.to_ascii_lowercase();
    let daily_message = lower.contains("reached today's ") && lower.contains(" limit");
    let per_day_limit = extract_per_day_limit_from_message(&lower);

    if !daily_message && per_day_limit.is_none() && error.per_day_quota.is_none() {
        return RateLimitClass::PerMinute;
    }

    if matches!(error.per_day_quota, Some(Some(0))) || per_day_limit == Some(0) {
        return RateLimitClass::ZeroDailyQuota;
    }

    RateLimitClass::Daily {
        reset_in_message: daily_message,
    }
}

fn extract_per_day_limit_from_message(lower: &str) -> Option<u64> {
    if !lower.contains("rate limit exceeded") {
        return None;
    }
    let marker = "(limit: ";
    let start = lower.find(marker)? + marker.len();
    let rest = &lower[start..];
    let digits_len = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits_len == 0 {
        return None;
    }
    let after_digits = &rest[digits_len..];
    if after_digits.starts_with(" tokens per day)")
        || after_digits.starts_with(" requests per day)")
    {
        return rest[..digits_len].parse::<u64>().ok();
    }
    None
}

fn classify_non_429_kind(status: u16, error: &ParsedGoogleErrorBody) -> ProviderErrorKind {
    if status == 401 {
        return ProviderErrorKind::Authentication;
    }
    if status == 403 && error.reason.as_deref() == Some("SERVICE_DISABLED") {
        return ProviderErrorKind::Unavailable;
    }
    if status == 403 {
        ProviderErrorKind::Authentication
    } else if (500..600).contains(&status) {
        ProviderErrorKind::Unavailable
    } else {
        ProviderErrorKind::InvalidRequest
    }
}

fn format_reset_duration(retry_after_header: Option<&str>) -> String {
    let Some(seconds) = retry_after_header
        .map(str::trim)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
    else {
        return String::new();
    };
    let total_minutes = (seconds / 60.0).ceil() as u64;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours > 0 {
        format!(" (resets in {hours}h {minutes}m)")
    } else {
        format!(" (resets in {minutes}m)")
    }
}

fn parse_google_error_body(body: &str) -> ParsedGoogleErrorBody {
    let trimmed = body.trim();
    let json_text = if trimmed.starts_with('{') || trimmed.starts_with('[') {
        Some(trimmed)
    } else {
        trimmed
            .lines()
            .find_map(|line| line.strip_prefix("data:").map(str::trim))
    };

    let Some(parsed) = json_text.and_then(|text| serde_json::from_str::<Value>(text).ok()) else {
        return ParsedGoogleErrorBody {
            message: trimmed.to_owned(),
            ..ParsedGoogleErrorBody::default()
        };
    };

    let first = match &parsed {
        Value::Array(items) => items.first(),
        other => Some(other),
    };
    let Some(error_obj) = first
        .and_then(|item| item.get("error"))
        .and_then(Value::as_object)
    else {
        return ParsedGoogleErrorBody {
            message: trimmed.to_owned(),
            ..ParsedGoogleErrorBody::default()
        };
    };

    let Some(message) = error_obj
        .get("message")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
    else {
        return ParsedGoogleErrorBody {
            message: trimmed.to_owned(),
            ..ParsedGoogleErrorBody::default()
        };
    };

    let details = error_obj
        .get("details")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let reason = details.iter().find_map(|detail| {
        let type_url = detail.get("@type").and_then(Value::as_str)?;
        type_url
            .ends_with(GOOGLE_RPC_ERROR_DETAIL_SUFFIX)
            .then(|| {
                detail
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .flatten()
    });

    let per_day_quota = details.iter().find_map(|detail| {
        let type_url = detail.get("@type").and_then(Value::as_str)?;
        if !type_url.ends_with(GOOGLE_RPC_QUOTA_FAILURE_SUFFIX) {
            return None;
        }
        let violations = detail.get("violations").and_then(Value::as_array)?;
        violations.iter().find_map(|violation| {
            let quota_id = violation.get("quotaId").and_then(Value::as_str)?;
            if !quota_id.contains("PerDay") {
                return None;
            }
            let limit = violation.get("quotaValue").and_then(|value| match value {
                Value::Number(num) => num.as_u64(),
                Value::String(text) => text.trim().parse::<u64>().ok(),
                _ => None,
            });
            Some(limit)
        })
    });

    ParsedGoogleErrorBody {
        message,
        reason,
        per_day_quota,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gemini_interactions_thinking_level;
    use agena_domain::{ReasoningEffort, ThinkingRequest};
    use serde_json::json;

    #[test]
    fn daily_limit_429_is_not_retryable_and_formats_reset_time() {
        let body = json!({
            "error": {
                "message": "Resource exhausted.",
                "details": [{
                    "@type": "type.googleapis.com/google.rpc.QuotaFailure",
                    "violations": [{
                        "quotaId": "GenerateRequestsPerDayPerProjectPerModel",
                        "quotaValue": "1000000"
                    }]
                }]
            }
        })
        .to_string();

        let classified = classify_google_interactions_error(
            429,
            Some("19104"),
            body.as_str(),
            "gemini-4-hydrogen",
        );
        assert!(!classified.retryable);
        assert_eq!(classified.kind, ProviderErrorKind::QuotaExceeded);
        assert!(
            classified
                .message
                .contains("Daily limit reached for gemini-4-hydrogen (resets in 5h 19m)")
        );

        let round_hour = classify_google_interactions_error(
            429,
            Some("7199"),
            body.as_str(),
            "gemini-4-hydrogen",
        );
        assert!(round_hour.message.contains("(resets in 2h 0m)"));
    }

    #[test]
    fn daily_limit_message_keeps_google_reset_text() {
        let body = json!({
            "error": {
                "code": "too_many_requests",
                "message": "You've reached today's Gemini limit for gemini-4-hydrogen. Your daily limit refreshes in 5 hours, 18 minutes."
            }
        })
        .to_string();

        let classified = classify_google_interactions_error(
            429,
            Some("19104"),
            body.as_str(),
            "gemini-4-hydrogen",
        );
        assert!(!classified.retryable);
        assert!(!classified.message.contains("(resets in"));
        assert!(
            classified
                .message
                .contains("Daily limit reached for gemini-4-hydrogen:")
        );
    }

    #[test]
    fn zero_daily_limit_marks_quota_exceeded() {
        let body = json!({
            "error": {
                "message": "Rate limit exceeded for model gemini-4-argon (limit: 0 tokens per day)"
            }
        })
        .to_string();
        let classified =
            classify_google_interactions_error(429, None, body.as_str(), "gemini-4-argon");
        assert!(!classified.retryable);
        assert_eq!(classified.kind, ProviderErrorKind::QuotaExceeded);
        assert!(classified.message.contains("zero daily quota"));
    }

    #[test]
    fn per_minute_429_remains_retryable() {
        let body = json!({ "error": { "message": "Too many requests." } }).to_string();
        let classified =
            classify_google_interactions_error(429, None, body.as_str(), "gemini-4-hydrogen");
        assert!(classified.retryable);
        assert_eq!(classified.kind, ProviderErrorKind::RateLimited);
    }

    #[test]
    fn distinguishes_disabled_service_and_auth_403_errors() {
        let detail_type = format!("type.googleapis.com/{GOOGLE_RPC_ERROR_DETAIL_SUFFIX}");
        let disabled_body = json!([{
            "error": {
                "code": 403,
                "message": "Permission denied.",
                "details": [{ "@type": detail_type, "reason": "SERVICE_DISABLED" }]
            }
        }])
        .to_string();
        let disabled = classify_google_interactions_error(
            403,
            None,
            disabled_body.as_str(),
            "gemini-4-hydrogen",
        );
        assert_eq!(disabled.kind, ProviderErrorKind::Unavailable);
        assert!(!disabled.retryable);

        let auth_denied = classify_google_interactions_error(
            403,
            None,
            r#"{"error":{"message":"API key not valid."}}"#,
            "gemini-4-hydrogen",
        );
        assert_eq!(auth_denied.kind, ProviderErrorKind::Authentication);
        assert!(!auth_denied.retryable);
    }

    #[test]
    fn parses_sse_error_frame_body() {
        let body = "event: error\ndata: {\"error\":{\"message\":\"Function call is missing a thought_signature.\"},\"event_type\":\"error\"}\n\n";
        let classified = classify_google_interactions_error(400, None, body, "gemini-4-hydrogen");
        assert_eq!(
            classified.message,
            "400 Function call is missing a thought_signature."
        );
        assert!(!classified.retryable);
    }

    #[test]
    fn usage_normalization_subtracts_cached_and_preserves_reasoning() {
        let usage = gemini_interactions_usage_to_completion(GeminiInteractionsUsage {
            total_input_tokens: Some(114),
            total_cached_tokens: Some(10),
            total_output_tokens: Some(26),
            total_thought_tokens: Some(44),
            total_tokens: Some(184),
        });
        assert_eq!(usage.input_tokens, 104);
        assert_eq!(usage.cache_read_tokens, 10);
        assert_eq!(usage.output_tokens, 26);
        assert_eq!(usage.reasoning_tokens, 44);
        assert_eq!(usage.other_tokens, 0);
    }

    #[test]
    fn gemini_4_thinking_level_clamps_unsupported_levels() {
        assert_eq!(
            gemini_interactions_thinking_level(
                "gemini-4-hydrogen",
                Some(&ThinkingRequest::Effort {
                    effort: ReasoningEffort::Minimal
                })
            ),
            Some("low")
        );
        assert_eq!(
            gemini_interactions_thinking_level(
                "gemini-4-hydrogen",
                Some(&ThinkingRequest::Effort {
                    effort: ReasoningEffort::Xhigh
                })
            ),
            Some("xhigh")
        );
        assert_eq!(
            gemini_interactions_thinking_level(
                "gemini-4-carbon",
                Some(&ThinkingRequest::Effort {
                    effort: ReasoningEffort::Xhigh
                })
            ),
            Some("high")
        );
        assert_eq!(
            gemini_interactions_thinking_level(
                "gemini-3.5-flash-lite",
                Some(&ThinkingRequest::Effort {
                    effort: ReasoningEffort::Minimal
                })
            ),
            Some("minimal")
        );
        assert_eq!(
            gemini_interactions_thinking_level(
                "gemini-3.5-flash-lite",
                Some(&ThinkingRequest::Disabled)
            ),
            None
        );
    }
}
