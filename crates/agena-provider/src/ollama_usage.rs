//! Ollama usage normalization.

use crate::{CompletionUsage, OllamaChatResponse};

/// Return usage only when Ollama reported at least one token counter.
pub fn ollama_usage_to_completion(response: &OllamaChatResponse) -> Option<CompletionUsage> {
    let input_tokens = response.prompt_eval_count.unwrap_or_default();
    let output_tokens = response.eval_count.unwrap_or_default();
    (input_tokens > 0 || output_tokens > 0).then_some(CompletionUsage {
        requests: 1,
        input_tokens,
        output_tokens,
        ..CompletionUsage::default()
    })
}
