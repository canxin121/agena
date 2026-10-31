//! Typed payloads used by current transcript projections and local activities.
//!
//! Durable session facts belong to the session kernel. User resources and
//! commands arrive in a ComposerDocument, while tool results use ToolCallContent.
//! This module does not decode or upgrade the retired Part-table representation.
//! ToolCallContent rejects unknown fields and contains one optional RawOutput;
//! display projections and provider feedback remain separate responsibilities.

use std::collections::BTreeMap;

use agena_failure::UserProblem;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::part::OperationPart;
use agena_domain::{
    OperationAuthorization, OperationError, OperationUserInput, RawOutput, StructuredObject,
    TimeRange, ToolApiCall, ToolInvocation, ToolResultState,
};

fn is_false(value: &bool) -> bool {
    !*value
}

/// Object decoder shared by every struct's `TryFrom<&Value>`. The target
/// struct controls strictness: open kinds flatten unknown keys into `extra`,
/// while `ToolCallContent` rejects them.
fn decode_object<T>(kind: &str, value: &Value) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    if !value.is_object() {
        return Err(format!(
            "{kind} content must be a JSON object, got {}",
            value
        ));
    }
    serde_json::from_value(value.clone()).map_err(|error| format!("decode {kind} content: {error}"))
}

// ---------------------------------------------------------------------------
// Transcript and activity payloads
// ---------------------------------------------------------------------------

/// `text` — plain text. `synthetic` marks internally produced text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TextContent {
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<agena_domain::ContentRef>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub synthetic: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl TextContent {
    pub const fn kind() -> &'static str {
        "text"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("text content is always JSON serializable")
    }
}

impl TryFrom<&Value> for TextContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `think` — reasoning. `summary` and `raw` hold the visible and raw reasoning
/// fragments; `encrypted_content` preserves provider-specific encrypted data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ThinkContent {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<agena_domain::ContentRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary: Vec<String>,
    #[serde(default, rename = "raw", skip_serializing_if = "Vec::is_empty")]
    pub raw: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ThinkContent {
    pub const fn kind() -> &'static str {
        "think"
    }

    pub fn as_value(&self) -> Value {
        let mut value =
            serde_json::to_value(self).expect("think content is always JSON serializable");
        if !self.summary.is_empty()
            && !self.raw.is_empty()
            && self.summary.concat() == self.raw.concat()
        {
            value.as_object_mut().unwrap().remove("raw");
        }
        value
    }
}

impl TryFrom<&Value> for ThinkContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `tool_call` — tool invocation with one canonical raw output.
///
/// The durable record stores the invocation identity (`name`/`plugin`/
/// `input`, plus the optional provider envelope `tool_api_call`) and one
/// [`RawOutput`] fact envelope; every presentation (model-visible text,
/// human-facing blocks, API projection) is derived from these facts at read
/// time. Nothing is duplicated: there is no `operation` bucket, no result
/// envelope, no stored blocks or model preview. The canonical identity,
/// lifecycle, and state fields are required when decoding; missing fields are
/// invalid rather than an implicit alternate shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolCallContent {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_api_call: Option<ToolApiCall>,
    pub call_id: i64,
    pub state: ToolResultState,
    #[serde(default, skip_serializing_if = "OperationAuthorization::is_empty")]
    pub authorization: OperationAuthorization,
    #[serde(default, skip_serializing_if = "OperationUserInput::is_empty")]
    pub user_input: OperationUserInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<RawOutput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<agena_domain::ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationError>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
    pub lifecycle: TimeRange,
}

impl ToolCallContent {
    pub const fn kind() -> &'static str {
        "tool_call"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("tool call content is always JSON serializable")
    }
}

impl TryFrom<&Value> for ToolCallContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `notice` — system notice (hook runs etc.). `title` is the 19.4 extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NoticeContent {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl NoticeContent {
    pub const fn kind() -> &'static str {
        "notice"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("notice content is always JSON serializable")
    }
}

impl TryFrom<&Value> for NoticeContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `hook` — one observed plugin hook run. `plugin_id` is the 19.4 extension.
///
/// `message` is the hook-sent continuation (for example the workflow plan
/// autorun's `agent.stop` continuation). It is carried by the hook activity
/// itself — never injected as a separate assistant message — and is projected
/// back into the model prompt as assistant text on the next run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HookContent {
    #[serde(default)]
    pub hook: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The message the hook sent to keep the run going, when it blocked the
    /// stop. Persisted on the hook part so the activity carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl HookContent {
    pub const fn kind() -> &'static str {
        "hook"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("hook content is always JSON serializable")
    }
}

