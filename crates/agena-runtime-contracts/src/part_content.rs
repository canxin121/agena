//! Typed content layer for persisted part payloads.
//!
//! The `parts` table stores every chat entity as a row with a `kind`
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

use serde::{Deserialize, Serialize};
use serde_json::Value;

use agena_domain::{
    OperationAuthorization, OperationError, OperationUserInput, RawOutput, StructuredObject,
    TimeRange, ToolApiCall, ToolInvocation, ToolResultState,
};
use agena_failure::{
    FailureCategory, FailureCode, FailureId, FailureImpact, FailureResponsibility,
    RecoveryDirective, RetryDirective, UserPresentation, UserProblem,
};

use crate::part::{
    AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource, NoticePart, OperationPart,
    SkillReference, SkillReferencePart,
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

/// `think` — reasoning. `summary` and `raw` hold the visible and raw reasoning
/// fragments; `encrypted_content` preserves provider-specific encrypted data.
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
/// the complete [`AttachmentItem`]/[`AttachmentSource`] metadata: `url`,
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

/// `skill_ref` — skill name/args reference plus a lossless snapshot under
/// `extra["skills"]` (name/description/instructions/content_hash/source/
/// aliases) so reload and provider projection retain the complete reference.
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

/// `error` — durable failure record. The full [`agena_failure::UserProblem`]
/// (id/code/responsibility/retry/recovery/impact/user) rides losslessly under
/// `extra["problem"]`; `category`/`message` are the canonical headline.
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
    ToolCall(Box<ToolCallContent>),
    FileRef(FileRefContent),
    PasteRef(PasteRefContent),
    SkillRef(SkillRefContent),
    Notice(NoticeContent),
    Hook(HookContent),
    SystemNotification(SystemNotificationContent),
    Compaction(CompactionContent),
    Error(ErrorContent),
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
        "tool_call" => TypedContent::ToolCall(Box::new(ToolCallContent::try_from(value)?)),
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

/// Project the canonical `file_ref` shape into an [`AttachmentPart`],
/// preferring the lossless `extra["attachments"]` list and otherwise
/// reconstructing a single item from the named keys + extended keys.
pub fn attachment_from_file_ref(part: &FileRefContent) -> AttachmentPart {
    if let Some(value) = part.extra.get("attachments") {
        match serde_json::from_value::<Vec<AttachmentItem>>(value.clone()) {
            Ok(items) => return AttachmentPart { attachments: items },
            Err(error) => tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode a persisted attachment snapshot",
                    &error,
                ),
                "persisted attachment snapshot is malformed; rebuilding it from canonical fields"
            ),
        }
    }
    let kind = match part.extra.get("kind").and_then(Value::as_str) {
        Some(value) => {
            match serde_json::from_value::<AttachmentKind>(Value::String(value.to_owned())) {
                Ok(kind) => kind,
                Err(error) => {
                    tracing::warn!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "decode a persisted attachment kind",
                            &error,
                        ),
                        "persisted attachment kind is malformed; using file"
                    );
                    AttachmentKind::File
                }
            }
        }
        None => AttachmentKind::File,
    };
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
    if let Some(value) = extra.get("source") {
        match serde_json::from_value::<AttachmentSource>(value.clone()) {
            Ok(source) => return source,
            Err(error) => tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode persisted attachment source",
                    &error,
                ),
                "persisted attachment source is malformed; using the legacy fallback"
            ),
        }
    }
    tracing::warn!("file_ref content has no usable attachment source; using an empty local path");
    AttachmentSource::LocalPath {
        path: String::new(),
    }
}

/// Project the lossless skill snapshot from `extra["skills"]` into a
/// [`SkillReferencePart`]. A missing or malformed snapshot yields no skills.
pub fn skill_reference_from_skill_ref(part: &SkillRefContent) -> SkillReferencePart {
    let skills = match part.extra.get("skills") {
        Some(value) => match serde_json::from_value::<Vec<SkillReference>>(value.clone()) {
            Ok(skills) => skills,
            Err(error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "decode persisted skill reference snapshot",
                        &error,
                    ),
                    "persisted skill reference snapshot is malformed; projecting an empty snapshot"
                );
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    SkillReferencePart { skills }
}

/// Project the canonical `error` shape into an [`agena_failure::UserProblem`],
/// shape, preferring the lossless `extra["problem"]` object and otherwise
/// constructing a minimal problem from the named keys.
pub fn user_problem_from_error(part: &ErrorContent) -> UserProblem {
    if let Some(value) = part.extra.get("problem") {
        match serde_json::from_value::<UserProblem>(value.clone()) {
            Ok(problem) => return problem,
            Err(error) => tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode persisted user problem",
                    &error,
                ),
                "persisted user problem is malformed; rebuilding it from canonical error fields"
            ),
        }
    }
    let category = part
        .category
        .as_deref()
        .map(|value| {
            serde_json::from_value::<FailureCategory>(Value::String(value.to_owned())).map_err(
                |error| {
                    tracing::warn!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "decode persisted failure category",
                            &error,
                        ),
                        "persisted failure category is malformed; using internal"
                    );
                },
            )
        })
        .transpose()
        .ok()
        .flatten()
        .unwrap_or(FailureCategory::Internal);
    UserProblem {
        id: FailureId::new(),
        code: FailureCode::new("runtime.error"),
        category,
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

/// Project the canonical `notice` shape into a [`NoticePart`].
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
        // No nested operation bucket or persisted presentation fields.
        assert!(value.get("operation").is_none());
        assert!(value.get("result").is_none());
        let back = ToolCallContent::try_from(&value).unwrap();
        assert_eq!(back, content);
        assert!(
            ToolCallContent::try_from(&json!({
                "name": "fs.read",
                "input": {},
                "payload": {"preview": "unsupported"}
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
        for removed in ["blocks", "content_blocks", "model_output", "human"] {
            let mut removed_shape = value.clone();
            removed_shape
                .as_object_mut()
                .expect("tool_call serializes as an object")
                .insert(removed.to_owned(), json!([]));
            assert!(
                ToolCallContent::try_from(&removed_shape).is_err(),
                "removed tool_call field {removed} must be rejected"
            );
        }
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
