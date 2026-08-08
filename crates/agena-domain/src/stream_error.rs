use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Error event on a stream.
pub struct StreamErrorEvent {
    pub session_id: i64,
    pub problem: agena_failure::UserProblem,
    pub ts_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::StreamErrorEvent;

    #[test]
    fn stream_error_payload_round_trips() {
        let event = StreamErrorEvent {
            session_id: 7,
            problem: agena_failure::Failure::new(
                agena_failure::FailureCode::new("stream.test"),
                agena_failure::FailureCategory::Internal,
                agena_failure::FailureResponsibility::System,
                agena_failure::RetryDirective::Unknown,
                agena_failure::RecoveryDirective::None,
                agena_failure::FailureImpact::OperationFailed,
                agena_failure::UserPresentation::new("stream-test", "Stream test failure."),
            )
            .into(),
            ts_ms: 42,
        };
        assert_eq!(
            serde_json::from_value::<StreamErrorEvent>(serde_json::to_value(&event).unwrap())
                .unwrap(),
            event
        );
    }
}
