use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of a notification hook.
pub struct NotificationInput {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub title: String,
    pub message: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}
