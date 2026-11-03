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

use crate::NoticePart;
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
/// into the next model prompt as a system message.
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

/// Decode the canonical human-facing notice shape from a kernel notice body.
///
/// Older sessions stored a serialized [`NoticePart`] directly in the notice
/// text. Requiring the exact, closed shape (and non-empty identity fields)
/// keeps arbitrary JSON or ordinary text from being mistaken for a structured
/// part.
pub fn decode_notice_part(text: &str) -> Option<NoticePart> {
    let notice: NoticePart = serde_json::from_str(text).ok()?;
    (!notice.kind.trim().is_empty() && !notice.summary.trim().is_empty()).then_some(notice)
}

/// One durable kernel notice classified by its canonical read-time shape.
///
/// Markers are recognized before legacy and ordinary text so every consumer
/// makes the same safe decision about malformed structured payloads. The
/// `Err` variants still identify the marker: a malformed hook, failure, or
/// system-notification marker must never be downgraded to user-visible text.
#[derive(Debug)]
pub enum DecodedNotice<'a> {
    Hook(serde_json::Result<HookContent>),
    LegacyHook(HookContent),
    LegacyStopHookFollowup(&'a str),
    SystemNotification(serde_json::Result<SystemNotificationContent>),
    Failure(serde_json::Result<agena_failure::Failure>),
    StructuredNotice(NoticePart),
    Plain(&'a str),
}

/// Classify a kernel notice body. This is the single ordering contract shared
/// by transcript projection, provider input, permission context, notification
/// aggregation, and delivery materialization.
pub fn decode_notice(text: &str) -> DecodedNotice<'_> {
    if let Some(decoded) = decode_hook_notice(text) {
        return DecodedNotice::Hook(decoded);
    }
    if let Some(hook) = decode_legacy_hook_observation(text) {
        return DecodedNotice::LegacyHook(hook);
    }
    if let Some(message) = legacy_stop_hook_followup_message(text) {
        return DecodedNotice::LegacyStopHookFollowup(message);
    }
    if let Some(decoded) = decode_system_notification_notice(text) {
        return DecodedNotice::SystemNotification(decoded);
    }
    if let Some(decoded) = decode_failure_notice(text) {
        return DecodedNotice::Failure(decoded);
    }
    if let Some(notice) = decode_notice_part(text) {
        return DecodedNotice::StructuredNotice(notice);
    }
    DecodedNotice::Plain(text)
}

impl DecodedNotice<'_> {
    /// Text that may enter provider or permission-classifier input. Structured
    /// user-facing notices, failures, and malformed markers are intentionally
    /// omitted; hook diagnostics contribute only their stop continuation.
    pub fn model_text(&self) -> Option<&str> {
        match self {
            Self::Hook(Ok(hook)) => hook
                .message
                .as_deref()
                .filter(|message| !message.trim().is_empty()),
            Self::LegacyHook(hook) => hook
                .message
                .as_deref()
                .filter(|message| !message.trim().is_empty()),
            Self::LegacyStopHookFollowup(message) => {
                (!message.trim().is_empty()).then_some(*message)
            }
            Self::SystemNotification(Ok(notification)) => {
                if !notification.body.trim().is_empty() {
                    Some(notification.body.as_str())
                } else if !notification.summary.trim().is_empty() {
                    Some(notification.summary.as_str())
                } else {
                    None
                }
            }
            Self::Plain(text) if !text.trim().is_empty() => Some(*text),
            Self::Hook(Err(_))
            | Self::SystemNotification(Err(_))
            | Self::Failure(_)
            | Self::StructuredNotice(_)
            | Self::Plain(_) => None,
        }
    }
}

/// Marker prefix for a synthetic notice that carries a background-operation
/// [`SystemNotificationContent`]. The kernel notice body is still a string, so
/// the typed content rides inside it and is projected back at read time.
pub const SYSTEM_NOTIFICATION_NOTICE_PREFIX: &str = "\u{1f}agena.system_notification:";

pub fn system_notification_notice_text(
    notification: &SystemNotificationContent,
) -> serde_json::Result<String> {
    Ok(format!(
        "{SYSTEM_NOTIFICATION_NOTICE_PREFIX}{}",
        serde_json::to_string(notification)?
    ))
}

