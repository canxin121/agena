use serde::{Deserialize, Serialize};

use crate::{
    AssistantReplyId, ExecutionAccess, ExecutionId, ExecutionOutcome, ExecutionSource,
    SubtaskStatus, TurnId,
};

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Event emitted when an execution starts.
pub struct ExecutionStartedEvent {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub turn_id: TurnId,
    pub reply_id: AssistantReplyId,
    pub source: ExecutionSource,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Event emitted when an execution finishes.
pub struct ExecutionFinishedEvent {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub reply_id: AssistantReplyId,
    pub outcome: ExecutionOutcome,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Event emitted when a subtask changes status.
pub struct SubtaskStatusChangedEvent {
    pub session_id: i64,
    pub parent_session_id: i64,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "ExecutionAccess::is_inherit")]
    pub access: ExecutionAccess,
    pub status: SubtaskStatus,
    #[serde(default, skip_serializing_if = "is_false")]
    pub resumed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::UserProblem>,
    pub ts_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::SubtaskStatusChangedEvent;
    use crate::{ExecutionAccess, SubtaskStatus};

    #[test]
    fn subtask_event_omits_absent_lifecycle_fields() {
        let value = SubtaskStatusChangedEvent {
            session_id: 2,
            parent_session_id: 1,
            task_id: "task".into(),
            access: ExecutionAccess::Inherit,
            status: SubtaskStatus::Created,
            resumed: false,
            started_at_ms: None,
            finished_at_ms: None,
            failure: None,
            ts_ms: 3,
        };
        let json = serde_json::to_value(value).unwrap();
        assert!(json.get("resumed").is_none());
        assert!(json.get("failure").is_none());
    }
}
