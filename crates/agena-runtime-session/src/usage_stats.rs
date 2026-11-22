use std::collections::{BTreeMap, BTreeSet};

use agena_domain::{
    ModelRef, ModelUsageBreakdown, ProviderUsageBreakdown, SessionUsageBreakdown,
    UsageBillableUnitTotal, UsageDailyBreakdown, UsagePerformanceTotals, UsageStats,
    UsageStatsQuery, UsageTotals,
};
use agena_provider::{AttributedCompletionUsage, CompletionUsage, CompletionUsageCostContribution};
use agena_storage::kernel::{StoredUsageObservation, StoredUsageSource};
use chrono::{DateTime, FixedOffset, Utc};

use crate::session::kernel::KernelProviderState;

/// One persisted provider-usage observation used to build usage statistics.
///
/// Storage adapters project their rows into this Runtime-owned value so usage
/// totals and cost summaries share one observation shape.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageStatRecord {
    pub session_id: i64,
    pub session_title: String,
    pub is_subagent: bool,
    pub created_at: DateTime<Utc>,
    pub provider_id: String,
    pub model_id: String,
    pub usage: CompletionUsage,
}

#[derive(serde::Deserialize)]
struct RoundUsage {
    version: u8,
    model: ModelRef,
    usage: CompletionUsage,
}

impl UsageStatRecord {
    pub(crate) fn from_stored(observation: StoredUsageObservation) -> Option<Self> {
        let attribution = match observation.source {
            StoredUsageSource::Round(payload) => serde_json::from_str::<RoundUsage>(&payload)
                .ok()
                .filter(|round| round.version == KernelProviderState::VERSION)
                .map(|round| {
                    (
                        round.model.provider_id.to_string(),
                        round.model.model_id.to_string(),
                        round.usage,
                    )
                }),
            StoredUsageSource::Tool(payload) => {
                serde_json::from_str::<AttributedCompletionUsage>(&payload)
                    .ok()
                    .map(|tool| (tool.provider_id, tool.model_id, *tool.usage))
            }
        };
        let Some((provider_id, model_id, usage)) = attribution else {
            tracing::warn!(
                session_id = observation.session_id.get(),
                "skipping an unreadable usage observation"
            );
            return None;
        };
        Some(Self {
            session_id: observation.session_id.get(),
            session_title: observation.session_title,
            is_subagent: observation.is_subagent,
            created_at: DateTime::from_timestamp_millis(observation.created_at_ms)?,
            provider_id,
            model_id,
            usage,
        })
    }
}

fn fold_billable_units(totals: &mut UsageTotals, usage: &CompletionUsage) {
    for item in &usage.billable_items {
        let quantity = if item.quantity.is_finite() {
            item.quantity.max(0.0)
        } else {
            0.0
        };
        let index = totals
            .billable_units
            .iter()
            .position(|current| current.kind == item.kind && current.unit == item.unit)
            .unwrap_or_else(|| {
                totals.billable_units.push(UsageBillableUnitTotal {
                    kind: item.kind.clone(),
                    unit: item.unit.clone(),
                    ..UsageBillableUnitTotal::default()
                });
                totals.billable_units.len() - 1
            });
        let current = &mut totals.billable_units[index];
        current.quantity += quantity;
        if let Some(cost) = item
            .cost_usd
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            current.priced_quantity += quantity;
            current.estimated_cost_usd += cost;
        } else {
            current.unpriced_quantity += quantity;
        }
    }
}

fn fold(totals: &mut UsageTotals, usage: &CompletionUsage, cost: CompletionUsageCostContribution) {
    let requests = usage.requests.max(u64::from(usage.has_own_usage()));
    totals.requests = totals.requests.saturating_add(requests);
    totals.input_tokens = totals.input_tokens.saturating_add(usage.input_tokens);
    totals.output_tokens = totals.output_tokens.saturating_add(usage.output_tokens);
    totals.reasoning_tokens = totals
        .reasoning_tokens
        .saturating_add(usage.reasoning_tokens);
    totals.cache_write_tokens = totals
        .cache_write_tokens
        .saturating_add(usage.cache_write_tokens);
    totals.cache_write_5m_tokens = totals
        .cache_write_5m_tokens
        .saturating_add(usage.cache_write_5m_tokens);
    totals.cache_write_1h_tokens = totals
        .cache_write_1h_tokens
        .saturating_add(usage.cache_write_1h_tokens);
    totals.cache_read_tokens = totals
        .cache_read_tokens
        .saturating_add(usage.cache_read_tokens);
    totals.tool_use_tokens = totals.tool_use_tokens.saturating_add(usage.tool_use_tokens);
    totals.other_tokens = totals.other_tokens.saturating_add(usage.other_tokens);
    totals.total_cost_usd += cost.total_cost_usd;
    totals.recorded_cost_usd += cost.recorded_cost_usd;
    totals.estimated_cost_usd += cost.estimated_cost_usd;
    totals.unpriced_requests = totals
        .unpriced_requests
        .saturating_add(cost.unpriced_requests);
    fold_billable_units(totals, usage);
}

