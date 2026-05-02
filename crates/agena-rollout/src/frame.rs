//! Wire format for rollout frames.
//!
//! One [`RolloutFrame`] per line; values are stable enough to import into
//! a fresh agena instance and reconstruct the conversation view (without
//! re-executing tools).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutFrame {
    pub seq: u64,
    pub ts: DateTime<Utc>,
    pub kind: RolloutKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RolloutKind {
    /// First frame of the file: identity + boot context.
    SessionMeta(SessionMeta),
    /// User-authored input.  Parts are encoded as opaque JSON to remain
    /// agena-agnostic — the importer is expected to know how to parse
    /// them.
    UserMessage {
        parts: Value,
    },
    /// LLM completion (one or more parts: text, tool calls, reasoning).
    AssistantMessage {
        parts: Value,
    },
    /// A tool call request landed and started executing.
    ToolCall {
        call_id: String,
        name: String,
        args: Value,
    },
    /// A tool call finished.
    ToolResult {
        call_id: String,
        output: Value,
        duration_ms: u64,
        error: Option<String>,
    },
    /// Permission decision relevant to the conversation history.
    Permission {
        request: Value,
        decision: Value,
    },
    /// Plan-mode markers.
    PlanEntered {
        slug: String,
        file_path: String,
    },
    PlanExited {
        slug: String,
        approved: bool,
    },
    /// A plugin event surfaced into the session timeline.
    PluginEvent {
        plugin: String,
        payload: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub agena_version: String,
    /// Free-form: provider id, model id, system prompt hash, env tag.
    /// Importers may ignore unknown fields.
    pub context: Value,
}
