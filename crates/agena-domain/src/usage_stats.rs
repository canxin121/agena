use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::UsagePeriod;

/// Format a count using decimal usage units with at most one decimal place.
pub fn format_usage_count(value: u64) -> String {
    for (unit, suffix) in [
        (1_000_000_000_000_000_000, "E"),
        (1_000_000_000_000_000, "P"),
        (1_000_000_000_000, "t"),
        (1_000_000_000, "b"),
        (1_000_000, "m"),
        (1_000, "k"),
    ] {
        // A value that rounds to 1000 of the previous unit uses this unit.
        if value >= unit - unit / 20_000 {
            let unit = u128::from(unit);
            let tenths = (u128::from(value) * 10 + unit / 2) / unit;
            let whole = tenths / 10;
            let decimal = tenths % 10;
            return if decimal == 0 {
                format!("{whole}{suffix}")
            } else {
                format!("{whole}.{decimal}{suffix}")
            };
        }
    }
    value.to_string()
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn compact_usage_promotes_rounding_boundaries_and_handles_large_counts() {
        for (value, label) in [
            (0, "0"),
            (999, "999"),
            (1_000, "1k"),
            (1_240, "1.2k"),
            (999_949, "999.9k"),
            (999_950, "1m"),
            (1_250_000, "1.3m"),
            (2_500_000_000, "2.5b"),
            (1_000_000_000_000, "1t"),
            (u64::MAX, "18.4E"),
        ] {
            assert_eq!(format_usage_count(value), label);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Total billable units for one unit kind.
pub struct UsageBillableUnitTotal {
    pub kind: String,
    pub unit: String,
    pub quantity: f64,
    pub priced_quantity: f64,
    pub unpriced_quantity: f64,
    pub estimated_cost_usd: f64,
}

/// Performance of completed physical provider requests with local timing.
///
/// Each rate uses tokens and time from the same measured requests. Historical
/// token usage without timing never contributes to a rate's numerator.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UsagePerformanceTotals {
    pub request_count: u64,
    pub ttft_count: u64,
    pub generation_count: u64,
    pub throughput_count: u64,
    pub total_duration_ms: u64,
    pub total_ttft_ms: u64,
    /// Time between the first and last effective output, for valid TPS samples.
    pub total_generation_ms: u64,
    pub timed_tokens: u64,
    pub timed_duration_ms: u64,
    pub generation_tokens: u64,
    pub average_duration_ms: Option<f64>,
    pub average_ttft_ms: Option<f64>,
    /// Nearest-rank percentiles over the measured physical requests.
    pub p50_duration_ms: Option<u64>,
    pub p95_duration_ms: Option<u64>,
    pub p50_ttft_ms: Option<u64>,
    pub p95_ttft_ms: Option<u64>,
    /// Provider-reported output plus reasoning tokens per generation second.
    pub tokens_per_second: Option<f64>,
    /// Provider-reported output plus reasoning tokens per whole-request second.
    pub end_to_end_tokens_per_second: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Aggregate usage totals.
pub struct UsageTotals {
    /// Number of provider requests, including requests issued by ordinary
    /// provider-backed tools.
    pub requests: u64,
    pub sessions: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_write_5m_tokens: u64,
    pub cache_write_1h_tokens: u64,
    pub cache_read_tokens: u64,
    /// Provider-reported informational tool-use token subset.
    pub tool_use_tokens: u64,
    /// Provider-authoritative token categories not otherwise classified.
    pub other_tokens: u64,
    pub total_tokens: u64,
    pub cache_input_tokens: u64,
    pub cache_hit_rate: f64,
    pub total_cost_usd: f64,
    pub recorded_cost_usd: f64,
    pub estimated_cost_usd: f64,
    pub unpriced_requests: u64,
    pub billable_units: Vec<UsageBillableUnitTotal>,
    #[serde(default)]
    pub performance: UsagePerformanceTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Per-day usage breakdown.
pub struct UsageDailyBreakdown {
    pub date: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Usage broken down by provider.
pub struct ProviderUsageBreakdown {
    pub provider_id: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Usage broken down by model.
pub struct ModelUsageBreakdown {
    pub provider_id: String,
    pub model_id: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Usage broken down by session.
pub struct SessionUsageBreakdown {
    pub session_id: i64,
    pub title: String,
    pub is_subagent: bool,
    pub first_message_at: DateTime<Utc>,
    pub last_message_at: DateTime<Utc>,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Full usage statistics: totals, daily, provider, model, and session breakdowns.
pub struct UsageStats {
    pub generated_at: DateTime<Utc>,
    pub period: UsagePeriod,
    pub period_label: String,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub timezone_offset_minutes: i32,
    pub totals: UsageTotals,
    pub active_days: u64,
    pub average_cost_per_request_usd: f64,
    pub average_tokens_per_request: f64,
    pub average_cost_per_active_day_usd: f64,
    pub average_tokens_per_active_day: f64,
    pub peak_cost_date: Option<String>,
    pub peak_cost_usd: f64,
    pub peak_tokens_date: Option<String>,
    pub peak_tokens: u64,
    pub by_day: Vec<UsageDailyBreakdown>,
    pub by_provider: Vec<ProviderUsageBreakdown>,
    pub by_model: Vec<ModelUsageBreakdown>,
    pub by_session: Vec<SessionUsageBreakdown>,
}
