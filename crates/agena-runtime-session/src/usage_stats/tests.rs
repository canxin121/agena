use super::*;
use agena_domain::RequestPerformance;
use agena_provider::{AttributedCompletionUsage, CompletionUsage};

fn record(usage: CompletionUsage) -> UsageStatRecord {
    UsageStatRecord {
        session_id: 1,
        session_title: "Performance".into(),
        is_subagent: false,
        created_at: "2026-01-02T23:00:00Z".parse().unwrap(),
        provider_id: "provider-a".into(),
        model_id: "model-a".into(),
        usage,
    }
}

fn summary(records: &[UsageStatRecord]) -> UsageStats {
    summarize_usage_records(records, &UsageStatsQuery::custom(None, None), Utc::now())
}

fn close(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("measurement");
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

#[test]
fn performance_uses_matched_tokens_and_durations_across_repair_attempts() {
    let measured = record(CompletionUsage {
        requests: 2,
        output_tokens: 900,
        reasoning_tokens: 200,
        performance: vec![
            RequestPerformance {
                duration_ms: 1_000,
                ttft_ms: Some(200),
                generation_ms: Some(500),
                generated_tokens: Some(100),
            },
            RequestPerformance {
                duration_ms: 12_000,
                ttft_ms: Some(2_000),
                generation_ms: Some(10_000),
                generated_tokens: Some(1_000),
            },
        ],
        ..Default::default()
    });
    let legacy = record(CompletionUsage {
        requests: 1,
        output_tokens: 500_000,
        ..Default::default()
    });
    let timing_only = record(CompletionUsage {
        requests: 1,
        performance: vec![RequestPerformance {
            duration_ms: 10_000,
            ttft_ms: Some(5_000),
            generation_ms: Some(1_000),
            generated_tokens: None,
        }],
        ..Default::default()
    });
    let non_streaming = record(CompletionUsage {
        requests: 1,
        output_tokens: 200,
        performance: vec![RequestPerformance {
            duration_ms: 4_000,
            generated_tokens: Some(200),
            ..Default::default()
        }],
        ..Default::default()
    });
    let stats = summary(&[measured, legacy, timing_only, non_streaming]);
    let performance = &stats.totals.performance;
    assert_eq!(stats.totals.requests, 5);
    assert_eq!(stats.totals.output_tokens, 501_100);
    assert_eq!(performance.request_count, 4);
    assert_eq!(performance.ttft_count, 3);
    assert_eq!(performance.generation_count, 2);
    assert_eq!(performance.throughput_count, 3);
    assert_eq!(performance.generation_tokens, 1_100);
    assert_eq!(performance.timed_tokens, 1_300);
    close(performance.tokens_per_second, 1_100_000.0 / 10_500.0);
    close(
        performance.end_to_end_tokens_per_second,
        1_300_000.0 / 17_000.0,
    );
    close(performance.average_duration_ms, 6_750.0);
    close(performance.average_ttft_ms, 2_400.0);
    assert_eq!(performance.p50_duration_ms, Some(4_000));
    assert_eq!(performance.p95_duration_ms, Some(12_000));
    assert_eq!(performance.p50_ttft_ms, Some(2_000));
    assert_eq!(performance.p95_ttft_ms, Some(5_000));
    assert_eq!(&stats.by_model[0].totals.performance, performance);
    assert_eq!(&stats.by_provider[0].totals.performance, performance);
    assert_eq!(&stats.by_session[0].totals.performance, performance);
    assert_eq!(&stats.by_day[0].totals.performance, performance);
}

#[test]
fn performance_follows_attribution_filters_and_local_day_boundaries() {
    let usage = CompletionUsage {
        requests: 1,
        performance: vec![RequestPerformance {
            duration_ms: 1_000,
            ..Default::default()
        }],
        attributed_usage: vec![AttributedCompletionUsage {
            provider_id: "tool-provider".into(),
            model_id: "tool-model".into(),
            operation: "completion".into(),
            request_id: Some("tool-1".into()),
            usage: Box::new(CompletionUsage {
                requests: 1,
                performance: vec![RequestPerformance {
                    duration_ms: 3_000,
                    generated_tokens: Some(300),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        }],
        ..Default::default()
    };
    let matching = record(usage);
    let mut subagent = matching.clone();
    subagent.session_id = 2;
    subagent.is_subagent = true;
    let mut out_of_range = matching.clone();
    out_of_range.created_at = "2026-01-01T00:00:00Z".parse().unwrap();
    let query = UsageStatsQuery::custom(
        Some("2026-01-02T22:00:00Z".parse().unwrap()),
        Some("2026-01-03T00:00:00Z".parse().unwrap()),
    )
    .with_timezone_offset(120)
    .with_filters(
        vec!["tool-provider".into()],
        vec!["tool-model".into()],
        vec![],
        false,
    );
    let stats = summarize_usage_records(&[matching, subagent, out_of_range], &query, Utc::now());
    assert_eq!(stats.totals.performance.request_count, 1);
    assert_eq!(stats.totals.sessions, 1);
    assert_eq!(stats.by_provider.len(), 1);
    assert_eq!(stats.by_provider[0].provider_id, "tool-provider");
    assert_eq!(stats.by_model[0].model_id, "tool-model");
    assert_eq!(stats.by_day[0].date, "2026-01-03");
    close(stats.totals.performance.end_to_end_tokens_per_second, 100.0);
    assert_eq!(stats.totals.performance.tokens_per_second, None);
    assert_eq!(stats.totals.performance.average_ttft_ms, None);
}

#[test]
fn performance_handles_unknown_zero_and_inconsistent_timings() {
    let stats = summary(&[record(CompletionUsage {
        requests: 4,
        performance: vec![
            RequestPerformance {
                ttft_ms: Some(0),
                generation_ms: Some(0),
                generated_tokens: Some(100),
                ..Default::default()
            },
            RequestPerformance {
                duration_ms: 1_000,
                ttft_ms: Some(2_000),
                generation_ms: Some(100),
                generated_tokens: Some(100),
            },
            RequestPerformance {
                duration_ms: 1_000,
                ttft_ms: Some(100),
                generation_ms: Some(950),
                generated_tokens: None,
            },
            RequestPerformance {
                duration_ms: 1_000,
                ttft_ms: Some(100),
                generation_ms: Some(500),
                generated_tokens: Some(1),
            },
        ],
        ..Default::default()
    })]);
    let performance = stats.totals.performance;
    assert_eq!(performance.request_count, 4);
    assert_eq!(performance.ttft_count, 3);
    assert_eq!(performance.generation_count, 0);
    assert_eq!(performance.tokens_per_second, None);
    assert_eq!(performance.p50_ttft_ms, Some(100));
    close(performance.average_duration_ms, 750.0);
    close(performance.end_to_end_tokens_per_second, 50.5);
}

#[test]
fn performance_percentiles_use_nearest_rank_without_averaging_groups() {
    let stats = summary(&[record(CompletionUsage {
        requests: 20,
        performance: (1..=20)
            .rev()
            .map(|duration_ms| RequestPerformance {
                duration_ms,
                ttft_ms: Some(duration_ms),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    })]);
    let performance = stats.totals.performance;
    assert_eq!(performance.p50_duration_ms, Some(10));
    assert_eq!(performance.p95_duration_ms, Some(19));
    assert_eq!(performance.p50_ttft_ms, Some(10));
    assert_eq!(performance.p95_ttft_ms, Some(19));
}

#[test]
fn legacy_usage_keeps_tokens_and_has_no_performance_measurement() {
    let usage = serde_json::from_str::<CompletionUsage>(
        r#"{"input_tokens":120,"output_tokens":40,"requests":1}"#,
    )
    .unwrap();
    let totals = summary(&[record(usage)]).totals;
    assert_eq!(totals.total_tokens, 160);
    assert_eq!(totals.performance, UsagePerformanceTotals::default());
    let mut old_json = serde_json::to_value(&totals).unwrap();
    old_json.as_object_mut().unwrap().remove("performance");
    let restored = serde_json::from_value::<UsageTotals>(old_json).unwrap();
    assert_eq!(restored, totals);
}