impl TryFrom<&Value> for HookContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `system_notification` — a background-operation completion (or event)
/// notification delivered to the model. AI-launched work uses an
/// Assistant-role part appended to the launching run (no new run); launch-less
/// scheduled delivery uses a Runtime ingress. This is the agena analog of
/// Claude Code's `<task-notification>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SystemNotificationContent {
    /// The background operation id (task id or process id).
    #[serde(default)]
    pub operation_id: String,
    /// "task" | "shell" | "workflow" | "monitor" | "scheduled_delivery".
    #[serde(default)]
    pub operation_kind: String,
    /// The launching tool_call's provider operation id (`agena.operation_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    /// "ready" | "event" | "completed" | "failed" | "cancelled" | "timed_out".
    #[serde(default)]
    pub status: String,
    /// One-line summary, e.g. `Task "explore" finished`.
    #[serde(default)]
    pub summary: String,
    /// Optional structured detail (failure reason, exit code, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The body the model sees — mirrors Claude's
    /// `<note>…<result>…</result>…</note>` shape.
    #[serde(default)]
    pub body: String,
    /// Monotonic notification delivery sequence (independent of shell raw-log cursors).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_seq: Option<u64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl SystemNotificationContent {
    pub const fn kind() -> &'static str {
        "system_notification"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("system notification content is always JSON serializable")
    }
}

impl TryFrom<&Value> for SystemNotificationContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `compaction` — compaction summary with the compacted window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CompactionContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<UserProblem>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl CompactionContent {
    pub const fn kind() -> &'static str {
        "compaction"
    }

    /// Human-facing text. The checkpoint owns its summary; never serialize a
    /// second copy or expose opaque native provider items as display text.
    pub fn summary_text(&self) -> Option<&str> {
        self.summary.as_deref().or_else(|| {
            self.checkpoint
                .as_ref()?
                .pointer("/content/summary")?
                .as_str()
        })
    }
}

impl TryFrom<&Value> for CompactionContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `error` — durable failure record. The full [`agena_failure::UserProblem`]
/// (id/code/responsibility/retry/recovery/impact/user) rides losslessly under
/// `extra["problem"]`; `category`/`message` are the canonical headline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ErrorContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<agena_failure::RetryProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ErrorContent {
    pub const fn kind() -> &'static str {
        "error"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("error content is always JSON serializable")
    }
}

/// Marker prefix for a synthetic notice that carries an execution-level
/// failure which has no failed provider round (for example a manager-level
/// configuration stop). Notices are plain strings in the kernel, so the
/// structured failure rides inside the text and is projected back into a
/// canonical `error` part at read time.
pub const FAILURE_NOTICE_PREFIX: &str = "\u{1f}agena.failure:";

pub fn failure_notice_text(failure: &agena_failure::Failure) -> serde_json::Result<String> {
    Ok(format!(
        "{FAILURE_NOTICE_PREFIX}{}",
        serde_json::to_string(failure)?
    ))
}

/// Decode a marker notice body. `None` means this is an ordinary notice that
/// must keep rendering as user-visible text.
pub fn decode_failure_notice(text: &str) -> Option<serde_json::Result<agena_failure::Failure>> {
    let payload = text.strip_prefix(FAILURE_NOTICE_PREFIX)?;
    Some(serde_json::from_str(payload))
}

impl TryFrom<&Value> for ErrorContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

// ---------------------------------------------------------------------------
// Read-time domain projections
// ---------------------------------------------------------------------------

/// Rebuild an [`OperationPart`] view from the canonical single-source
/// `tool_call` shape. The operation is a pure projection of the flat content:
/// invocation identity is reassembled and every presentation field (blocks,
/// model text) is derived by the consumer, never persisted.
pub fn operation_from_tool_call(part: &ToolCallContent) -> OperationPart {
    OperationPart {
        call_id: part.call_id,
        tool_api_call: part.tool_api_call.clone(),
        invocation: ToolInvocation {
            name: part.name.clone(),
            plugin_name: part.plugin.clone(),
            input: StructuredObject::try_from(part.input.clone()).unwrap_or_default(),
        },
        authorization: part.authorization.clone(),
        user_input: part.user_input.clone(),
        output: part.output.clone(),
        resources: part.resources.clone(),
        state: part.state,
        error: part.error.clone(),
        metadata: part.metadata.clone(),
        lifecycle: part.lifecycle.clone(),
    }
}
