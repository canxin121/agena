use serde::{Deserialize, Serialize};

use crate::{ExecutionId, ExecutionOutcome, ExecutionSource, MessageId, PartId, SubtaskStatus};

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionStartedEvent {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub activity_message_id: MessageId,
    pub activity_part_id: PartId,
    pub source: ExecutionSource,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionFinishedEvent {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub outcome: ExecutionOutcome,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubtaskStatusChangedEvent {
    pub session_id: i64,
    pub parent_session_id: i64,
    pub task_id: String,
    pub profile: String,
    pub status: SubtaskStatus,
    #[serde(default, skip_serializing_if = "is_false")]
    pub resumed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub ts_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::SubtaskStatusChangedEvent;
    use crate::SubtaskStatus;

    #[test]
    fn subtask_event_omits_absent_lifecycle_fields() {
        let value = SubtaskStatusChangedEvent {
            session_id: 2,
            parent_session_id: 1,
            task_id: "task".into(),
            profile: "default".into(),
            status: SubtaskStatus::Created,
            resumed: false,
            started_at_ms: None,
            finished_at_ms: None,
            error: None,
            ts_ms: 3,
        };
        let json = serde_json::to_value(value).unwrap();
        assert!(json.get("resumed").is_none());
        assert!(json.get("error").is_none());
    }
}
