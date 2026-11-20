//! Performance observations at the normalized provider boundary.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use agena_domain::RequestPerformance;
use agena_provider::{CompletionStreamEvent, CompletionUsage};

pub(super) struct RequestPerformanceTracker {
    started_at: Instant,
    first_output_at: Option<Instant>,
    last_output_at: Option<Instant>,
    tool_calls: BTreeMap<String, (Option<String>, String)>,
}

impl RequestPerformanceTracker {
    pub(super) fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            first_output_at: None,
            last_output_at: None,
            tool_calls: BTreeMap::new(),
        }
    }

    /// Observe raw events before replay alignment or Tool API buffering. This
    /// keeps the interval paired with the physical request's complete usage.
    pub(super) fn observe(&mut self, event: &CompletionStreamEvent, at: Instant) {
        let output = match event {
            CompletionStreamEvent::TextDelta { delta, .. }
            | CompletionStreamEvent::ThinkingDelta { delta, .. } => !delta.is_empty(),
            CompletionStreamEvent::ToolCallDelta {
                stream_key,
                name,
                arguments_delta,
                ..
            } => {
                let state = self.tool_calls.entry(stream_key.clone()).or_default();
                let new_name = name.as_ref().filter(|name| !name.is_empty());
                let changed_name = new_name.is_some_and(|name| state.0.as_ref() != Some(name));
                if let Some(name) = new_name {
                    state.0 = Some(name.clone());
                }
                state.1.push_str(arguments_delta);
                changed_name || !arguments_delta.is_empty()
            }
            CompletionStreamEvent::ToolCallSnapshot {
                stream_key,
                name,
                arguments_json,
                ..
            } => {
                let state = self.tool_calls.entry(stream_key.clone()).or_default();
                let new_name = name.as_ref().filter(|name| !name.is_empty());
                let changed_name = new_name.is_some_and(|name| state.0.as_ref() != Some(name));
                if let Some(name) = new_name {
                    state.0 = Some(name.clone());
                }
                // A repeated final snapshot is bookkeeping, not another
                // generated token. Do not extend the interval into its tail.
                let snapshot_is_degenerate =
                    arguments_json.trim().is_empty() || arguments_json.trim() == "{}";
                let changed_arguments = !arguments_json.is_empty()
                    && state.1 != *arguments_json
                    && (!snapshot_is_degenerate || state.1.trim().is_empty());
                if changed_arguments {
                    state.1.clone_from(arguments_json);
                }
                changed_name || changed_arguments
            }
            _ => false,
        };
        if output {
            self.first_output_at.get_or_insert(at);
            self.last_output_at = Some(at);
        }
    }

    pub(super) fn finish(self, usage: &mut Option<CompletionUsage>, at: Instant) {
        let sample = RequestPerformance {
            duration_ms: milliseconds(at.saturating_duration_since(self.started_at)),
            ttft_ms: self
                .first_output_at
                .map(|first| milliseconds(first.saturating_duration_since(self.started_at))),
            generation_ms: self
                .first_output_at
                .zip(self.last_output_at)
                .map(|(first, last)| milliseconds(last.saturating_duration_since(first))),
            generated_tokens: usage
                .as_ref()
                .map(|usage| usage.output_tokens.saturating_add(usage.reasoning_tokens)),
        };
        record_performance(usage, sample);
    }
}

pub(super) fn record_non_streaming_performance(
    usage: &mut Option<CompletionUsage>,
    started_at: Instant,
) {
    RequestPerformanceTracker::new(started_at).finish(usage, Instant::now());
}

fn record_performance(usage: &mut Option<CompletionUsage>, sample: RequestPerformance) {
    let usage = usage.get_or_insert_with(|| CompletionUsage {
        // The request completed, but its token/cost usage is still unknown.
        cost_estimate_incomplete: true,
        ..CompletionUsage::default()
    });
    usage.requests = usage.requests.max(1);
    usage.performance.push(sample);
    usage.request_performance = Some(sample);
}

