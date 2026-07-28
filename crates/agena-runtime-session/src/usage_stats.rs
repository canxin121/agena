use std::collections::{BTreeMap, BTreeSet};

use agena_domain::{
    ModelUsageBreakdown, ProviderUsageBreakdown, SessionUsageBreakdown, UsageBillableUnitTotal,
    UsageDailyBreakdown, UsageStats, UsageStatsQuery, UsageTotals,
};
use chrono::{DateTime, FixedOffset, Utc};

/// One persisted provider-usage observation used to build usage statistics.
///
/// Storage adapters project their rows into this Runtime-owned value. The
/// legacy session-cost reducer may consume it while message/session ownership
/// continues its staged migration.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageStatRecord {
    pub session_id: i64,
    pub session_title: String,
    pub is_subagent: bool,
    pub created_at: DateTime<Utc>,
    pub provider_id: String,
    pub model_id: String,
    pub usage: agena_provider::CompletionUsage,
}

fn fold_billable_units(totals: &mut UsageTotals, usage: &agena_provider::CompletionUsage) {
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

fn fold(
    totals: &mut UsageTotals,
    provider_id: &str,
    model_id: &str,
    usage: &agena_provider::CompletionUsage,
) {
    let requests = usage.requests.max(u64::from(usage.has_own_usage()));
    totals.runs = totals.runs.saturating_add(requests);
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
    let cost = agena_provider::completion_usage_own_cost_contribution(provider_id, model_id, usage);
    totals.total_cost_usd += cost.total_cost_usd;
    totals.recorded_cost_usd += cost.recorded_cost_usd;
    totals.estimated_cost_usd += cost.estimated_cost_usd;
    totals.unpriced_runs = totals.unpriced_runs.saturating_add(cost.unpriced_runs);
    fold_billable_units(totals, usage);
}

fn for_each_usage_observation(
    provider_id: &str,
    model_id: &str,
    usage: &agena_provider::CompletionUsage,
    visitor: &mut impl FnMut(&str, &str, &agena_provider::CompletionUsage),
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
struct TotalsAccumulator {
    totals: UsageTotals,
    sessions: BTreeSet<i64>,
}

impl TotalsAccumulator {
    fn fold(
        &mut self,
        session_id: i64,
        provider_id: &str,
        model_id: &str,
        usage: &agena_provider::CompletionUsage,
    ) {
        self.sessions.insert(session_id);
        fold(&mut self.totals, provider_id, model_id, usage);
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
        provider_id: &str,
        model_id: &str,
        usage: &agena_provider::CompletionUsage,
    ) {
        self.first_message_at = self.first_message_at.min(record.created_at);
        self.last_message_at = self.last_message_at.max(record.created_at);
        self.totals
            .fold(record.session_id, provider_id, model_id, usage);
    }
}

/// Aggregate storage-projected usage records into the Runtime-owned reporting
/// document. This deliberately does not depend on private message/session types.
pub fn summarize_usage_records(
    records: &[UsageStatRecord],
    query: &UsageStatsQuery,
    generated_at: DateTime<Utc>,
) -> UsageStats {
    let mut totals = TotalsAccumulator::default();
    let mut by_day = BTreeMap::<String, TotalsAccumulator>::new();
    let mut by_provider = BTreeMap::<String, TotalsAccumulator>::new();
    let mut by_model = BTreeMap::<(String, String), TotalsAccumulator>::new();
    let mut by_session = BTreeMap::<i64, SessionAccumulator>::new();
    for record in records {
        let date_key = usage_date_key(record.created_at, query.timezone_offset_minutes);
        for_each_usage_observation(
            record.provider_id.as_str(),
            record.model_id.as_str(),
            &record.usage,
            &mut |provider_id, model_id, usage| {
                if !query.matches(record.session_id, record.is_subagent, provider_id, model_id) {
                    return;
                }
                totals.fold(record.session_id, provider_id, model_id, usage);
                by_day.entry(date_key.clone()).or_default().fold(
                    record.session_id,
                    provider_id,
                    model_id,
                    usage,
                );
                by_provider.entry(provider_id.to_owned()).or_default().fold(
                    record.session_id,
                    provider_id,
                    model_id,
                    usage,
                );
                by_model
                    .entry((provider_id.to_owned(), model_id.to_owned()))
                    .or_default()
                    .fold(record.session_id, provider_id, model_id, usage);
                by_session
                    .entry(record.session_id)
                    .or_insert_with(|| SessionAccumulator {
                        title: record.session_title.clone(),
                        is_subagent: record.is_subagent,
                        first_message_at: record.created_at,
                        last_message_at: record.created_at,
                        totals: TotalsAccumulator::default(),
                    })
                    .fold(record, provider_id, model_id, usage);
            },
        );
    }
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
        .map(|(session_id, item)| SessionUsageBreakdown {
            session_id,
            title: item.title,
            is_subagent: item.is_subagent,
            first_message_at: item.first_message_at,
            last_message_at: item.last_message_at,
            totals: item.totals.into_totals(),
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
        average_cost_per_run_usd: average(totals.total_cost_usd, totals.runs),
        average_tokens_per_run: average(totals.total_tokens as f64, totals.runs),
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

fn compare_totals(left: &UsageTotals, right: &UsageTotals) -> std::cmp::Ordering {
    right
        .total_cost_usd
        .partial_cmp(&left.total_cost_usd)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then(right.runs.cmp(&left.runs))
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
mod tests {
    use agena_domain::{UsagePeriod, UsageStatsQuery};
    use chrono::{TimeZone, Utc};

    use super::{UsageStatRecord, summarize_usage_records};

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
            provider_id: provider.to_owned(),
            model_id: model.to_owned(),
            usage: agena_provider::CompletionUsage {
                input_tokens: 100,
                output_tokens: 20,
                reasoning_tokens: 5,
                cache_write_tokens: 10,
                cache_read_tokens: 40,
                total_cost: 0.25,
                ..agena_provider::CompletionUsage::default()
            },
        }
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
                "claude",
                true,
            ),
        ];
        let query = UsageStatsQuery::custom(None, Some(generated_at))
            .with_timezone_offset(480)
            .with_filters(
                vec!["OpenAI".to_owned()],
                vec!["GPT-5".to_owned()],
                Vec::new(),
                false,
            );
        let stats = summarize_usage_records(&records, &query, generated_at);
        assert_eq!(stats.totals.runs, 1);
        assert_eq!(stats.totals.sessions, 1);
        assert_eq!(stats.totals.total_tokens, 175);
        assert_eq!(stats.by_day[0].date, "2026-07-11");
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