fn for_each_usage_observation(
    provider_id: &str,
    model_id: &str,
    usage: &CompletionUsage,
    visitor: &mut impl FnMut(&str, &str, &CompletionUsage),
) {
    if usage.has_own_usage() {
        visitor(provider_id, model_id, usage);
    }
    for attributed in &usage.attributed_usage {
        for_each_usage_observation(
            attributed.provider_id.as_str(),
            attributed.model_id.as_str(),
            attributed.usage.as_ref(),
            visitor,
        );
    }
}

#[derive(Debug, Clone, Default)]
struct PerformanceAccumulator {
    totals: UsagePerformanceTotals,
    durations: Vec<u64>,
    first_outputs: Vec<u64>,
}

impl PerformanceAccumulator {
    fn observe(&mut self, sample: &agena_domain::RequestPerformance) {
        let totals = &mut self.totals;
        totals.request_count = totals.request_count.saturating_add(1);
        totals.total_duration_ms = totals.total_duration_ms.saturating_add(sample.duration_ms);
        self.durations.push(sample.duration_ms);

        // Reject inconsistent imported timings without losing the other useful
        // measurements on the request. A real zero-millisecond TTFT is valid.
        let ttft = sample.ttft_ms.filter(|value| *value <= sample.duration_ms);
        if let Some(ttft) = ttft {
            totals.ttft_count = totals.ttft_count.saturating_add(1);
            totals.total_ttft_ms = totals.total_ttft_ms.saturating_add(ttft);
            self.first_outputs.push(ttft);
        }
        let Some(tokens) = sample.generated_tokens.filter(|tokens| *tokens > 0) else {
            return;
        };
        if sample.duration_ms > 0 {
            totals.throughput_count = totals.throughput_count.saturating_add(1);
            totals.timed_tokens = totals.timed_tokens.saturating_add(tokens);
            totals.timed_duration_ms = totals.timed_duration_ms.saturating_add(sample.duration_ms);
        }
        if let Some(duration) = sample.generation_ms.filter(|duration| {
            tokens > 1
                && *duration > 0
                && ttft.is_some_and(|ttft| *duration <= sample.duration_ms - ttft)
        }) {
            totals.generation_count = totals.generation_count.saturating_add(1);
            totals.generation_tokens = totals.generation_tokens.saturating_add(tokens);
            totals.total_generation_ms = totals.total_generation_ms.saturating_add(duration);
        }
    }

    fn finish(mut self) -> UsagePerformanceTotals {
        let totals = &mut self.totals;
        totals.average_duration_ms = measured_ratio(totals.total_duration_ms, totals.request_count);
        totals.average_ttft_ms = measured_ratio(totals.total_ttft_ms, totals.ttft_count);
        totals.tokens_per_second =
            measured_ratio(totals.generation_tokens, totals.total_generation_ms)
                .map(|tokens_per_ms| tokens_per_ms * 1_000.0);
        totals.end_to_end_tokens_per_second =
            measured_ratio(totals.timed_tokens, totals.timed_duration_ms)
                .map(|tokens_per_ms| tokens_per_ms * 1_000.0);
        totals.p50_duration_ms = percentile(&mut self.durations, 50);
        totals.p95_duration_ms = percentile(&mut self.durations, 95);
        totals.p50_ttft_ms = percentile(&mut self.first_outputs, 50);
        totals.p95_ttft_ms = percentile(&mut self.first_outputs, 95);
        self.totals
    }
}

fn measured_ratio(total: u64, count: u64) -> Option<f64> {
    (count > 0).then(|| total as f64 / count as f64)
}

