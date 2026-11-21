use serde::Deserialize;

use crate::CompletionUsage;

/// OpenAI-compatible Chat Completions usage payload.
#[derive(Debug, Default, Deserialize)]
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
    #[serde(default)]
    pub cache_creation_tokens: Option<u64>,
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
        .and_then(|details| details.cache_write_tokens.or(details.cache_creation_tokens))
        .or_else(|| {
            usage
                .input_tokens_details
                .as_ref()
                .and_then(|details| details.cache_write_tokens.or(details.cache_creation_tokens))
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
    let reasoning_tokens = usage
        .reasoning_tokens
        .or(detailed_reasoning_tokens)
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
    let mut normalized = CompletionUsage {
        requests: 1,
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_write_tokens,
        cache_read_tokens,
        recorded_cost: recorded_cost.unwrap_or_default(),
        recorded_cost_available: recorded_cost.is_some(),
        ..CompletionUsage::default()
    };
    let classified = normalized.own_total_tokens();
    // A total may include extra categories or be the only counter returned by
    // a compatible endpoint. Leave the unexplained part available for billing
    // totals without assigning it to reasoning or generation speed.
    normalized.other_tokens = usage
        .total_tokens
        .unwrap_or_default()
        .saturating_sub(classified);
    normalized.cost_estimate_incomplete = recorded_cost.is_none()
        && usage
            .total_tokens
            .is_some_and(|total| total > 0 && total < classified);
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn partial_usage_preserves_unclassified_tokens_instead_of_inflating_generation() {
        for (wire, expected_input, expected_output, expected_other) in [
            (json!({"total_tokens": 500}), 0, 0, 500),
            (
                json!({"prompt_tokens": 300, "completion_tokens": 100, "total_tokens": 500}),
                300,
                100,
                100,
            ),
        ] {
            let usage = chat_usage_to_completion(serde_json::from_value(wire).unwrap());
            assert_eq!(usage.input_tokens, expected_input);
            assert_eq!(usage.output_tokens, expected_output);
            assert_eq!(usage.reasoning_tokens, 0);
            assert_eq!(usage.other_tokens, expected_other);
            assert_eq!(usage.own_total_tokens(), 500);
        }
    }

    #[test]
    fn explicit_cache_and_reasoning_reconcile_in_chat_and_responses() {
        let wire = json!({
            "input_tokens": 100, "output_tokens": 40, "total_tokens": 150,
            "input_tokens_details": {"cached_tokens": 25, "cache_creation_tokens": 5},
            "output_tokens_details": {"reasoning_tokens": 10},
        });
        let chat = chat_usage_to_completion(serde_json::from_value(wire.clone()).unwrap());
        let responses =
            crate::openai_responses_usage_to_completion(serde_json::from_value(wire).unwrap());
        assert_eq!(chat, responses);
        assert_eq!(chat.input_tokens, 70);
        assert_eq!(chat.cache_read_tokens, 25);
        assert_eq!(chat.cache_write_tokens, 5);
        assert_eq!(chat.output_tokens, 40);
        assert_eq!(chat.reasoning_tokens, 10);
        assert_eq!(chat.own_total_tokens(), 150);

        let subset: ChatUsage = serde_json::from_value(json!({
            "prompt_tokens": 100, "completion_tokens": 40, "total_tokens": 147,
            "completion_tokens_details": {"reasoning_tokens": 10},
        }))
        .unwrap();
        let usage = chat_usage_to_completion(subset);
        assert_eq!(usage.output_tokens, 30);
        assert_eq!(usage.reasoning_tokens, 10);
        assert_eq!(usage.other_tokens, 7);
        assert_eq!(usage.own_total_tokens(), 147);
    }
}
