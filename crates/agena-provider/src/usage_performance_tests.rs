use super::*;
use agena_domain::RequestPerformance;

fn measured(duration_ms: u64, tokens: u64) -> CompletionUsage {
    let sample = RequestPerformance {
        duration_ms,
        ttft_ms: Some(duration_ms / 4),
        generation_ms: Some(duration_ms / 2),
        generated_tokens: Some(tokens),
    };
    CompletionUsage {
        requests: 1,
        output_tokens: tokens,
        performance: vec![sample],
        request_performance: Some(sample),
        ..Default::default()
    }
}

#[test]
fn performance_round_trips_and_old_usage_has_no_measurement() {
    let legacy: CompletionUsage = serde_json::from_value(serde_json::json!({
        "input_tokens": 100,
        "output_tokens": 20
    }))
    .unwrap();
    assert!(legacy.performance.is_empty());
    assert_eq!(legacy.request_performance, None);
    let usage = measured(2_000, 100);
    let decoded: CompletionUsage =
        serde_json::from_value(serde_json::to_value(&usage).unwrap()).unwrap();
    assert_eq!(decoded, usage);
}

#[test]
fn performance_merge_preserves_final_request_and_all_samples() {
    let mut final_request = measured(1_000, 20);
    let snapshot = final_request.request_performance;
    let rejected_request = measured(3_000, 60);
    final_request.add_assign(&rejected_request);
    assert_eq!(final_request.requests, 2);
    assert_eq!(final_request.output_tokens, 80);
    assert_eq!(final_request.performance.len(), 2);
    assert_eq!(final_request.request_performance, snapshot);
    assert_eq!(snapshot.unwrap().generated_tokens, Some(20));
}

#[test]
fn performance_deltas_preserve_sample_multiplicity_and_do_not_repeat() {
    let earlier = measured(1_000, 20);
    let mut total = earlier.clone();
    total.add_assign(&earlier);
    let delta = total.saturating_sub(&earlier);
    assert_eq!(delta.requests, 1);
    assert_eq!(delta.performance, earlier.performance);
    let empty = total.saturating_sub(&total);
    assert!(empty.performance.is_empty());
    assert_eq!(empty.request_performance, None);
    assert!(!empty.has_own_usage());
}

#[test]
fn performance_flattening_includes_attributed_requests_once() {
    let mut own = measured(1_000, 20);
    let nested = measured(2_000, 40);
    own.attributed_usage.push(AttributedCompletionUsage {
        provider_id: "image-provider".into(),
        model_id: "image-model".into(),
        operation: "generate".into(),
        request_id: None,
        usage: Box::new(nested.clone()),
    });
    let flat = own.flattened();
    assert_eq!(flat.performance.len(), 2);
    assert_eq!(flat.requests, 2);
    assert!(flat.attributed_usage.is_empty());
    assert_eq!(flat.request_performance, own.request_performance);
    assert_eq!(flat.flattened(), flat);
}
