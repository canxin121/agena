use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamErrorEvent {
    pub session_id: i64,
    pub error: ErrorInfo,
    pub ts_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::{ErrorInfo, StreamErrorEvent};

    #[test]
    fn stream_error_payload_round_trips() {
        let event = StreamErrorEvent {
            session_id: 7,
            error: ErrorInfo {
                code: "overloaded".into(),
                message: "try again".into(),
            },
            ts_ms: 42,
        };
        assert_eq!(
            serde_json::from_value::<StreamErrorEvent>(serde_json::to_value(&event).unwrap())
                .unwrap(),
            event
        );
    }
}
