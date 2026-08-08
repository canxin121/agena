use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ── chat.message ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Direction of a chat message relative to the provider.
pub enum ChatDirection {
    FromUser,
    ToProvider,
    FromProvider,
    ToUser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A chat message observed by a chat hook.
pub struct ChatMessage {
    pub role: String,
    pub content: serde_json::Value,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: serde_json::Value::String(content.into()),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: serde_json::Value::String(content.into()),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: serde_json::Value::String(content.into()),
        }
    }

    pub fn text(&self) -> Option<&str> {
        self.content.as_str()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of a chat message hook.
pub struct ChatMessageInput {
    pub session_id: i64,
    pub direction: ChatDirection,
    pub message: ChatMessage,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied to a chat message by a hook.
pub struct ChatMessagePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<ChatMessage>,
    /// Drop this message entirely from the provider request.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub drop: bool,
}

// ── chat.params ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of a chat params hook.
pub struct ChatParamsInput {
    pub provider: String,
    pub model: String,
    pub params: serde_json::Value,
    /// Session the chat.params hook runs for, when known. Used to attribute
    /// the hook run to the right session transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied to chat params by a hook.
pub struct ChatParamsPatch {
    /// Sparse object merged into the existing params. Only keys present are
    /// overwritten; absent keys are left unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

// ── chat.headers ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of a chat headers hook.
pub struct ChatHeadersInput {
    pub provider: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied to chat headers by a hook.
pub struct ChatHeadersPatch {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub set: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove: Vec<String>,
}

// ── chat.system.transform ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of a chat system transform hook.
pub struct ChatSystemTransformInput {
    pub session_id: i64,
    pub current_system: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied to the system prompt by a hook.
pub struct ChatSystemTransformPatch {
    /// Fully replace the system prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Append to the existing system prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub append: Option<String>,
    /// Prepend to the existing system prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepend: Option<String>,
}

// ── chat.messages.transform ────────────────────────────────────────────────

/// Fired before the full message history is sent to the provider. Unlike
/// `chat.message` (which intercepts one message at a time), this hook
/// receives the complete list and can perform batch rewrites, filtering,
/// or summarisation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessagesTransformInput {
    pub session_id: i64,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied to the chat messages by a hook.
pub struct ChatMessagesTransformPatch {
    /// Replace the entire message list. If `None` the original is kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<ChatMessage>>,
}
