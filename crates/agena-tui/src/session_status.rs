//! Display-only session-status projection.
//!
//! Runtime and Application provide scalar token usage. This module owns the
//! terminal-facing context percentage, compact labels, and summary ordering;
//! it deliberately has no session, API, or Runtime dependency.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Status of token usage in a session.
pub enum TokenUsageStatus {
    PercentUsed(u64),
    UsedTokens(u64),
}

impl TokenUsageStatus {
    pub fn label(self) -> String {
        match self {
            Self::PercentUsed(percent_used) => format_token_progress_label(percent_used),
            Self::UsedTokens(tokens) => format!("{} used", format_tokens_k(tokens)),
        }
    }
}

/// Projects raw usage scalars into the display status shown in the terminal.
pub fn token_usage_status(
    current_tokens: u64,
    input_limit_tokens: Option<u64>,
) -> TokenUsageStatus {
    input_limit_tokens
        .filter(|limit| *limit > 0)
        .map(|limit| {
            TokenUsageStatus::PercentUsed(context_usage_percent_used(current_tokens, limit))
        })
        .unwrap_or(TokenUsageStatus::UsedTokens(current_tokens))
}

/// Orders compact session status values for the terminal summary line.
pub fn session_summary_status_parts(
    model_part: Option<String>,
    agent: Option<String>,
    token_usage: Option<TokenUsageStatus>,
) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(agent) = agent
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        parts.push(agent);
    }
    if let Some(model_part) = model_part
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        parts.push(model_part);
    }
    if let Some(token_usage) = token_usage {
        parts.push(token_usage.label());
    }
    parts
}

pub fn format_token_progress_label(percent_used: u64) -> String {
    format!("{}%", percent_used.min(100))
}

pub fn format_tokens_k(tokens: u64) -> String {
    if tokens == 0 {
        return "0k".to_string();
    }

    let value = tokens as f64 / 1_000.0;
    if value < 10.0 {
        return format!("{value:.1}k");
    }
    format!("{value:.0}k")
}

fn context_usage_percent_used(current_tokens: u64, input_limit_tokens: u64) -> u64 {
    (((current_tokens as f64 / input_limit_tokens as f64) * 100.0)
        .clamp(0.0, 100.0)
        .round()) as u64
}

#[cfg(test)]
mod tests {
    use super::{TokenUsageStatus, session_summary_status_parts, token_usage_status};

    #[test]
    fn session_summary_places_agent_before_model_and_usage() {
        assert_eq!(
            session_summary_status_parts(
                Some("GPT 5.4".to_owned()),
                Some("build".to_owned()),
                Some(TokenUsageStatus::PercentUsed(42)),
            ),
            vec!["build".to_owned(), "GPT 5.4".to_owned(), "42%".to_owned()],
        );
    }

    #[test]
    fn usage_projection_uses_the_admission_budget_and_bounds_percentages() {
        assert_eq!(
            token_usage_status(616_620, Some(616_000)),
            TokenUsageStatus::PercentUsed(100)
        );
        assert_eq!(
            token_usage_status(616_620, Some(968_000)),
            TokenUsageStatus::PercentUsed(64)
        );
        assert_eq!(
            token_usage_status(1_000, Some(8_000)),
            TokenUsageStatus::PercentUsed(13)
        );
        assert_eq!(
            token_usage_status(u64::MAX, Some(100_000)),
            TokenUsageStatus::PercentUsed(100),
        );
        assert_eq!(
            token_usage_status(1_500, None),
            TokenUsageStatus::UsedTokens(1_500),
        );
    }
}
