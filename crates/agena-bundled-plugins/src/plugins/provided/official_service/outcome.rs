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
    // A legacy Images endpoint has no completion status. Nonempty image data
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
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn http_200_envelopes_preserve_failure_refusal_incomplete_and_unknown() {
        for (provider, value, expected) in [
            (
                "chatgpt",
                json!({"status":"failed","error":{"message":"failed"}}),
                Outcome::Failed,
            ),
            (
                "chatgpt",
                json!({"status":"incomplete"}),
                Outcome::Incomplete,
            ),
            (
                "chatgpt",
                json!({"status":"completed","output":[{"content":[{"type":"refusal"}]}]}),
                Outcome::Refused,
            ),
            (
                "claude",
                json!({"stop_reason":"max_tokens"}),
                Outcome::Incomplete,
            ),
            (
                "claude",
                json!({"stop_reason":"end_turn"}),
                Outcome::Completed,
            ),
            (
                "gemini",
                json!({"promptFeedback":{"blockReason":"SAFETY"}}),
                Outcome::Refused,
            ),
            (
                "gemini",
                json!({"candidates":[{"finishReason":"STOP"}]}),
                Outcome::Completed,
            ),
            (
                "chatgpt",
                json!({"status":"new_unrecognized_status"}),
                Outcome::Unknown,
            ),
        ] {
            assert_eq!(classify(provider, &value, false), expected, "{value}");
        }
        assert_eq!(
            classify("chatgpt", &json!({"status":"completed"}), true),
            Outcome::Pending
        );
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::super::*;
    #[tokio::test]
    async fn charged_response_is_not_lost_when_receipt_storage_fails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".agena"), "blocks artifact directory").unwrap();
        let host: Arc<dyn HostClient> = Arc::new(agena_plugin_host::sdk::NoopHostClient);
        let response = ProviderHttpResponse {
            value: serde_json::json!({"id":"response_fixture","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"returned response"}]}],"usage":{"input_tokens":2,"output_tokens":3}}),
            request_id: Some("request_fixture".into()),
        };
        let output = provider_output(
            &host,
            dir.path(),
            "chatgpt",
            "web_search",
            "fixture-model",
            "Fixture",
            ProviderUsageKind::OpenAiResponses,
            response,
        )
        .await
        .unwrap();
        let payload = output.payload.unwrap();
        assert_eq!(payload["response_received"], true);
        assert_eq!(payload["response_id"], "response_fixture");
        assert_eq!(payload["outcome"], "completed");
        assert!(payload["response_receipt"].is_null());
        assert!(
            !payload["persistence_warnings"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(payload["safe_to_reexecute_automatically"], false);
        assert!(
            output
                .metadata
                .contains_key(PROVIDER_TOOL_USAGE_METADATA_KEY)
        );
    }
}

#[cfg(test)]
mod partial_images_tests {
    use super::super::*;
    #[tokio::test]
    async fn a_bad_later_image_does_not_hide_an_earlier_saved_image() {
        let dir = tempfile::tempdir().unwrap();
        let host: Arc<dyn HostClient> = Arc::new(agena_plugin_host::sdk::NoopHostClient);
        let value = serde_json::json!({"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9WP0fooAAAAASUVORK5CYII="},{"b64_json":"not/base64!!!"}]});
        let (attachments, warnings) =
            persist_images(&host, dir.path(), "fixture", "fixture", &value).await;
        assert_eq!(attachments.len(), 1);
        assert_eq!(warnings.len(), 1);
        match &attachments[0].source {
            AttachmentSource::LocalPath { path } => assert!(Path::new(path).is_file()),
            _ => panic!("expected persisted image"),
        }
    }
}
