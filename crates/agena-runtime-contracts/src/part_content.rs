//! Typed content layer for v2 part payloads.
//!
//! The v2 `parts` table stores every chat entity as a row with a `kind`
//! column and a canonical JSON payload on `parts.content` (design 4.1.1).
//! This module is the single content model: one struct per kind whose named
//! fields are the canonical keys (4.1.1) plus the extended keys (19.4). Most
//! non-tool kinds retain an open `extra` bucket. `tool_call` is deliberately
//! strict: it rejects unknown/removed result and presentation fields and has
//! exactly one optional [`RawOutput`] result.
//!
//! Non-tool kinds preserve their existing lenient decoding. Tool calls use
//! `#[serde(deny_unknown_fields)]`; incompatible development rows fail fast
//! instead of being migrated or silently interpreted as the current shape.
//!
//! The module builds only on `serde_json`, `std`, `agena_domain`,
//! `agena_failure`, and the shared contracts part types (which re-export the
//! plugin SDK attachment types). It never depends on `agena-storage`; the
//! mapping between a storage [`Part`] row and these payloads lives in the
//! consuming crate (provider/session), which owns the storage-facing
//! `PartRole`/`PartState` conversions.

use std::collections::BTreeMap;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use agena_domain::{
    OperationAuthorization, OperationError, OperationUserInput, RawOutput, StructuredObject,
    TimeRange, ToolApiCall, ToolInvocation, ToolResultState, UserInputReply, UserInputRequest,
};
use agena_failure::{
    FailureCategory, FailureCode, FailureId, FailureImpact, FailureResponsibility,
    RecoveryDirective, RetryDirective, UserPresentation, UserProblem,
};

use crate::part::{
    AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource, InteractiveRequestPart,
    NoticePart, OperationPart, RequestPart, SkillReference, SkillReferencePart,
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
// Per-kind canonical shapes (4.1.1) + extended keys (19.4)
// ---------------------------------------------------------------------------

/// `run` — turn/run marker. Extended keys: `provider_id`, `model_id`,
/// `turn_id`, `reply_id` (written by [`crate::session::store::run_marker_content`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RunContent {
    #[serde(default)]
    pub run_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abort_reason: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl RunContent {
    pub const fn kind() -> &'static str {
        "run"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("run content is always JSON serializable")
    }
}

impl TryFrom<&Value> for RunContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `text` — plain text. `synthetic` marks internally produced text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TextContent {
    #[serde(default)]
    pub text: String,
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

/// `think` — reasoning. v1 `ReasoningPart` maps `raw_content` onto the
/// canonical `raw` key; `encrypted_content` is the v1 encrypted-reasoning key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ThinkContent {
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
        serde_json::to_value(self).expect("think content is always JSON serializable")
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
/// invalid rather than an implicit legacy shape.
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

/// `file_ref` — reference to a file (no blob stored). Extended keys preserve
/// the full v1 [`AttachmentItem`]/[`AttachmentSource`] breadth: `url`,
/// `data_url`, `base64`, `file_id`, `kind`, `source`, `title`, `size_bytes`,
/// `width`, `height`, `duration_ms`, `page_count` (19.4). When the source part
/// carried multiple attachments the lossless full array rides in
/// `extra["attachments"]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FileRefContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl FileRefContent {
    pub const fn kind() -> &'static str {
        "file_ref"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("file ref content is always JSON serializable")
    }
}

impl TryFrom<&Value> for FileRefContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `paste_ref` — pasted text stored inline (full content, no blob cache).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PasteRefContent {
    #[serde(default)]
    pub text: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl PasteRefContent {
    pub const fn kind() -> &'static str {
        "paste_ref"
    }
}

impl TryFrom<&Value> for PasteRefContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `skill_ref` — skill name/args reference only (19.4 D12). During the
/// transition the engine still writes the v1 [`SkillReferencePart`] snapshot
/// under `extra["skills"]` (name/description/instructions/content_hash/
/// source/aliases) so reload can rebuild it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SkillRefContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl SkillRefContent {
    pub const fn kind() -> &'static str {
        "skill_ref"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("skill ref content is always JSON serializable")
    }
}

