//! Runtime adapter for per-session message-history cost summaries.
//!
//! Cross-session usage-stat aggregation is Runtime-owned because it consumes
//! only Runtime records plus Domain/Provider contracts.

use std::collections::BTreeMap;

use agena_domain::{ModelCostBreakdown, Role, SessionCostSummary};
use agena_provider::CompletionUsage;

use crate::message::Message;

fn fold_model_cost(summary: &mut ModelCostBreakdown, usage: &CompletionUsage) {
    summary.runs = summary.runs.saturating_add(1);
    summary.input_tokens = summary.input_tokens.saturating_add(usage.input_tokens);
    summary.output_tokens = summary.output_tokens.saturating_add(usage.output_tokens);
    summary.reasoning_tokens = summary
        .reasoning_tokens
        .saturating_add(usage.reasoning_tokens);
    summary.cache_write_tokens = summary
        .cache_write_tokens
        .saturating_add(usage.cache_write_tokens);
    summary.cache_read_tokens = summary
        .cache_read_tokens
        .saturating_add(usage.cache_read_tokens);
    let cost = agena_provider::completion_usage_cost_contribution(
        &summary.provider_id,
        &summary.model_id,
        usage,
    );
    summary.total_cost_usd += cost.total_cost_usd;
    summary.recorded_cost_usd += cost.recorded_cost_usd;
    summary.estimated_cost_usd += cost.estimated_cost_usd;
    summary.unpriced_runs = summary.unpriced_runs.saturating_add(cost.unpriced_runs);
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
        let provider_id = message.metadata.model_provider_id.clone();
        let model_id = message.metadata.model_id.clone();
        result.runs = result.runs.saturating_add(1);
        result.input_tokens = result.input_tokens.saturating_add(usage.input_tokens);
        result.output_tokens = result.output_tokens.saturating_add(usage.output_tokens);
        result.reasoning_tokens = result
            .reasoning_tokens
            .saturating_add(usage.reasoning_tokens);
        result.cache_write_tokens = result
            .cache_write_tokens
            .saturating_add(usage.cache_write_tokens);
        result.cache_read_tokens = result
            .cache_read_tokens
            .saturating_add(usage.cache_read_tokens);
        let cost =
            agena_provider::completion_usage_cost_contribution(&provider_id, &model_id, usage);
        result.total_cost_usd += cost.total_cost_usd;
        result.recorded_cost_usd += cost.recorded_cost_usd;
        result.estimated_cost_usd += cost.estimated_cost_usd;
        result.unpriced_runs = result.unpriced_runs.saturating_add(cost.unpriced_runs);
        let item = by_model
            .entry((provider_id.clone(), model_id.clone()))
            .or_insert_with(|| ModelCostBreakdown {
                provider_id,
                model_id,
                ..ModelCostBreakdown::default()
            });
        fold_model_cost(item, usage);
    }
    result.by_model = by_model.into_values().collect();
    result
}
