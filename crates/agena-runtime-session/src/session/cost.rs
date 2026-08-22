//! Runtime adapter for per-session message-history cost summaries.
//!
//! Cross-session usage-stat aggregation is Runtime-owned because it consumes
//! only Runtime records plus Domain/Provider contracts.

use std::collections::BTreeMap;

use agena_domain::{ModelCostBreakdown, SessionCostSummary};
use agena_provider::CompletionUsage;
use agena_storage::store::{Part, PartRole};

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

/// Parse a `usage` object embedded in a run marker's content into a provider
/// `CompletionUsage`. Tolerant: any subset of fields parses (serde defaults
/// fill the rest); empty objects are ignored.
fn usage_from_run_marker_content(content: &serde_json::Value) -> Option<CompletionUsage> {
    let usage = content.get("usage")?;
    let parsed: CompletionUsage = match serde_json::from_value(usage.clone()) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode persisted run-marker usage for cost summary",
                    &error,
                ),
                "session cost summary skipped malformed persisted usage"
            );
            return None;
        }
    };
    (parsed.requests > 0 || parsed.own_total_tokens() > 0).then_some(parsed)
}

/// Build a per-session summary from the parts projection.
///
/// Usage and model identity are read from the assistant run markers, which are
/// the durable home for run cost input (the engine folds attributed usage into
/// `content["usage"]`, mirroring the v2 `aggregate_usage()` projection).
pub(crate) fn summarize(parts: &[Part]) -> SessionCostSummary {
    let mut result = SessionCostSummary::default();
    let mut by_model: BTreeMap<(String, String), ModelCostBreakdown> = BTreeMap::new();
    for part in parts {
        if !part.is_run_marker() || part.role != PartRole::Assistant {
            continue;
        }
        let Some(usage) = usage_from_run_marker_content(&part.content) else {
            continue;
        };
        let provider_id = part
            .content
            .get("provider_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let model_id = part
            .content
            .get("model_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        for_each_usage_observation(
            provider_id,
            model_id,
            &usage,
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
