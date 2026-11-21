//! Display values shared by the request status and usage dashboard.

/// Scalar projection of measured provider-request performance.
#[derive(Debug, Clone, Default)]
pub struct PerformanceSummary {
    pub request_count: u64,
    pub ttft_count: u64,
    pub generation_count: u64,
    pub throughput_count: u64,
    pub average_duration_ms: Option<f64>,
    pub average_ttft_ms: Option<f64>,
    pub average_generation_ms: Option<f64>,
    pub tokens_per_second: Option<f64>,
    pub end_to_end_tokens_per_second: Option<f64>,
    pub p95_duration_ms: Option<f64>,
    pub p95_ttft_ms: Option<f64>,
}

/// Unknown and invalid measurements stay visibly distinct from measured zero.
pub fn format_duration_ms(value: Option<f64>) -> String {
    match value.filter(|value| value.is_finite() && *value >= 0.0) {
        Some(value) if value < 1_000.0 => format!("{value:.0} ms"),
        Some(value) if value < 60_000.0 => format!("{:.2} s", value / 1_000.0),
        Some(value) => format!("{:.1} min", value / 60_000.0),
        None => "—".to_owned(),
    }
}

pub fn format_token_rate(value: Option<f64>) -> String {
    match value.filter(|value| value.is_finite() && *value >= 0.0) {
        Some(value) if value >= 1_000.0 => agena_domain::format_usage_count(value as u64),
        Some(value) => format!("{value:.1}"),
        None => "—".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_formatting_preserves_missing_zero_and_units() {
        for invalid in [None, Some(-1.0), Some(f64::NAN), Some(f64::INFINITY)] {
            assert_eq!(format_duration_ms(invalid), "—");
            assert_eq!(format_token_rate(invalid), "—");
        }
        assert_eq!(format_duration_ms(Some(0.0)), "0 ms");
        assert_eq!(format_token_rate(Some(0.0)), "0.0");
        assert_eq!(format_duration_ms(Some(450.0)), "450 ms");
        assert_eq!(format_duration_ms(Some(1_250.0)), "1.25 s");
        assert_eq!(format_duration_ms(Some(90_000.0)), "1.5 min");
        assert_eq!(format_token_rate(Some(42.56)), "42.6");
        assert_eq!(format_token_rate(Some(12_345.0)), "12.3k");
    }
}
