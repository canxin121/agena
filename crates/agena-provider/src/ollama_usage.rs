//! Ollama usage normalization.

use crate::{CompletionUsage, OllamaChatResponse};

/// Return usage only when Ollama reported at least one token counter.
pub fn ollama_usage_to_completion(response: &OllamaChatResponse) -> Option<CompletionUsage> {
    let input_tokens = response.prompt_eval_count.unwrap_or_default();
    let output_tokens = response.eval_count.unwrap_or_default();
    (input_tokens > 0 || output_tokens > 0).then_some(CompletionUsage {
        input_tokens,
        output_tokens,
        reasoning_tokens: 0,
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        total_cost: 0.0,
    })
}
