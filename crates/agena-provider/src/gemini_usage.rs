//! Gemini usage wire records and provider-neutral token normalization.

use serde::Deserialize;

use crate::CompletionUsage;

#[derive(Debug, Deserialize, Clone)]
pub struct GeminiUsageMetadata {
    #[serde(default, rename = "promptTokenCount")]
    pub prompt_token_count: Option<u64>,
    #[serde(default, rename = "candidatesTokenCount")]
    pub candidates_token_count: Option<u64>,
    #[serde(default, rename = "thoughtsTokenCount")]
    pub thoughts_token_count: Option<u64>,
    #[serde(default, rename = "cachedContentTokenCount")]
    pub cached_content_token_count: Option<u64>,
    #[serde(default, rename = "toolUsePromptTokenCount")]
    pub tool_use_prompt_token_count: Option<u64>,
    #[serde(default, rename = "totalTokenCount")]
    pub total_token_count: Option<u64>,
}

/// Normalize Gemini's inclusive prompt total and separate thought count into
/// Agena's provider-wide input/output/reasoning usage convention.
pub fn gemini_usage_to_completion(usage: GeminiUsageMetadata) -> CompletionUsage {
    let prompt_tokens = usage.prompt_token_count.unwrap_or_default();
    let cache_read_tokens = usage.cached_content_token_count.unwrap_or_default();
    let reasoning_tokens = usage.thoughts_token_count.unwrap_or_default();
    let output_tokens = usage.candidates_token_count.unwrap_or_default();
    let known = prompt_tokens
        .saturating_add(output_tokens)
        .saturating_add(reasoning_tokens);
    CompletionUsage {
        requests: 1,
        input_tokens: prompt_tokens.saturating_sub(cache_read_tokens),
        output_tokens,
        reasoning_tokens,
        cache_read_tokens,
        tool_use_tokens: usage.tool_use_prompt_token_count.unwrap_or_default(),
        other_tokens: usage
            .total_token_count
            .unwrap_or_default()
            .saturating_sub(known),
        ..CompletionUsage::default()
    }
}
