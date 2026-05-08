use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSONRPC_VERSION: &str = "2.0";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

impl From<i64> for RequestId {
    fn from(value: i64) -> Self {
        Self::Number(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InboundMessage {
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
    Response(JsonRpcResponse),
}

impl InboundMessage {
    pub fn from_value(value: Value) -> Result<Self, serde_json::Error> {
        let has_method = value.get("method").is_some();
        let has_id = value.get("id").is_some();
        if has_method && has_id {
            Ok(Self::Request(serde_json::from_value(value)?))
        } else if has_method {
            Ok(Self::Notification(serde_json::from_value(value)?))
        } else {
            Ok(Self::Response(serde_json::from_value(value)?))
        }
    }
}

pub mod method {
    pub const SESSION_CREATE: &str = "session/create";
    pub const TURN_SUBMIT: &str = "turn/submit";
    pub const PERMISSION_REPLY: &str = "permission/reply";
    pub const SESSIONS_LIST: &str = "sessions/list";
    pub const MESSAGES_LIST: &str = "messages/list";
    pub const TURN_CANCEL: &str = "turn/cancel";
    pub const EVENTS_SUBSCRIBE: &str = "events/subscribe";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateSessionParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateSessionResult {
    pub session_id: i64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubmitTurnParams {
    pub session_id: i64,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubmitTurnResult {
    pub session_id: i64,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionReplyParams {
    pub session_id: i64,
    pub request_id: String,
    pub decision: PermissionDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remember: Option<PermissionRememberScope>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRememberScope {
    Session,
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionReplyResult {
    pub session_id: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListSessionsParams {
    #[serde(default)]
    pub offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionListItem {
    pub session_id: i64,
    pub title: String,
    pub status: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListSessionsResult {
    pub sessions: Vec<SessionListItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadMessagesParams {
    pub session_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageItem {
    pub message_id: i64,
    pub role: String,
    pub status: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadMessagesResult {
    pub messages: Vec<MessageItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelTurnParams {
    pub session_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelTurnResult {
    pub session_id: i64,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppServerNotification {
    MessageDelta {
        session_id: i64,
        text: String,
    },
    ToolEvent {
        session_id: Option<i64>,
        payload: Value,
    },
    PermissionRequest {
        session_id: i64,
        request_id: String,
        reason: String,
    },
    SessionStateChanged {
        session_id: i64,
        status: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_round_trips() {
        let request = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: RequestId::String("abc".to_owned()),
            method: method::TURN_SUBMIT.to_owned(),
            params: Some(json!({"session_id": 7, "prompt": "hello"})),
        };
        let value = serde_json::to_value(&request).unwrap();
        let decoded: JsonRpcRequest = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn notification_round_trips() {
        let notification = AppServerNotification::SessionStateChanged {
            session_id: 1,
            status: "blocked".to_owned(),
        };
        let value = serde_json::to_value(&notification).unwrap();
        assert_eq!(value["kind"], "session_state_changed");
        let decoded: AppServerNotification = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, notification);
    }
}
