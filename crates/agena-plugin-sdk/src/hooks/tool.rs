use std::collections::BTreeMap;

pub use agena_domain::{ProcessShell, ShellLaunchMode};
use serde::{Deserialize, Serialize};

use crate::attachment::AttachmentItem;
use crate::identity::{PluginKey, ToolKey};
use crate::manifest::ToolTag;

/// Stored with a completed receipt when a hook withholds its result from the model.
pub const TOOL_MODEL_OUTPUT_WITHHELD: &str = "model_output_withheld";

/// Stored in tool metadata when a hook requests ending the current model turn after this tool call.
pub const TOOL_END_TURN: &str = "agena_tool_end_turn";

// ── tool.before ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of the tool-before hook.
pub struct ToolBeforeInput {
    pub tool: ToolKey,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    /// Resolved local working directory of a shell invocation, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<ToolTag>,
    pub input: serde_json::Value,
    /// Carry-through: accumulated title override from prior plugins in the chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_override: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_context: Vec<String>,
    /// Approval requested by a prior hook for this invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_ask: Option<String>,
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
    /// Ask for approval of the effective invocation after the remaining hooks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_ask: Option<String>,
    /// Override the pending-state title shown in the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_override: Option<String>,
    /// Context to send alongside the eventual tool result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_context: Vec<String>,
    /// Key-value metadata merged into the tool execution record.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

// ── tool.after ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// The originating invocation and output when a shell tool collects a final result.
pub struct ShellProcessCompletion {
    pub invocation: agena_domain::ShellInvocation,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of the tool-after hook.
pub struct ToolAfterInput {
    pub tool: ToolKey,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    /// The directory in which the shell invocation started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_completion: Option<ShellProcessCompletion>,
    /// Effective tool arguments after the pre-execution hooks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    pub title: String,
    pub summary: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_context: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_output: Option<String>,
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

/// The completed local tool calls from one provider round, in request order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBatchInput {
    pub session_id: i64,
    pub calls: Vec<ToolBatchCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBatchCall {
    pub call_id: i64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolKey>,
    pub input: serde_json::Value,
    pub model_output: String,
}

/// Context for the next model round, or a reason to end the current turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolBatchPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied after a tool executes.
pub struct ToolAfterPatch {
    /// Model context accompanying the tool's result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_context: Vec<String>,
    /// Replace the model's projection while retaining the tool receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_output: Option<String>,
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

// ── tool.render ────────────────────────────────────────────────────────────

/// Pure runtime projection input. The host sends the original invocation
/// input and the one durable raw result; plugins must not read or mutate the
/// session record while rendering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolRenderInput {
    pub tool_name: String,
    pub input: serde_json::Value,
    pub output: agena_domain::RawOutput,
}

/// Human-facing projection owned by the tool/plugin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolHumanPresentation {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<agena_domain::ViewBlock>,
}

/// Optional plugin-owned projections. Each missing side independently falls
/// back to the runtime's tool renderer and then the system renderer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolRenderOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human: Option<ToolHumanPresentation>,
}

// ── tool.failure ───────────────────────────────────────────────────

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
    /// Elapsed execution time, present once the tool's effect was admitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Safe reason from a definite denial by the automatic approval model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_denial: Option<String>,
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
    pub input_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
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
    pub input_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
}

// ── tool.invoke (custom plugin tools) ─────────────────────────────────────

/// Host-resolved sender identity for one execution. This is metadata, never a
/// model-supplied tool argument; host callbacks bind it to their authority token.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerIdentity {
    pub platform: String,
    pub tenant: String,
    pub account: String,
    pub actor: String,
    #[serde(default)]
    pub group_ids: Vec<String>,
    pub resource: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    pub role: String,
}

impl From<&agena_domain::ChannelAuthority> for CallerIdentity {
    fn from(authority: &agena_domain::ChannelAuthority) -> Self {
        Self {
            platform: authority.platform.clone(),
            tenant: authority.identity.tenant.clone(),
            account: authority.identity.account.clone(),
            actor: authority.actor.clone(),
            group_ids: authority.group_ids.clone(),
            resource: authority.resource.clone(),
            thread: authority.thread.clone(),
            role: authority.role.as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of the tool invoke hook.
pub struct ToolInvokeInput {
    pub tool_name: String,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<CallerIdentity>,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone)]
/// Context of a tool invocation hook.
pub struct ToolInvokeContext<'a> {
    pub tool_name: &'a str,
    pub session_id: i64,
    pub call_id: i64,
    pub workspace_root: &'a str,
    pub caller: Option<&'a CallerIdentity>,
    /// Invocation-scoped cancellation, also available to spawned child work.
    pub cancellation: crate::CancellationToken,
}

impl ToolInvokeContext<'_> {
    /// Whether this invocation has no project working directory.
    pub fn is_global(&self) -> bool {
        self.workspace_root.is_empty()
    }
}

impl ToolInvokeInput {
    pub fn context(&self) -> ToolInvokeContext<'_> {
        ToolInvokeContext {
            tool_name: self.tool_name.as_str(),
            session_id: self.session_id,
            call_id: self.call_id,
            workspace_root: self.workspace_root.as_str(),
            caller: self.caller.as_ref(),
            cancellation: crate::current_cancellation_token().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Output of a tool invocation hook.
pub struct ToolInvokeOutput {
    pub title: String,
    pub summary: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
    /// Context for the model alongside the result, separate from the visible output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_context: Vec<String>,
}

impl ToolInvokeOutput {
    pub fn text(s: impl Into<String>) -> Self {
        let s = s.into();
        Self {
            title: String::new(),
            summary: agena_plugin_contracts::normalize_tool_summary(&s),
            output_text: s,
            payload: None,
            metadata: BTreeMap::new(),
            attachments: Vec::new(),
            additional_context: Vec::new(),
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
            title: agena_plugin_contracts::normalize_tool_title(title.into()),
            summary: agena_plugin_contracts::normalize_tool_summary(summary.into()),
            output_text: output_text.into(),
            payload,
            metadata,
            attachments,
            additional_context: Vec::new(),
        }
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
#[serde(deny_unknown_fields)]
pub struct ToolStreamChunk {
    pub stream_id: String,
    /// Typed source data. Routing metadata and final outcome are separate.
    pub payload: agena_domain::ContentInput,
}

/// Final marker that closes the stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStreamEnd {
    pub stream_id: String,
    pub title: String,
    pub summary: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_context: Vec<String>,
}

impl ToolStreamEnd {
    pub fn text(stream_id: impl Into<String>, output_text: impl Into<String>) -> Self {
        let output_text = output_text.into();
        Self {
            stream_id: stream_id.into(),
            title: String::new(),
            summary: agena_plugin_contracts::normalize_tool_summary(&output_text),
            output_text,
            payload: None,
            metadata: BTreeMap::new(),
            attachments: Vec::new(),
            additional_context: Vec::new(),
        }
    }

    pub fn from_output(stream_id: impl Into<String>, output: ToolInvokeOutput) -> Self {
        Self {
            stream_id: stream_id.into(),
            title: output.title,
            summary: output.summary,
            output_text: output.output_text,
            payload: output.payload,
            metadata: output.metadata,
            attachments: output.attachments,
            additional_context: output.additional_context,
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
            title: agena_plugin_contracts::normalize_tool_title(title.into()),
            summary: agena_plugin_contracts::normalize_tool_summary(summary.into()),
            output_text: output_text.into(),
            payload,
            metadata,
            attachments,
            additional_context: Vec::new(),
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
