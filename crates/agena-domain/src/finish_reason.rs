use serde::{Deserialize, Serialize};
use strum::Display;

/// Normalized terminal reason for a model run.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FinishReason {
    #[default]
    Stop,
    ToolCalls,
    MaxTokens,
    ContentFilter,
    Error,
    Other,
}

/// Why an in-flight run terminated without completing normally.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RunAbortReason {
    /// Detected on session load — an in-flight run from a prior process.
    ProcessRestart,
    /// User cancelled the run (e.g. via UI cancel button).
    UserCancelled,
    /// Provider returned an error before the run could close.
    ProviderError,
    /// The run was superseded by a newer user message or steer input before
    /// it completed; the old turn is replaced, not failed.
    Replaced,
    /// The run was truncated because the session was forked or rewound while
    /// the run was still in flight: the branch view stops at the fork cutoff
    /// and never carries the parent's live turn.
    ForkCutoff,
    /// A configured usage or step budget (max turns, token/cost limit) was
    /// exhausted and the run stopped early on purpose.
    BudgetLimited,
    /// Internal scheduling error.
    Internal,
}

#[cfg(test)]
mod tests {
    use super::{FinishReason, RunAbortReason};

    #[test]
    fn finish_reason_has_stable_default_and_wire_spelling() {
        assert_eq!(FinishReason::default(), FinishReason::Stop);
        assert_eq!(
            serde_json::to_string(&FinishReason::ToolCalls).unwrap(),
            "\"tool_calls\""
        );
    }

    #[test]
    fn abort_reason_has_stable_wire_spelling() {
        assert_eq!(
            serde_json::to_string(&RunAbortReason::ProcessRestart).unwrap(),
            "\"process_restart\""
        );
    }
}
