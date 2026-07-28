//! Runtime adapter for per-session message-history cost summaries.
//!
//! Cross-session usage-stat aggregation is Runtime-owned because it consumes
//! only Runtime records plus Domain/Provider contracts.

use std::collections::BTreeMap;

use agena_domain::{ModelCostBreakdown, Role, SessionCostSummary};
use agena_provider::CompletionUsage;

use crate::message::Message;

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
    summary.runs = summary
        .runs
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
    summary.unpriced_runs = summary.unpriced_runs.saturating_add(cost.unpriced_runs);
    fold_billable_units(&mut summary.billable_units, usage);
}

fn fold_session_cost(
    summary: &mut SessionCostSummary,
    provider_id: &str,
    model_id: &str,
    usage: &CompletionUsage,
) {
    summary.runs = summary
        .runs
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
    summary.unpriced_runs = summary.unpriced_runs.saturating_add(cost.unpriced_runs);
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

/// Build a per-session summary from Runtime's concrete message history.
pub(crate) fn summarize(messages: &[Message]) -> SessionCostSummary {
    let mut result = SessionCostSummary::default();
    let mut by_model: BTreeMap<(String, String), ModelCostBreakdown> = BTreeMap::new();
    for message in messages {
        if message.role != Role::Assistant {
            continue;
        }
        let Some(usage) = message.usage.as_ref() else {
            continue;
        };
        for_each_usage_observation(
            message.metadata.model_provider_id.as_str(),
            message.metadata.model_id.as_str(),
            usage,
            &mut |provider_id, model_id, observation| {
                fold_session_cost(&mut result, provider_id, model_id, observation);
                let item = by_model
                    .entry((provider_id.to_owned(), model_id.to_owned()))
                    .or_insert_with(|| ModelCostBreakdown {
                        provider_id: provider_id.to_owned(),
                        model_id: model_id.to_owned(),
                        ..ModelCostBreakdown::default()
                    });
                fold_model_cost(item, observation);
            },
        );
    }
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
            item
        })
        .collect();
    result
}