/// Nearest rank: at least this percentage of measurements is at or below it.
fn percentile(samples: &mut [u64], percent: usize) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let rank = (samples.len() as u128 * percent as u128).div_ceil(100) as usize;
    Some(*samples.select_nth_unstable(rank - 1).1)
}

#[derive(Debug, Clone, Default)]
struct TotalsAccumulator {
    totals: UsageTotals,
    sessions: BTreeSet<i64>,
    performance: PerformanceAccumulator,
}

impl TotalsAccumulator {
    fn fold(
        &mut self,
        session_id: i64,
        usage: &CompletionUsage,
        cost: CompletionUsageCostContribution,
    ) {
        self.sessions.insert(session_id);
        self.fold_usage(usage, cost);
    }

    fn fold_usage(&mut self, usage: &CompletionUsage, cost: CompletionUsageCostContribution) {
        fold(&mut self.totals, usage, cost);
        for sample in &usage.performance {
            self.performance.observe(sample);
        }
    }

    fn into_totals(mut self) -> UsageTotals {
        self.totals.sessions = self.sessions.len() as u64;
        self.totals.total_tokens = self
            .totals
            .input_tokens
            .saturating_add(self.totals.output_tokens)
            .saturating_add(self.totals.reasoning_tokens)
            .saturating_add(self.totals.cache_write_tokens)
            .saturating_add(self.totals.cache_read_tokens)
            .saturating_add(self.totals.other_tokens);
        self.totals.cache_input_tokens = self
            .totals
            .input_tokens
            .saturating_add(self.totals.cache_write_tokens)
            .saturating_add(self.totals.cache_read_tokens);
        self.totals.cache_hit_rate = ratio(
            self.totals.cache_read_tokens,
            self.totals.cache_input_tokens,
        );
        self.totals.billable_units.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.unit.cmp(&right.unit))
        });
        self.totals.performance = self.performance.finish();
        self.totals
    }
}

#[derive(Debug, Clone)]
struct SessionAccumulator {
    title: String,
    is_subagent: bool,
    first_message_at: DateTime<Utc>,
    last_message_at: DateTime<Utc>,
    totals: TotalsAccumulator,
}

impl SessionAccumulator {
    fn fold(
        &mut self,
        record: &UsageStatRecord,
        usage: &CompletionUsage,
        cost: CompletionUsageCostContribution,
    ) {
        self.first_message_at = self.first_message_at.min(record.created_at);
        self.last_message_at = self.last_message_at.max(record.created_at);
        self.totals.fold_usage(usage, cost);
    }
}

