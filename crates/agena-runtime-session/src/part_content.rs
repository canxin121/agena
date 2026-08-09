//! Typed content layer for v2 part payloads.
//!
//! The v2 `parts` table stores every chat entity as a row with a `kind`
//! column and a canonical JSON payload on `parts.content` (design 4.1.1).
//! The execution engine still operates on the v1 [`PartContent`] vocabulary
//! (that module is removed in R6), so this module is the typed contract
//! between the two: one struct per kind whose named fields are the canonical
//! keys (4.1.1) plus the extended keys (19.4), and a lossless `extra` bucket
//! (`#[serde(flatten)]`) that captures every key this crate does not name, so
//! a round-trip never drops data even when a producer writes richer payloads
//! than the canonical spec lists.
//!
//! Decoding is deliberately lenient ("reload 宁缺勿崩"): every named field is
//! `#[serde(default)]`, so a payload missing canonical keys decodes to its
//! defaults and never fails — the only failure is a non-object payload.
//!
//! The module deliberately does NOT depend on `agena-runtime-contracts` (R6
//! deletes it). It builds only on `serde_json`, `std`, `agena_domain`, and the
//! plugin SDK attachment types. The store adapter (`session/store.rs`) owns the
//! v1 ⇄ typed mapping in both directions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn is_false(value: &bool) -> bool {
    !*value
}

/// Lenient object decoder shared by every struct's `TryFrom<&Value>`:
/// a non-object is an error; anything else decodes with missing named keys
/// defaulted and unknown keys captured in `extra`.
fn decode_object<T>(kind: &str, value: &Value) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    if !value.is_object() {
        return Err(format!("{kind} content must be a JSON object, got {}", value));
    }
    serde_json::from_value(value.clone()).map_err(|error| format!("decode {kind} content: {error}"))
}

// ---------------------------------------------------------------------------
// Per-kind canonical shapes (4.1.1) + extended keys (19.4)
// ---------------------------------------------------------------------------

/// `run` — turn/run marker. Extended keys: `provider_id`, `model_id`,
/// `turn_id`, `reply_id` (written by [`crate::session::store::run_marker_content`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct RunContent {
    #[serde(default)]
    pub(crate) run_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) abort_reason: Option<String>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl RunContent {
    pub(crate) const fn kind() -> &'static str {
        "run"
    }

    #[cfg(test)]
    pub(crate) fn as_value(&self) -> Value {
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
pub(crate) struct TextContent {
    #[serde(default)]
    pub(crate) text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) synthetic: bool,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl TextContent {
    pub(crate) const fn kind() -> &'static str {
        "text"
    }

    pub(crate) fn as_value(&self) -> Value {
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
pub(crate) struct ThinkContent {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) summary: Vec<String>,
    #[serde(default, rename = "raw", skip_serializing_if = "Vec::is_empty")]
    pub(crate) raw: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) encrypted_content: Option<String>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl ThinkContent {
    pub(crate) const fn kind() -> &'static str {
        "think"
    }

    pub(crate) fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("think content is always JSON serializable")
    }
}

impl TryFrom<&Value> for ThinkContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `tool_call` — tool invocation. Named keys are the v1
/// [`agena_domain::ToolInvocation`] identity; the full v1
/// [`OperationPart`] payload rides in `extra["operation"]` so a reload can
/// rebuild the rich operation (call id, result envelope, details, lifecycle,
/// authorization) losslessly. Extended keys also include `tool_api_call`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct ToolCallContent {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) plugin: Option<String>,
    #[serde(default)]
    pub(crate) input: Value,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl ToolCallContent {
    pub(crate) const fn kind() -> &'static str {
        "tool_call"
    }

    pub(crate) fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("tool call content is always JSON serializable")
    }
}

impl TryFrom<&Value> for ToolCallContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `tool_result` — result of a tool call (child of the `tool_call` part via
/// `parent_part_id`). Extended keys preserve the full v1
/// [`OperationPart`] result envelope: `structured`, `model_preview`,
/// `managed_outputs`, `display`, `attachments`, `metadata`, `raw`, `error`,
/// `state` (19.4). The engine today keeps results inside the `tool_call`
/// operation, so the store serializes `tool_call`; this shape is the future
/// split part and is decoded for completeness.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct ToolResultContent {
    #[serde(default)]
    pub(crate) output: String,
    #[serde(default)]
    pub(crate) ok: bool,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl ToolResultContent {
    pub(crate) const fn kind() -> &'static str {
        "tool_result"
    }

    #[cfg(test)]
    pub(crate) fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("tool result content is always JSON serializable")
    }
}

