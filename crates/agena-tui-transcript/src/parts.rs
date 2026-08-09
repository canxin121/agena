//! v2 parts → transcript entry projection for terminal presentation.
//!
//! The wire transcript is an ordered v2 part list (`SessionExecutionResource
//! .parts`, database-design-v2.md §4.1.1). Each `run` marker is one turn/run:
//! it starts a [`TranscriptEntry`] whose `role`/`state`/`created_at` mirror the
//! marker, and every following content part (`run_id` backlinked) becomes a
//! part of that entry. `kind`/`role`/`state` are stable open-set strings, so
//! the mapping is defensive: well-known kinds project to the rich render-model
//! variants, and unknown or malformed parts fall back to a plain text part so
//! nothing in the transcript is silently dropped.
//!
//! Unlike [`super::transcript_entries`] (which borrows from a domain
//! `TranscriptSnapshot`), this projection owns every value: parts arrive as
//! JSON and are decoded into the render model here, so entries are `'static`.

use agena_api::{
    message_part::{
        MessageAttachmentPartResource, MessageErrorPartResource, MessageHookPartResource,
        MessageReasoningPartResource, MessageRequestPartResource,
        MessageSkillReferencePartResource, MessageTextPartResource, OperationPartResource,
        PartExecutionStatusResource, StructuredFieldResource, StructuredObjectResource,
        StructuredValueResource, ToolInvocationResource,
    },
    resource::{
        MessageAttachment, MessageAttachmentKind, MessageAttachmentSource, MessageRole,
        MessageSkillReference, MessageStatus, SessionTranscriptPart,
    },
};
use serde_json::Value;

use crate::{
    TranscriptActivityContent, TranscriptAssistantReplyLifecycle, TranscriptContentId,
    TranscriptEntry, TranscriptEntryId, TranscriptEntryPart, TranscriptPartContent,
};

/// Project an ordered v2 part list into transcript entries, one per `run`
/// marker. Content parts without an enclosing marker become their own entry.
pub fn parts_entries(parts: &[SessionTranscriptPart]) -> Vec<TranscriptEntry<'static>> {
    let mut entries = Vec::new();
    let mut current: Option<TranscriptEntry<'static>> = None;

    for part in parts {
        if part.kind == "run" {
            if let Some(entry) = current.take() {
                entries.push(finalize_run_entry(entry));
            }
            current = Some(run_marker_entry(part));
        } else if let Some(entry) = current.as_mut() {
            entry.parts.push(entry_part(part));
        } else {
            entries.push(orphan_entry(part));
        }
    }
    if let Some(entry) = current.take() {
        entries.push(finalize_run_entry(entry));
    }
    entries
}

/// Whether any run marker in the part list has a non-terminal state. The app
/// uses this as a safety net to force a refresh after a terminal execution.
pub fn parts_have_non_terminal_runs(parts: &[SessionTranscriptPart]) -> bool {
    parts
        .iter()
        .filter(|part| part.kind == "run")
        .any(|marker| !message_status_is_terminal(message_status_from_string(&marker.state)))
}

/// Count of user run markers that carry at least one text part. This is the
/// optimistic-entry boundary: a user input becomes visible exactly when its
/// run's text part appears in the projection.
pub fn parts_visible_user_inputs(parts: &[SessionTranscriptPart]) -> usize {
    let mut count = 0usize;
    let mut current_role: Option<&str> = None;
    let mut current_has_text = false;
    for part in parts {
        if part.kind == "run" {
            if current_role == Some("user") && current_has_text {
                count += 1;
            }
            current_role = Some(part.role.as_str());
            current_has_text = false;
        } else if part.kind == "text" {
            current_has_text = true;
        }
    }
    if current_role == Some("user") && current_has_text {
        count += 1;
    }
    count
}

