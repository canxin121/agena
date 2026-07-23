/// Conservative character-to-token ratio shared by prompt and context policy.
pub const APPROX_CHARS_PER_TOKEN: usize = 4;
/// Smallest usable prompt budget retained after context/output reservations.
pub const MIN_PROMPT_BUDGET_TOKENS: u32 = 512;

/// Computes the usable prompt-token budget after reserving room for output.
pub fn prompt_token_budget(
    context_window_tokens: Option<u32>,
    max_input_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
) -> Option<u32> {
    const MIN_CONTEXT_RESERVE_TOKENS: u32 = 1_024;

    let context_window_tokens = context_window_tokens.filter(|value| *value > 0);
    let max_input_tokens = max_input_tokens.filter(|value| *value > 0);
    if context_window_tokens.is_none() {
        return max_input_tokens;
    }
    let context_window_tokens = context_window_tokens?;
    let min_prompt_tokens = MIN_PROMPT_BUDGET_TOKENS.min(context_window_tokens);
    let max_reserve_tokens = context_window_tokens
        .saturating_sub(min_prompt_tokens)
        .max(1);
    let min_reserve_tokens = MIN_CONTEXT_RESERVE_TOKENS.min(max_reserve_tokens).max(1);
    let requested_reserve_tokens = max_output_tokens
        .unwrap_or(context_window_tokens / 8)
        .max(context_window_tokens / 8);
    let reserve_tokens = requested_reserve_tokens
        .max(min_reserve_tokens)
        .min(max_reserve_tokens);
    let context_prompt_tokens = context_window_tokens
        .saturating_sub(reserve_tokens)
        .max(min_prompt_tokens);
    Some(max_input_tokens.map_or(context_prompt_tokens, |max_input| {
        context_prompt_tokens.min(max_input)
    }))
}

/// Estimate prompt tokens from payload characters using the shared conservative
/// four-characters-per-token approximation.
pub fn estimate_prompt_tokens_from_chars(chars: usize) -> u64 {
    if chars == 0 {
        return 0;
    }
    chars
        .saturating_add(APPROX_CHARS_PER_TOKEN.saturating_sub(1))
        .checked_div(APPROX_CHARS_PER_TOKEN)
        .unwrap_or(usize::MAX) as u64
}

#[cfg(test)]
mod tests {
    use super::{estimate_prompt_tokens_from_chars, prompt_token_budget};

    #[test]
    fn prompt_budget_respects_output_reserve_and_max_input() {
        assert_eq!(
            prompt_token_budget(Some(200_000), None, Some(100_000)),
            Some(100_000)
        );
        assert_eq!(
            prompt_token_budget(Some(200_000), Some(80_000), Some(20_000)),
            Some(80_000)
        );
        assert_eq!(prompt_token_budget(None, Some(65_536), None), Some(65_536));
        assert_eq!(estimate_prompt_tokens_from_chars(0), 0);
        assert_eq!(estimate_prompt_tokens_from_chars(5), 2);
    }
}