impl TryFrom<&Value> for SkillRefContent {
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
    /// "event" | "completed" | "failed" | "cancelled" | "timed_out".
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
    /// Monotonic per-monitor event sequence (see `agena.monitor_event_seq`).
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
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl CompactionContent {
    pub const fn kind() -> &'static str {
        "compaction"
    }
}

impl TryFrom<&Value> for CompactionContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `error` — durable failure record. The full v1 [`agena_failure::UserProblem`]
/// (id/code/responsibility/retry/recovery/impact/user, 19.4) rides losslessly
/// under `extra["problem"]`; `category`/`message` are the canonical headline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ErrorContent {
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

impl TryFrom<&Value> for ErrorContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `interaction` — user processing point. `kind` (renamed to `type` on the
/// wire; `kind` accepted as a legacy v1-flat alias) discriminates the display
/// style (`ask_user` / `review` / custom). Extended keys carry the full v1
/// [`RequestPart::UserInput`] payload: `request` and `reply` as complete
/// [`UserInputRequest`]/[`UserInputReply`] objects, plus the correlation keys
/// `request_id` / `tool_part_id` / `operation_id` mirrored by the writer.
///
/// The [`#[serde(flatten)]`] `extra` bucket means both the canonical shape
/// (`type`/`prompt`/`options`/`request`/…) and the legacy v1-flat shape
/// (`kind`/`request_id`/`prompt`/`tool_part_id`/`request`/`response`) decode
/// into the SAME struct — the v1 keys land in `extra` / the aliased `kind`.
/// The typed accessors below read either shape, so consumers never touch raw
/// JSON keys.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct InteractionContent {
    #[serde(rename = "type", alias = "kind", default)]
    pub kind: agena_domain::UserInputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl InteractionContent {
    pub const fn kind() -> &'static str {
        "interaction"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("interaction content is always JSON serializable")
    }

    /// The lossless typed request from `extra["request"]`, if present. Both
    /// shapes store the full [`UserInputRequest`] under the top-level `request`
    /// key (canonical flattens it out of `extra`), so this decodes both.
    pub fn request(&self) -> Option<UserInputRequest> {
        self.extra
            .get("request")
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }

    /// The typed reply, if answered: `extra["reply"]` (canonical) or the
    /// top-level `response` key (v1-flat).
    pub fn reply(&self) -> Option<UserInputReply> {
        self.extra
            .get("reply")
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .or_else(|| {
                self.response
                    .as_ref()
                    .and_then(|value| serde_json::from_value(value.clone()).ok())
            })
    }

    /// The correlation id: `extra["request_id"]` (both shapes) with a fallback
    /// to the nested request's id for legacy rows that only carry it there.
    pub fn request_id(&self) -> Option<String> {
        self.extra
            .get("request_id")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| self.request().map(|request| request.request_id))
    }

    /// The owning tool part id, when the request is bound to a tool.
    pub fn tool_part_id(&self) -> Option<i64> {
        self.extra
            .get("tool_part_id")
            .and_then(serde_json::Value::as_i64)
    }

    /// The owning operation id, when the request is bound to a tool operation.
    pub fn operation_id(&self) -> Option<&str> {
        self.extra
            .get("operation_id")
            .and_then(serde_json::Value::as_str)
    }

    /// The typed origin of this interaction: `Host` for the runtime's own
    /// `ask_user` (previously correlated by the `host-input:` request-id
    /// prefix), `Plugin` for third-party/tool asks.
    ///
    /// New rows store the typed `source` on the nested request. Legacy rows
    /// predate the field and deserialize to `Plugin`, so their real origin is
    /// recovered from the correlation ids: host parts always wrote
    /// `request_id = host-input:...` ≠ `operation_id`, while plugin parts
    /// wrote `request_id == operation_id`.
    pub fn source(&self) -> agena_domain::UserInputSource {
        if let Some(request) = self.request() {
            let stored = self
                .extra
                .get("request")
                .and_then(serde_json::Value::as_object)
                .map(|object| object.contains_key("source"))
                .unwrap_or(false);
            if stored {
                return request.source;
            }
        }
        // Legacy inference from the correlation ids.
        if self.request_id().as_deref() != self.operation_id() {
            agena_domain::UserInputSource::Host
        } else {
            agena_domain::UserInputSource::Plugin
        }
    }
}

