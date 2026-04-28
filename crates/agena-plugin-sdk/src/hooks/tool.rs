use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::attachment::AttachmentItem;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    Builtin,
    Plugin { plugin: String },
}

// ── tool.execute.before ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBeforeInput {
    pub tool_name: String,
    pub source: ToolSource,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    pub input: serde_json::Value,
    /// Carry-through: accumulated title override from prior plugins in the chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_override: Option<String>,
    /// Carry-through: accumulated metadata from prior plugins in the chain.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolBeforePatch {
    /// Override the tool's argument JSON before execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    /// Override the pending-state title shown in the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_override: Option<String>,
    /// Key-value metadata merged into the tool execution record.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

// ── tool.execute.after ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolAfterInput {
    pub tool_name: String,
    pub source: ToolSource,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    pub title: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolAfterPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

// ── tool.execute.failure ───────────────────────────────────────────────────

/// Fired when a tool execution fails. Notification — no patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFailureInput {
    pub tool_name: String,
    pub source: ToolSource,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    pub input: serde_json::Value,
    pub error: String,
    /// True when the failure was triggered by a user interrupt / cancellation.
    #[serde(default)]
    pub is_interrupt: bool,
}

// ── tool.definition ────────────────────────────────────────────────────────

/// Sent once per tool before it is listed to the LLM. Plugins can override
/// the description and/or parameter schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinitionInput {
    pub tool_name: String,
    pub source: ToolSource,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolDefinitionPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
}

// ── tool.invoke (custom plugin tools) ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvokeInput {
    pub tool_name: String,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvokeOutput {
    pub title: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
}

impl ToolInvokeOutput {
    pub fn text(s: impl Into<String>) -> Self {
        let s = s.into();
        Self {
            title: String::new(),
            output_text: s,
            payload: None,
            metadata: BTreeMap::new(),
            attachments: Vec::new(),
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = Some(payload);
        self
    }

    pub fn with_metadata(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.metadata.insert(k.into(), v.into());
        self
    }

    pub fn with_attachment(mut self, att: AttachmentItem) -> Self {
        self.attachments.push(att);
        self
    }

    pub fn with_attachments(mut self, atts: impl IntoIterator<Item = AttachmentItem>) -> Self {
        self.attachments.extend(atts);
        self
    }
}

// ── streaming tool invocation ──────────────────────────────────────────────

/// Initial response to `hooks/tool.invoke.stream`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvokeStreamHandle {
    pub stream_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// One chunk pushed by the plugin while the stream is open.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStreamChunk {
    pub stream_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_delta: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// Final marker that closes the stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStreamEnd {
    pub stream_id: String,
    pub title: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
}
