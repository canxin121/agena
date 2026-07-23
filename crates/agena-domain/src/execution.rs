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
    Failed {
        failure_kind: ExecutionFailureKind,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{ExecutionFailureKind, ExecutionOutcome, ExecutionPhase, ExecutionSource};

    #[test]
    fn execution_values_have_stable_wire_shapes() {
        assert_eq!(ExecutionSource::default(), ExecutionSource::User);
        assert_eq!(
            serde_json::to_string(&ExecutionPhase::PreparingModel).unwrap(),
            "\"preparing_model\""
        );
        assert_eq!(
            serde_json::to_value(ExecutionOutcome::Failed {
                failure_kind: ExecutionFailureKind::ProcessRestart,
                message: "interrupted".to_owned(),
            })
            .unwrap(),
            serde_json::json!({
                "kind": "failed",
                "failure_kind": "process_restart",
                "message": "interrupted",
            })
        );
    }
}
