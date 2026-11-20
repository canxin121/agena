use serde::{Deserialize, Serialize};

/// Client-observed performance of one completed physical provider request.
///
/// Streaming timestamps use the first and last non-empty text, reasoning, or
/// tool-call output. Provider metadata and terminal usage events do not extend
/// the generation interval. Durations exclude local tool execution and retry
/// backoff. A non-streaming response has no observable TTFT/generation interval.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RequestPerformance {
    /// Time from starting this provider attempt through its terminal response.
    pub duration_ms: u64,
    pub ttft_ms: Option<u64>,
    pub generation_ms: Option<u64>,
    /// Provider-reported output plus reasoning tokens. None means the provider
    /// did not report token usage; it must not be displayed as zero throughput.
    pub generated_tokens: Option<u64>,
}

impl RequestPerformance {
    /// Observed streaming output rate, including reasoning tokens.
    pub fn tokens_per_second(&self) -> Option<f64> {
        let tokens = self.generated_tokens.filter(|tokens| *tokens > 1)?;
        let first = self.ttft_ms.filter(|first| *first <= self.duration_ms)?;
        let duration = self
            .generation_ms
            .filter(|duration| *duration > 0 && *duration <= self.duration_ms - first)?;
        Some(tokens as f64 * 1_000.0 / duration as f64)
    }

    /// Output rate over the whole completed provider request.
    pub fn end_to_end_tokens_per_second(&self) -> Option<f64> {
        let tokens = self.generated_tokens.filter(|tokens| *tokens > 0)?;
        (self.duration_ms > 0).then(|| tokens as f64 * 1_000.0 / self.duration_ms as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::RequestPerformance;

    #[test]
    fn rates_use_their_own_observed_intervals() {
        let sample = RequestPerformance {
            duration_ms: 4_000,
            ttft_ms: Some(1_000),
            generation_ms: Some(2_000),
            generated_tokens: Some(100),
        };
        assert_eq!(sample.tokens_per_second(), Some(50.0));
        assert_eq!(sample.end_to_end_tokens_per_second(), Some(25.0));
    }

    #[test]
    fn absent_tokens_and_unmeasurable_streams_have_no_streaming_rate() {
        for sample in [
            RequestPerformance::default(),
            RequestPerformance {
                duration_ms: 100,
                generation_ms: Some(90),
                ..Default::default()
            },
            RequestPerformance {
                duration_ms: 100,
                generated_tokens: Some(100),
                ..Default::default()
            },
            RequestPerformance {
                duration_ms: 100,
                generation_ms: Some(0),
                generated_tokens: Some(100),
                ..Default::default()
            },
            RequestPerformance {
                duration_ms: 100,
                generation_ms: Some(90),
                generated_tokens: Some(1),
                ..Default::default()
            },
        ] {
            assert_eq!(sample.tokens_per_second(), None);
        }
        assert_eq!(
            RequestPerformance::default().end_to_end_tokens_per_second(),
            None
        );
    }

    #[test]
    fn older_serialized_values_default_missing_observations() {
        let sample: RequestPerformance = serde_json::from_str(r#"{"duration_ms":42}"#).unwrap();
        assert_eq!(sample.duration_ms, 42);
        assert_eq!(sample.ttft_ms, None);
        assert_eq!(sample.generated_tokens, None);
    }
}
