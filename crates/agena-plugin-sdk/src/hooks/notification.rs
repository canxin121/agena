use serde::{Deserialize, Serialize};

/// A user-attention event shared by terminal and browser delivery adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    Completed,
    Failed,
    Cancelled,
    Permission,
    Question,
}

impl AttentionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Permission => "permission",
            Self::Question => "question",
        }
    }

    pub fn is_request(self) -> bool {
        matches!(self, Self::Permission | Self::Question)
    }
}

/// Stable identity and lifecycle of a plugin's attention notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionNotification {
    pub id: String,
    pub kind: AttentionKind,
    pub created_at_ms: i64,
    /// Resolved requests remain in recent history but must no longer alert.
    #[serde(default)]
    pub resolved: bool,
}

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
