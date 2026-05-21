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

pub use ids::{MessageId, PartId, ToolCallId, TurnId};

pub use context_governor::ContextGovernor;
pub use context_policy::ContextPolicy;
pub use cost::{
    ModelCostBreakdown, ModelUsageBreakdown, ProviderUsageBreakdown, SessionCostSummary,
    SessionUsageBreakdown, UsageDailyBreakdown, UsagePeriod, UsageStatRecord, UsageStats,
    UsageStatsQuery, UsageTotals,
};
pub use doom_loop::{DoomLoopHit, DoomLoopPolicy};
pub use manager::{
    SessionAutoCompactionConfig, SessionCacheStats, SessionCompactRequest, SessionContinueRequest,
    SessionCreateRequest, SessionForkRequest, SessionGoalCreateRequest, SessionGoalUpdateRequest,
    SessionManager, SessionManagerConfig, SessionPermissionReplyRequest, SessionRewindRequest,
    SessionRunOptions, SessionSubtaskRequest, SessionSubtaskResponse, SessionUsage,
    SessionUsageLimitBasis, SessionUserInputReplyRequest, SessionUserTurnRequest,
};
pub use model::{
    GoalStatus, MAX_SESSION_GOAL_OBJECTIVE_CHARS, PlanState, PromptCompactionRuntime,
    PromptCompactionStrategy, PromptTokenRuntime, PromptTokenUsageSnapshot, PromptWindowRuntime,
    ProviderPromptAnchor, Session, SessionExecutionContext, SessionGoal, SessionListRequest,
    SessionRuntimeState, SessionRuntimeStatus, SessionStatus, SessionSummary,
    validate_session_goal_objective,
};
pub use processor::SessionProcessor;

pub use history::ProjectedMessageHeader;
/// Audit-only payload carried by `RewindCheckpoint` system notices.
/// Exposed publicly so callers of `SessionManager::list_rewind_checkpoints`
/// can name the return type.
pub use history::{RewindCheckpoint, RewindCheckpointEntry};

pub const DEFAULT_AUTO_COMPACTION_RESERVED_TOKENS_CAP: u32 = 20_000;

pub fn estimate_auto_compaction_reserve_tokens(
    context_window_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
    configured_reserved_tokens: Option<u32>,
) -> Option<u32> {
    configured_reserved_tokens.or_else(|| {
        max_output_tokens
            .or_else(|| {
                context_window_tokens
                    .filter(|value| *value > 0)
                    .map(|value| (value / 8).max(1_024))
            })
            .map(|value| value.min(DEFAULT_AUTO_COMPACTION_RESERVED_TOKENS_CAP))
    })
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
    use super::{estimate_auto_compaction_reserve_tokens, estimate_session_context_usable_tokens};

    #[test]
    fn estimate_session_context_usable_tokens_prefers_input_limit_and_subtracts_reserve() {
        assert_eq!(
            estimate_session_context_usable_tokens(
                Some(200_000),
                Some(120_000),
                Some(8_192),
                Some(4_096)
            ),
            Some(115_904)
        );
    }

    #[test]
    fn estimate_auto_compaction_reserve_tokens_defaults_from_output_budget() {
        assert_eq!(
            estimate_auto_compaction_reserve_tokens(Some(128_000), Some(32_768), None),
            Some(20_000)
        );
        assert_eq!(
            estimate_auto_compaction_reserve_tokens(Some(128_000), Some(4_096), None),
            Some(4_096)
        );
    }
}