/// Aggregate storage-projected usage records into the Runtime-owned reporting
/// document. This deliberately does not depend on private message/session types.
#[derive(Default)]
pub(crate) struct UsageStatsAccumulator {
    totals: TotalsAccumulator,
    by_day: BTreeMap<String, TotalsAccumulator>,
    by_provider: BTreeMap<String, TotalsAccumulator>,
    by_model: BTreeMap<(String, String), TotalsAccumulator>,
    by_session: BTreeMap<i64, SessionAccumulator>,
}
impl UsageStatsAccumulator {
    pub(crate) fn observe(&mut self, record: &UsageStatRecord, query: &UsageStatsQuery) {
        if query.from.is_some_and(|from| record.created_at < from)
            || query.to.is_some_and(|to| record.created_at >= to)
        {
            return;
        }
        let Self {
            totals,
            by_day,
            by_provider,
            by_model,
            by_session,
        } = self;
        let date_key = usage_date_key(record.created_at, query.timezone_offset_minutes);
        for_each_usage_observation(
            record.provider_id.as_str(),
            record.model_id.as_str(),
            &record.usage,
            &mut |provider_id, model_id, usage| {
                if !query.matches(record.session_id, record.is_subagent, provider_id, model_id) {
                    return;
                }
                let cost = agena_provider::completion_usage_own_cost_contribution(
                    provider_id,
                    model_id,
                    usage,
                );
                totals.fold(record.session_id, usage, cost);
                by_day
                    .entry(date_key.clone())
                    .or_default()
                    .fold(record.session_id, usage, cost);
                by_provider.entry(provider_id.to_owned()).or_default().fold(
                    record.session_id,
                    usage,
                    cost,
                );
                by_model
                    .entry((provider_id.to_owned(), model_id.to_owned()))
                    .or_default()
                    .fold(record.session_id, usage, cost);
                by_session
                    .entry(record.session_id)
                    .or_insert_with(|| SessionAccumulator {
                        title: record.session_title.clone(),
                        is_subagent: record.is_subagent,
                        first_message_at: record.created_at,
                        last_message_at: record.created_at,
                        totals: TotalsAccumulator::default(),
                    })
                    .fold(record, usage, cost);
            },
        );
    }
    pub(crate) fn finish(self, query: &UsageStatsQuery, generated_at: DateTime<Utc>) -> UsageStats {
        let Self {
            totals,
            by_day,
            by_provider,
            by_model,
            by_session,
        } = self;
        let mut by_provider = by_provider
            .into_iter()
            .map(|(provider_id, totals)| ProviderUsageBreakdown {
                provider_id,
                totals: totals.into_totals(),
            })
            .collect::<Vec<_>>();
        by_provider.sort_by(|left, right| compare_totals(&left.totals, &right.totals));
        let mut by_model = by_model
            .into_iter()
            .map(|((provider_id, model_id), totals)| ModelUsageBreakdown {
                provider_id,
                model_id,
                totals: totals.into_totals(),
            })
            .collect::<Vec<_>>();
        by_model.sort_by(|left, right| compare_totals(&left.totals, &right.totals));
        let mut by_session = by_session
            .into_iter()
            .map(|(session_id, item)| {
                let mut totals = item.totals.into_totals();
                totals.sessions = 1;
                SessionUsageBreakdown {
                    session_id,
                    title: item.title,
                    is_subagent: item.is_subagent,
                    first_message_at: item.first_message_at,
                    last_message_at: item.last_message_at,
                    totals,
                }
            })
            .collect::<Vec<_>>();
        by_session.sort_by(|left, right| compare_totals(&left.totals, &right.totals));
        let by_day = by_day
            .into_iter()
            .map(|(date, totals)| UsageDailyBreakdown {
                date,
                totals: totals.into_totals(),
            })
            .collect::<Vec<_>>();
        let totals = totals.into_totals();
        let peak_cost = by_day.iter().max_by(|left, right| {
            left.totals
                .total_cost_usd
                .partial_cmp(&right.totals.total_cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let peak_tokens = by_day.iter().max_by_key(|day| day.totals.total_tokens);
        UsageStats {
            generated_at,
            period: query.period,
            period_label: query.period.label().to_string(),
            from: query.from.to_owned(),
            to: query.to.to_owned(),
            timezone_offset_minutes: query.timezone_offset_minutes,
            active_days: by_day.len() as u64,
            average_cost_per_request_usd: average(totals.total_cost_usd, totals.requests),
            average_tokens_per_request: average(totals.total_tokens as f64, totals.requests),
            average_cost_per_active_day_usd: average(totals.total_cost_usd, by_day.len() as u64),
            average_tokens_per_active_day: average(totals.total_tokens as f64, by_day.len() as u64),
            peak_cost_date: peak_cost.map(|item| item.date.clone()),
            peak_cost_usd: peak_cost
                .map(|item| item.totals.total_cost_usd)
                .unwrap_or_default(),
            peak_tokens_date: peak_tokens.map(|item| item.date.clone()),
            peak_tokens: peak_tokens
                .map(|item| item.totals.total_tokens)
                .unwrap_or_default(),
            totals,
            by_day,
            by_provider,
            by_model,
            by_session,
        }
    }
}

pub fn summarize_usage_records(
    records: &[UsageStatRecord],
    query: &UsageStatsQuery,
    generated_at: DateTime<Utc>,
) -> UsageStats {
    let mut result = UsageStatsAccumulator::default();
    for record in records {
        result.observe(record, query);
    }
    result.finish(query, generated_at)
}

fn compare_totals(left: &UsageTotals, right: &UsageTotals) -> std::cmp::Ordering {
    right
        .total_cost_usd
        .partial_cmp(&left.total_cost_usd)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then(right.requests.cmp(&left.requests))
}

fn usage_date_key(timestamp: DateTime<Utc>, timezone_offset_minutes: i32) -> String {
    timestamp
        .with_timezone(&fixed_offset(timezone_offset_minutes))
        .format("%Y-%m-%d")
        .to_string()
}

fn fixed_offset(minutes: i32) -> FixedOffset {
    FixedOffset::east_opt(minutes.clamp(-1_439, 1_439) * 60)
        .expect("clamped timezone offset is valid")
}

fn average(total: f64, count: u64) -> f64 {
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

#[cfg(test)]
mod tests;
