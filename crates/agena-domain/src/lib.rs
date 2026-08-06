//! Stable domain values shared across Agena layers.
//!
//! This crate intentionally contains no database, runtime, transport, UI, or
//! provider SDK dependency.

mod access;
mod activity;
mod background_activity;
mod auto_compaction;
mod command_events;
mod context_policy;
mod doom_loop;
mod event;
mod execution;
mod execution_access;
mod execution_events;
mod execution_lifecycle;
mod execution_selection;
mod execution_status;
mod finish_reason;
mod ids;
mod interaction_notification;
mod json_path;
mod message_activity;
mod message_activity_values;
mod message_source;
mod model;
mod model_capabilities;
mod model_catalog_values;
mod model_metadata;
mod model_request_override;
mod model_selection;
mod model_values;
mod network_permission;
mod network_target;
mod operation_error;
mod part_kind;
mod path_access;
mod path_permission;
mod pending_interactive_request;
mod permission;
mod permission_config;
mod permission_events;
mod permission_interaction;
mod permission_outcome;
mod permission_request;
mod permission_resolution;
mod plugin_invocation;
mod process_values;
mod prompt_compaction;
mod prompt_tokens;
mod provider_retry;
mod reasoning;
mod role;
mod session_cache;
mod session_cost;
mod session_state;
mod session_summary;
mod session_usage;
mod stream_error;
mod structured;
mod thinking;
mod time_range;
mod tool_api;
mod tool_effects;
mod tool_invocation;
mod tool_output;
mod tool_permission;
mod tool_permission_config;
mod tool_permission_contract;
mod tool_result;
mod usage_period;
mod usage_query;
mod usage_stats;
mod user_input;