impl TryFrom<&Value> for InteractionContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

// ---------------------------------------------------------------------------
// Kind dispatch
// ---------------------------------------------------------------------------

/// A typed view over one part's canonical content payload, dispatched by the
/// part's `kind` column.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedContent {
    Run(RunContent),
    Text(TextContent),
    Think(ThinkContent),
    ToolCall(ToolCallContent),
    FileRef(FileRefContent),
    PasteRef(PasteRefContent),
    SkillRef(SkillRefContent),
    Notice(NoticeContent),
    Hook(HookContent),
    SystemNotification(SystemNotificationContent),
    Compaction(CompactionContent),
    Error(ErrorContent),
    Interaction(InteractionContent),
}

/// Decode a part's canonical JSON payload into its typed shape, dispatching on
/// the part's `kind` column (4.1.1). Unknown kinds are an error; every known
/// kind uses its declared serde contract. In particular, `tool_call` rejects
/// unknown fields and the removed standalone `tool_result` kind is unknown.
pub fn decode(kind: &str, value: &Value) -> Result<TypedContent, String> {
    Ok(match kind {
        "run" => TypedContent::Run(RunContent::try_from(value)?),
        "text" => TypedContent::Text(TextContent::try_from(value)?),
        "think" => TypedContent::Think(ThinkContent::try_from(value)?),
        "tool_call" => TypedContent::ToolCall(ToolCallContent::try_from(value)?),
        "file_ref" => TypedContent::FileRef(FileRefContent::try_from(value)?),
        "paste_ref" => TypedContent::PasteRef(PasteRefContent::try_from(value)?),
        "skill_ref" => TypedContent::SkillRef(SkillRefContent::try_from(value)?),
        "notice" => TypedContent::Notice(NoticeContent::try_from(value)?),
        "hook" => TypedContent::Hook(HookContent::try_from(value)?),
        "system_notification" => {
            TypedContent::SystemNotification(SystemNotificationContent::try_from(value)?)
        }
        "compaction" => TypedContent::Compaction(CompactionContent::try_from(value)?),
        "error" => TypedContent::Error(ErrorContent::try_from(value)?),
        "interaction" => TypedContent::Interaction(InteractionContent::try_from(value)?),
        other => return Err(format!("unknown part kind: {other}")),
    })
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
        invocation: ToolInvocation {
            tool_api_call: part.tool_api_call.clone(),
            name: part.name.clone(),
            plugin_name: part.plugin.clone(),
            input: StructuredObject::try_from(part.input.clone()).unwrap_or_default(),
        },
        authorization: part.authorization.clone(),
        user_input: part.user_input.clone(),
        output: part.output.clone(),
        state: part.state,
        error: part.error.clone(),
        metadata: part.metadata.clone(),
        lifecycle: part.lifecycle.clone(),
    }
}

/// Project an operation onto the exact durable `tool_call` shape. This is the
/// inverse of [`operation_from_tool_call`]; neither direction creates or
/// accepts presentation fields.
pub fn tool_call_from_operation(operation: &OperationPart) -> ToolCallContent {
    ToolCallContent {
        name: operation.invocation.name.clone(),
        plugin: operation.invocation.plugin_name.clone(),
        input: Value::from(operation.invocation.input.clone()),
        tool_api_call: operation.invocation.tool_api_call.clone(),
        call_id: operation.call_id,
        state: operation.state,
        authorization: operation.authorization.clone(),
        user_input: operation.user_input.clone(),
        output: operation.output.clone(),
        error: operation.error.clone(),
        metadata: operation.metadata.clone(),
        lifecycle: operation.lifecycle.clone(),
    }
}

