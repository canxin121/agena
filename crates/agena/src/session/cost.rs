//! Session-level cost / token aggregation.
//!
//! Walks a session's assistant messages, sums up their `MessageUsage`
//! totals, and produces a [`SessionCostSummary`] suitable for `/cost`-style
//! UI panels. Per-model breakdowns are preserved so a session that switched
//! providers mid-flight surfaces both contributions.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Duration, Utc};
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
    #[serde(rename = "last_7_days")]
    Last7Days,
    #[serde(rename = "last_30_days")]
    Last30Days,
    MonthToDate,
    AllTime,
}

impl UsagePeriod {
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Last7Days => "last_7_days",
            Self::Last30Days => "last_30_days",
            Self::MonthToDate => "month_to_date",
            Self::AllTime => "all_time",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageStatsQuery {
    pub period: UsagePeriod,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl UsageStatsQuery {
    pub fn for_period(period: UsagePeriod, now: DateTime<Utc>) -> Self {
        let from = match period {
            UsagePeriod::Today => Some(start_of_day(now)),
            UsagePeriod::Last7Days => Some(now - Duration::days(7)),
            UsagePeriod::Last30Days => Some(now - Duration::days(30)),
            UsagePeriod::MonthToDate => Some(start_of_month(now)),
            UsagePeriod::AllTime => None,
        };
        Self {
            period,
            from,
            to: Some(now),
        }
    }

    pub fn custom(from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>) -> Self {
        Self {
            period: UsagePeriod::AllTime,
            from,
            to,
        }
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
            .saturating_add(self.reasoning_tokens);
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
    pub totals: UsageTotals,
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
struct UsageTotalsBuilder {
    totals: UsageTotals,
    sessions: BTreeSet<i64>,
}

impl UsageTotalsBuilder {
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
struct SessionUsageBuilder {
    title: String,
    is_subagent: bool,
    first_message_at: DateTime<Utc>,
    last_message_at: DateTime<Utc>,
    totals: UsageTotalsBuilder,
}

impl SessionUsageBuilder {
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
    let mut totals = UsageTotalsBuilder::default();
    let mut by_day: BTreeMap<String, UsageTotalsBuilder> = BTreeMap::new();
    let mut by_provider: BTreeMap<String, UsageTotalsBuilder> = BTreeMap::new();
    let mut by_model: BTreeMap<(String, String), UsageTotalsBuilder> = BTreeMap::new();
    let mut by_session: BTreeMap<i64, SessionUsageBuilder> = BTreeMap::new();

    for record in records {
        totals.fold(
            record.session_id,
            &record.provider_id,
            &record.model_id,
            &record.usage,
        );

        by_day
            .entry(record.created_at.format("%Y-%m-%d").to_string())
            .or_default()
            .fold(
                record.session_id,
                &record.provider_id,
                &record.model_id,
                &record.usage,
            );
        by_provider
            .entry(record.provider_id.clone())
            .or_default()
            .fold(
                record.session_id,
                &record.provider_id,
                &record.model_id,
                &record.usage,
            );
        by_model
            .entry((record.provider_id.clone(), record.model_id.clone()))
            .or_default()
            .fold(
                record.session_id,
                &record.provider_id,
                &record.model_id,
                &record.usage,
            );
        by_session
            .entry(record.session_id)
            .or_insert_with(|| SessionUsageBuilder {
                title: record.session_title.clone(),
                is_subagent: record.is_subagent,
                first_message_at: record.created_at,
                last_message_at: record.created_at,
                totals: UsageTotalsBuilder::default(),
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

    UsageStats {
        generated_at,
        period: query.period,
        period_label: query.period.label().to_string(),
        from: query.from.to_owned(),
        to: query.to.to_owned(),
        totals: totals.into_totals(),
        by_day: by_day
            .into_iter()
            .map(|(date, builder)| UsageDailyBreakdown {
                date,
                totals: builder.into_totals(),
            })
            .collect(),
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
    let mut totals = SessionCostSummary::default();
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

        totals.runs = totals.runs.saturating_add(1);
        totals.input_tokens = totals.input_tokens.saturating_add(usage.input_tokens);
        totals.output_tokens = totals.output_tokens.saturating_add(usage.output_tokens);
        totals.reasoning_tokens = totals
            .reasoning_tokens
            .saturating_add(usage.reasoning_tokens);
        totals.cache_write_tokens = totals
            .cache_write_tokens
            .saturating_add(usage.cache_write_tokens);
        totals.cache_read_tokens = totals
            .cache_read_tokens
            .saturating_add(usage.cache_read_tokens);
        let cost = cost_contribution(&provider, &model, usage);
        totals.total_cost_usd += cost.total_cost_usd;
        totals.recorded_cost_usd += cost.recorded_cost_usd;
        totals.estimated_cost_usd += cost.estimated_cost_usd;
        totals.unpriced_runs = totals.unpriced_runs.saturating_add(cost.unpriced_runs);

        let entry = by_key
            .entry((provider.clone(), model.clone()))
            .or_insert_with(|| ModelCostBreakdown {
                provider_id: provider,
                model_id: model,
                ..ModelCostBreakdown::default()
            });
        entry.fold(usage);
    }

    totals.by_model = by_key.into_values().collect();
    totals
}

fn start_of_day(now: DateTime<Utc>) -> DateTime<Utc> {
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight is valid")
        .and_utc()
}

fn start_of_month(now: DateTime<Utc>) -> DateTime<Utc> {
    now.date_naive()
        .with_day(1)
        .expect("day 1 is valid")
        .and_hms_opt(0, 0, 0)
        .expect("midnight is valid")
        .and_utc()
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
