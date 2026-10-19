//! Context occupancy derived from provider-reported usage.

use agena_provider::CompletionUsage;

/// Prompt tokens for the final physical provider request.
///
/// Every adapter reports `input_tokens` as the uncached portion and keeps
/// cached prompt tokens in the cache fields. Cached tokens are still part of
/// the prompt, so context accounting must add them back.
pub(crate) fn prompt_tokens(usage: &CompletionUsage) -> u64 {
    match usage.request_usage.as_ref() {
        Some(request) => request.prompt_tokens(),
        None => usage.own_cache_input_tokens(),
    }
}

/// Prompt plus generated tokens: the context the next request must carry.
pub(crate) fn context_tokens(usage: &CompletionUsage) -> u64 {
    match usage.request_usage.as_ref() {
        Some(request) => request.total_tokens(),
        None => prompt_tokens(usage)
            .saturating_add(usage.output_tokens)
            .saturating_add(usage.reasoning_tokens),
    }
}

#[cfg(test)]
mod tests {
    use super::{context_tokens, prompt_tokens};
    use agena_domain::PromptTokenUsageSnapshot;
    use agena_provider::CompletionUsage;

    #[test]
    fn cached_prompt_tokens_count_towards_context() {
        let usage = CompletionUsage {
            input_tokens: 620,
            output_tokens: 242,
            reasoning_tokens: 0,
            cache_read_tokens: 9_104,
            ..CompletionUsage::default()
        };
        assert_eq!(prompt_tokens(&usage), 9_724);
        assert_eq!(context_tokens(&usage), 9_966);
    }

    #[test]
    fn cache_writes_and_reasoning_are_part_of_the_context() {
        let usage = CompletionUsage {
            input_tokens: 100,
            output_tokens: 20,
            reasoning_tokens: 5,
            cache_write_tokens: 300,
            cache_read_tokens: 900,
            ..CompletionUsage::default()
        };
        assert_eq!(prompt_tokens(&usage), 1_300);
        assert_eq!(context_tokens(&usage), 1_325);
    }

    #[test]
    fn final_request_snapshot_wins_over_aggregated_usage() {
        let usage = CompletionUsage {
            input_tokens: 50_000,
            output_tokens: 9_000,
            reasoning_tokens: 1_000,
            cache_read_tokens: 900_000,
            request_usage: Some(PromptTokenUsageSnapshot {
                input_tokens: 120,
                output_tokens: 40,
                reasoning_tokens: 10,
                cache_write_tokens: 30,
                cache_read_tokens: 800,
            }),
            ..CompletionUsage::default()
        };
        assert_eq!(prompt_tokens(&usage), 950);
        assert_eq!(context_tokens(&usage), 1_000);
    }
}
