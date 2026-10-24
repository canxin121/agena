use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Error event on a stream.
pub struct StreamErrorEvent {
    pub session_id: i64,
    pub problem: agena_failure::UserProblem,
    pub ts_ms: i64,
}
