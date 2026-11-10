//! Runtime adapter for per-session message-history cost summaries.
//!
//! Cross-session usage-stat aggregation is Runtime-owned because it consumes
//! only Runtime records plus Domain/Provider contracts.

use std::collections::BTreeMap;

use agena_domain::{ModelCostBreakdown, SessionCostSummary};
use agena_provider::CompletionUsage;

fn fold_billable_units(
    totals: &mut Vec<agena_domain::UsageBillableUnitTotal>,
    usage: &CompletionUsage,
) {
    for item in &usage.billable_items {
        let quantity = if item.quantity.is_finite() {
            item.quantity.max(0.0)
        } else {
            0.0
        };
        let index = totals
            .iter()
            .position(|current| current.kind == item.kind && current.unit == item.unit)
            .unwrap_or_else(|| {
                totals.push(agena_domain::UsageBillableUnitTotal {
                    kind: item.kind.clone(),
                    unit: item.unit.clone(),
                    ..Default::default()
                });
                totals.len() - 1
            });
        let current = &mut totals[index];
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

fn fold_model_cost(summary: &mut ModelCostBreakdown, usage: &CompletionUsage) {
    summary.requests = summary
        .requests
        .saturating_add(usage.requests.max(u64::from(usage.has_own_usage())));
    summary.input_tokens = summary.input_tokens.saturating_add(usage.input_tokens);
    summary.output_tokens = summary.output_tokens.saturating_add(usage.output_tokens);
    summary.reasoning_tokens = summary
        .reasoning_tokens
        .saturating_add(usage.reasoning_tokens);
    summary.cache_write_tokens = summary
        .cache_write_tokens
        .saturating_add(usage.cache_write_tokens);
    summary.cache_write_5m_tokens = summary
        .cache_write_5m_tokens
        .saturating_add(usage.cache_write_5m_tokens);
    summary.cache_write_1h_tokens = summary
        .cache_write_1h_tokens
        .saturating_add(usage.cache_write_1h_tokens);
    summary.cache_read_tokens = summary
        .cache_read_tokens
        .saturating_add(usage.cache_read_tokens);
    summary.tool_use_tokens = summary
        .tool_use_tokens
        .saturating_add(usage.tool_use_tokens);
    summary.other_tokens = summary.other_tokens.saturating_add(usage.other_tokens);
    let cost = agena_provider::completion_usage_own_cost_contribution(
        &summary.provider_id,
        &summary.model_id,
        usage,
    );
    summary.total_cost_usd += cost.total_cost_usd;
    summary.recorded_cost_usd += cost.recorded_cost_usd;
    summary.estimated_cost_usd += cost.estimated_cost_usd;
    summary.unpriced_requests = summary
        .unpriced_requests
        .saturating_add(cost.unpriced_requests);
    fold_billable_units(&mut summary.billable_units, usage);
}

fn fold_session_cost(
    summary: &mut SessionCostSummary,
    provider_id: &str,
    model_id: &str,
    usage: &CompletionUsage,
) {
    summary.requests = summary
        .requests
        .saturating_add(usage.requests.max(u64::from(usage.has_own_usage())));
    summary.input_tokens = summary.input_tokens.saturating_add(usage.input_tokens);
    summary.output_tokens = summary.output_tokens.saturating_add(usage.output_tokens);
    summary.reasoning_tokens = summary
        .reasoning_tokens
        .saturating_add(usage.reasoning_tokens);
    summary.cache_write_tokens = summary
        .cache_write_tokens
        .saturating_add(usage.cache_write_tokens);
    summary.cache_write_5m_tokens = summary
        .cache_write_5m_tokens
        .saturating_add(usage.cache_write_5m_tokens);
    summary.cache_write_1h_tokens = summary
        .cache_write_1h_tokens
        .saturating_add(usage.cache_write_1h_tokens);
    summary.cache_read_tokens = summary
        .cache_read_tokens
        .saturating_add(usage.cache_read_tokens);
    summary.tool_use_tokens = summary
        .tool_use_tokens
        .saturating_add(usage.tool_use_tokens);
    summary.other_tokens = summary.other_tokens.saturating_add(usage.other_tokens);
    let cost = agena_provider::completion_usage_own_cost_contribution(provider_id, model_id, usage);
    summary.total_cost_usd += cost.total_cost_usd;
    summary.recorded_cost_usd += cost.recorded_cost_usd;
    summary.estimated_cost_usd += cost.estimated_cost_usd;
    summary.unpriced_requests = summary
        .unpriced_requests
        .saturating_add(cost.unpriced_requests);
    fold_billable_units(&mut summary.billable_units, usage);
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

/// Incremental aggregation over provider-round observations. No transcript
/// Parts or full-session materialization are needed.
#[derive(Default)]
pub(crate) struct CostAccumulator {
    result: SessionCostSummary,
    by_model: BTreeMap<(String, String), ModelCostBreakdown>,
}
impl CostAccumulator {
    pub(crate) fn observe(&mut self, provider_id: &str, model_id: &str, usage: &CompletionUsage) {
        for_each_usage_observation(
            provider_id,
            model_id,
            usage,
            &mut |provider_id, model_id, observation| {
                fold_session_cost(&mut self.result, provider_id, model_id, observation);
                let item = self
                    .by_model
                    .entry((provider_id.into(), model_id.into()))
                    .or_insert_with(|| ModelCostBreakdown {
                        provider_id: provider_id.into(),
                        model_id: model_id.into(),
                        ..Default::default()
                    });
                fold_model_cost(item, observation);
            },
        );
    }
    pub(crate) fn finish(self) -> SessionCostSummary {
        let Self {
            mut result,
            by_model,
        } = self;
        // The reporting document carries its own cache metrics so viewers do not
        // recompute them; keep the values in lockstep with the flat tokens.
        result.refresh_derived_totals();
        result.billable_units.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.unit.cmp(&right.unit))
        });
        result.by_model = by_model
            .into_values()
            .map(|mut item| {
                item.billable_units.sort_by(|left, right| {
                    left.kind
                        .cmp(&right.kind)
                        .then_with(|| left.unit.cmp(&right.unit))
                });
                item.refresh_derived_totals();
                item
            })
            .collect();
        result
    }
}