/// Rebuild a v1 [`AttachmentPart`] from the canonical `file_ref` shape,
/// preferring the lossless `extra["attachments"]` list and otherwise
/// reconstructing a single item from the named keys + extended keys.
pub fn attachment_from_file_ref(part: &FileRefContent) -> AttachmentPart {
    if let Some(items) = part
        .extra
        .get("attachments")
        .and_then(|value| serde_json::from_value::<Vec<AttachmentItem>>(value.clone()).ok())
    {
        return AttachmentPart { attachments: items };
    }
    let kind = part
        .extra
        .get("kind")
        .and_then(Value::as_str)
        .and_then(|value| {
            serde_json::from_value::<AttachmentKind>(Value::String(value.to_owned())).ok()
        })
        .unwrap_or(AttachmentKind::File);
    let source = attachment_source_from_file_ref(part);
    AttachmentPart {
        attachments: vec![AttachmentItem {
            kind,
            mime: part.mime.clone().unwrap_or_default(),
            source,
            filename: part.name.clone(),
            title: part
                .extra
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_owned),
            size_bytes: part.extra.get("size_bytes").and_then(Value::as_u64),
            sha256: part.sha.clone(),
            width: part
                .extra
                .get("width")
                .and_then(Value::as_u64)
                .map(|v| v as u32),
            height: part
                .extra
                .get("height")
                .and_then(Value::as_u64)
                .map(|v| v as u32),
            duration_ms: part.extra.get("duration_ms").and_then(Value::as_u64),
            page_count: part
                .extra
                .get("page_count")
                .and_then(Value::as_u64)
                .map(|v| v as u32),
        }],
    }
}

/// Rebuild an [`AttachmentSource`] from the canonical `file_ref` named keys
/// and extended keys (`url`, `data_url`, `base64`, `file_id`, `path`).
pub fn attachment_source_from_file_ref(part: &FileRefContent) -> AttachmentSource {
    let extra = &part.extra;
    if let Some(url) = extra.get("url").and_then(Value::as_str) {
        return AttachmentSource::Url {
            url: url.to_owned(),
        };
    }
    if let Some(url) = extra.get("data_url").and_then(Value::as_str) {
        return AttachmentSource::DataUrl {
            url: url.to_owned(),
        };
    }
    if let Some(data) = extra.get("base64").and_then(Value::as_str) {
        return AttachmentSource::Base64 {
            data: data.to_owned(),
        };
    }
    if let Some(file_id) = extra.get("file_id").and_then(Value::as_str) {
        return AttachmentSource::FileId {
            file_id: file_id.to_owned(),
        };
    }
    if let Some(path) = part.path.as_deref() {
        return AttachmentSource::LocalPath {
            path: path.to_owned(),
        };
    }
    if let Some(source) = extra
        .get("source")
        .and_then(|value| serde_json::from_value::<AttachmentSource>(value.clone()).ok())
    {
        return source;
    }
    AttachmentSource::LocalPath {
        path: String::new(),
    }
}

/// Rebuild a v1 [`SkillReferencePart`] from `extra["skills"]` (empty when the
/// snapshot is missing or does not match the v1 shape).
pub fn skill_reference_from_skill_ref(part: &SkillRefContent) -> SkillReferencePart {
    let skills = part
        .extra
        .get("skills")
        .and_then(|value| serde_json::from_value::<Vec<SkillReference>>(value.clone()).ok())
        .unwrap_or_default();
    SkillReferencePart { skills }
}

/// Rebuild a v1 [`agena_failure::UserProblem`] from the canonical `error`
/// shape, preferring the lossless `extra["problem"]` object and otherwise
/// constructing a minimal problem from the named keys.
pub fn user_problem_from_error(part: &ErrorContent) -> UserProblem {
    if let Some(problem) = part
        .extra
        .get("problem")
        .and_then(|value| serde_json::from_value::<UserProblem>(value.clone()).ok())
    {
        return problem;
    }
    UserProblem {
        id: FailureId::new(),
        code: FailureCode::new("runtime.error"),
        category: part
            .category
            .as_deref()
            .and_then(|value| {
                serde_json::from_value::<FailureCategory>(Value::String(value.to_owned())).ok()
            })
            .unwrap_or(FailureCategory::Internal),
        responsibility: FailureResponsibility::System,
        retry: RetryDirective::Never,
        recovery: RecoveryDirective::None,
        impact: FailureImpact::OperationFailed,
        user: UserPresentation {
            key: part
                .category
                .clone()
                .unwrap_or_else(|| "runtime-error".to_owned()),
            fallback: part.message.clone(),
            detail_key: None,
        },
    }
}

