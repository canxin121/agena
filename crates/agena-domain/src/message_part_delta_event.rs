use serde::{Deserialize, Serialize};

use crate::{ExecutionId, PartDeltaField, RunId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessagePartDeltaEvent {
    pub session_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<ExecutionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    pub message_id: i64,
    pub part_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<i64>,
    pub field: PartDeltaField,
    pub delta: String,
    pub seq: u64,
    pub ts_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::MessagePartDeltaEvent;
    use crate::PartDeltaField;

    #[test]
    fn message_part_delta_omits_absent_optional_ids() {
        let event = MessagePartDeltaEvent {
            session_id: 1,
            execution_id: None,
            run_id: None,
            message_id: 2,
            part_id: 3,
            call_id: None,
            field: PartDeltaField::Text,
            delta: "hi".into(),
            seq: 4,
            ts_ms: 5,
        };
        let json = serde_json::to_value(event).unwrap();
        assert!(json.get("execution_id").is_none());
        assert_eq!(json["field"]["field"], "text");
    }
}
