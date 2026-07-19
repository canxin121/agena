mod cache;
mod context_governor;
mod context_policy;
pub mod cost;
mod doom_loop;
mod execution;
pub(crate) mod execution_registry;
pub(crate) mod history;
pub mod ids;
mod manager;
mod model;
mod processor;
mod prompt_window;
mod store;

pub use execution::{
    ExecutionFailureKind, ExecutionLifecycle, ExecutionOutcome, ExecutionPhase, ExecutionSource,
    ExecutionTransitionError,
};
pub use ids::{ExecutionId, MessageId, PartId, RunId, ToolCallId};

pub use context_governor::ContextGovernor;
pub use context_policy::ContextPolicy;
pub use cost::{
    ModelCostBreakdown, ModelUsageBreakdown, ProviderUsageBreakdown, SessionCostSummary,
    SessionUsageBreakdown, UsageDailyBreakdown, UsagePeriod, UsageStatRecord, UsageStats,
    UsageStatsQuery, UsageTotals,
};
pub use doom_loop::{DoomLoopHit, DoomLoopPolicy};
pub use manager::{
    AuthorizedToolInvocation, DEFAULT_MAX_CONCURRENT_TOOLS, DEFAULT_SESSION_CACHE_MAX_BYTES,
    DEFAULT_SESSION_CACHE_MAX_SESSIONS, DEFAULT_SESSION_CACHE_TTL_SECS,
    SessionAutoCompactionConfig, SessionCacheStats, SessionCreateRequest,
    SessionExecutionReplyRequest, SessionExecutionRequest, SessionForkRequest, SessionManager,
    SessionManagerConfig, SessionPermissionReplyRequest, SessionRewindRequest, SessionRunOptions,
    SessionSubtaskRequest, SessionSubtaskResponse, SessionUsage, SessionUsageLimitBasis,
    SessionUserMessageRequest, ToolInvocationAuthorization,
};
pub use model::{
    PromptCompactionActivity, PromptCompactionContent, PromptCompactionMessage,
    PromptCompactionRuntime, PromptCompactionStrategy, PromptCompactionTrigger, PromptTokenRuntime,
    PromptTokenUsageSnapshot, PromptWindowRuntime, ProviderPromptAnchor, Session,
    SessionExecutionContext, SessionLifecycleState, SessionListRequest, SessionRelationKind,
    SessionRuntimeState, SessionSummary, SubtaskRuntimeState, SubtaskStatus, WorkflowRuntimeState,
    WorkflowState,
};
pub use processor::SessionProcessor;

pub use history::ProjectedMessageHeader;

pub const EFFECTIVE_CONTEXT_WINDOW_PERCENT: u32 = 95;
pub const CONTEXT_USAGE_BASELINE_TOKENS: u64 = 12_000;

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
    let base_tokens = prompt_window::prompt_token_budget(
        context_window_tokens,
        max_input_tokens,
        max_output_tokens,
    )?;
    Some(base_tokens.saturating_sub(reserved_tokens.unwrap_or_default()) as u64)
}

pub fn estimate_effective_context_window_tokens(context_window_tokens: Option<u32>) -> Option<u64> {
    let context_window_tokens = context_window_tokens.filter(|value| *value > 0)? as u64;
    Some(context_window_tokens.saturating_mul(EFFECTIVE_CONTEXT_WINDOW_PERCENT as u64) / 100)
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
        None,
        max_output_tokens,
        policy.max_prompt_chars,
        None,
        &[],
    );
    prompt_window::approximate_tokens_from_chars(policy.proactive_char_threshold(max_prompt_chars))
}