/// The last assistant reply's text from the parts projection: the text parts
/// of the final assistant `run` marker (design §4.1.1 marker == turn), joined
/// with newlines. Returns `None` when no assistant run carries text.
pub fn last_assistant_reply_text(parts: &[SessionTranscriptPart]) -> Option<String> {
    let mut last: Option<(i64, String)> = None;
    let mut current_run: Option<(i64, &str)> = None;
    let mut current_text = String::new();
    for part in parts {
        if part.kind == "run" {
            // The previous run just ended: keep its text only when it is an
            // assistant run, replacing any earlier assistant reply.
            if let Some((run_id, role)) = current_run.take() {
                if role == "assistant" {
                    last = Some((run_id, std::mem::take(&mut current_text)));
                } else {
                    current_text.clear();
                }
            }
            current_run = Some((part.part_id, part.role.as_str()));
        } else if let Some((run_id, _)) = current_run
            && part.run_id == Some(run_id)
            && part.kind == "text"
            && let Some(text) = string_field(&part.content, "text")
        {
            if !current_text.is_empty() {
                current_text.push('\n');
            }
            current_text.push_str(&text);
        }
    }
    // Flush the trailing run.
    if let Some((run_id, role)) = current_run
        && role == "assistant"
    {
        last = Some((run_id, current_text));
    }
    let (_, text) = last?;
    if text.trim().is_empty() {
        return None;
    }
    Some(text)
}

fn run_marker_entry(marker: &SessionTranscriptPart) -> TranscriptEntry<'static> {
    TranscriptEntry {
        id: TranscriptEntryId::StoredMessage(marker.part_id),
        role: role_from_string(&marker.role),
        state: message_status_from_string(&marker.state),
        created_at: timestamp(marker.created_at_ms),
        parts: Vec::new(),
    }
}

fn finalize_run_entry(mut entry: TranscriptEntry<'static>) -> TranscriptEntry<'static> {
    // Mirror the v1 reply projection: an assistant run renders a trailing
    // lifecycle row when it has no content yet (the active envelope) or when
    // it ended without a durable Error part carrying the outcome (failed,
    // cancelled, denied). A completed run with body content needs no row.
    if entry.role == Some(MessageRole::Assistant) {
        let state = entry.state;
        let has_error_part = entry.parts.iter().any(|part| {
            matches!(
                part.content,
                TranscriptPartContent::Activity(TranscriptActivityContent::Error(_))
            )
        });
        let requires_outcome = assistant_reply_state_requires_outcome(state);
        if entry.parts.is_empty() || (requires_outcome && !has_error_part) {
            let marker_id = match entry.id {
                TranscriptEntryId::StoredMessage(id) => id,
                _ => 0,
            };
            entry.parts.push(TranscriptEntryPart {
                id: TranscriptContentId::StoredPart(marker_id),
                status: part_status_from_state(state),
                content: TranscriptPartContent::Activity(
                    TranscriptActivityContent::AssistantReplyLifecycle(assistant_reply_lifecycle(
                        state, None,
                    )),
                ),
            });
        }
    }
    entry
}

fn orphan_entry(part: &SessionTranscriptPart) -> TranscriptEntry<'static> {
    TranscriptEntry {
        id: TranscriptEntryId::StoredMessage(part.part_id),
        role: role_from_string(&part.role),
        state: message_status_from_string(&part.state),
        created_at: timestamp(part.created_at_ms),
        parts: vec![entry_part(part)],
    }
}

fn entry_part(part: &SessionTranscriptPart) -> TranscriptEntryPart<'static> {
    TranscriptEntryPart {
        id: TranscriptContentId::StoredPart(part.part_id),
        status: part_status_from_string(&part.state),
        content: part_content(part),
    }
}

