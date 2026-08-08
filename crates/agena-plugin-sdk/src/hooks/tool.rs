use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::attachment::AttachmentItem;
use crate::identity::{PluginKey, ToolKey};
use crate::manifest::ToolTag;
pub use agena_domain::ToolPresentationSection;

// ── tool.execute.before ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of the tool-before hook.
pub struct ToolBeforeInput {
    pub tool: ToolKey,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<ToolTag>,
    /// The tool's full permission contract, so a hook can make authority
    /// decisions without ever treating a tag as a permission.
    #[serde(default)]
    pub contract: crate::manifest::ToolPermissionContract,
    pub input: serde_json::Value,
    /// Carry-through: accumulated title override from prior plugins in the chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_override: Option<String>,
    /// Carry-through: accumulated metadata from prior plugins in the chain.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl ToolBeforeInput {
    pub fn tool_name(&self) -> &str {
        self.tool.name()
    }

    pub fn plugin_key(&self) -> &PluginKey {
        self.tool.plugin()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied before a tool executes.
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
/// Input of the tool-after hook.
pub struct ToolAfterInput {
    pub tool: ToolKey,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    pub title: String,
    pub summary: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl ToolAfterInput {
    pub fn tool_name(&self) -> &str {
        self.tool.name()
    }

    pub fn plugin_key(&self) -> &PluginKey {
        self.tool.plugin()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied after a tool executes.
pub struct ToolAfterPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
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
    pub tool: ToolKey,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    pub input: serde_json::Value,
    /// Safe user/API projection. Diagnostic sources and model feedback are
    /// intentionally unavailable at the plugin hook boundary.
    pub failure: agena_failure::UserProblem,
}

impl ToolFailureInput {
    pub fn tool_name(&self) -> &str {
        self.tool.name()
    }

    pub fn plugin_key(&self) -> &PluginKey {
        self.tool.plugin()
    }
}

// ── tool.definition ────────────────────────────────────────────────────────

/// Sent once per tool before it is listed to the LLM. Plugins can override
/// the summary and/or parameter schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinitionInput {
    pub tool: ToolKey,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_mode: Option<crate::manifest::ToolDescriptionMode>,
    pub input_schema: serde_json::Value,
}

impl ToolDefinitionInput {
    pub fn tool_name(&self) -> &str {
        self.tool.name()
    }

    pub fn plugin_key(&self) -> &PluginKey {
        self.tool.plugin()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied to a tool definition by a hook.
pub struct ToolDefinitionPatch {
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
/// Input of the tool invoke hook.
pub struct ToolInvokeInput {
    pub tool_name: String,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Copy)]
/// Context of a tool invocation hook.
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
/// Input of the tool permission paths hook.
pub struct ToolPermissionPathsInput {
    pub tool_name: String,
    pub workspace_root: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of the tool permission networks hook.
pub struct ToolPermissionNetworksInput {
    pub tool_name: String,
    pub workspace_root: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Output of a tool invocation hook.
pub struct ToolInvokeOutput {
    pub title: String,
    pub summary: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<ToolPresentationSection>,
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
            summary: agena_tool::normalize_tool_summary(&s),
            output_text: s,
            sections: Vec::new(),
            payload: None,
            metadata: BTreeMap::new(),
            attachments: Vec::new(),
        }
    }

    pub fn from_parts(
        title: impl Into<String>,
        summary: impl Into<String>,
        output_text: impl Into<String>,
        payload: Option<serde_json::Value>,
        metadata: BTreeMap<String, String>,
        attachments: Vec<AttachmentItem>,
    ) -> Self {
        Self {
            title: agena_tool::normalize_tool_title(title.into()),
            summary: agena_tool::normalize_tool_summary(summary.into()),
            output_text: output_text.into(),
            sections: Vec::new(),
            payload,
            metadata,
            attachments,
        }
    }

    pub fn with_section(mut self, title: impl Into<String>, text: impl Into<String>) -> Self {
        self.sections.push(ToolPresentationSection {
            title: title.into(),
            text: text.into(),
        });
        self
    }
}

/// Conversion into a tool invocation output.
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
        Ok(ToolInvokeOutput::from_parts(
            String::new(),
            crate::macro_support::typed_tool_summary(&self),
            output_text,
            Some(self),
            BTreeMap::new(),
            Vec::new(),
        ))
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// Final marker that closes the stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStreamEnd {
    pub stream_id: String,
    pub title: String,
    pub summary: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<ToolPresentationSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
}

impl ToolStreamEnd {
    pub fn text(stream_id: impl Into<String>, output_text: impl Into<String>) -> Self {
        let output_text = output_text.into();
        Self {
            stream_id: stream_id.into(),
            title: String::new(),
            summary: agena_tool::normalize_tool_summary(&output_text),
            output_text,
            sections: Vec::new(),
            payload: None,
            metadata: BTreeMap::new(),
            attachments: Vec::new(),
        }
    }

    pub fn from_output(stream_id: impl Into<String>, output: ToolInvokeOutput) -> Self {
        Self {
            stream_id: stream_id.into(),
            title: output.title,
            summary: output.summary,
            output_text: output.output_text,
            sections: output.sections,
            payload: output.payload,
            metadata: output.metadata,
            attachments: output.attachments,
        }
    }

    pub fn from_parts(
        stream_id: impl Into<String>,
        title: impl Into<String>,
        summary: impl Into<String>,
        output_text: impl Into<String>,
        payload: Option<serde_json::Value>,
        metadata: BTreeMap<String, String>,
        attachments: Vec<AttachmentItem>,
    ) -> Self {
        Self {
            stream_id: stream_id.into(),
            title: agena_tool::normalize_tool_title(title.into()),
            summary: agena_tool::normalize_tool_summary(summary.into()),
            output_text: output_text.into(),
            sections: Vec::new(),
            payload,
            metadata,
            attachments,
        }
    }
}

/// Conversion into a tool stream end.
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
