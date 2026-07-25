//! JSON-RPC 2.0 envelopes shared between SDK and host. Method names live as
//! constants in [`method`] so both sides agree on a single source of truth.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: JsonRpcVersion,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: JsonRpcVersion,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: JsonRpcVersion,
    pub id: RequestId,
    #[serde(flatten)]
    pub payload: ResponsePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponsePayload {
    Ok { result: Value },
    Err { error: ErrorObject },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonRpcVersion;

impl Default for JsonRpcVersion {
    fn default() -> Self {
        JsonRpcVersion
    }
}

impl Serialize for JsonRpcVersion {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        ser.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for JsonRpcVersion {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        if s == "2.0" {
            Ok(JsonRpcVersion)
        } else {
            Err(serde::de::Error::custom("jsonrpc must be 2.0"))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum RequestId {
    Num(i64),
    Str(String),
}

impl From<i64> for RequestId {
    fn from(v: i64) -> Self {
        RequestId::Num(v)
    }
}

/// Either side of the wire receives a [`Frame`] and must demux it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Frame {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

/// JSON-RPC error code ranges. Custom agena codes live in -33xxx.
pub mod codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;

    pub const PLUGIN_GENERIC: i32 = -33001;
    pub const PLUGIN_TIMEOUT: i32 = -33002;
    pub const PLUGIN_NOT_IMPLEMENTED: i32 = -33003;
    pub const PLUGIN_INVALID_PARAMS: i32 = -33004;
    pub const PLUGIN_DISCONNECTED: i32 = -33005;
    pub const PLUGIN_PANICKED: i32 = -33006;
    pub const HOST_UNAVAILABLE: i32 = -33007;
}

/// Method-name constants. **Both sides** import from here.
pub mod method {
    // host -> plugin
    pub const META_INIT: &str = "meta/init";
    pub const META_SHUTDOWN: &str = "meta/shutdown";
    pub const META_MANIFEST: &str = "meta/manifest";
    pub const META_PING: &str = "meta/ping";

    pub const HOOK_EVENT: &str = "hooks/event";
    pub const HOOK_TOOL_BEFORE: &str = "hooks/tool.execute.before";
    pub const HOOK_TOOL_AFTER: &str = "hooks/tool.execute.after";
    pub const HOOK_TOOL_INVOKE: &str = "hooks/tool.invoke";
    pub const HOOK_TOOL_PERMISSION_PATHS: &str = "hooks/tool.permission_paths";
    pub const HOOK_TOOL_PERMISSION_NETWORKS: &str = "hooks/tool.permission_networks";
    pub const HOOK_TOOL_INVOKE_STREAM: &str = "hooks/tool.invoke.stream";
    pub const COMMAND_INVOKE: &str = "command/invoke";
    /// Notification: plugin → host, one chunk in an open stream.
    pub const TOOL_STREAM_CHUNK: &str = "tool.stream.chunk";
    /// Notification: plugin → host, terminal frame in a stream.
    pub const TOOL_STREAM_END: &str = "tool.stream.end";
    /// Notification: plugin → host, stream finished with an error.
    pub const TOOL_STREAM_ERROR: &str = "tool.stream.error";
    pub const HOOK_CHAT_MESSAGE: &str = "hooks/chat.message";
    pub const HOOK_CHAT_PARAMS: &str = "hooks/chat.params";
    pub const HOOK_CHAT_HEADERS: &str = "hooks/chat.headers";
    pub const HOOK_CHAT_SYSTEM_TRANSFORM: &str = "hooks/chat.system.transform";
    pub const HOOK_AUTH: &str = "hooks/auth";
    pub const HOOK_PROVIDER_LIST: &str = "hooks/provider.list";
    pub const HOOK_PERMISSION_ASK: &str = "hooks/permission.ask_permission";
    pub const HOOK_NOTIFICATION: &str = "hooks/notification";
    pub const HOOK_COMMAND_BEFORE: &str = "hooks/command.execute.before";
    pub const HOOK_SHELL_ENV: &str = "hooks/shell.env";
    pub const HOOK_CONFIG: &str = "hooks/config";
    pub const HOOK_SESSION_START: &str = "hooks/session.start";
    pub const HOOK_SESSION_END: &str = "hooks/session.end";
    pub const HOOK_USER_PROMPT_SUBMIT: &str = "hooks/user.prompt.submit";
    pub const HOOK_TOOL_FAILURE: &str = "hooks/tool.execute.failure";
    pub const HOOK_TOOL_DEFINITION: &str = "hooks/tool.definition";
    pub const HOOK_AGENT_STOP: &str = "hooks/agent.stop";
    pub const HOOK_COMMAND_AFTER: &str = "hooks/command.execute.after";
    pub const HOOK_CHAT_MESSAGES_TRANSFORM: &str = "hooks/chat.messages.transform";
    pub const HOOK_PRE_RUN: &str = "hooks/pre_run";
    pub const HOOK_POST_RUN: &str = "hooks/post_run";

    // plugin -> host
    pub const HOST_LOG: &str = "host/log";
    pub const HOST_EVENT_PUBLISH: &str = "host/event.publish";
    pub const HOST_EVENT_SUBSCRIBE: &str = "host/event.subscribe";
    pub const HOST_EVENT_UNSUBSCRIBE: &str = "host/event.unsubscribe";
    pub const HOST_PERMISSION_ASK: &str = "host/permission.ask_permission";
    pub const HOST_PERMISSION_CHECK_PATH: &str = "host/permission.check_path";
    pub const HOST_PERMISSION_CHECK_NETWORK: &str = "host/permission.check_network";
    pub const HOST_CONFIG_READ: &str = "host/config.read";
    pub const HOST_CONFIG_RELOAD: &str = "host/config.reload";
    pub const HOST_TOOL_INVOKE: &str = "host/tool.invoke";
    pub const HOST_ASK_USER: &str = "host/ask_user";
    pub const HOST_SUBTASK_RUN: &str = "host/subtask.run";
    pub const HOST_SUBTASK_CANCEL: &str = "host/subtask.cancel";
    pub const HOST_SUBTASK_MESSAGE: &str = "host/subtask.message";
    pub const HOST_SUBTASK_OUTPUT: &str = "host/subtask.output";
    pub const HOST_TOOL_LIST: &str = "host/tool.list";
    pub const HOST_CONTEXT_STATUS: &str = "host/context.status";
    pub const HOST_SESSION_SET_MODEL: &str = "host/session.set_model";
    pub const HOST_SNAPSHOT_ENTER: &str = "host/snapshot.enter";
    pub const HOST_SNAPSHOT_EXIT: &str = "host/snapshot.exit";
    pub const HOST_MONITOR_START: &str = "host/monitor.start";
    pub const HOST_MONITOR_LIST: &str = "host/monitor.list";
    pub const HOST_MONITOR_READ: &str = "host/monitor.read";
    pub const HOST_MONITOR_STOP: &str = "host/monitor.stop";
    pub const HOST_TOOL_REGISTRY_REGISTER: &str = "host/tool.registry.register";
    pub const HOST_TOOL_REGISTRY_UPDATE: &str = "host/tool.registry.update";
    pub const HOST_TOOL_REGISTRY_REMOVE: &str = "host/tool.registry.remove";
    pub const HOST_TOOL_REGISTRY_LIST: &str = "host/tool.registry.list";
    pub const HOST_STORAGE_GET: &str = "host/storage.get";
    pub const HOST_STORAGE_SET: &str = "host/storage.set";
    pub const HOST_STORAGE_DELETE: &str = "host/storage.delete";
    pub const HOST_STORAGE_LIST: &str = "host/storage.list";
    pub const HOST_SECRET_GET: &str = "host/secret.get";
    pub const HOST_SECRET_SET: &str = "host/secret.set";
    pub const HOST_SECRET_DELETE: &str = "host/secret.delete";
    pub const HOST_SECRET_LIST: &str = "host/secret.list";
    pub const HOST_PLUGIN_STATUS_LIST: &str = "host/plugin.status.list";
    pub const HOST_PLUGIN_STATUS_GET: &str = "host/plugin.status.get";
    pub const HOST_LSP_LIST_SERVERS: &str = "host/lsp.list_servers";
    pub const HOST_LSP_LIST_DIAGNOSTICS: &str = "host/lsp.list_diagnostics";
    pub const HOST_SNAPSHOT_LIST: &str = "host/snapshot.list";
    pub const HOST_SCHEDULER_LIST: &str = "host/scheduler.list";
    pub const HOST_SCHEDULER_CREATE: &str = "host/scheduler.create";
    pub const HOST_SCHEDULER_DELETE: &str = "host/scheduler.delete";
    pub const HOST_AGENT_REGISTER: &str = "host/agent.register";
    pub const HOST_AGENT_REMOVE: &str = "host/agent.remove";
    pub const HOST_AGENT_LIST: &str = "host/agent.list";
    pub const HOST_AGENT_GET: &str = "host/agent.get";
    pub const HOST_AGENT_SWITCH: &str = "host/agent.switch";
    pub const HOST_AGENT_RESTORE: &str = "host/agent.restore";
    pub const HOST_HOOK_LIST: &str = "host/hook.list";
    pub const HOST_MCP_LIST_SERVERS: &str = "host/mcp.list_servers";
    pub const HOST_MCP_ADD_SERVER: &str = "host/mcp.add_server";
    pub const HOST_MCP_REMOVE_SERVER: &str = "host/mcp.remove_server";
    pub const HOST_UI_STATUSLINE_CONTRIBUTE: &str = "host/ui.statusline.contribute";
    pub const HOST_UI_STATUSLINE_LIST: &str = "host/ui.statusline.list";
    pub const HOST_UI_STATUSLINE_REMOVE: &str = "host/ui.statusline.remove";
    pub const HOST_UI_THEME_REGISTER: &str = "host/ui.theme.register";
    pub const HOST_UI_THEME_LIST: &str = "host/ui.theme.list";
    pub const HOST_UI_THEME_REMOVE: &str = "host/ui.theme.remove";
    pub const HOST_UI_PERMISSION_SET_HANDLER: &str = "host/ui.permission.set_handler";
    pub const HOST_UI_PERMISSION_CLEAR_HANDLER: &str = "host/ui.permission.clear_handler";
}
