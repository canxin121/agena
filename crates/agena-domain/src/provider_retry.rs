use serde::{Deserialize, Serialize};

use crate::{AssistantReplyId, ExecutionId};

/// A retryable provider failure observed while streaming a completion.
///
/// The runtime broadcasts this live (never persisted to the event log) so a
/// terminal UI can render an immediate retry-progress activity for the reply.
/// A subsequent [`ProviderRetryResolvedEvent`] removes the live node on
/// success; a final failure is persisted through the durable error activity
/// written by the `ExecutionFinished` projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRetryEvent {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub reply_id: AssistantReplyId,
    /// Zero-based retry counter at the provider retry loop.
    pub retry_index: u32,
    /// One-based attempt number displayed to the user (retry_index + 1).
    pub attempt: u32,
    /// Maximum attempts for this retry phase (startup/early or replay-safe).
    pub max_retries: u32,
    /// Provider error summary shown as the activity's detail line.
    pub message: String,
    pub ts_ms: i64,
}

/// Marks the end of a retry sequence for one reply.
///
/// `succeeded` is `true` only when the run completed after at least one
/// retry; in that case the live retry-progress node must be removed. A final
/// failure is persisted through the durable `ExecutionFinished` error
/// activity instead, so `succeeded == false` carries no extra projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRetryResolvedEvent {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub reply_id: AssistantReplyId,
    pub succeeded: bool,
    pub ts_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_retry_events_round_trip() {
        let retry = ProviderRetryEvent {
            session_id: 7,
            execution_id: ExecutionId::new(),
            reply_id: AssistantReplyId::new(),
            retry_index: 0,
            attempt: 1,
            max_retries: 5,
            message: "provider unavailable".to_owned(),
            ts_ms: 42,
        };
        assert_eq!(
            serde_json::from_value::<ProviderRetryEvent>(serde_json::to_value(&retry).unwrap())
                .unwrap(),
            retry
        );

        let resolved = ProviderRetryResolvedEvent {
            session_id: 7,
            execution_id: ExecutionId::new(),
            reply_id: AssistantReplyId::new(),
            succeeded: true,
            ts_ms: 43,
        };
        assert_eq!(
            serde_json::from_value::<ProviderRetryResolvedEvent>(
                serde_json::to_value(&resolved).unwrap()
            )
            .unwrap(),
            resolved
        );
    }
}
