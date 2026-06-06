use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::attachment::AttachmentItem;
use crate::manifest::ToolTag;

// ── tool.execute.before ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBeforeInput {
    pub tool_name: String,
    pub plugin_name: String,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<ToolTag>,
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
    /// Abort execution before the tool runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abort_reason: Option<String>,
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
    pub plugin_name: String,
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
    pub plugin_name: String,
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
    pub plugin_name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_mode: Option<crate::manifest::ToolDescriptionMode>,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolDefinitionPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_mode: Option<crate::manifest::ToolDescriptionMode>,
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

#[derive(Debug, Clone, Copy)]
pub struct ToolInvokeContext<'a> {
    pub tool_name: &'a str,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: &'a str,
}

impl ToolInvokeInput {
    pub fn context(&self) -> ToolInvokeContext<'_> {
        ToolInvokeContext {
            tool_name: self.tool_name.as_str(),
            session_id: self.session_id,
            call_id: self.call_id,
            workspace_root: self.workspace_root.as_str(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermissionPathsInput {
    pub tool_name: String,
    pub workspace_root: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermissionNetworksInput {
    pub tool_name: String,
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

pub trait IntoToolInvokeOutput {
    fn into_tool_invoke_output(self) -> crate::Result<ToolInvokeOutput>;
}

impl IntoToolInvokeOutput for ToolInvokeOutput {
    fn into_tool_invoke_output(self) -> crate::Result<ToolInvokeOutput> {
        Ok(self)
    }
}

impl IntoToolInvokeOutput for String {
    fn into_tool_invoke_output(self) -> crate::Result<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text(self))
    }
}

impl IntoToolInvokeOutput for &str {
    fn into_tool_invoke_output(self) -> crate::Result<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text(self))
    }
}

impl IntoToolInvokeOutput for serde_json::Value {
    fn into_tool_invoke_output(self) -> crate::Result<ToolInvokeOutput> {
        let output_text = match &self {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(value) => value.clone(),
            _ => self.to_string(),
        };
        Ok(ToolInvokeOutput::text(output_text).with_payload(self))
    }
}

impl IntoToolInvokeOutput for () {
    fn into_tool_invoke_output(self) -> crate::Result<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text(String::new()))
    }
}

impl<T, E> IntoToolInvokeOutput for std::result::Result<T, E>
where
    T: IntoToolInvokeOutput,
    E: Into<crate::PluginError>,
{
    fn into_tool_invoke_output(self) -> crate::Result<ToolInvokeOutput> {
        match self {
            Ok(value) => value.into_tool_invoke_output(),
            Err(err) => Err(err.into()),
        }
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

impl ToolStreamEnd {
    pub fn text(stream_id: impl Into<String>, output_text: impl Into<String>) -> Self {
        Self {
            stream_id: stream_id.into(),
            title: String::new(),
            output_text: output_text.into(),
            payload: None,
            metadata: BTreeMap::new(),
            attachments: Vec::new(),
        }
    }

    pub fn from_output(stream_id: impl Into<String>, output: ToolInvokeOutput) -> Self {
        Self {
            stream_id: stream_id.into(),
            title: output.title,
            output_text: output.output_text,
            payload: output.payload,
            metadata: output.metadata,
            attachments: output.attachments,
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

pub trait IntoToolStreamEnd {
    fn into_tool_stream_end(self, stream_id: String) -> crate::Result<ToolStreamEnd>;
}

impl IntoToolStreamEnd for ToolStreamEnd {
    fn into_tool_stream_end(self, _stream_id: String) -> crate::Result<ToolStreamEnd> {
        Ok(self)
    }
}

impl IntoToolStreamEnd for ToolInvokeOutput {
    fn into_tool_stream_end(self, stream_id: String) -> crate::Result<ToolStreamEnd> {
        Ok(ToolStreamEnd::from_output(stream_id, self))
    }
}

impl IntoToolStreamEnd for String {
    fn into_tool_stream_end(self, stream_id: String) -> crate::Result<ToolStreamEnd> {
        Ok(ToolStreamEnd::text(stream_id, self))
    }
}

impl IntoToolStreamEnd for &str {
    fn into_tool_stream_end(self, stream_id: String) -> crate::Result<ToolStreamEnd> {
        Ok(ToolStreamEnd::text(stream_id, self))
    }
}

impl IntoToolStreamEnd for serde_json::Value {
    fn into_tool_stream_end(self, stream_id: String) -> crate::Result<ToolStreamEnd> {
        self.into_tool_invoke_output()?
            .into_tool_stream_end(stream_id)
    }
}

impl IntoToolStreamEnd for () {
    fn into_tool_stream_end(self, stream_id: String) -> crate::Result<ToolStreamEnd> {
        Ok(ToolStreamEnd::text(stream_id, String::new()))
    }
}

impl<T, E> IntoToolStreamEnd for std::result::Result<T, E>
where
    T: IntoToolStreamEnd,
    E: Into<crate::PluginError>,
{
    fn into_tool_stream_end(self, stream_id: String) -> crate::Result<ToolStreamEnd> {
        match self {
            Ok(value) => value.into_tool_stream_end(stream_id),
            Err(err) => Err(err.into()),
        }
    }
}

/// Terminal error marker for `hooks/tool.invoke.stream`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStreamError {
    pub stream_id: String,
    pub error: crate::PluginError,
}