fn part_content(part: &SessionTranscriptPart) -> TranscriptPartContent<'static> {
    let content = &part.content;
    match part.kind.as_str() {
        "text" => TranscriptPartContent::Text(MessageTextPartResource {
            text: string_field(content, "text").unwrap_or_default(),
            synthetic: bool_field(content, "synthetic").unwrap_or(false),
        }),
        "think" => TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(
            MessageReasoningPartResource {
                summary: string_array_field(content, "summary"),
                raw_content: string_array_field(content, "raw"),
                encrypted_content: string_field(content, "encrypted_content"),
            },
        )),
        "tool_call" => {
            let input = match content.get("input") {
                Some(Value::Object(map)) => StructuredObjectResource {
                    fields: map
                        .iter()
                        .map(|(name, value)| StructuredFieldResource {
                            name: name.clone(),
                            value: structured_value(value),
                        })
                        .collect(),
                },
                _ => StructuredObjectResource::default(),
            };
            TranscriptPartContent::Activity(TranscriptActivityContent::Operation(Box::new(
                OperationPartResource {
                    call_id: integer_field(content, "call_id").unwrap_or(part.part_id),
                    invocation: ToolInvocationResource {
                        gateway_function: None,
                        name: string_field(content, "name").unwrap_or_default(),
                        plugin_name: string_field(content, "plugin"),
                        input,
                    },
                    title: string_field(content, "title").unwrap_or_default(),
                    summary: string_field(content, "summary").unwrap_or_default(),
                    ..Default::default()
                },
            )))
        }
        "tool_result" => TranscriptPartContent::Text(MessageTextPartResource {
            text: string_field(content, "output")
                .or_else(|| string_field(content, "text"))
                .unwrap_or_default(),
            synthetic: false,
        }),
        "file_ref" => TranscriptPartContent::Activity(TranscriptActivityContent::Attachment(
            MessageAttachmentPartResource {
                attachments: vec![MessageAttachment {
                    kind: MessageAttachmentKind::File,
                    mime: string_field(content, "mime").unwrap_or_default(),
                    source: string_field(content, "path")
                        .map(|path| MessageAttachmentSource::LocalPath { path })
                        .unwrap_or_else(|| MessageAttachmentSource::Url { url: String::new() }),
                    filename: string_field(content, "name"),
                    title: None,
                    size_bytes: None,
                    sha256: string_field(content, "sha"),
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: None,
                }],
            },
        )),
        "paste_ref" => TranscriptPartContent::Text(MessageTextPartResource {
            text: string_field(content, "text").unwrap_or_default(),
            synthetic: true,
        }),
        "skill_ref" => TranscriptPartContent::Activity(TranscriptActivityContent::SkillReference(
            MessageSkillReferencePartResource {
                skills: vec![MessageSkillReference {
                    name: string_field(content, "skill").unwrap_or_default(),
                    description: string_field(content, "description").unwrap_or_default(),
                    instructions: string_field(content, "instructions").unwrap_or_default(),
                    content_hash: string_field(content, "content_hash").unwrap_or_default(),
                    source: string_field(content, "source").unwrap_or_default(),
                    aliases: Vec::new(),
                }],
            },
        )),
        "notice" | "hook" => TranscriptPartContent::Activity(TranscriptActivityContent::Hook(
            Box::new(MessageHookPartResource {
                hook: string_field(content, "hook")
                    .or_else(|| string_field(content, "kind"))
                    .unwrap_or_else(|| part.kind.clone()),
                plugin_id: string_field(content, "plugin_id"),
                summary: string_field(content, "summary").unwrap_or_default(),
                detail: string_field(content, "detail"),
            }),
        )),
        "compaction" => TranscriptPartContent::Activity(TranscriptActivityContent::Hook(Box::new(
            MessageHookPartResource {
                hook: "compaction".to_owned(),
                plugin_id: None,
                summary: string_field(content, "summary").unwrap_or_default(),
                detail: content.get("window").map(|window| window.to_string()),
            },
        ))),
        "error" => match serde_json::from_value::<MessageErrorPartResource>(part.content.clone()) {
            Ok(error) => TranscriptPartContent::Activity(TranscriptActivityContent::Error(error)),
            Err(_) => TranscriptPartContent::Text(MessageTextPartResource {
                text: string_field(content, "message")
                    .or_else(|| string_field(content, "summary"))
                    .unwrap_or_else(|| fallback_json_text(content)),
                synthetic: false,
            }),
        },
        "interaction" => {
            match serde_json::from_value::<MessageRequestPartResource>(part.content.clone()) {
                Ok(request) => TranscriptPartContent::Activity(TranscriptActivityContent::Request(
                    Box::new(request),
                )),
                Err(_) => TranscriptPartContent::Text(MessageTextPartResource {
                    text: fallback_json_text(content),
                    synthetic: false,
                }),
            }
        }
        _ => TranscriptPartContent::Text(MessageTextPartResource {
            text: fallback_json_text(content),
            synthetic: false,
        }),
    }
}

