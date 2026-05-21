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
    SessionCacheStats, SessionCompactRequest, SessionContinueRequest, SessionCreateRequest,
    SessionForkRequest, SessionGoalCreateRequest, SessionGoalUpdateRequest, SessionManager,
    SessionManagerConfig, SessionPermissionReplyRequest, SessionRewindRequest, SessionRunOptions,
    SessionSubtaskRequest, SessionSubtaskResponse, SessionUserInputReplyRequest,
    SessionUserTurnRequest,
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