pub use access::{AccessKind, AccessSelector};
pub use background_activity::{
    BackgroundActivity, BackgroundActivityChangedEvent, BackgroundActivityEventReason,
    BackgroundActivityFilter, BackgroundActivityKind, BackgroundActivityLogLine,
    BackgroundActivityLogRead, BackgroundActivityStatus,
};
pub use activity::{
    ActivityActor, ActivityLifecycle, ActivityNode, ActivityOwner, ActivityPayload,
    ActivityProvenance, ActivityState, AssistantReplySnapshot, AssistantReplyStatus,
    CancellationResult, ChecklistActivity, ComposerActivity, ComposerDocument, ComposerNode,
    ContentDocument, ContentNode, ContentPosition, CustomActivity, ErrorActivity, ExecutionTarget,
    FileChangesActivity, HookActivity, InteractionActivity, MaintenanceActivity, NestedTaskActivity,
    OperationActivity, OperationActivityError, OperationAuthorization, OperationPermission,
    ProgressActivity, ReasoningActivity, ResourceActivity, ResourceKind, ResourceReference,
    SearchActivity, SkillExecutionActivity, SkillReferenceActivity, TextArtifactActivity,
    TextSegment, TextSegmentActivity, TranscriptPatch, TranscriptSnapshot, TurnSnapshot,
};
pub use auto_compaction::SessionAutoCompactionConfig;
pub use command_events::{
    CommandBeginEvent, CommandContext, CommandEndEvent, CommandOutputDeltaEvent,
    CommandOutputStream,
};
pub use context_policy::ContextPolicy;
pub use doom_loop::{DoomLoopHit, DoomLoopPolicy};
pub use event::{
    EVENT_ENVELOPE_SCHEMA_VERSION, EventEnvelope, EventFilter, EventKindTag, EventMeta, EventScope,
    KindMatcher, KindPersistence, MESSAGE_CREATED_EVENT_KIND_TAGS,
};
pub use execution::{ExecutionFailureKind, ExecutionOutcome, ExecutionPhase, ExecutionSource};
pub use execution_access::ExecutionAccess;
pub use execution_events::{
    ExecutionFinishedEvent, ExecutionStartedEvent, SubtaskStatusChangedEvent,
};
pub use execution_lifecycle::{ExecutionLifecycle, ExecutionTransitionError};
pub use execution_selection::ExecutionSelection;
pub use execution_status::{ExecutionStatus, ExecutionStatusTransitionError};
pub use finish_reason::{FinishReason, RunAbortReason};
pub use ids::{
    ActivityId, AssistantReplyId, ExecutionId, MessageId, PartId, RunId, TextSegmentId, ToolCallId,
    TurnId,
};
pub use interaction_notification::InteractionNotificationLevel;
pub use json_path::{JsonPathError, format_json_path, get_json_path, parse_json_path};
pub use message_activity::{
    ArtifactRef, FileChangeKind, SearchResultItem, TableColumn, TodoItem, TodoPriority, TodoStatus,
    UserInputOption, UserInputQuestion, UserInputReply, UserInputRequest,
    deserialize_user_input_answers, user_input_answers_is_empty,
};
pub use message_activity_values::{
    ErrorPart, FileChangeRecord, ReasoningPart, TextPart, WebSearchResult,
};
pub use message_source::MessageSource;
pub use model::{AdapterId, IdentifierError, ModelId, ModelRef, ModelRefParseError, ProviderId};
pub use model_capabilities::ModelCapabilities;
pub use model_catalog_values::{
    Model, ModelSpeedMode, ModelThinkingMode, compare_thinking_mode_strength,
};
pub use model_metadata::{
    ModelMetadata, ModelPricing, ModelPricingTier, ModelTokenLimits, non_empty_model_pricing,
    normalize_model_assistant_reasoning_field, normalize_model_default_temperature,
    normalize_model_default_top_k, normalize_model_default_top_p,
    normalize_model_output_modalities,
};
pub use model_request_override::ModelSpeedModeRequestOverride;
pub use model_selection::{ApprovalModelSelection, ModelSelectionConfig};
pub use model_values::{CapabilitySupport, ModelInputModality, ModelLifecycle};
pub use network_permission::NetworkPermissionConfig;
pub use network_target::{NetworkTarget, NetworkTargetParseError};
pub use operation_error::OperationError;
pub use part_kind::PartKind;
pub use path_access::{PathAccessModes, PathAccessRuleConfig};
pub use path_permission::PathPermissionConfig;
pub use pending_interactive_request::{
    PendingInteractiveRequest, PendingInteractiveRequestContext,
};
pub use permission::{PermissionMode, PermissionReplyKind, PermissionScope};
pub use permission_config::PermissionConfig;
pub use permission_events::{
    PermissionRepliedEvent, PermissionRequestedEvent, PermissionRuleEvent, ToolPolicyDeniedEvent,
    ToolUserDeclinedEvent,
};
pub use permission_interaction::{PendingPermission, PermissionReply, PermissionRequest};
pub use permission_outcome::{PermissionAuthorityKind, PolicyDeniedResult, UserDeclinedResult};
pub use permission_request::{
    ActionSpec, DecisionTrace, DecisionTraceStep, PermissionAction, PolicySourceKind,
};
pub use permission_resolution::{
    PermissionDecision, PermissionResolution, PermissionResolutionSource, decide_from_mode,
};
pub use plugin_invocation::PluginInvocation;
pub use process_values::{
    ProcessEvent, ProcessShell, ProcessStatus, ProcessStream, ProcessSummary,
};
pub use prompt_compaction::{
    PromptCompactionActivity, PromptCompactionCompletedEvent, PromptCompactionStrategy,
    PromptCompactionTrigger,
};
pub use prompt_tokens::PromptTokenUsageSnapshot;
pub use provider_retry::{ProviderRetryEvent, ProviderRetryResolvedEvent};
pub use reasoning::AssistantReasoningField;
pub use role::Role;
pub use session_cache::{SessionCacheLimits, SessionCacheStats};
pub use session_cost::{ModelCostBreakdown, SessionCostSummary};
pub use session_state::{SessionLifecycleState, SessionRelationKind, SubtaskStatus, WorkflowState};
pub use session_summary::{SessionListRequest, SessionSummary};
pub use session_usage::{SessionUsage, SessionUsageLimitBasis};
pub use stream_error::StreamErrorEvent;
pub use structured::{StructuredField, StructuredObject, StructuredValue};
pub use thinking::{ReasoningEffort, ThinkingDisplay, ThinkingRequest};
pub use time_range::TimeRange;
pub use tool_api::ToolApiFunction;
pub use tool_effects::{FilesystemAccess, FilesystemEffect, FilesystemEffects, NetworkEffect};
pub use tool_invocation::{ToolApiCall, ToolInvocation};
pub use tool_output::{ToolManagedOutput, ToolOutput};
pub use tool_permission::ToolPermissionRules;
pub use tool_permission_config::ToolPermissionConfig;
pub use tool_permission_contract::{
    InputNetworkSpec, InputPathSpec, NetworkAccessSpec, PathAccessSpec, PathKind,
    ToolPermissionContract,
};
pub use tool_result::{ToolPresentationSection, ToolResultDisplay, ToolResultState};
pub use usage_period::UsagePeriod;
pub use usage_query::UsageStatsQuery;
pub use usage_stats::{
    ModelUsageBreakdown, ProviderUsageBreakdown, SessionUsageBreakdown, UsageBillableUnitTotal,
    UsageDailyBreakdown, UsageStats, UsageTotals,
};
pub use user_input::{PendingInteractiveRequestKind, UserInputReplyKind};
mod availability_outcome;
pub use availability_outcome::{
    CapabilitySourceKind, CapabilityUnavailableResult, ToolUnavailableResult,
};
