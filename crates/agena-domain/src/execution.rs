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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    Starting,
    PreparingModel,
    StreamingModel,
    ExecutingTools,
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

#[cfg(test)]
mod tests {
    use super::{ExecutionOutcome, ExecutionPhase, ExecutionSource};

    #[test]
    fn execution_values_have_stable_wire_shapes() {
        assert_eq!(ExecutionSource::default(), ExecutionSource::User);
        assert_eq!(
            serde_json::to_string(&ExecutionPhase::PreparingModel).unwrap(),
            "\"preparing_model\""
        );
        let failure = agena_failure::Failure::new(
            agena_failure::FailureCode::new("execution.interrupted"),
            agena_failure::FailureCategory::DependencyUnavailable,
            agena_failure::FailureResponsibility::System,
            agena_failure::RetryDirective::AfterUserAction,
            agena_failure::RecoveryDirective::Retry,
            agena_failure::FailureImpact::OperationFailed,
            agena_failure::UserPresentation::new(
                "execution-interrupted",
                "The response was interrupted.",
            ),
        );
        let id = failure.id.to_string();
        let value = serde_json::to_value(ExecutionOutcome::Failed {
            failure: failure.into(),
        })
        .unwrap();
        assert_eq!(value["kind"], "failed");
        assert_eq!(value["failure"]["id"], id);
        assert_eq!(value["failure"]["code"], "execution.interrupted");
        assert_eq!(
            value["failure"]["user"]["fallback"],
            "The response was interrupted."
        );
        assert!(value.get("message").is_none());
        assert!(value["failure"].get("model").is_none());
    }
}