/// Rebuild a v1 [`RequestPart`] from the canonical `interaction` shape,
/// preferring the lossless `extra["request"]`/`extra["reply"]` objects and
/// otherwise reconstructing from the named display keys.
pub fn interaction_from_content(part: &InteractionContent) -> RequestPart {
    let extra = &part.extra;
    let request = extra
        .get("request")
        .and_then(|value| serde_json::from_value::<UserInputRequest>(value.clone()).ok())
        .unwrap_or_else(|| UserInputRequest {
            request_id: format!("restored-{}", part.kind.as_str()),
            session_id: None,
            title: part.prompt.clone().unwrap_or_default(),
            body_markdown: String::new(),
            kind: part.kind.clone(),
            source: Default::default(),
            auto_resolution_ms: None,
            presented_at: None,
            questions: part
                .options
                .as_ref()
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .unwrap_or_default(),
            created_at: Utc::now(),
        });
    let reply = extra
        .get("reply")
        .and_then(|value| serde_json::from_value::<UserInputReply>(value.clone()).ok())
        .or_else(|| {
            part.response
                .as_ref()
                .and_then(|value| serde_json::from_value::<UserInputReply>(value.clone()).ok())
        });
    RequestPart::UserInput(InteractiveRequestPart { request, reply })
}

/// Rebuild a v1 [`NoticePart`] from the canonical `notice` shape.
pub fn notice_part_from_notice_content(part: &NoticeContent) -> NoticePart {
    NoticePart {
        kind: part.kind.clone(),
        summary: part.summary.clone(),
        detail: part.detail.clone(),
        title: part.title.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_round_trips_and_preserves_unknown_keys() {
        let content = TextContent {
            text: "hello".to_owned(),
            synthetic: true,
            extra: BTreeMap::from([("marker".to_owned(), json!("x"))]),
        };
        let value = content.as_value();
        assert_eq!(value["text"], json!("hello"));
        assert_eq!(value["synthetic"], json!(true));
        assert_eq!(value["marker"], json!("x"));
        let back = TextContent::try_from(&value).unwrap();
        assert_eq!(back.text, "hello");
        assert!(back.synthetic);
        assert_eq!(back.extra["marker"], json!("x"));
        // Missing canonical keys default; unknown keys are preserved.
        let sparse = TextContent::try_from(&json!({"text": "hi", "custom": 1})).unwrap();
        assert!(!sparse.synthetic);
        assert_eq!(sparse.extra["custom"], json!(1));
    }

    #[test]
    fn run_round_trips_with_marker_extras() {
        let content = RunContent {
            run_kind: "user_send".to_owned(),
            abort_reason: None,
            extra: BTreeMap::from([
                ("provider_id".to_owned(), json!("anthropic")),
                ("model_id".to_owned(), json!("claude-3-5-sonnet")),
            ]),
        };
        let value = content.as_value();
        assert_eq!(value["run_kind"], json!("user_send"));
        assert_eq!(value["provider_id"], json!("anthropic"));
        let back = RunContent::try_from(&value).unwrap();
        assert_eq!(back.run_kind, "user_send");
        assert_eq!(back.extra["model_id"], json!("claude-3-5-sonnet"));
    }

    #[test]
    fn think_round_trips_summary_raw_and_encrypted() {
        let content = ThinkContent {
            summary: vec!["step 1".to_owned(), "step 2".to_owned()],
            raw: vec!["raw reasoning".to_owned()],
            encrypted_content: Some("opaque".to_owned()),
            extra: BTreeMap::from([("budget_ratio".to_owned(), json!(0.42))]),
        };
        let value = content.as_value();
        assert_eq!(value["raw"], json!(["raw reasoning"]));
        assert_eq!(value["summary"], json!(["step 1", "step 2"]));
        assert_eq!(value["encrypted_content"], json!("opaque"));
        let back = ThinkContent::try_from(&value).unwrap();
        assert_eq!(back.summary, content.summary);
        assert_eq!(back.raw, content.raw);
        assert_eq!(back.encrypted_content, content.encrypted_content);
        assert_eq!(back.extra["budget_ratio"], json!(0.42));
    }

    #[test]
    fn tool_call_round_trips_only_single_source_fields_and_rejects_removed_keys() {
        let content = ToolCallContent {
            name: "fs.read".to_owned(),
            plugin: Some("builtin".to_owned()),
            input: json!({"file_path": "/tmp/x.txt", "offset": 3}),
            tool_api_call: Some(agena_domain::ToolApiCall {
                function: agena_domain::ToolApiFunction::Call,
                arguments: agena_domain::StructuredObject::default(),
            }),
            call_id: 7,
            state: ToolResultState::Completed,
            authorization: OperationAuthorization::default(),
            user_input: OperationUserInput::default(),
            output: Some(RawOutput {
                payload: Some(json!({"preview": "hello", "truncated": false})),
                ..Default::default()
            }),
            error: None,
            metadata: BTreeMap::from([("agena.operation_id".to_owned(), json!("op-1"))]),
            lifecycle: TimeRange {
                start_ms: 1000,
                end_ms: Some(2000),
            },
        };
        let value = content.as_value();
        assert_eq!(value["name"], json!("fs.read"));
        assert_eq!(value["input"]["file_path"], json!("/tmp/x.txt"));
        assert_eq!(value["output"]["payload"]["preview"], json!("hello"));
        // No operation bucket and no stored blocks/model preview.
        assert!(value.get("operation").is_none());
        assert!(value.get("result").is_none());
        let back = ToolCallContent::try_from(&value).unwrap();
        assert_eq!(back, content);
        assert!(
            ToolCallContent::try_from(&json!({
                "name": "fs.read",
                "input": {},
                "payload": {"preview": "legacy"}
            }))
            .is_err()
        );
        assert!(
            ToolCallContent::try_from(&json!({
                "name": "fs.read",
                "input": {},
                "unknown_ext": true
            }))
            .is_err()
        );
        for required in ["name", "input", "call_id", "state", "lifecycle"] {
            let mut incomplete = value.clone();
            incomplete
                .as_object_mut()
                .expect("tool_call serializes as an object")
                .remove(required);
            assert!(
                ToolCallContent::try_from(&incomplete).is_err(),
                "missing required field {required} must be rejected"
            );
        }
    }

    #[test]
    fn file_ref_round_trips_path_and_media_extras() {
        let content = FileRefContent {
            path: Some("/tmp/img.png".to_owned()),
            name: Some("img.png".to_owned()),
            mime: Some("image/png".to_owned()),
            sha: Some("abc123".to_owned()),
            extra: BTreeMap::from([
                ("kind".to_owned(), json!("image")),
                ("width".to_owned(), json!(800)),
                ("height".to_owned(), json!(600)),
                ("duration_ms".to_owned(), json!(0)),
                ("page_count".to_owned(), json!(1)),
            ]),
        };
        let value = content.as_value();
        assert_eq!(value["path"], json!("/tmp/img.png"));
        assert_eq!(value["width"], json!(800));
        let back = FileRefContent::try_from(&value).unwrap();
        assert_eq!(back.path.as_deref(), Some("/tmp/img.png"));
        assert_eq!(back.name.as_deref(), Some("img.png"));
        assert_eq!(back.mime.as_deref(), Some("image/png"));
        assert_eq!(back.sha.as_deref(), Some("abc123"));
        assert_eq!(back.extra["kind"], json!("image"));
        assert_eq!(back.extra["height"], json!(600));
        assert_eq!(back.extra["page_count"], json!(1));
    }

    #[test]
    fn interaction_round_trips_ask_user_with_full_request_reply() {
        let content = InteractionContent {
            kind: "ask_user".into(),
            prompt: Some("Pick an option".to_owned()),
            options: Some(json!([{"question": "Which?", "options": [{"label": "A"}]}])),
            response: Some(json!({"request_id": "r1", "kind": "submit", "answers": {"q1": ["A"]}})),
            extra: BTreeMap::from([
                (
                    "request".to_owned(),
                    json!({
                        "request_id": "r1",
                        "session_id": 7,
                        "title": "Pick an option",
                        "kind": "ask_user",
                        "questions": [{"question": "Which?", "options": [{"label": "A"}]}],
                        "created_at": "2026-01-01T00:00:00Z",
                    }),
                ),
                (
                    "reply".to_owned(),
                    json!({"request_id": "r1", "kind": "submit", "answers": {"q1": ["A"]}}),
                ),
            ]),
        };
        let value = content.as_value();
        assert_eq!(value["type"], json!("ask_user"));
        assert_eq!(value["prompt"], json!("Pick an option"));
        let back = InteractionContent::try_from(&value).unwrap();
        assert_eq!(back.kind, "ask_user".into());
        assert_eq!(back.prompt.as_deref(), Some("Pick an option"));
        assert_eq!(back.extra["request"]["session_id"], json!(7));
        assert_eq!(back.extra["reply"]["kind"], json!("submit"));
        assert_eq!(
            back.response.as_ref().unwrap()["answers"]["q1"],
            json!(["A"])
        );
    }

    /// One struct, two shapes: the legacy v1-flat payload (`kind` alias, flat
    /// `request_id`/`tool_part_id`/`request`/`response` keys) and the canonical
    /// payload (`type`, display keys, full `request`/`reply` objects, mirrored
    /// correlation keys including `operation_id`) both decode into
    /// `InteractionContent`, and the typed accessors return equivalent values.
    #[test]
    fn interaction_content_decodes_v1_flat_and_canonical_shapes_identically() {
        let request = json!({
            "request_id": "r1",
            "session_id": 7,
            "title": "Approve?",
            "kind": "review",
            "questions": [{"question": "Approve?", "options": [{"label": "Approve"}]}],
            "created_at": "2026-01-01T00:00:00Z",
        });
        let reply = json!({
            "request_id": "r1",
            "kind": "submit",
            "answers": {"0": ["Approve"]},
        });
        let v1_flat = json!({
            "kind": "review",
            "request_id": "r1",
            "prompt": "Approve?",
            "tool_part_id": 5,
            "request": request,
            "response": reply,
        });
        let canonical = json!({
            "type": "review",
            "prompt": "Approve?",
            "options": [{"question": "Approve?", "options": [{"label": "Approve"}]}],
            "response": reply,
            "request": request,
            "reply": reply,
            "request_id": "r1",
            "tool_part_id": 5,
            "operation_id": "op-1",
        });
        let flat = InteractionContent::try_from(&v1_flat).unwrap();
        let canon = InteractionContent::try_from(&canonical).unwrap();

        // The kind coerces to the typed enum from both shapes.
        assert_eq!(flat.kind, agena_domain::UserInputKind::Review);
        assert_eq!(canon.kind, agena_domain::UserInputKind::Review);
        assert_eq!(flat.kind, canon.kind);

        // Typed accessors agree across shapes.
        assert_eq!(flat.request_id(), Some("r1".to_owned()));
        assert_eq!(canon.request_id(), flat.request_id());
        assert_eq!(flat.tool_part_id(), Some(5));
        assert_eq!(canon.tool_part_id(), flat.tool_part_id());
        // The v1-flat payload never carried an operation id; only the canonical
        // shape mirrors it.
        assert_eq!(flat.operation_id(), None);
        assert_eq!(canon.operation_id(), Some("op-1"));

        let flat_request = flat.request().expect("v1-flat request decodes");
        let canon_request = canon.request().expect("canonical request decodes");
        assert_eq!(flat_request, canon_request);
        assert_eq!(flat_request.request_id, "r1");
        assert_eq!(flat_request.title, "Approve?");
        assert_eq!(flat_request.kind, agena_domain::UserInputKind::Review);

        // reply() reads extra["reply"] (canonical) or the top-level response
        // (v1-flat) and yields the same typed reply.
        let flat_reply = flat.reply().expect("v1-flat reply decodes from response");
        let canon_reply = canon.reply().expect("canonical reply decodes from reply");
        assert_eq!(flat_reply, canon_reply);
        assert_eq!(flat_reply.request_id, "r1");
        assert_eq!(flat_reply.kind, agena_domain::UserInputReplyKind::Submit);
    }

    #[test]
    fn interaction_source_infers_legacy_origin_and_honors_typed_rows() {
        // Legacy host part: `request_id` (host-input:...) ≠ `operation_id`.
        let host = InteractionContent::try_from(&json!({
            "type": "ask_user",
            "request_id": "host-input:1:98:0",
            "operation_id": "op-7",
            "request": {"request_id": "host-input:1:98:0", "created_at": "2026-01-01T00:00:00Z"},
        }))
        .unwrap();
        assert_eq!(
            host.source(),
            agena_domain::UserInputSource::Host,
            "legacy host part infers Host from request_id != operation_id"
        );

        // Legacy plugin part: `request_id == operation_id`.
        let plugin = InteractionContent::try_from(&json!({
            "type": "ask_user",
            "request_id": "op-7",
            "operation_id": "op-7",
            "request": {"request_id": "op-7", "created_at": "2026-01-01T00:00:00Z"},
        }))
        .unwrap();
        assert_eq!(
            plugin.source(),
            agena_domain::UserInputSource::Plugin,
            "legacy plugin part infers Plugin from request_id == operation_id"
        );

        // Typed row: honors the stored source even when the correlation ids
        // would infer the opposite (a host input on a plugin-tool operation).
        let typed = InteractionContent::try_from(&json!({
            "type": "ask_user",
            "request_id": "op-7",
            "operation_id": "op-7",
            "request": {
                "request_id": "op-7",
                "source": "host",
                "created_at": "2026-01-01T00:00:00Z",
            },
        }))
        .unwrap();
        assert_eq!(
            typed.source(),
            agena_domain::UserInputSource::Host,
            "typed row honors the stored source over the id inference"
        );
    }

    #[test]
    fn error_round_trips_full_user_problem() {
        let content = ErrorContent {
            category: Some("internal".to_owned()),
            message: "boom".to_owned(),
            detail: None,
            extra: BTreeMap::from([(
                "problem".to_owned(),
                json!({
                    "id": "00000000-0000-0000-0000-000000000001",
                    "code": "runtime.internal",
                    "category": "internal",
                    "responsibility": "system",
                    "retry": "never",
                    "recovery": "none",
                    "impact": "operation_failed",
                    "user": {"key": "runtime-internal", "fallback": "boom"}
                }),
            )]),
        };
        let value = content.as_value();
        assert_eq!(value["category"], json!("internal"));
        assert_eq!(value["message"], json!("boom"));
        let back = ErrorContent::try_from(&value).unwrap();
        assert_eq!(back.category.as_deref(), Some("internal"));
        assert_eq!(back.message, "boom");
        assert_eq!(back.extra["problem"]["code"], json!("runtime.internal"));
        assert_eq!(back.extra["problem"]["user"]["fallback"], json!("boom"));
    }

    #[test]
    fn notice_part_rebuilt_from_canonical_shape() {
        let content = NoticeContent {
            kind: "hook.completed".to_owned(),
            summary: "Hook ran".to_owned(),
            detail: Some("details".to_owned()),
            title: Some("Title".to_owned()),
            extra: Default::default(),
        };
        let part = notice_part_from_notice_content(&content);
        assert_eq!(part.kind, "hook.completed");
        assert_eq!(part.summary, "Hook ran");
        assert_eq!(part.detail.as_deref(), Some("details"));
        assert_eq!(part.title.as_deref(), Some("Title"));
    }

    #[test]
    fn decode_dispatches_by_kind_and_rejects_unknown() {
        assert!(matches!(
            decode("text", &json!({"text": "x"})),
            Ok(TypedContent::Text(_))
        ));
        assert!(matches!(
            decode("interaction", &json!({"type": "permission"})),
            Ok(TypedContent::Interaction(_))
        ));
        assert!(decode("bogus", &json!({})).is_err());
        // Non-object payloads are the only hard decode failure.
        assert!(decode("text", &json!("not an object")).is_err());
        // A known kind with a completely empty object still decodes (all defaults).
        assert!(matches!(
            decode("compaction", &json!({})),
            Ok(TypedContent::Compaction(_))
        ));
    }
}