/// Decode a system-notification marker. `None` means this is an ordinary
/// notice; `Some(Err(_))` means the marker was present but malformed and must
/// not be exposed to a model or rendered as user text.
pub fn decode_system_notification_notice(
    text: &str,
) -> Option<serde_json::Result<SystemNotificationContent>> {
    let payload = text.strip_prefix(SYSTEM_NOTIFICATION_NOTICE_PREFIX)?;
    Some(serde_json::from_str(payload))
}

/// Marker prefix for a synthetic notice that carries one observed plugin hook
/// run. Hook activity is a first-class transcript part in the read contract,
/// but the kernel notice payload is still a plain string, so the structured
/// [`HookContent`] rides inside the text and is projected back at read time.
/// Consumers that build model input must use only `HookContent::message` and
/// must never send the marker itself.
pub const HOOK_NOTICE_PREFIX: &str = "\u{1f}agena.hook:";

pub fn hook_notice_text(hook: &HookContent) -> serde_json::Result<String> {
    Ok(format!(
        "{HOOK_NOTICE_PREFIX}{}",
        serde_json::to_string(hook)?
    ))
}

/// Decode a hook marker notice body. `None` means this is an ordinary notice;
/// `Some(Err(_))` means the marker was present but its payload was malformed
/// and must not be exposed to a model or rendered as user text.
pub fn decode_hook_notice(text: &str) -> Option<serde_json::Result<HookContent>> {
    let payload = text.strip_prefix(HOOK_NOTICE_PREFIX)?;
    Some(serde_json::from_str(payload))
}

/// Prefix used by the pre-kernel hook recorder. These notices predate the
/// structured hook marker, so they are decoded only while reading history;
/// no new write may use this format.
pub const LEGACY_HOOK_OBSERVATIONS_PREFIX: &str = "Plugin hook observations:\n";

/// Prefix of the duplicate continuation notice written by the pre-kernel stop
/// hook path immediately after [`LEGACY_HOOK_OBSERVATIONS_PREFIX`]. The same
/// continuation is already present in the observation detail.
pub const LEGACY_STOP_HOOK_FOLLOWUP_PREFIX: &str = "Plugin stop hook follow-up:\n";

/// Decode one hook observation written before hook runs became structured
/// parts. The old payload is a human-readable line rather than JSON:
///
/// `agena.plan agent.stop Applied: <summary> (<continuation>)`
///
/// The old writer could batch several runs into one notice. The first header
/// identifies the activity; any following text stays in `detail`, so the
/// original record is not lost. The stop-hook path emitted one run per notice.
pub fn decode_legacy_hook_observation(text: &str) -> Option<HookContent> {
    let payload = text.strip_prefix(LEGACY_HOOK_OBSERVATIONS_PREFIX)?;
    let payload = payload.strip_suffix('\n').unwrap_or(payload);
    let (plugin_id, rest) = payload.split_once(' ')?;
    let (hook, rest) = rest.split_once(' ')?;
    let (status, body) = rest.split_once(": ")?;
    if !matches!(status, "Applied" | "Skipped" | "Failed" | "TimedOut") {
        return None;
    }
    let (summary, detail) = match body.rsplit_once(" (") {
        Some((summary, detail)) if detail.ends_with(')') => (
            summary.trim_end().to_owned(),
            Some(detail[..detail.len() - 1].to_owned()),
        ),
        _ => (body.trim_end().to_owned(), None),
    };
    if summary.is_empty() {
        return None;
    }
    let message = detail
        .as_ref()
        .filter(|_| hook == "agent.stop" && summary.contains("blocked stop"))
        .cloned();
    Some(HookContent {
        hook: hook.to_owned(),
        plugin_id: Some(plugin_id.to_owned()),
        summary,
        detail,
        message,
        extra: [(
            "status".to_owned(),
            Value::String(status.to_ascii_lowercase()),
        )]
        .into_iter()
        .collect(),
    })
}

