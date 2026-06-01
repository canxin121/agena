mod cache;
mod context_governor;
mod context_policy;
pub(crate) mod control;
pub mod cost;
mod doom_loop;
pub(crate) mod history;
pub mod ids;
mod manager;
mod model;
mod processor;
mod prompt_window;
mod store;

pub use ids::{MessageId, PartId, RunId, ToolCallId};

pub use context_governor::ContextGovernor;
pub use context_policy::ContextPolicy;
pub use cost::{
    ModelCostBreakdown, ModelUsageBreakdown, ProviderUsageBreakdown, SessionCostSummary,
    SessionUsageBreakdown, UsageDailyBreakdown, UsagePeriod, UsageStatRecord, UsageStats,
    UsageStatsQuery, UsageTotals,
};
pub use doom_loop::{DoomLoopHit, DoomLoopPolicy};
pub use manager::{
    SessionAutoCompactionConfig, SessionCacheStats, SessionCreateRequest,
    SessionExecutionReplyRequest, SessionExecutionRequest, SessionForkRequest,
    SessionManager, SessionManagerConfig, SessionPermissionReplyRequest, SessionRewindRequest,
    SessionRunOptions, SessionSubtaskRequest, SessionSubtaskResponse, SessionUsage,
    SessionUsageLimitBasis, SessionUserMessageRequest,
};
pub use model::{
    PromptCompactionRuntime, PromptCompactionStrategy, PromptTokenRuntime,
    PromptTokenUsageSnapshot, PromptWindowRuntime, ProviderPromptAnchor, RunStatus, Session,
    SessionExecutionContext, SessionListRequest, SessionRuntimeState, SessionStatus,
    SessionSummary,
};
pub use processor::SessionProcessor;

pub use history::ProjectedMessageHeader;
/// Audit-only payload carried by `RewindCheckpoint` system notices.
/// Exposed publicly so callers of `SessionManager::list_rewind_checkpoints`
/// can name the return type.
pub use history::{RewindCheckpoint, RewindCheckpointRecord};

pub const EFFECTIVE_CONTEXT_WINDOW_PERCENT: u32 = 95;
pub const AUTO_COMPACTION_CONTEXT_WINDOW_PERCENT: u32 = 90;
pub const CONTEXT_USAGE_BASELINE_TOKENS: u64 = 12_000;

pub fn estimate_auto_compaction_reserve_tokens(
    context_window_tokens: Option<u32>,
    _max_output_tokens: Option<u32>,
    configured_reserved_tokens: Option<u32>,
) -> Option<u32> {
    let context_window_tokens = context_window_tokens.filter(|value| *value > 0)?;
    Some(
        configured_reserved_tokens
            .filter(|value| *value < context_window_tokens)
            .unwrap_or_else(|| {
                let limit =
                    estimate_auto_compaction_limit_tokens(Some(context_window_tokens), None)
                        .unwrap_or(context_window_tokens as u64);
                context_window_tokens.saturating_sub(limit as u32)
            }),
    )
}

pub fn estimate_session_context_usable_tokens(
    context_window_tokens: Option<u32>,
    max_input_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
    reserved_tokens: Option<u32>,
) -> Option<u64> {
    let base_tokens = max_input_tokens
        .or_else(|| prompt_window::prompt_token_budget(context_window_tokens, max_output_tokens))?;
    Some(base_tokens.saturating_sub(reserved_tokens.unwrap_or_default()) as u64)
}

pub fn estimate_effective_context_window_tokens(context_window_tokens: Option<u32>) -> Option<u64> {
    let context_window_tokens = context_window_tokens.filter(|value| *value > 0)? as u64;
    Some(context_window_tokens.saturating_mul(EFFECTIVE_CONTEXT_WINDOW_PERCENT as u64) / 100)
}

pub fn estimate_auto_compaction_limit_tokens(
    context_window_tokens: Option<u32>,
    configured_reserved_tokens: Option<u32>,
) -> Option<u64> {
    let context_window_tokens = context_window_tokens.filter(|value| *value > 0)?;
    if let Some(reserved_tokens) = configured_reserved_tokens
        && reserved_tokens < context_window_tokens
    {
        return Some(context_window_tokens.saturating_sub(reserved_tokens) as u64);
    }
    Some(
        (context_window_tokens as u64)
            .saturating_mul(AUTO_COMPACTION_CONTEXT_WINDOW_PERCENT as u64)
            / 100,
    )
}

pub fn context_usage_percent_used(current_tokens: u64, context_window_tokens: u32) -> u64 {
    let Some(effective_window) =
        estimate_effective_context_window_tokens(Some(context_window_tokens))
    else {
        return 0;
    };
    if effective_window <= CONTEXT_USAGE_BASELINE_TOKENS {
        return 100;
    }

    let usable_window = effective_window.saturating_sub(CONTEXT_USAGE_BASELINE_TOKENS);
    let used = current_tokens.saturating_sub(CONTEXT_USAGE_BASELINE_TOKENS);
    (((used as f64 / usable_window as f64) * 100.0)
        .clamp(0.0, 100.0)
        .round()) as u64
}

pub fn estimate_prompt_budget_threshold_tokens(
    context_window_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
) -> u64 {
    let policy = ContextPolicy::default();
    let max_prompt_chars = prompt_window::prompt_char_budget(
        context_window_tokens,
        max_output_tokens,
        policy.max_prompt_chars,
        None,
        &[],
    );
    prompt_window::approximate_tokens_from_chars(policy.proactive_char_threshold(max_prompt_chars))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_window_helpers_do_not_guess_unknown_model_limits() {
        assert_eq!(estimate_effective_context_window_tokens(None), None);
        assert_eq!(estimate_auto_compaction_limit_tokens(None, None), None);
        assert_eq!(
            estimate_auto_compaction_reserve_tokens(None, None, None),
            None
        );
        assert_eq!(
            estimate_effective_context_window_tokens(Some(272_000)),
            Some(258_400)
        );
        assert_eq!(
            estimate_auto_compaction_limit_tokens(Some(272_000), None),
            Some(244_800)
        );
        assert_eq!(
            estimate_auto_compaction_reserve_tokens(Some(272_000), None, None),
            Some(27_200)
        );
    }

    #[test]
    fn context_usage_percent_used_subtracts_baseline() {
        assert_eq!(context_usage_percent_used(0, 272_000), 0);
        assert_eq!(context_usage_percent_used(12_000, 272_000), 0);
        assert_eq!(context_usage_percent_used(135_200, 272_000), 50);
        assert_eq!(context_usage_percent_used(258_400, 272_000), 100);
        assert_eq!(context_usage_percent_used(u64::MAX, 272_000), 100);
    }
}