impl TryFrom<&Value> for ToolResultContent {
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
pub(crate) struct FileRefContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sha: Option<String>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl FileRefContent {
    pub(crate) const fn kind() -> &'static str {
        "file_ref"
    }

    pub(crate) fn as_value(&self) -> Value {
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
pub(crate) struct PasteRefContent {
    #[serde(default)]
    pub(crate) text: String,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl PasteRefContent {
    pub(crate) const fn kind() -> &'static str {
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
pub(crate) struct SkillRefContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) args: Option<Value>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl SkillRefContent {
    pub(crate) const fn kind() -> &'static str {
        "skill_ref"
    }

    pub(crate) fn as_value(&self) -> Value {
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
pub(crate) struct NoticeContent {
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl NoticeContent {
    pub(crate) const fn kind() -> &'static str {
        "notice"
    }

    pub(crate) fn as_value(&self) -> Value {
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct HookContent {
    #[serde(default)]
    pub(crate) hook: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) plugin_id: Option<String>,
    #[serde(default)]
    pub(crate) summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl HookContent {
    pub(crate) const fn kind() -> &'static str {
        "hook"
    }

    pub(crate) fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("hook content is always JSON serializable")
    }
}

impl TryFrom<&Value> for HookContent {
    type Error = String;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        decode_object(Self::kind(), value)
    }
}

/// `compaction` — compaction summary with the compacted window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct CompactionContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) window: Option<Vec<Value>>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl CompactionContent {
    pub(crate) const fn kind() -> &'static str {
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
pub(crate) struct ErrorContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) category: Option<String>,
    #[serde(default)]
    pub(crate) message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<Value>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl ErrorContent {
    pub(crate) const fn kind() -> &'static str {
        "error"
    }

    pub(crate) fn as_value(&self) -> Value {
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
/// wire) is one of `ask_user` / `plan_review` / `permission`. Extended keys
/// carry the full v1 [`RequestPart::UserInput`] payload: `request` and `reply`
/// as complete [`UserInputRequest`]/[`UserInputReply`] objects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct InteractionContent {
    #[serde(rename = "type", default)]
    pub(crate) kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) options: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) response: Option<Value>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl InteractionContent {
    pub(crate) const fn kind() -> &'static str {
        "interaction"
    }

