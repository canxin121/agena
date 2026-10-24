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
