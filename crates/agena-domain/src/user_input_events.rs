use serde::{Deserialize, Serialize};

/// Published when the runtime registers a durable interactive user-input
/// request (host/plugin `ask_user` such as a plan approval, or the
/// `interaction.ask` tool). Clients use it as an invalidation signal to
/// re-read the session execution so the pending request (approval modal,
/// question prompt) is surfaced immediately instead of waiting for an
/// unrelated refresh.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserInputRequestedEvent {
    pub session_id: i64,
    pub operation_id: String,
    pub call_id: i64,
    pub request_id: String,
    pub ts_ms: i64,
}
