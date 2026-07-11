//! Session-level cost / token aggregation.
//!
//! Walks a session's assistant messages, sums up their `MessageUsage`
//! totals, and produces a [`SessionCostSummary`] suitable for `/cost`-style
//! UI panels. Per-model breakdowns are preserved so a session that switched
//! providers mid-flight surfaces both contributions.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Duration, FixedOffset, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::message::{Message, MessageUsage};
use crate::role::Role;

const PER_MILLION: f64 = 1_000_000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct ModelTokenRates {
    input_per_million: f64,
    output_per_million: f64,
    cache_write_per_million: f64,
    cache_read_per_million: f64,
}

impl ModelTokenRates {
    const fn new(
        input_per_million: f64,
        output_per_million: f64,
        cache_write_per_million: f64,
        cache_read_per_million: f64,
    ) -> Self {
        Self {
            input_per_million,
            output_per_million,
            cache_write_per_million,
            cache_read_per_million,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CostContribution {
    total_cost_usd: f64,
    recorded_cost_usd: f64,
    estimated_cost_usd: f64,
    unpriced_runs: u64,
}

fn cost_contribution(provider_id: &str, model_id: &str, usage: &MessageUsage) -> CostContribution {
    if usage.total_cost.is_finite() && usage.total_cost > 0.0 {
        return CostContribution {
            total_cost_usd: usage.total_cost,
            recorded_cost_usd: usage.total_cost,
            estimated_cost_usd: 0.0,
            unpriced_runs: 0,
        };
    }

    if let Some(estimated_cost_usd) = estimate_usage_cost_usd(provider_id, model_id, usage) {
        return CostContribution {
            total_cost_usd: estimated_cost_usd,
            recorded_cost_usd: 0.0,
            estimated_cost_usd,
            unpriced_runs: 0,
        };
    }

    CostContribution {
        total_cost_usd: 0.0,
        recorded_cost_usd: 0.0,
        estimated_cost_usd: 0.0,
        unpriced_runs: 1,
    }
}

/// Estimate cost from a compact built-in pricing table when providers do not
/// return a charge. The rates are intentionally conservative and family-based;
/// exact provider billing should still be preferred when `MessageUsage`
/// carries `total_cost`.
pub fn estimate_usage_cost_usd(
    provider_id: &str,
    model_id: &str,
    usage: &MessageUsage,
) -> Option<f64> {
    let rates = estimate_model_token_rates(provider_id, model_id)?;
    let output_tokens = usage.output_tokens.saturating_add(usage.reasoning_tokens) as f64;
    let cost = (usage.input_tokens as f64 * rates.input_per_million
        + output_tokens * rates.output_per_million
        + usage.cache_write_tokens as f64 * rates.cache_write_per_million
        + usage.cache_read_tokens as f64 * rates.cache_read_per_million)
        / PER_MILLION;
    cost.is_finite().then_some(cost.max(0.0))
}

fn estimate_model_token_rates(provider_id: &str, model_id: &str) -> Option<ModelTokenRates> {
    let provider = provider_id.trim().to_ascii_lowercase();
    let model = normalize_model_id(model_id);

    if looks_like_local_model(model.as_str()) || provider == "ollama" {
        return Some(ModelTokenRates::new(0.0, 0.0, 0.0, 0.0));
    }

    for (prefix, rates) in PRICING_PREFIXES {
        if model == *prefix || model.starts_with(&format!("{prefix}-")) {
            return Some(*rates);
        }
    }

    if provider.contains("anthropic") {
        if model.contains("opus") {
            return Some(ModelTokenRates::new(15.0, 75.0, 18.75, 1.50));
        }
        if model.contains("sonnet") {
            return Some(ModelTokenRates::new(3.0, 15.0, 3.75, 0.30));
        }
        if model.contains("haiku") {
            return Some(ModelTokenRates::new(0.80, 4.0, 1.00, 0.08));
        }
    }

    if provider.contains("openai") && model.contains("gpt-5") {
        return Some(ModelTokenRates::new(1.25, 10.0, 1.25, 0.125));
    }

    if provider.contains("gemini") || provider.contains("google") {
        if model.contains("pro") {
            return Some(ModelTokenRates::new(1.25, 10.0, 1.25, 0.125));
        }
        if model.contains("flash") {
            return Some(ModelTokenRates::new(0.30, 2.50, 0.30, 0.03));
        }
    }

    None
}

const PRICING_PREFIXES: &[(&str, ModelTokenRates)] = &[
    (
        "claude-opus-4-7",
        ModelTokenRates::new(5.0, 25.0, 6.25, 0.50),
    ),
    (
        "claude-opus-4-6",
        ModelTokenRates::new(5.0, 25.0, 6.25, 0.50),
    ),
    (
        "claude-opus-4-5",
        ModelTokenRates::new(5.0, 25.0, 6.25, 0.50),
    ),
    (
        "claude-opus-4",
        ModelTokenRates::new(15.0, 75.0, 18.75, 1.50),
    ),
    (
        "claude-sonnet-4",
        ModelTokenRates::new(3.0, 15.0, 3.75, 0.30),
    ),
    (
        "claude-haiku-4-5",
        ModelTokenRates::new(1.0, 5.0, 1.25, 0.10),
    ),
    (
        "claude-3-5-haiku",
        ModelTokenRates::new(0.80, 4.0, 1.00, 0.08),
    ),
    (
        "claude-haiku-3-5",
        ModelTokenRates::new(0.80, 4.0, 1.00, 0.08),
    ),
    (
        "claude-haiku-3",
        ModelTokenRates::new(0.25, 1.25, 0.30, 0.03),
    ),
    ("gpt-5.5-pro", ModelTokenRates::new(30.0, 180.0, 30.0, 30.0)),
    ("gpt-5.5", ModelTokenRates::new(5.0, 30.0, 5.0, 0.50)),
    ("gpt-5.4-pro", ModelTokenRates::new(30.0, 180.0, 30.0, 30.0)),
    (
        "gpt-5.4-mini",
        ModelTokenRates::new(0.75, 4.50, 0.75, 0.075),
    ),
    ("gpt-5.4-nano", ModelTokenRates::new(0.20, 1.25, 0.20, 0.02)),
    ("gpt-5.4", ModelTokenRates::new(2.50, 15.0, 2.50, 0.25)),
    (
        "gpt-5.3-codex",
        ModelTokenRates::new(1.75, 14.0, 1.75, 0.175),
    ),
    (
        "gpt-5.2-codex",
        ModelTokenRates::new(1.75, 14.0, 1.75, 0.175),
    ),
    ("gpt-5.2", ModelTokenRates::new(1.75, 14.0, 1.75, 0.175)),
    ("gpt-5-nano", ModelTokenRates::new(0.05, 0.40, 0.05, 0.005)),
    ("gpt-5-mini", ModelTokenRates::new(0.25, 2.0, 0.25, 0.025)),
    ("gpt-5", ModelTokenRates::new(1.25, 10.0, 1.25, 0.125)),
    (
        "gpt-4.1-nano",
        ModelTokenRates::new(0.10, 0.40, 0.10, 0.025),
    ),
    ("gpt-4.1-mini", ModelTokenRates::new(0.40, 1.60, 0.40, 0.10)),
    ("gpt-4.1", ModelTokenRates::new(2.0, 8.0, 2.0, 0.50)),
    ("gpt-4o-mini", ModelTokenRates::new(0.15, 0.60, 0.15, 0.075)),
    ("gpt-4o", ModelTokenRates::new(2.50, 10.0, 2.50, 1.25)),
    (
        "gemini-3.1-pro-preview",
        ModelTokenRates::new(2.0, 12.0, 0.20, 0.20),
    ),
    (
        "gemini-3.1-flash-lite",
        ModelTokenRates::new(0.25, 1.50, 0.025, 0.025),
    ),
    (
        "gemini-3-flash-preview",
        ModelTokenRates::new(0.50, 3.0, 0.05, 0.05),
    ),
    (
        "gemini-2.5-pro",
        ModelTokenRates::new(1.25, 10.0, 0.125, 0.125),
    ),
    (
        "gemini-2.5-flash",
        ModelTokenRates::new(0.30, 2.50, 0.03, 0.03),
    ),
    (
        "gemini-2.5-flash-lite",
        ModelTokenRates::new(0.10, 0.40, 0.01, 0.01),
    ),
    (
        "gemini-2.0-flash",
        ModelTokenRates::new(0.10, 0.40, 0.025, 0.025),
    ),
];

fn normalize_model_id(model_id: &str) -> String {
    let mut model = model_id.trim().to_ascii_lowercase();
    if let Some((_, stripped)) = model.rsplit_once('/') {
        model = stripped.to_string();
    }
    if let Some((stripped, _)) = model.split_once('@') {
        model = stripped.to_string();
    }
    if model.len() > 9
        && model.get(model.len().saturating_sub(9)..model.len().saturating_sub(8)) == Some("-")
        && model[model.len().saturating_sub(8)..]
            .chars()
            .all(|ch| ch.is_ascii_digit())
    {
        model.truncate(model.len() - 9);
    }

    match model.as_str() {
        "claude-sonnet-4.6" | "claude-4.6-sonnet" | "claude-4.6-sonnet-thinking" => {
            "claude-sonnet-4-6".to_string()
        }
        "claude-sonnet-4.5" | "claude-4.5-sonnet" | "claude-4.5-sonnet-thinking" => {
            "claude-sonnet-4-5".to_string()
        }
        "claude-opus-4.7" | "claude-4.7-opus" => "claude-opus-4-7".to_string(),
        "claude-opus-4.6" | "claude-4.6-opus" => "claude-opus-4-6".to_string(),
        "claude-opus-4.5" | "claude-4.5-opus" => "claude-opus-4-5".to_string(),
        "claude-haiku-4.5" | "claude-4.5-haiku" => "claude-haiku-4-5".to_string(),
        "cursor-auto" | "cursor-agent-auto" | "kiro-auto" | "cline-auto" => {
            "claude-sonnet-4-5".to_string()
        }
        "gpt-5.1-codex" => "gpt-5".to_string(),
        _ => model,
    }
}

fn looks_like_local_model(model: &str) -> bool {
    model.contains(':')
        || model.ends_with(".gguf")
        || model.contains("-gguf")
        || model.contains("-q4")
        || model.contains("-q5")
        || model.contains("-q6")
        || model.contains("-q8")
        || model.contains("llama")
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ModelCostBreakdown {
    pub provider_id: String,
    pub model_id: String,
    pub runs: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost_usd: f64,
    pub recorded_cost_usd: f64,
    pub estimated_cost_usd: f64,
    pub unpriced_runs: u64,
}

impl ModelCostBreakdown {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
    }

    pub fn cache_input_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
    }

    pub fn cache_hit_rate(&self) -> f64 {
        ratio(self.cache_read_tokens, self.cache_input_tokens())
    }

    fn fold(&mut self, usage: &MessageUsage) {
        self.runs = self.runs.saturating_add(1);
        self.input_tokens = self.input_tokens.saturating_add(usage.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(usage.output_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(usage.reasoning_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(usage.cache_write_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(usage.cache_read_tokens);
        let cost = cost_contribution(&self.provider_id, &self.model_id, usage);
        self.total_cost_usd += cost.total_cost_usd;
        self.recorded_cost_usd += cost.recorded_cost_usd;
        self.estimated_cost_usd += cost.estimated_cost_usd;
        self.unpriced_runs = self.unpriced_runs.saturating_add(cost.unpriced_runs);
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct SessionCostSummary {
    pub runs: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost_usd: f64,
    pub recorded_cost_usd: f64,
    pub estimated_cost_usd: f64,
    pub unpriced_runs: u64,
    pub by_model: Vec<ModelCostBreakdown>,
}

impl SessionCostSummary {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
    }

    pub fn cache_input_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
    }

    pub fn cache_hit_rate(&self) -> f64 {
        ratio(self.cache_read_tokens, self.cache_input_tokens())
    }

    pub fn is_empty(&self) -> bool {
        self.runs == 0
    }

    /// One-line human summary, e.g.
    /// `1,234 in + 100 cache + 567 out + 12 reasoning = 1,913 tokens · $0.0420 over 4 runs`.
    pub fn one_line(&self) -> String {
        if self.is_empty() {
            return "no usage recorded yet".to_string();
        }
        let cache_tokens = self
            .cache_write_tokens
            .saturating_add(self.cache_read_tokens);
        format!(
            "{} in{} + {} out{} = {} tokens{} · ${:.4} over {} run{}",
            format_count(self.input_tokens),
            if cache_tokens > 0 {
                format!(" + {} cache", format_count(cache_tokens))
            } else {
                String::new()
            },
            format_count(self.output_tokens),
            if self.reasoning_tokens > 0 {
                format!(" + {} reasoning", format_count(self.reasoning_tokens))
            } else {
                String::new()
            },
            format_count(self.total_tokens()),
            if self.cache_input_tokens() > 0 {
                format!(" · {:.1}% cache hit", self.cache_hit_rate() * 100.0)
            } else {
                String::new()
            },
            self.total_cost_usd,
            self.runs,
            if self.runs == 1 { "" } else { "s" },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsagePeriod {
    Today,
    Yesterday,
    #[serde(rename = "last_7_days")]
    Last7Days,
    #[serde(rename = "last_14_days")]
    Last14Days,
    #[serde(rename = "last_30_days")]
    Last30Days,
    #[serde(rename = "last_90_days")]
    Last90Days,
    MonthToDate,
    YearToDate,
    AllTime,
}

impl UsagePeriod {
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Yesterday => "yesterday",
            Self::Last7Days => "last_7_days",
            Self::Last14Days => "last_14_days",
            Self::Last30Days => "last_30_days",
            Self::Last90Days => "last_90_days",
            Self::MonthToDate => "month_to_date",
            Self::YearToDate => "year_to_date",
            Self::AllTime => "all_time",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageStatsQuery {
    pub period: UsagePeriod,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub provider_ids: Vec<String>,
    pub model_ids: Vec<String>,
    pub session_ids: Vec<i64>,
    pub include_subagents: bool,
    pub timezone_offset_minutes: i32,
}

impl UsageStatsQuery {
    pub fn for_period(period: UsagePeriod, now: DateTime<Utc>) -> Self {
        Self::for_period_with_offset(period, now, 0)
    }

    /// Build a calendar-aligned reporting window in the caller's timezone.
    /// The offset is deliberately explicit so REST and TUI clients get the
    /// same day boundaries instead of inheriting the server process timezone.
    pub fn for_period_with_offset(
        period: UsagePeriod,
        now: DateTime<Utc>,
        timezone_offset_minutes: i32,
    ) -> Self {
        let timezone_offset_minutes = clamp_timezone_offset(timezone_offset_minutes);
        let offset = fixed_offset(timezone_offset_minutes);
        let local_now = now.with_timezone(&offset);
        let today = start_of_local_day(local_now, offset);
        let (from, to) = match period {
            UsagePeriod::Today => (Some(today), Some(now)),
            UsagePeriod::Yesterday => (
                Some(today - Duration::days(1)),
                Some(today - Duration::milliseconds(1)),
            ),
            UsagePeriod::Last7Days => (Some(today - Duration::days(6)), Some(now)),
            UsagePeriod::Last14Days => (Some(today - Duration::days(13)), Some(now)),
            UsagePeriod::Last30Days => (Some(today - Duration::days(29)), Some(now)),
            UsagePeriod::Last90Days => (Some(today - Duration::days(89)), Some(now)),
            UsagePeriod::MonthToDate => (Some(start_of_local_month(local_now, offset)), Some(now)),
            UsagePeriod::YearToDate => (Some(start_of_local_year(local_now, offset)), Some(now)),
            UsagePeriod::AllTime => (None, Some(now)),
        };
        Self {
            period,
            from,
            to,
            provider_ids: Vec::new(),
            model_ids: Vec::new(),
            session_ids: Vec::new(),
            include_subagents: true,
            timezone_offset_minutes,
        }
    }

    pub fn custom(from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>) -> Self {
        Self {
            period: UsagePeriod::AllTime,
            from,
            to,
            provider_ids: Vec::new(),
            model_ids: Vec::new(),
            session_ids: Vec::new(),
            include_subagents: true,
            timezone_offset_minutes: 0,
        }
    }

    pub fn with_timezone_offset(mut self, timezone_offset_minutes: i32) -> Self {
        self.timezone_offset_minutes = clamp_timezone_offset(timezone_offset_minutes);
        self
    }

    pub fn with_filters(
        mut self,
        provider_ids: Vec<String>,
        model_ids: Vec<String>,
        session_ids: Vec<i64>,
        include_subagents: bool,
    ) -> Self {
        self.provider_ids = normalized_filters(provider_ids);
        self.model_ids = normalized_filters(model_ids);
        self.session_ids = session_ids;
        self.session_ids.sort_unstable();
        self.session_ids.dedup();
        self.include_subagents = include_subagents;
        self
    }

    pub fn matches_record(&self, record: &UsageStatRecord) -> bool {
        (self.include_subagents || !record.is_subagent)
            && (self.session_ids.is_empty() || self.session_ids.contains(&record.session_id))
            && matches_text_filter(self.provider_ids.as_slice(), record.provider_id.as_str())
            && matches_text_filter(self.model_ids.as_slice(), record.model_id.as_str())
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct UsageTotals {
    pub runs: u64,
    pub sessions: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_tokens: u64,
    pub cache_input_tokens: u64,
    pub cache_hit_rate: f64,
    pub total_cost_usd: f64,
    pub recorded_cost_usd: f64,
    pub estimated_cost_usd: f64,
    pub unpriced_runs: u64,
}

impl UsageTotals {
    fn fold(&mut self, provider_id: &str, model_id: &str, usage: &MessageUsage) {
        self.runs = self.runs.saturating_add(1);
        self.input_tokens = self.input_tokens.saturating_add(usage.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(usage.output_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(usage.reasoning_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(usage.cache_write_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(usage.cache_read_tokens);
        let cost = cost_contribution(provider_id, model_id, usage);
        self.total_cost_usd += cost.total_cost_usd;
        self.recorded_cost_usd += cost.recorded_cost_usd;
        self.estimated_cost_usd += cost.estimated_cost_usd;
        self.unpriced_runs = self.unpriced_runs.saturating_add(cost.unpriced_runs);
    }

    fn finalize(&mut self, session_count: usize) {
        self.sessions = session_count as u64;
        self.total_tokens = self
            .input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens);
        self.cache_input_tokens = self
            .input_tokens
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens);
        self.cache_hit_rate = ratio(self.cache_read_tokens, self.cache_input_tokens);
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UsageDailyBreakdown {
    pub date: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProviderUsageBreakdown {
    pub provider_id: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelUsageBreakdown {
    pub provider_id: String,
    pub model_id: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct UsageStatRecord {
    pub session_id: i64,
    pub session_title: String,
    pub is_subagent: bool,
    pub created_at: DateTime<Utc>,
    pub provider_id: String,
    pub model_id: String,
    pub usage: MessageUsage,
}

#[derive(Debug, Clone, Default)]
struct UsageTotalsAccumulator {
    totals: UsageTotals,
    sessions: BTreeSet<i64>,
}

impl UsageTotalsAccumulator {
    fn fold(&mut self, session_id: i64, provider_id: &str, model_id: &str, usage: &MessageUsage) {
        self.sessions.insert(session_id);
        self.totals.fold(provider_id, model_id, usage);
    }

    fn into_totals(mut self) -> UsageTotals {
        self.totals.finalize(self.sessions.len());
        self.totals
    }
}

#[derive(Debug, Clone)]
struct SessionUsageAccumulator {
    title: String,
    is_subagent: bool,
    first_message_at: DateTime<Utc>,
    last_message_at: DateTime<Utc>,
    totals: UsageTotalsAccumulator,
}

impl SessionUsageAccumulator {
    fn fold(&mut self, record: &UsageStatRecord) {
        if record.created_at < self.first_message_at {
            self.first_message_at = record.created_at;
        }
        if record.created_at > self.last_message_at {
            self.last_message_at = record.created_at;
        }
        self.totals.fold(
            record.session_id,
            &record.provider_id,
            &record.model_id,
            &record.usage,
        );
    }
}

pub fn summarize_usage_records(
    records: &[UsageStatRecord],
    query: &UsageStatsQuery,
    generated_at: DateTime<Utc>,
) -> UsageStats {
    let mut totals = UsageTotalsAccumulator {
        totals: UsageTotals::default(),
        sessions: BTreeSet::new(),
    };
    let mut by_day: BTreeMap<String, UsageTotalsAccumulator> = BTreeMap::new();
    let mut by_provider: BTreeMap<String, UsageTotalsAccumulator> = BTreeMap::new();
    let mut by_model: BTreeMap<(String, String), UsageTotalsAccumulator> = BTreeMap::new();
    let mut by_session: BTreeMap<i64, SessionUsageAccumulator> = BTreeMap::new();

    for record in records.iter().filter(|record| query.matches_record(record)) {
        totals.fold(
            record.session_id,
            &record.provider_id,
            &record.model_id,
            &record.usage,
        );

        by_day
            .entry(usage_date_key(
                record.created_at,
                query.timezone_offset_minutes,
            ))
            .or_insert_with(|| UsageTotalsAccumulator {
                totals: UsageTotals::default(),
                sessions: BTreeSet::new(),
            })
            .fold(
                record.session_id,
                &record.provider_id,
                &record.model_id,
                &record.usage,
            );
        by_provider
            .entry(record.provider_id.clone())
            .or_insert_with(|| UsageTotalsAccumulator {
                totals: UsageTotals::default(),
                sessions: BTreeSet::new(),
            })
            .fold(
                record.session_id,
                &record.provider_id,
                &record.model_id,
                &record.usage,
            );
        by_model
            .entry((record.provider_id.clone(), record.model_id.clone()))
            .or_insert_with(|| UsageTotalsAccumulator {
                totals: UsageTotals::default(),
                sessions: BTreeSet::new(),
            })
            .fold(
                record.session_id,
                &record.provider_id,
                &record.model_id,
                &record.usage,
            );
        by_session
            .entry(record.session_id)
            .or_insert_with(|| SessionUsageAccumulator {
                title: record.session_title.clone(),
                is_subagent: record.is_subagent,
                first_message_at: record.created_at,
                last_message_at: record.created_at,
                totals: UsageTotalsAccumulator {
                    totals: UsageTotals::default(),
                    sessions: BTreeSet::new(),
                },
            })
            .fold(record);
    }

    let mut by_provider = by_provider
        .into_iter()
        .map(|(provider_id, builder)| ProviderUsageBreakdown {
            provider_id,
            totals: builder.into_totals(),
        })
        .collect::<Vec<_>>();
    by_provider.sort_by(compare_usage_totals_desc);

    let mut by_model = by_model
        .into_iter()
        .map(|((provider_id, model_id), builder)| ModelUsageBreakdown {
            provider_id,
            model_id,
            totals: builder.into_totals(),
        })
        .collect::<Vec<_>>();
    by_model.sort_by(compare_usage_totals_desc);

    let mut by_session = by_session
        .into_iter()
        .map(|(session_id, builder)| SessionUsageBreakdown {
            session_id,
            title: builder.title,
            is_subagent: builder.is_subagent,
            first_message_at: builder.first_message_at,
            last_message_at: builder.last_message_at,
            totals: builder.totals.into_totals(),
        })
        .collect::<Vec<_>>();
    by_session.sort_by(compare_usage_totals_desc);

    let by_day = by_day
        .into_iter()
        .map(|(date, builder)| UsageDailyBreakdown {
            date,
            totals: builder.into_totals(),
        })
        .collect::<Vec<_>>();
    let active_days = by_day.len() as u64;
    let peak_cost = by_day.iter().max_by(|left, right| {
        left.totals
            .total_cost_usd
            .partial_cmp(&right.totals.total_cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let peak_tokens = by_day.iter().max_by_key(|day| day.totals.total_tokens);
    let totals = totals.into_totals();

    UsageStats {
        generated_at,
        period: query.period,
        period_label: query.period.label().to_string(),
        from: query.from.to_owned(),
        to: query.to.to_owned(),
        timezone_offset_minutes: query.timezone_offset_minutes,
        active_days,
        average_cost_per_run_usd: safe_average(totals.total_cost_usd, totals.runs),
        average_tokens_per_run: safe_average(totals.total_tokens as f64, totals.runs),
        average_cost_per_active_day_usd: safe_average(totals.total_cost_usd, active_days),
        average_tokens_per_active_day: safe_average(totals.total_tokens as f64, active_days),
        peak_cost_date: peak_cost.map(|day| day.date.clone()),
        peak_cost_usd: peak_cost
            .map(|day| day.totals.total_cost_usd)
            .unwrap_or_default(),
        peak_tokens_date: peak_tokens.map(|day| day.date.clone()),
        peak_tokens: peak_tokens
            .map(|day| day.totals.total_tokens)
            .unwrap_or_default(),
        totals,
        by_day,
        by_provider,
        by_model,
        by_session,
    }
}

fn compare_usage_totals_desc<T>(left: &T, right: &T) -> std::cmp::Ordering
where
    T: HasUsageTotals,
{
    right
        .totals()
        .total_cost_usd
        .partial_cmp(&left.totals().total_cost_usd)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then(right.totals().runs.cmp(&left.totals().runs))
}

trait HasUsageTotals {
    fn totals(&self) -> &UsageTotals;
}

impl HasUsageTotals for ProviderUsageBreakdown {
    fn totals(&self) -> &UsageTotals {
        &self.totals
    }
}

impl HasUsageTotals for ModelUsageBreakdown {
    fn totals(&self) -> &UsageTotals {
        &self.totals
    }
}

impl HasUsageTotals for SessionUsageBreakdown {
    fn totals(&self) -> &UsageTotals {
        &self.totals
    }
}

/// Build a per-session summary from the session's message history.
pub fn summarize(messages: &[Message]) -> SessionCostSummary {
    let mut runs = 0_u64;
    let mut input_tokens = 0_u64;
    let mut output_tokens = 0_u64;
    let mut reasoning_tokens = 0_u64;
    let mut cache_write_tokens = 0_u64;
    let mut cache_read_tokens = 0_u64;
    let mut total_cost_usd = 0.0_f64;
    let mut recorded_cost_usd = 0.0_f64;
    let mut estimated_cost_usd = 0.0_f64;
    let mut unpriced_runs = 0_u64;
    let mut by_key: BTreeMap<(String, String), ModelCostBreakdown> = BTreeMap::new();

    for message in messages {
        if message.role != Role::Assistant {
            continue;
        }
        let Some(usage) = message.usage.as_ref() else {
            continue;
        };
        let provider = message.metadata.model_provider_id.clone();
        let model = message.metadata.model_id.clone();

        runs = runs.saturating_add(1);
        input_tokens = input_tokens.saturating_add(usage.input_tokens);
        output_tokens = output_tokens.saturating_add(usage.output_tokens);
        reasoning_tokens = reasoning_tokens.saturating_add(usage.reasoning_tokens);
        cache_write_tokens = cache_write_tokens.saturating_add(usage.cache_write_tokens);
        cache_read_tokens = cache_read_tokens.saturating_add(usage.cache_read_tokens);
        let cost = cost_contribution(&provider, &model, usage);
        total_cost_usd += cost.total_cost_usd;
        recorded_cost_usd += cost.recorded_cost_usd;
        estimated_cost_usd += cost.estimated_cost_usd;
        unpriced_runs = unpriced_runs.saturating_add(cost.unpriced_runs);

        let entry = by_key
            .entry((provider.clone(), model.clone()))
            .or_insert_with(|| ModelCostBreakdown {
                provider_id: provider,
                model_id: model,
                ..ModelCostBreakdown::default()
            });
        entry.fold(usage);
    }

    SessionCostSummary {
        runs,
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_write_tokens,
        cache_read_tokens,
        total_cost_usd,
        recorded_cost_usd,
        estimated_cost_usd,
        unpriced_runs,
        by_model: by_key.into_values().collect(),
    }
}

fn start_of_local_day(now: DateTime<FixedOffset>, offset: FixedOffset) -> DateTime<Utc> {
    offset
        .from_local_datetime(
            &now.date_naive()
                .and_hms_opt(0, 0, 0)
                .expect("midnight is valid"),
        )
        .single()
        .expect("fixed offsets have unambiguous local datetimes")
        .with_timezone(&Utc)
}

fn start_of_local_month(now: DateTime<FixedOffset>, offset: FixedOffset) -> DateTime<Utc> {
    offset
        .from_local_datetime(
            &now.date_naive()
                .with_day(1)
                .expect("day 1 is valid")
                .and_hms_opt(0, 0, 0)
                .expect("midnight is valid"),
        )
        .single()
        .expect("fixed offsets have unambiguous local datetimes")
        .with_timezone(&Utc)
}

fn start_of_local_year(now: DateTime<FixedOffset>, offset: FixedOffset) -> DateTime<Utc> {
    offset
        .from_local_datetime(
            &now.date_naive()
                .with_month(1)
                .and_then(|date| date.with_day(1))
                .expect("January 1 is valid")
                .and_hms_opt(0, 0, 0)
                .expect("midnight is valid"),
        )
        .single()
        .expect("fixed offsets have unambiguous local datetimes")
        .with_timezone(&Utc)
}

fn usage_date_key(timestamp: DateTime<Utc>, timezone_offset_minutes: i32) -> String {
    timestamp
        .with_timezone(&fixed_offset(timezone_offset_minutes))
        .format("%Y-%m-%d")
        .to_string()
}

fn fixed_offset(timezone_offset_minutes: i32) -> FixedOffset {
    FixedOffset::east_opt(clamp_timezone_offset(timezone_offset_minutes) * 60)
        .expect("clamped timezone offset is valid")
}

fn clamp_timezone_offset(timezone_offset_minutes: i32) -> i32 {
    timezone_offset_minutes.clamp(-1_439, 1_439)
}

fn normalized_filters(filters: Vec<String>) -> Vec<String> {
    let mut filters = filters
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    filters.sort();
    filters.dedup();
    filters
}

fn matches_text_filter(filters: &[String], value: &str) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .any(|filter| value.eq_ignore_ascii_case(filter))
}

fn safe_average(total: f64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn format_count(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{UsagePeriod, UsageStatRecord, UsageStatsQuery, summarize_usage_records};
    use crate::message::MessageUsage;

    fn record(
        session_id: i64,
        timestamp: chrono::DateTime<Utc>,
        provider: &str,
        model: &str,
        is_subagent: bool,
    ) -> UsageStatRecord {
        UsageStatRecord {
            session_id,
            session_title: format!("session {session_id}"),
            is_subagent,
            created_at: timestamp,
            provider_id: provider.to_string(),
            model_id: model.to_string(),
            usage: MessageUsage {
                input_tokens: 100,
                output_tokens: 20,
                reasoning_tokens: 5,
                cache_write_tokens: 10,
                cache_read_tokens: 40,
                total_cost: 0.25,
            },
        }
    }

    #[test]
    fn period_boundaries_follow_the_requested_timezone() {
        let now = Utc.with_ymd_and_hms(2026, 7, 11, 3, 30, 0).unwrap();
        let today = UsageStatsQuery::for_period_with_offset(UsagePeriod::Today, now, 480);
        assert_eq!(
            today.from,
            Some(Utc.with_ymd_and_hms(2026, 7, 10, 16, 0, 0).unwrap())
        );
        assert_eq!(today.to, Some(now));

        let yesterday = UsageStatsQuery::for_period_with_offset(UsagePeriod::Yesterday, now, 480);
        assert_eq!(
            yesterday.from,
            Some(Utc.with_ymd_and_hms(2026, 7, 9, 16, 0, 0).unwrap())
        );
        assert_eq!(
            yesterday.to,
            Some(
                Utc.with_ymd_and_hms(2026, 7, 10, 15, 59, 59).unwrap()
                    + chrono::Duration::milliseconds(999)
            )
        );
    }

    #[test]
    fn aggregation_applies_filters_and_local_daily_buckets() {
        let generated_at = Utc.with_ymd_and_hms(2026, 7, 11, 12, 0, 0).unwrap();
        let records = vec![
            record(
                1,
                Utc.with_ymd_and_hms(2026, 7, 10, 20, 0, 0).unwrap(),
                "openai",
                "gpt-5",
                false,
            ),
            record(
                2,
                Utc.with_ymd_and_hms(2026, 7, 10, 21, 0, 0).unwrap(),
                "anthropic",
                "claude-sonnet-4",
                true,
            ),
        ];
        let query = UsageStatsQuery::custom(None, Some(generated_at))
            .with_timezone_offset(480)
            .with_filters(
                vec!["OpenAI".to_string()],
                vec!["GPT-5".to_string()],
                Vec::new(),
                false,
            );
        let stats = summarize_usage_records(&records, &query, generated_at);

        assert_eq!(stats.totals.runs, 1);
        assert_eq!(stats.totals.sessions, 1);
        assert_eq!(stats.totals.total_tokens, 175);
        assert_eq!(stats.totals.cache_input_tokens, 150);
        assert_eq!(stats.by_day.len(), 1);
        assert_eq!(stats.by_day[0].date, "2026-07-11");
        assert_eq!(stats.peak_cost_date.as_deref(), Some("2026-07-11"));
        assert_eq!(stats.active_days, 1);
        assert_eq!(stats.by_provider[0].provider_id, "openai");
    }

    #[test]
    fn empty_aggregation_has_finite_derived_metrics() {
        let generated_at = Utc.with_ymd_and_hms(2026, 7, 11, 12, 0, 0).unwrap();
        let query = UsageStatsQuery::for_period(UsagePeriod::Last7Days, generated_at);
        let stats = summarize_usage_records(&[], &query, generated_at);

        assert_eq!(stats.totals.total_tokens, 0);
        assert_eq!(stats.average_cost_per_run_usd, 0.0);
        assert_eq!(stats.average_tokens_per_active_day, 0.0);
        assert!(stats.peak_cost_date.is_none());
    }
}