    pub(crate) fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("interaction content is always JSON serializable")
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
pub(crate) enum TypedContent {
    Run(RunContent),
    Text(TextContent),
    Think(ThinkContent),
    ToolCall(ToolCallContent),
    ToolResult(ToolResultContent),
    FileRef(FileRefContent),
    PasteRef(PasteRefContent),
    SkillRef(SkillRefContent),
    Notice(NoticeContent),
    Hook(HookContent),
    Compaction(CompactionContent),
    Error(ErrorContent),
    Interaction(InteractionContent),
}

/// Decode a part's canonical JSON payload into its typed shape, dispatching on
/// the part's `kind` column (4.1.1). Unknown kinds are an error; every known
/// kind decodes leniently (missing keys default, unknown keys land in `extra`).
pub(crate) fn decode(kind: &str, value: &Value) -> Result<TypedContent, String> {
    Ok(match kind {
        "run" => TypedContent::Run(RunContent::try_from(value)?),
        "text" => TypedContent::Text(TextContent::try_from(value)?),
        "think" => TypedContent::Think(ThinkContent::try_from(value)?),
        "tool_call" => TypedContent::ToolCall(ToolCallContent::try_from(value)?),
        "tool_result" => TypedContent::ToolResult(ToolResultContent::try_from(value)?),
        "file_ref" => TypedContent::FileRef(FileRefContent::try_from(value)?),
        "paste_ref" => TypedContent::PasteRef(PasteRefContent::try_from(value)?),
        "skill_ref" => TypedContent::SkillRef(SkillRefContent::try_from(value)?),
        "notice" => TypedContent::Notice(NoticeContent::try_from(value)?),
        "hook" => TypedContent::Hook(HookContent::try_from(value)?),
        "compaction" => TypedContent::Compaction(CompactionContent::try_from(value)?),
        "error" => TypedContent::Error(ErrorContent::try_from(value)?),
        "interaction" => TypedContent::Interaction(InteractionContent::try_from(value)?),
        other => return Err(format!("unknown part kind: {other}")),
    })
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
    fn tool_call_round_trips_input_object_and_unknown_keys() {
        let content = ToolCallContent {
            name: "fs.read".to_owned(),
            plugin: Some("builtin".to_owned()),
            input: json!({"file_path": "/tmp/x.txt", "offset": 3}),
            extra: BTreeMap::from([
                (
                    "tool_api_call".to_owned(),
                    json!({"function": "fs_read", "arguments": {"file_path": "/tmp/x.txt"}}),
                ),
                ("unknown_ext".to_owned(), json!({"nested": true})),
            ]),
        };
        let value = content.as_value();
        assert_eq!(value["name"], json!("fs.read"));
        assert_eq!(value["input"]["file_path"], json!("/tmp/x.txt"));
        assert_eq!(value["tool_api_call"]["function"], json!("fs_read"));
        let back = ToolCallContent::try_from(&value).unwrap();
        assert_eq!(back.name, "fs.read");
        assert_eq!(back.plugin.as_deref(), Some("builtin"));
        assert_eq!(back.input, json!({"file_path": "/tmp/x.txt", "offset": 3}));
        assert_eq!(back.extra["tool_api_call"]["function"], json!("fs_read"));
        assert_eq!(back.extra["unknown_ext"]["nested"], json!(true));
        // Sparse tool_call (no plugin, no input) decodes leniently.
        let sparse = ToolCallContent::try_from(&json!({"name": "ping"})).unwrap();
        assert_eq!(sparse.name, "ping");
        assert_eq!(sparse.plugin, None);
        assert_eq!(sparse.input, Value::Null);
    }

    #[test]
    fn tool_result_round_trips_extended_envelope_keys() {
        let content = ToolResultContent {
            output: "3 lines".to_owned(),
            ok: true,
            extra: BTreeMap::from([
                ("structured".to_owned(), json!({"lines": 3})),
                (
                    "model_preview".to_owned(),
                    json!({"text": "3 lines", "truncated": false}),
                ),
                ("state".to_owned(), json!("completed")),
                ("metadata".to_owned(), json!({"provider": "cli"})),
            ]),
        };
        let value = content.as_value();
        assert_eq!(value["output"], json!("3 lines"));
        assert_eq!(value["ok"], json!(true));
        let back = ToolResultContent::try_from(&value).unwrap();
        assert_eq!(back.output, "3 lines");
        assert!(back.ok);
        assert_eq!(back.extra["structured"], json!({"lines": 3}));
        assert_eq!(back.extra["state"], json!("completed"));
        assert_eq!(back.extra["metadata"]["provider"], json!("cli"));
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
            kind: "ask_user".to_owned(),
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
        assert_eq!(back.kind, "ask_user");
        assert_eq!(back.prompt.as_deref(), Some("Pick an option"));
        assert_eq!(back.extra["request"]["session_id"], json!(7));
        assert_eq!(back.extra["reply"]["kind"], json!("submit"));
        assert_eq!(back.response.as_ref().unwrap()["answers"]["q1"], json!(["A"]));
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
    fn decode_dispatches_by_kind_and_rejects_unknown() {
        assert!(matches!(decode("text", &json!({"text": "x"})), Ok(TypedContent::Text(_))));
        assert!(matches!(
            decode("interaction", &json!({"type": "permission"})),
            Ok(TypedContent::Interaction(_))
        ));
        assert!(decode("bogus", &json!({})).is_err());
        // Non-object payloads are the only hard decode failure.
        assert!(decode("text", &json!("not an object")).is_err());
        // A known kind with a completely empty object still decodes (all defaults).
        assert!(matches!(decode("compaction", &json!({})), Ok(TypedContent::Compaction(_))));
    }
}