fn milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(delta: &str) -> CompletionStreamEvent {
        CompletionStreamEvent::TextDelta {
            provider_id: agena_domain::ProviderId::new("provider".to_owned()),
            model: agena_domain::ModelId::new("model".to_owned()),
            delta: delta.into(),
        }
    }

    fn tool(arguments: &str, snapshot: bool) -> CompletionStreamEvent {
        if snapshot {
            CompletionStreamEvent::ToolCallSnapshot {
                provider_id: agena_domain::ProviderId::new("provider".to_owned()),
                model: agena_domain::ModelId::new("model".to_owned()),
                stream_key: "call".into(),
                id: Some("call".into()),
                name: Some("search".into()),
                arguments_json: arguments.into(),
            }
        } else {
            CompletionStreamEvent::ToolCallDelta {
                provider_id: agena_domain::ProviderId::new("provider".to_owned()),
                model: agena_domain::ModelId::new("model".to_owned()),
                stream_key: "call".into(),
                id: Some("call".into()),
                name: Some("search".into()),
                arguments_delta: arguments.into(),
            }
        }
    }

    #[test]
    fn performance_uses_first_and_last_output_and_excludes_usage_tail() {
        let start = Instant::now();
        let mut tracker = RequestPerformanceTracker::new(start);
        tracker.observe(&text(""), start + Duration::from_millis(20));
        tracker.observe(
            &CompletionStreamEvent::ThinkingDelta {
                provider_id: agena_domain::ProviderId::new("provider".to_owned()),
                model: agena_domain::ModelId::new("model".to_owned()),
                delta: "reason".into(),
            },
            start + Duration::from_millis(100),
        );
        tracker.observe(&text("answer"), start + Duration::from_millis(600));
        let mut usage = Some(CompletionUsage {
            output_tokens: 20,
            reasoning_tokens: 30,
            ..Default::default()
        });
        tracker.finish(&mut usage, start + Duration::from_millis(1_000));
        let usage = usage.unwrap();
        let sample = usage.request_performance.unwrap();
        assert_eq!(sample.duration_ms, 1_000);
        assert_eq!(sample.ttft_ms, Some(100));
        assert_eq!(sample.generation_ms, Some(500));
        assert_eq!(sample.generated_tokens, Some(50));
        assert_eq!(sample.tokens_per_second(), Some(100.0));
        assert_eq!(usage.performance, vec![sample]);
        assert_eq!(usage.requests, 1);
    }

    #[test]
    fn performance_observes_tool_only_output_without_counting_duplicate_snapshots() {
        let start = Instant::now();
        let mut tracker = RequestPerformanceTracker::new(start);
        tracker.observe(&tool("{", false), start + Duration::from_millis(100));
        tracker.observe(&tool("}", false), start + Duration::from_millis(200));
        tracker.observe(&tool("{}", true), start + Duration::from_millis(800));
        let mut usage = Some(CompletionUsage {
            output_tokens: 10,
            ..Default::default()
        });
        tracker.finish(&mut usage, start + Duration::from_millis(1_000));
        let sample = usage.unwrap().request_performance.unwrap();
        assert_eq!(sample.ttft_ms, Some(100));
        assert_eq!(sample.generation_ms, Some(100));
    }

    #[test]
    fn performance_without_usage_and_non_streaming_do_not_invent_token_rates() {
        let start = Instant::now();
        let mut unknown = None;
        RequestPerformanceTracker::new(start)
            .finish(&mut unknown, start + Duration::from_millis(700));
        let unknown = unknown.unwrap();
        assert!(unknown.cost_estimate_incomplete);
        assert_eq!(unknown.requests, 1);
        assert_eq!(unknown.request_performance.unwrap().generated_tokens, None);
        assert_eq!(
            unknown.request_performance.unwrap().tokens_per_second(),
            None
        );
        let mut reported = Some(CompletionUsage {
            output_tokens: 70,
            ..Default::default()
        });
        RequestPerformanceTracker::new(start)
            .finish(&mut reported, start + Duration::from_millis(700));
        let sample = reported.unwrap().request_performance.unwrap();
        assert_eq!(sample.ttft_ms, None);
        assert_eq!(sample.generation_ms, None);
        assert_eq!(sample.tokens_per_second(), None);
        assert_eq!(sample.end_to_end_tokens_per_second(), Some(100.0));
    }

    #[test]
    fn performance_empty_and_single_chunk_streams_have_no_generation_rate() {
        let start = Instant::now();
        let mut tracker = RequestPerformanceTracker::new(start);
        tracker.observe(
            &CompletionStreamEvent::ProviderRetryStarted {
                provider_id: agena_domain::ProviderId::new("provider".to_owned()),
                model: agena_domain::ModelId::new("model".to_owned()),
            },
            start + Duration::from_millis(100),
        );
        tracker.observe(&text("only chunk"), start + Duration::from_millis(200));
        let mut usage = Some(CompletionUsage {
            output_tokens: 100,
            ..Default::default()
        });
        tracker.finish(&mut usage, start + Duration::from_millis(400));
        let sample = usage.unwrap().request_performance.unwrap();
        assert_eq!(sample.ttft_ms, Some(200));
        assert_eq!(sample.generation_ms, Some(0));
        assert_eq!(sample.tokens_per_second(), None);
    }
}
