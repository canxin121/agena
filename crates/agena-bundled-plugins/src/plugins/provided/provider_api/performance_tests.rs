use super::*;

fn input(value: Value) -> ResourceInput {
    serde_json::from_value(value).unwrap()
}

#[test]
fn performance_resource_duration_only_pairs_single_synchronous_create() {
    let completed = json!({"response":{"status":"completed","usage":{"output_tokens":30}}});
    assert_eq!(
        synchronous_request_duration(
            Family::Inference,
            &input(json!({"action":"create"})),
            &completed,
            Some(1_000),
        ),
        Some(1_000)
    );
    for family in [
        Family::Inference,
        Family::Tasks,
        Family::Images,
        Family::Embeddings,
    ] {
        for action in ["wait", "get", "results"] {
            assert_eq!(
                synchronous_request_duration(
                    family,
                    &input(json!({"action":action})),
                    &completed,
                    Some(1_000),
                ),
                None
            );
        }
    }
    for family in [
        Family::Batches,
        Family::Agents,
        Family::Files,
        Family::Tokens,
    ] {
        assert_eq!(
            synchronous_request_duration(
                family,
                &input(json!({"action":"create"})),
                &completed,
                Some(1_000),
            ),
            None
        );
    }
    assert_eq!(
        synchronous_request_duration(
            Family::Inference,
            &input(json!({"action":"create","stream":true})),
            &completed,
            Some(1_000),
        ),
        None
    );
    for pending_response in [
        json!({"response":{"status":"queued"}}),
        json!({"response":{"status":"in_progress"}}),
        json!({"response":{"name":"operations/123","done":false}}),
    ] {
        assert_eq!(
            synchronous_request_duration(
                Family::Inference,
                &input(json!({"action":"create"})),
                &pending_response,
                Some(1_000),
            ),
            None
        );
    }
}

#[test]
fn performance_resource_duration_excludes_failed_or_incomplete_usage() {
    let create = input(json!({"action":"create"}));
    for status in [
        "failed",
        "cancelled",
        "expired",
        "incomplete",
        "unknown_status",
    ] {
        let response = json!({"response":{"status":status,"usage":{"output_tokens":30}}});
        assert_eq!(
            synchronous_request_duration(Family::Inference, &create, &response, Some(1_000)),
            None,
            "status={status}",
        );
    }
    for response in [
        json!({"response":{"done":true,"error":{"code":500},"usage":{"output_tokens":30}}}),
        json!({"response":{"done":true,"response":{"error":{"code":500},"usage":{"output_tokens":30}}}}),
        json!({"response":{"done":true,"response":{"status":"incomplete","usage":{"output_tokens":30}}}}),
        json!({"response":{"status":"completed","incomplete_details":{"reason":"max_output_tokens"},"usage":{"output_tokens":30}}}),
        json!({"response":{"stop_reason":"max_tokens","usage":{"output_tokens":30}}}),
        json!({"response":{"candidates":[{"finishReason":"MAX_TOKENS"}],"usageMetadata":{"candidatesTokenCount":30}}}),
        json!({"response":{"choices":[{"finish_reason":"length"}],"usage":{"completion_tokens":30}}}),
    ] {
        assert_eq!(
            synchronous_request_duration(Family::Inference, &create, &response, Some(1_000)),
            None,
            "response={response}",
        );
    }
}

#[test]
fn performance_resource_duration_accepts_successful_responses_without_status() {
    let create = input(json!({"action":"create"}));
    for response in [
        json!({"response":{"stop_reason":"end_turn","usage":{"output_tokens":30}}}),
        json!({"response":{"candidates":[{"finishReason":"STOP"}],"usageMetadata":{"candidatesTokenCount":30}}}),
        json!({"response":{"choices":[{"finish_reason":"stop"}],"usage":{"completion_tokens":30}}}),
        json!({"response":{"data":[{"embedding":[0.1]}],"usage":{"prompt_tokens":10}}}),
        json!({"response":{"done":true,"response":{"usageMetadata":{"candidatesTokenCount":30}}}}),
        json!({"response":{"status":"succeeded","usage":{"output_tokens":30}}}),
        json!({"response":{"status":"completed","metadata":{"state":"project-label"},"usage":{"output_tokens":30}}}),
    ] {
        assert_eq!(
            synchronous_request_duration(Family::Inference, &create, &response, Some(1_000)),
            Some(1_000),
            "response={response}",
        );
    }
}

#[test]
fn performance_resource_duration_never_comes_from_vendor_response_fields() {
    let response = json!({"response":{"status":"completed","request_duration_ms":1_000,
        "usage":{"output_tokens":30}}});
    assert_eq!(
        synchronous_request_duration(
            Family::Inference,
            &input(json!({"action":"create"})),
            &response,
            None,
        ),
        None
    );
}
