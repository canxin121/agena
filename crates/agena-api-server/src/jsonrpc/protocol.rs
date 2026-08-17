use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSONRPC_VERSION: &str = "2.0";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
/// Identifier of a JSON-RPC request.
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
/// A JSON-RPC request.
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A JSON-RPC notification.
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A JSON-RPC error object.
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A JSON-RPC response.
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, PartialEq)]
/// A message received by the JSON-RPC server.
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
    pub const MESSAGE_SUBMIT: &str = "message/submit";
    pub const PERMISSION_REPLY: &str = "permission/reply";
    pub const SESSIONS_LIST: &str = "sessions/list";
    pub const MESSAGES_LIST: &str = "messages/list";
    pub const RUN_CANCEL: &str = "run/cancel";
    // The v1 `events/subscribe` method is removed in the v2 protocol: session
    // mutations are delivered as SessionChange part-patch notifications (see
    // `AppServerNotification`) rather than through an explicit subscription.
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Params of the create-session method.
pub struct CreateSessionParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Result of the create-session method.
pub struct CreateSessionResult {
    pub session_id: i64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Params of the submit-message method.
pub struct SubmitRunParams {
    pub session_id: i64,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Result of the submit-message method: the accepted v2 run result.
///
/// `run_id` is the accepted run marker part id; `parts` carries that run's
/// marker plus its content parts in creation order, aligned with the storage
/// `SubmitOutcome{run_id, created, parts}` contract. The run marker's `state`
/// conveys the run status that v1 reported as `status`; the v1 `text` field is
/// replaced by reading the assistant `text` parts from `parts`.
pub struct SubmitRunResult {
    pub session_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<agena_api::resource::SessionTranscriptPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Params of the permission-reply method.
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
/// Decision of a permission reply.
pub enum PermissionDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Scope remembered for a permission decision.
pub enum PermissionRememberScope {
    Session,
    Workspace,
    Global,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Result of the permission-reply method.
pub struct PermissionReplyResult {
    pub session_id: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Params of the list-sessions method.
pub struct ListSessionsParams {
    #[serde(default)]
    pub offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// One session in a session listing.
pub struct SessionListItem {
    pub session_id: i64,
    pub title: String,
    pub status: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Result of the list-sessions method.
pub struct ListSessionsResult {
    pub sessions: Vec<SessionListItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Params of the read-messages method.
pub struct ReadPartsParams {
    pub session_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Result of the read-messages method: the session's v2 part transcript.
pub struct ReadPartsResult {
    pub parts: Vec<agena_api::resource::SessionTranscriptPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Params of the cancel-run method.
pub struct CancelRunParams {
    pub session_id: i64,
    #[serde(default)]
    pub execution_id: Option<agena_domain::ExecutionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Result of the cancel-run method.
pub struct CancelRunResult {
    pub session_id: i64,
    pub result: agena_domain::CancellationResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// Server-initiated notification sent to clients.
///
/// The v1 `MessageDelta` / `ToolEvent` payloads are replaced by v2 part
/// patches: every committed session mutation is delivered as a
/// `PartAdded` / `PartUpdated` / `PartRemoved` / `SessionMetaUpdated`
/// notification (mirroring `agena_api::live::SessionChangeResource`), with the
/// part payload in the v2 `SessionTranscriptPart` shape. `PermissionRequest`
/// and `SessionStateChanged` remain dedicated lifecycle signals.
pub enum AppServerNotification {
    PartAdded {
        session_id: i64,
        part: Box<agena_api::resource::SessionTranscriptPart>,
    },
    PartUpdated {
        session_id: i64,
        part: Box<agena_api::resource::SessionTranscriptPart>,
    },
    PartRemoved {
        session_id: i64,
        part_id: i64,
    },
    SessionMetaUpdated {
        session_id: i64,
        version: i64,
        title: String,
        updated_at_ms: i64,
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
