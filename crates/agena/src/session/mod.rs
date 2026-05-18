mod cache;
mod compaction_worker;
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
    SessionCacheStats, SessionContinueRequest, SessionCreateRequest, SessionForkRequest,
    SessionGoalCreateRequest, SessionGoalUpdateRequest, SessionManager, SessionManagerConfig,
    SessionPermissionReplyRequest, SessionRewindRequest, SessionRunOptions, SessionSubtaskRequest,
    SessionSubtaskResponse, SessionUnrewindRequest, SessionUserInputReplyRequest,
    SessionUserTurnRequest,
};
pub use model::{
    GoalStatus, MAX_SESSION_GOAL_OBJECTIVE_CHARS, PlanState, PromptTokenRuntime,
    PromptTokenUsageSnapshot, PromptWindowRuntime, ProviderPromptAnchor, Session,
    SessionExecutionContext, SessionGoal, SessionListRequest, SessionRuntimeState,
    SessionRuntimeStatus, SessionStatus, SessionSummary, validate_session_goal_objective,
};
#[allow(unused_imports)]
pub(crate) use model::{
    MESSAGE_TAG_ATTACHMENT_PAYLOAD_STRIPPED, MESSAGE_TAG_PROMPT_COMPACTED,
    MESSAGE_TAG_TOOL_RESULT_PRUNED,
};
pub use processor::SessionProcessor;

pub use history::ProjectedMessageHeader;
/// Audit-only payload carried by `RewindCheckpoint` system notices.
/// Exposed publicly so callers of `SessionManager::list_rewind_checkpoints`
/// can name the return type.
pub use history::{RewindCheckpoint, RewindCheckpointEntry};
