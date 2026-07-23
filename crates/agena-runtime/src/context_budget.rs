/// Conservative prompt-size threshold used when a model does not expose a
/// usable context-window limit. This contains no session/message state and is
/// therefore Runtime-owned.
pub fn estimate_prompt_budget_threshold_tokens(
    context_window_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
) -> u64 {
    let policy = agena_domain::ContextPolicy::default();
    let minimum_prompt_chars =
        crate::APPROX_CHARS_PER_TOKEN * crate::MIN_PROMPT_BUDGET_TOKENS as usize;
    let max_prompt_chars =
        crate::prompt_token_budget(context_window_tokens, None, max_output_tokens)
            .map(|tokens| tokens as usize * crate::APPROX_CHARS_PER_TOKEN)
            .unwrap_or(policy.max_prompt_chars)
            .max(minimum_prompt_chars);
    crate::estimate_prompt_tokens_from_chars(policy.proactive_char_threshold(max_prompt_chars))
}

pub fn estimate_auto_compaction_reserve_tokens(
    context_window_tokens: Option<u32>,
    max_input_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
    configured_reserved_tokens: Option<u32>,
) -> Option<u32> {
    let context_window_tokens = context_window_tokens.filter(|value| *value > 0)?;
    let limit = estimate_auto_compaction_limit_tokens(
        Some(context_window_tokens),
        max_input_tokens,
        max_output_tokens,
        configured_reserved_tokens,
    )?;
    Some(context_window_tokens.saturating_sub(limit.min(u32::MAX as u64) as u32))
}

pub fn estimate_session_context_usable_tokens(
    context_window_tokens: Option<u32>,
    max_input_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
    reserved_tokens: Option<u32>,
) -> Option<u64> {
    let base_tokens =
        crate::prompt_token_budget(context_window_tokens, max_input_tokens, max_output_tokens)?;
    Some(base_tokens.saturating_sub(reserved_tokens.unwrap_or_default()) as u64)
}

pub fn estimate_auto_compaction_limit_tokens(
    context_window_tokens: Option<u32>,
    max_input_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
    configured_reserved_tokens: Option<u32>,
) -> Option<u64> {
    let hard_limit = estimate_session_context_usable_tokens(
        context_window_tokens,
        max_input_tokens,
        max_output_tokens,
        None,
    )?;
    let headroom = configured_reserved_tokens
        .map(u64::from)
        .unwrap_or_else(|| {
            let proportional = context_window_tokens
                .map(|tokens| u64::from(tokens) * 5 / 100)
                .unwrap_or(hard_limit * 5 / 100);
            proportional.clamp(4_096, 20_000)
        });
    Some(hard_limit.saturating_sub(headroom).max(512.min(hard_limit)))
}

#[cfg(test)]
mod tests {
    use super::{estimate_auto_compaction_limit_tokens, estimate_session_context_usable_tokens};

    #[test]
    fn context_budget_respects_input_and_output_limits() {
        assert_eq!(
            estimate_session_context_usable_tokens(Some(200_000), Some(80_000), Some(20_000), None),
            Some(80_000)
        );
        assert_eq!(
            estimate_auto_compaction_limit_tokens(Some(200_000), None, Some(100_000), None),
            Some(90_000)
        );
    }
}
