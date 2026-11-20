use super::*;
use serde_json::json;

#[test]
fn performance_sync_http_uses_reported_generation_tokens_and_has_no_streaming_rate() {
    for (kind, provider, response, expected) in [
        (
            ProviderUsageKind::OpenAiResponses,
            "chatgpt",
            json!({
                "usage":{"input_tokens":10,"output_tokens":100,"output_tokens_details":{"reasoning_tokens":20}}
            }),
            100,
        ),
        (
            ProviderUsageKind::AnthropicMessages,
            "claude",
            json!({
                "usage":{"input_tokens":10,"output_tokens":50}
            }),
            50,
        ),
        (
            ProviderUsageKind::GeminiGenerateContent,
            "gemini",
            json!({
                "usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":20,"thoughtsTokenCount":30}
            }),
            50,
        ),
        (
            ProviderUsageKind::GeminiInteractions,
            "gemini",
            json!({
                "usage":{"total_input_tokens":10,"total_output_tokens":20,"total_thought_tokens":30}
            }),
            50,
        ),
    ] {
        let mut usage = parse_provider_usage(kind, provider, "inference", "model", &response);
        let original_requests = usage.requests;
        record_request_performance(&mut usage, &response, Some(1_000));
        let sample = usage.request_performance.unwrap();
        assert_eq!(sample.generated_tokens, Some(expected));
        assert_eq!(sample.end_to_end_tokens_per_second(), Some(expected as f64));
        assert_eq!(sample.tokens_per_second(), None);
        assert_eq!(sample.ttft_ms, None);
        assert_eq!(sample.generation_ms, None);
        assert_eq!(usage.performance, vec![sample]);
        assert_eq!(usage.requests, original_requests);
    }
}

#[test]
fn performance_sync_http_keeps_absent_output_counters_unknown() {
    for response in [
        json!({}),
        json!({"usage":{}}),
        json!({"usage":{"input_tokens":100}}),
        json!({"usageMetadata":{"promptTokenCount":100}}),
    ] {
        let mut usage = CompletionUsage::default();
        record_request_performance(&mut usage, &response, Some(500));
        let sample = usage.request_performance.unwrap();
        assert_eq!(sample.generated_tokens, None);
        assert_eq!(sample.end_to_end_tokens_per_second(), None);
    }
    let response = json!({"usage":{"output_tokens":0}});
    let mut zero = CompletionUsage::default();
    record_request_performance(&mut zero, &response, Some(500));
    assert_eq!(zero.request_performance.unwrap().generated_tokens, Some(0));
}

#[test]
fn performance_sync_http_without_matching_timing_adds_no_sample() {
    let response = json!({"usage":{"input_tokens":10,"output_tokens":20}});
    let mut usage = parse_provider_usage(
        ProviderUsageKind::OpenAiResponses,
        "chatgpt",
        "inference",
        "model",
        &response,
    );
    let before = usage.clone();
    record_request_performance(&mut usage, &response, None);
    assert_eq!(usage, before);
}