#[cfg(test)]
mod cache_metric_tests {
    use super::*;

    fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> CompletionUsage {
        CompletionUsage {
            requests: 1,
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
            ..CompletionUsage::default()
        }
    }

    fn summary(observations: &[CompletionUsage]) -> SessionCostSummary {
        let mut accumulator = CostAccumulator::default();
        for observation in observations {
            accumulator.observe("testkit", "model", observation);
        }
        accumulator.finish()
    }

    #[test]
    fn hit_rate_is_cache_read_over_every_cacheable_prompt_token() {
        // 100 fresh + 300 read + 600 written = 1000 cacheable prompt tokens,
        // of which 300 came from the cache.
        let result = summary(&[usage(100, 50, 300, 600)]);
        assert_eq!(result.cache_input_tokens, 1_000);
        assert_eq!(result.cache_read_tokens, 300);
        assert!((result.cache_hit_rate - 0.3).abs() < 1e-12);
        assert_eq!(result.total_tokens, 1_050);
    }

    #[test]
    fn cache_metrics_accumulate_across_requests_and_models() {
        let result = summary(&[
            usage(100, 10, 100, 0),
            usage(100, 10, 0, 100),
            usage(0, 5, 50, 50),
        ]);
        assert_eq!(result.cache_input_tokens, 500);
        assert_eq!(result.cache_read_tokens, 150);
        assert!((result.cache_hit_rate - 0.3).abs() < 1e-12);
        assert_eq!(result.total_tokens, 525);
        let model = &result.by_model[0];
        assert_eq!(model.cache_input_tokens, 500);
        assert!((model.cache_hit_rate - 0.3).abs() < 1e-12);
        assert_eq!(model.total_tokens, result.total_tokens);
    }

    #[test]
    fn a_request_without_cacheable_tokens_reports_no_hit_rate() {
        let result = summary(&[CompletionUsage {
            requests: 1,
            output_tokens: 12,
            ..CompletionUsage::default()
        }]);
        assert_eq!(result.cache_input_tokens, 0);
        assert_eq!(result.cache_hit_rate, 0.0);
        assert_eq!(result.total_tokens, 12);
    }
}
