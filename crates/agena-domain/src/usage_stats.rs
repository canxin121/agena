use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::UsagePeriod;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
/// Total billable units for one unit kind.
pub struct UsageBillableUnitTotal {
    pub kind: String,
    pub unit: String,
    pub quantity: f64,
    pub priced_quantity: f64,
    pub unpriced_quantity: f64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
/// Aggregate usage totals.
pub struct UsageTotals {
    /// Number of provider requests, including requests issued by ordinary
    /// provider-backed tools. The field name is retained for API compatibility.
    pub runs: u64,
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
    pub unpriced_runs: u64,
    pub billable_units: Vec<UsageBillableUnitTotal>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
/// Per-day usage breakdown.
pub struct UsageDailyBreakdown {
    pub date: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
/// Usage broken down by provider.
pub struct ProviderUsageBreakdown {
    pub provider_id: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
/// Usage broken down by model.
pub struct ModelUsageBreakdown {
    pub provider_id: String,
    pub model_id: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, PartialEq)]
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
    pub average_cost_per_run_usd: f64,
    pub average_tokens_per_run: f64,
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