/// The pre-kernel stop-hook path also wrote the continuation as a separate
/// plain-text notice. Pair-aware readers suppress it when it directly follows
/// an observation; standalone readers can still render it as hook activity.
pub fn legacy_stop_hook_followup_message(text: &str) -> Option<&str> {
    let message = text.strip_prefix(LEGACY_STOP_HOOK_FOLLOWUP_PREFIX)?;
    let message = message.strip_suffix('\n').unwrap_or(message);
    (!message.trim().is_empty()).then_some(message)
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

#[cfg(test)]
mod notice_marker_tests {
    use super::*;

    #[test]
    fn hook_notice_round_trips_structured_activity() {
        let hook = HookContent {
            hook: "agent.stop".into(),
            plugin_id: Some("agena.plan".into()),
            summary: "agent.stop hook blocked stop".into(),
            detail: Some("workflow plan autorun".into()),
            message: Some("Continue the active approved plan.".into()),
            ..Default::default()
        };
        let text = hook_notice_text(&hook).unwrap();
        assert!(text.starts_with(HOOK_NOTICE_PREFIX));
        let decoded = decode_hook_notice(&text).unwrap().unwrap();
        assert_eq!(decoded, hook);
    }

    #[test]
    fn malformed_hook_payload_is_never_mistaken_for_plain_text() {
        let text = format!("{HOOK_NOTICE_PREFIX}{{");
        assert!(decode_hook_notice(&text).is_some_and(|result| result.is_err()));
        assert!(decode_hook_notice("ordinary notice").is_none());
    }

    #[test]
    fn system_notification_marker_round_trips_as_structured_activity() {
        let notification = SystemNotificationContent {
            operation_id: "proc_1".into(),
            operation_kind: "shell".into(),
            status: "completed".into(),
            summary: "Command finished".into(),
            body: "exit code 0".into(),
            event_seq: Some(7),
            ..Default::default()
        };
        let text = system_notification_notice_text(&notification).unwrap();
        assert!(text.starts_with(SYSTEM_NOTIFICATION_NOTICE_PREFIX));
        let decoded = decode_system_notification_notice(&text).unwrap().unwrap();
        assert_eq!(decoded, notification);
        assert!(matches!(
            decode_notice(&text),
            DecodedNotice::SystemNotification(Ok(_))
        ));
    }

    #[test]
    fn malformed_system_notification_is_classified_but_not_exposed() {
        let text = format!("{SYSTEM_NOTIFICATION_NOTICE_PREFIX}{{");
        assert!(decode_system_notification_notice(&text).is_some_and(|result| result.is_err()));
        let decoded = decode_notice(&text);
        assert!(matches!(decoded, DecodedNotice::SystemNotification(Err(_))));
        assert!(decoded.model_text().is_none());
    }

    #[test]
    fn notice_part_decoding_requires_the_closed_non_empty_shape() {
        let text = br#"{"kind":"max_turns_exhausted","summary":"Turn limit reached"}"#;
        let notice = decode_notice_part(std::str::from_utf8(text).unwrap()).unwrap();
        assert_eq!(notice.kind, "max_turns_exhausted");

        assert!(decode_notice_part("ordinary notice").is_none());
        assert!(decode_notice_part(r#"{"kind":"","summary":"missing kind"}"#).is_none());
        assert!(decode_notice_part(r#"{"kind":"notice","summary":""}"#).is_none());
        assert!(
            decode_notice_part(
                r#"{"kind":"notice","summary":"summary","unexpected":"not a NoticePart"}"#,
            )
            .is_none()
        );
    }

    #[test]
    fn legacy_hook_observation_decodes_as_hook_activity() {
        let text = concat!(
            "Plugin hook observations:\n",
            "agena.plan agent.stop Applied: agent.stop hook blocked stop: workflow plan autorun ",
            "(<plan_context>\nContinue the active approved plan.\n</plan_context>)\n",
        );
        let hook = decode_legacy_hook_observation(text).unwrap();
        assert_eq!(hook.hook, "agent.stop");
        assert_eq!(hook.plugin_id.as_deref(), Some("agena.plan"));
        assert_eq!(
            hook.summary,
            "agent.stop hook blocked stop: workflow plan autorun"
        );
        assert_eq!(
            hook.detail.as_deref(),
            Some("<plan_context>\nContinue the active approved plan.\n</plan_context>")
        );
        assert_eq!(hook.message, hook.detail);
        assert_eq!(hook.extra["status"], "applied");
    }

    #[test]
    fn legacy_followup_is_recognized_separately() {
        let text = "Plugin stop hook follow-up:\n<plan_context>\nContinue\n</plan_context>";
        assert_eq!(
            legacy_stop_hook_followup_message(text),
            Some("<plan_context>\nContinue\n</plan_context>")
        );
        assert!(decode_legacy_hook_observation(text).is_none());
    }
}