fn structured_value(value: &Value) -> StructuredValueResource {
    match value {
        Value::Null => StructuredValueResource::Null,
        Value::Bool(value) => StructuredValueResource::Boolean { value: *value },
        Value::Number(value) => value
            .as_i64()
            .map(|value| StructuredValueResource::Integer { value })
            .unwrap_or_else(|| StructuredValueResource::Number {
                value: value.to_string(),
            }),
        Value::String(value) => StructuredValueResource::Text {
            value: value.clone(),
        },
        Value::Array(items) => StructuredValueResource::Array {
            items: items.iter().map(structured_value).collect(),
        },
        Value::Object(map) => StructuredValueResource::Object {
            fields: map
                .iter()
                .map(|(name, value)| StructuredFieldResource {
                    name: name.clone(),
                    value: structured_value(value),
                })
                .collect(),
        },
    }
}

/// A readable fallback for parts whose kind we do not render specially: use a
/// short summary field when present, otherwise compact JSON.
fn fallback_json_text(content: &Value) -> String {
    for key in ["text", "summary", "output", "message"] {
        if let Some(text) = string_field(content, key) {
            return text;
        }
    }
    content.to_string()
}

fn string_field(content: &Value, key: &str) -> Option<String> {
    content.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn string_array_field(content: &Value, key: &str) -> Vec<String> {
    content
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn bool_field(content: &Value, key: &str) -> Option<bool> {
    content.get(key).and_then(Value::as_bool)
}

fn integer_field(content: &Value, key: &str) -> Option<i64> {
    content.get(key).and_then(Value::as_i64)
}

fn timestamp(created_at_ms: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(created_at_ms).unwrap_or_default()
}

/// Parse the wire role string. Roles beyond the render model's set (for
/// example `runtime`) project to no role: such entries render as activity-like
/// rows without a message header rather than as a user/assistant message.
pub(crate) fn role_from_string(role: &str) -> Option<MessageRole> {
    match role {
        "user" => Some(MessageRole::User),
        "assistant" => Some(MessageRole::Assistant),
        "tool" => Some(MessageRole::Tool),
        "system" => Some(MessageRole::System),
        _ => None,
    }
}

/// Parse the wire part/run state string into the render model's message state.
pub(crate) fn message_status_from_string(state: &str) -> MessageStatus {
    match state {
        "pending" => MessageStatus::Pending,
        "in_progress" | "running" => MessageStatus::InProgress,
        "completed" => MessageStatus::Completed,
        "policy_denied" => MessageStatus::PolicyDenied,
        "user_declined" => MessageStatus::UserDeclined,
        "capability_unavailable" => MessageStatus::CapabilityUnavailable,
        "tool_unavailable" => MessageStatus::ToolUnavailable,
        "failed" => MessageStatus::Failed,
        "cancelled" | "canceled" => MessageStatus::Cancelled,
        _ => MessageStatus::Pending,
    }
}

/// Parse the wire part state string into the render model's part state.
pub(crate) fn part_status_from_string(state: &str) -> PartExecutionStatusResource {
    match message_status_from_string(state) {
        MessageStatus::Pending => PartExecutionStatusResource::Pending,
        MessageStatus::InProgress => PartExecutionStatusResource::InProgress,
        MessageStatus::Completed => PartExecutionStatusResource::Completed,
        MessageStatus::PolicyDenied => PartExecutionStatusResource::PolicyDenied,
        MessageStatus::UserDeclined => PartExecutionStatusResource::UserDeclined,
        MessageStatus::CapabilityUnavailable => PartExecutionStatusResource::CapabilityUnavailable,
        MessageStatus::ToolUnavailable => PartExecutionStatusResource::ToolUnavailable,
        MessageStatus::Failed => PartExecutionStatusResource::Failed,
        MessageStatus::Cancelled => PartExecutionStatusResource::Cancelled,
    }
}

fn part_status_from_state(state: MessageStatus) -> PartExecutionStatusResource {
    match state {
        MessageStatus::Pending => PartExecutionStatusResource::Pending,
        MessageStatus::InProgress => PartExecutionStatusResource::InProgress,
        MessageStatus::Completed => PartExecutionStatusResource::Completed,
        MessageStatus::PolicyDenied => PartExecutionStatusResource::PolicyDenied,
        MessageStatus::UserDeclined => PartExecutionStatusResource::UserDeclined,
        MessageStatus::CapabilityUnavailable => PartExecutionStatusResource::CapabilityUnavailable,
        MessageStatus::ToolUnavailable => PartExecutionStatusResource::ToolUnavailable,
        MessageStatus::Failed => PartExecutionStatusResource::Failed,
        MessageStatus::Cancelled => PartExecutionStatusResource::Cancelled,
    }
}

/// Whether a wire run/part state string is terminal. Exposed for the app's
/// optimistic-entry reconciliation, which inspects marker states directly.
pub fn part_state_is_terminal(state: &str) -> bool {
    message_status_is_terminal(message_status_from_string(state))
}

const fn message_status_is_terminal(state: MessageStatus) -> bool {
    matches!(
        state,
        MessageStatus::Completed
            | MessageStatus::PolicyDenied
            | MessageStatus::UserDeclined
            | MessageStatus::CapabilityUnavailable
            | MessageStatus::ToolUnavailable
            | MessageStatus::Failed
            | MessageStatus::Cancelled
    )
}

const fn assistant_reply_state_requires_outcome(state: MessageStatus) -> bool {
    matches!(
        state,
        MessageStatus::Failed
            | MessageStatus::Cancelled
            | MessageStatus::PolicyDenied
            | MessageStatus::UserDeclined
            | MessageStatus::CapabilityUnavailable
            | MessageStatus::ToolUnavailable
    )
}

fn assistant_reply_lifecycle(
    state: MessageStatus,
    problem: Option<agena_failure::UserProblem>,
) -> TranscriptAssistantReplyLifecycle {
    match state {
        MessageStatus::Failed
        | MessageStatus::PolicyDenied
        | MessageStatus::UserDeclined
        | MessageStatus::CapabilityUnavailable
        | MessageStatus::ToolUnavailable => TranscriptAssistantReplyLifecycle::Failed { problem },
        MessageStatus::Cancelled => TranscriptAssistantReplyLifecycle::Cancelled,
        MessageStatus::Completed => TranscriptAssistantReplyLifecycle::Completed,
        MessageStatus::Pending | MessageStatus::InProgress => {
            TranscriptAssistantReplyLifecycle::Running
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_api::resource::SessionTranscriptPart;

    fn run(part_id: i64, role: &str, state: &str) -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id,
            kind: "run".to_owned(),
            role: role.to_owned(),
            state: state.to_owned(),
            content: serde_json::json!({ "run_kind": "user_send" }),
            summary: None,
            created_at_ms: part_id * 10,
            parent_part_id: None,
            run_id: Some(part_id),
        }
    }

    fn content_part(
        part_id: i64,
        kind: &str,
        role: &str,
        content: serde_json::Value,
    ) -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id,
            kind: kind.to_owned(),
            role: role.to_owned(),
            state: "completed".to_owned(),
            content,
            summary: None,
            created_at_ms: part_id * 10,
            parent_part_id: None,
            run_id: Some(1),
        }
    }

    #[test]
    fn run_markers_group_content_parts_into_entries() {
        let parts = vec![
            run(1, "user", "completed"),
            content_part(2, "text", "user", serde_json::json!({ "text": "hello" })),
            run(3, "assistant", "completed"),
            content_part(
                4,
                "think",
                "assistant",
                serde_json::json!({ "summary": ["thinking"] }),
            ),
            content_part(5, "text", "assistant", serde_json::json!({ "text": "hi" })),
        ];
        let entries = parts_entries(&parts);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, TranscriptEntryId::StoredMessage(1));
        assert_eq!(entries[0].role, Some(MessageRole::User));
        assert_eq!(entries[0].state, MessageStatus::Completed);
        assert_eq!(entries[0].parts.len(), 1);
        assert!(matches!(
            entries[0].parts[0].content,
            TranscriptPartContent::Text(_)
        ));
        assert_eq!(entries[1].role, Some(MessageRole::Assistant));
        assert_eq!(entries[1].parts.len(), 2);
        assert!(matches!(
            entries[1].parts[0].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(_))
        ));
        assert!(matches!(
            entries[1].parts[1].content,
            TranscriptPartContent::Text(_)
        ));
    }

    #[test]
    fn failed_assistant_run_renders_a_lifecycle_outcome_without_an_error_part() {
        let parts = vec![run(1, "assistant", "failed")];
        let entries = parts_entries(&parts);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].parts.len(), 1);
        assert!(matches!(
            entries[0].parts[0].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::AssistantReplyLifecycle(
                TranscriptAssistantReplyLifecycle::Failed { .. }
            ))
        ));
    }

    #[test]
    fn completed_assistant_run_with_body_needs_no_lifecycle_row() {
        let parts = vec![
            run(1, "assistant", "completed"),
            content_part(
                2,
                "text",
                "assistant",
                serde_json::json!({ "text": "done" }),
            ),
        ];
        let entries = parts_entries(&parts);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].parts.len(),
            1,
            "body text only, no lifecycle row"
        );
    }

    #[test]
    fn runtime_role_entries_have_no_header_role() {
        let parts = vec![run(1, "runtime", "completed")];
        let entries = parts_entries(&parts);
        assert_eq!(entries[0].role, None);
    }

    #[test]
    fn orphan_content_parts_become_their_own_entry() {
        let parts = vec![content_part(
            9,
            "text",
            "assistant",
            serde_json::json!({ "text": "standalone" }),
        )];
        let entries = parts_entries(&parts);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, TranscriptEntryId::StoredMessage(9));
        assert_eq!(entries[0].parts.len(), 1);
    }

    #[test]
    fn unknown_kinds_fall_back_to_a_text_part() {
        let parts = vec![
            run(1, "assistant", "completed"),
            content_part(
                2,
                "widget",
                "assistant",
                serde_json::json!({ "summary": "opaque" }),
            ),
        ];
        let entries = parts_entries(&parts);
        assert_eq!(entries[0].parts.len(), 1);
        assert!(matches!(
            entries[0].parts[0].content,
            TranscriptPartContent::Text(_)
        ));
    }

    #[test]
    fn non_terminal_runs_are_detected_across_markers() {
        let done = vec![run(1, "assistant", "completed")];
        assert!(!parts_have_non_terminal_runs(&done));
        let active = vec![run(1, "assistant", "in_progress")];
        assert!(parts_have_non_terminal_runs(&active));
    }

    #[test]
    fn visible_user_inputs_count_only_user_runs_with_text() {
        let parts = vec![
            run(1, "user", "completed"),
            content_part(2, "text", "user", serde_json::json!({ "text": "a" })),
            run(3, "assistant", "completed"),
            content_part(4, "text", "assistant", serde_json::json!({ "text": "b" })),
            run(5, "user", "in_progress"),
        ];
        // The trailing in-progress user run has no text yet, so only the
        // first user run counts.
        assert_eq!(parts_visible_user_inputs(&parts), 1);
    }

    #[test]
    fn last_assistant_reply_text_takes_the_final_assistant_run() {
        // Each content part backlinks its enclosing run marker, matching the
        // durable projection's `run_id` contract.
        let content = |run_id: i64, part_id: i64, role: &str, text: &str| SessionTranscriptPart {
            part_id,
            kind: "text".to_owned(),
            role: role.to_owned(),
            state: "completed".to_owned(),
            content: serde_json::json!({ "text": text }),
            summary: None,
            created_at_ms: part_id * 10,
            parent_part_id: None,
            run_id: Some(run_id),
        };
        let parts = vec![
            run(1, "user", "completed"),
            content(1, 2, "user", "hello"),
            run(3, "assistant", "completed"),
            content(3, 4, "assistant", "first"),
            run(5, "user", "completed"),
            content(5, 6, "user", "again"),
            run(7, "assistant", "in_progress"),
            content(7, 8, "assistant", "second"),
            content(7, 9, "assistant", "tail"),
        ];
        assert_eq!(
            last_assistant_reply_text(&parts).as_deref(),
            Some("second\ntail")
        );
    }

    #[test]
    fn last_assistant_reply_text_is_none_without_an_assistant_text() {
        let parts = vec![
            run(1, "user", "completed"),
            content_part(2, "text", "user", serde_json::json!({ "text": "hello" })),
        ];
        assert_eq!(last_assistant_reply_text(&parts), None);
        let bare = vec![run(3, "assistant", "completed")];
        assert_eq!(last_assistant_reply_text(&bare), None);
    }
}
