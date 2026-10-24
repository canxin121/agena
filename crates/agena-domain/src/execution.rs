use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Cause that started a session execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSource {
    #[default]
    User,
    Continue,
    Compaction,
    PermissionReply,
    UserInputReply,
}

/// Active phase of a session execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    Starting,
    PreparingModel,
    StreamingModel,
    ExecutingTools,
    AwaitingInteraction,
    Cancelling,
}

/// Stable category for a failed execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFailureKind {
    Provider,
    Internal,
    ProcessRestart,
}

/// Terminal result of a session execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionOutcome {
    Completed,
    Cancelled,
    Failed { failure: agena_failure::UserProblem },
}
