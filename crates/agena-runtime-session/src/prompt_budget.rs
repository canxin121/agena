/// Conservative character-to-token ratio shared by prompt and context policy.
pub const APPROX_CHARS_PER_TOKEN: usize = 4;
/// Smallest usable prompt budget retained after context/output reservations.
pub const MIN_PROMPT_BUDGET_TOKENS: u32 = 512;
/// Per-request output budget, rather than a model's theoretical output maximum.
pub const DEFAULT_SESSION_OUTPUT_TOKENS: u32 = 32_000;

const OUTPUT_TOKEN_PATCH_PATHS: &[(&str, Option<&str>)] = &[
    ("max_output_tokens", None),
    ("max_completion_tokens", None),
    ("max_tokens", None),
    ("generationConfig", Some("maxOutputTokens")),
    ("inferenceConfig", Some("maxTokens")),
];

/// Body patches are applied after adapter serialization. Include their output
/// limit in admission, then keep the patch aligned with the bounded request.
pub(crate) fn requested_output_tokens_from_override(
    request_override: &agena_domain::ModelSpeedModeRequestOverride,
) -> Option<u32> {
    OUTPUT_TOKEN_PATCH_PATHS
        .iter()
        .filter_map(|(key, nested)| {
            let value = request_override.body_patch.get(*key)?;
            let value = match nested {
                Some(nested) => value.get(*nested)?,
                None => value,
            };
            value
                .as_u64()
                .filter(|tokens| *tokens > 0)
                .map(|tokens| tokens.min(u64::from(u32::MAX)) as u32)
        })
        .max()
}

pub(crate) fn align_output_token_overrides(
    request_override: &mut agena_domain::ModelSpeedModeRequestOverride,
    output_tokens: u32,
) {
    for (key, nested) in OUTPUT_TOKEN_PATCH_PATHS {
        let Some(value) = request_override.body_patch.get_mut(*key) else {
            continue;
        };
        let value = match nested {
            Some(nested) => match value.get_mut(*nested) {
                Some(value) => value,
                None => continue,
            },
            None => value,
        };
        *value = serde_json::json!(output_tokens);
    }
}

pub fn session_output_token_budget(
    context_window_tokens: Option<u32>,
    model_max_output_tokens: Option<u32>,
    requested_output_tokens: Option<u32>,
) -> u32 {
    let requested = requested_output_tokens
        .filter(|tokens| *tokens > 0)
        .unwrap_or_else(|| {
            context_window_tokens
                .filter(|tokens| *tokens > 0)
                .map_or(DEFAULT_SESSION_OUTPUT_TOKENS, |tokens| {
                    DEFAULT_SESSION_OUTPUT_TOKENS.min((tokens / 8).max(1))
                })
        });
    let model_limit = model_max_output_tokens.filter(|tokens| *tokens > 0);
    let context_limit = context_window_tokens
        .filter(|tokens| *tokens > 0)
        .map(|tokens| {
            tokens
                .saturating_sub(MIN_PROMPT_BUDGET_TOKENS.min(tokens))
                .max(1)
        });
    model_limit
        .into_iter()
        .chain(context_limit)
        .fold(requested, u32::min)
}

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
        .filter(|tokens| *tokens > 0)
        .unwrap_or(DEFAULT_SESSION_OUTPUT_TOKENS.min(context_window_tokens / 8));
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
    use super::{
        estimate_prompt_tokens_from_chars, prompt_token_budget, session_output_token_budget,
    };

    #[test]
    fn provider_output_patches_share_the_admitted_budget() {
        let mut request_override = agena_domain::ModelSpeedModeRequestOverride::default();
        request_override
            .body_patch
            .insert("max_output_tokens".into(), serde_json::json!(64000));
        request_override.body_patch.insert(
            "generationConfig".into(),
            serde_json::json!({"maxOutputTokens": 64000, "temperature": 0.5}),
        );
        assert_eq!(
            super::requested_output_tokens_from_override(&request_override),
            Some(64000)
        );
        super::align_output_token_overrides(&mut request_override, 32000);
        assert_eq!(
            super::requested_output_tokens_from_override(&request_override),
            Some(32000)
        );
        assert_eq!(
            request_override.body_patch["generationConfig"]["temperature"],
            0.5
        );
        assert!(!request_override.body_patch.contains_key("max_tokens"));
    }

    #[test]
    fn large_output_capabilities_do_not_become_a_request_reservation() {
        let output = session_output_token_budget(Some(1_000_000), Some(384_000), None);
        assert_eq!(output, 32_000);
        assert_eq!(
            prompt_token_budget(Some(1_000_000), Some(1_000_000), Some(output)),
            Some(968_000)
        );
        assert_eq!(
            session_output_token_budget(Some(1_000_000), Some(384_000), Some(64_000)),
            64_000
        );
        assert_eq!(session_output_token_budget(Some(8_192), None, None), 1_024);
        assert_eq!(
            session_output_token_budget(Some(8_192), Some(2_048), Some(64_000)),
            2_048
        );
    }

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
