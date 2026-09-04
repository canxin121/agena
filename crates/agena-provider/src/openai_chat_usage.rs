use serde::Deserialize;

use crate::CompletionUsage;

/// OpenAI-compatible Chat Completions usage payload.
#[derive(Debug, Deserialize)]
pub struct ChatUsage {
    #[serde(default)]
    pub prompt_tokens: Option<u64>,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
    /// xAI reports exact request cost in 10^-10 USD ticks.
    #[serde(default)]
    pub cost_in_usd_ticks: Option<u64>,
    /// GitHub Copilot Chat reports reasoning separately at the usage top level.
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens_details: Option<ChatOutputTokensDetails>,
    #[serde(default)]
    pub output_tokens_details: Option<ChatOutputTokensDetails>,
    #[serde(default)]
    pub prompt_tokens_details: Option<ChatInputTokensDetails>,
    #[serde(default)]
    pub input_tokens_details: Option<ChatInputTokensDetails>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of chat output token details.
pub struct ChatOutputTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of chat input token details.
pub struct ChatInputTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
}

pub fn chat_usage_to_completion(usage: ChatUsage) -> CompletionUsage {
    let prompt_tokens = usage
        .prompt_tokens
        .or(usage.input_tokens)
        .unwrap_or_default();
    let cache_read_tokens = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens)
        .or_else(|| {
            usage
                .input_tokens_details
                .as_ref()
                .and_then(|details| details.cached_tokens)
        })
        .unwrap_or_default();
    let cache_write_tokens = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cache_write_tokens)
        .or_else(|| {
            usage
                .input_tokens_details
                .as_ref()
                .and_then(|details| details.cache_write_tokens)
        })
        .unwrap_or_default();
    let input_tokens = prompt_tokens
        .saturating_sub(cache_read_tokens)
        .saturating_sub(cache_write_tokens);
    let detailed_reasoning_tokens = usage
        .completion_tokens_details
        .and_then(|details| details.reasoning_tokens)
        .or_else(|| {
            usage
                .output_tokens_details
                .and_then(|details| details.reasoning_tokens)
        });
    let raw_output_tokens = usage
        .completion_tokens
        .or(usage.output_tokens)
        .unwrap_or_default();
    let inferred_separate_reasoning = usage.total_tokens.and_then(|total| {
        total
            .checked_sub(prompt_tokens.saturating_add(raw_output_tokens))
            .filter(|tokens| *tokens > 0)
    });
    let reasoning_tokens = usage
        .reasoning_tokens
        .or(detailed_reasoning_tokens)
        .or(inferred_separate_reasoning)
        .unwrap_or_default();
    let total_without_separate_reasoning = prompt_tokens.saturating_add(raw_output_tokens);
    let total_with_separate_reasoning =
        total_without_separate_reasoning.saturating_add(reasoning_tokens);
    let output_includes_reasoning = match usage.total_tokens {
        Some(total)
            if reasoning_tokens > 0
                && total == total_with_separate_reasoning
                && total != total_without_separate_reasoning =>
        {
            false
        }
        Some(total) if total == total_without_separate_reasoning => reasoning_tokens > 0,
        _ => usage.reasoning_tokens.is_none() && detailed_reasoning_tokens.is_some(),
    };
    let output_tokens = if output_includes_reasoning {
        raw_output_tokens.saturating_sub(reasoning_tokens)
    } else {
        raw_output_tokens
    };
    let recorded_cost = usage
        .cost_in_usd_ticks
        .map(|ticks| ticks as f64 / 10_000_000_000.0);
    CompletionUsage {
        requests: 1,
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_write_tokens,
        cache_read_tokens,
        recorded_cost: recorded_cost.unwrap_or_default(),
        recorded_cost_available: recorded_cost.is_some(),
        ..CompletionUsage::default()
    }
}
