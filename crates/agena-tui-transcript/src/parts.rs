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
    part::{
        AttachmentPartResource, ErrorPartResource, HookPartResource, OperationPartResource,
        PartExecutionStatusResource, ReasoningPartResource, RequestPartResource,
        SkillReferencePartResource, StructuredFieldResource, StructuredObjectResource,
        StructuredValueResource, TextPartResource, ToolInvocationResource,
    },
    resource::{
        PartAttachment, PartAttachmentKind, PartAttachmentSource, PartSkillReference, RunRole,
        RunStatus, SessionTranscriptPart, UserInputOption, UserInputQuestion, UserInputReply,
        UserInputReplyKind, UserInputRequest,
    },
};
use agena_runtime_contracts::part_content::InteractionContent;
use serde_json::Value;

use crate::{
    TranscriptActivityContent, TranscriptAssistantReplyLifecycle, TranscriptContentId,
    TranscriptEntry, TranscriptEntryId, TranscriptEntryPart, TranscriptPartContent,
};

/// Project an ordered v2 part list into transcript entries. Each `run` marker
/// starts an entry, except that consecutive assistant runs fold into a single
/// entry: a multi-round tool loop emits one run marker per model turn (tool
/// calls, then the final answer) and the user expects those to render as one
/// assistant block. This mirrors `canonical_turn_span` (history.rs), which
/// groups every non-user run after a user run into one canonical turn.
/// Content parts without an enclosing marker become their own entry.
pub fn parts_entries(parts: &[SessionTranscriptPart]) -> Vec<TranscriptEntry<'static>> {
    let mut entries = Vec::new();
    let mut current: Option<TranscriptEntry<'static>> = None;

    for part in parts {
        if part.kind == "run" {
            let marker = run_marker_entry(part);
            if current
                .as_ref()
                .is_some_and(|entry| entry.role == Some(RunRole::Assistant))
                && marker.role == Some(RunRole::Assistant)
            {
                // Fold this run marker into the open assistant entry: keep the
                // first marker's created_at for stable ordering, but fold the
                // state to the most-terminal one (mirroring history's
                // `more_terminal_status`), so a live block stays InProgress
                // until the whole turn terminalizes and a mid-loop failure is
                // not hidden by a later successful turn.
                let entry = current.as_mut().expect("foldable assistant entry exists");
                entry.state = fold_run_status(entry.state, marker.state);
                continue;
            }
            if let Some(entry) = current.take() {
                entries.push(finalize_run_entry(entry));
            }
            current = Some(marker);
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
    if entry.role == Some(RunRole::Assistant) {
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
        "text" => TranscriptPartContent::Text(TextPartResource {
            text: string_field(content, "text").unwrap_or_default(),
            synthetic: bool_field(content, "synthetic").unwrap_or(false),
        }),
        "think" => TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(
            ReasoningPartResource {
                summary: string_array_field(content, "summary"),
                raw_content: string_array_field(content, "raw"),
                encrypted_content: string_field(content, "encrypted_content"),
            },
        )),
        "tool_call" => {
            if let Some(operation) = operation_resource_from_content(content) {
                return TranscriptPartContent::Activity(TranscriptActivityContent::Operation(
                    Box::new(operation),
                ));
            }
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
        "tool_result" => TranscriptPartContent::Text(TextPartResource {
            text: string_field(content, "output")
                .or_else(|| string_field(content, "text"))
                .unwrap_or_default(),
            synthetic: false,
        }),
        "file_ref" => TranscriptPartContent::Activity(TranscriptActivityContent::Attachment(
            AttachmentPartResource {
                attachments: vec![PartAttachment {
                    kind: PartAttachmentKind::File,
                    mime: string_field(content, "mime").unwrap_or_default(),
                    source: string_field(content, "path")
                        .map(|path| PartAttachmentSource::LocalPath { path })
                        .unwrap_or_else(|| PartAttachmentSource::Url { url: String::new() }),
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
        "paste_ref" => TranscriptPartContent::Text(TextPartResource {
            text: string_field(content, "text").unwrap_or_default(),
            synthetic: true,
        }),
        "skill_ref" => TranscriptPartContent::Activity(TranscriptActivityContent::SkillReference(
            SkillReferencePartResource {
                skills: vec![PartSkillReference {
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
            Box::new(HookPartResource {
                hook: string_field(content, "hook")
                    .or_else(|| string_field(content, "kind"))
                    .unwrap_or_else(|| part.kind.clone()),
                plugin_id: string_field(content, "plugin_id"),
                summary: string_field(content, "summary").unwrap_or_default(),
                detail: string_field(content, "detail"),
            }),
        )),
        "compaction" => TranscriptPartContent::Activity(TranscriptActivityContent::Hook(Box::new(
            HookPartResource {
                hook: "compaction".to_owned(),
                plugin_id: None,
                summary: string_field(content, "summary").unwrap_or_default(),
                detail: content.get("window").map(|window| window.to_string()),
            },
        ))),
        "error" => match serde_json::from_value::<ErrorPartResource>(part.content.clone()) {
            Ok(error) => TranscriptPartContent::Activity(TranscriptActivityContent::Error(error)),
            Err(_) => TranscriptPartContent::Text(TextPartResource {
                text: string_field(content, "message")
                    .or_else(|| string_field(content, "summary"))
                    .unwrap_or_else(|| fallback_json_text(content)),
                synthetic: false,
            }),
        },
        "interaction" => {
            // v1 requests arrive in the `RequestPartResource` shape (tagged
            // `request_type`). The v2 canonical `interaction` shape instead
            // carries `type`/`prompt`/`options` plus the lossless `request`
            // and `reply` objects under `extra`. Decode both so the row
            // renders as a friendly awaiting-user-input part rather than
            // dumping its raw JSON as body text. The v2 shape is read through
            // the typed [`InteractionContent`] accessors rather than raw JSON
            // keys, so request/reply reconstruction stays in one place.
            match serde_json::from_value::<RequestPartResource>(part.content.clone()) {
                Ok(request) => TranscriptPartContent::Activity(TranscriptActivityContent::Request(
                    Box::new(request),
                )),
                Err(_) => match InteractionContent::try_from(content) {
                    Ok(interaction) => match interaction_request_resource(&interaction) {
                        Some(request) => {
                            TranscriptPartContent::Activity(TranscriptActivityContent::Request(
                                Box::new(RequestPartResource::UserInput {
                                    request,
                                    reply: interaction_reply_resource(&interaction),
                                }),
                            ))
                        }
                        None => TranscriptPartContent::Text(TextPartResource {
                            text: fallback_json_text(content),
                            synthetic: false,
                        }),
                    },
                    Err(_) => TranscriptPartContent::Text(TextPartResource {
                        text: fallback_json_text(content),
                        synthetic: false,
                    }),
                },
            }
        }
        _ => TranscriptPartContent::Text(TextPartResource {
            text: fallback_json_text(content),
            synthetic: false,
        }),
    }
}

/// Recover the lossless operation envelope stored below canonical
/// `tool_call.operation`. The v2 top-level keys intentionally carry only the
/// invocation identity; execution output, display text and lifecycle live in
/// this nested envelope. Projecting only the shallow keys makes a completed
/// tool look expandable in the TUI while giving it an empty body.
fn operation_resource_from_content(content: &Value) -> Option<OperationPartResource> {
    let mut operation = content
        .get("operation")
        .and_then(|value| serde_json::from_value::<OperationPartResource>(value.clone()).ok())?;

    if operation.invocation.gateway_function.is_none() {
        let gateway_name = content
            .get("tool_api_call")
            .and_then(|call| call.get("function"))
            .and_then(Value::as_str)
            .or_else(|| {
                content
                    .get("operation")
                    .and_then(|value| value.get("invocation"))
                    .and_then(|value| value.get("tool_api_call"))
                    .and_then(|value| value.get("function"))
                    .and_then(Value::as_str)
            })
            .unwrap_or(operation.invocation.name.as_str());
        operation.invocation.gateway_function = gateway_function(gateway_name);
    }

    // Runtime operations keep their authoritative output in `result`; the
    // public presentation resource also exposes convenient flattened mirrors
    // consumed by the transcript renderer. Fill those mirrors without
    // discarding the original envelope.
    if operation.title.is_empty() {
        operation.title = operation.result.display.title.clone();
    }
    if operation.summary.is_empty() {
        operation.summary = operation.result.display.summary.clone();
    }
    if operation.model_output.is_empty() {
        operation.model_output = operation.result.model_preview.clone();
    }
    if operation.blocks.is_empty() {
        operation.blocks = operation.result.content.clone();
    }
    if operation.attachments.is_empty() {
        operation.attachments = operation.result.attachments.clone();
    }
    if operation.structured.is_none() {
        operation.structured = operation.result.structured.clone();
    }
    Some(operation)
}

fn gateway_function(name: &str) -> Option<agena_api::part::ToolGatewayFunctionResource> {
    use agena_api::part::ToolGatewayFunctionResource as Gateway;

    match name {
        "tools_list" => Some(Gateway::List),
        "tools_search" => Some(Gateway::Search),
        "tools_help" => Some(Gateway::Help),
        "tools_tags" => Some(Gateway::Tags),
        "tools_call" => Some(Gateway::Call),
        _ => None,
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

/// Recover a [`UserInputRequest`] from the typed `interaction` content:
/// prefer the lossless typed `request()` accessor, otherwise reconstruct from
/// the display keys (`kind`/`prompt`/`options`). Returns `None` only when the
/// content carries neither a usable request nor enough display fields.
fn interaction_request_resource(interaction: &InteractionContent) -> Option<UserInputRequest> {
    if let Some(request) = interaction.request() {
        return Some(user_input_request_resource(request));
    }
    let kind = interaction.kind.as_str();
    let questions = interaction
        .options
        .as_ref()
        .and_then(|value| serde_json::from_value::<Vec<UserInputQuestion>>(value.clone()).ok())
        .unwrap_or_default();
    if interaction.kind == agena_domain::UserInputKind::AskUser
        && interaction.prompt.is_none()
        && questions.is_empty()
    {
        return None;
    }
    Some(UserInputRequest {
        request_id: interaction
            .request_id()
            .unwrap_or_else(|| format!("interaction-{}", kind)),
        session_id: None,
        title: interaction.prompt.clone().unwrap_or_default(),
        body_markdown: interaction
            .request()
            .map(|request| request.body_markdown)
            .unwrap_or_default(),
        kind: kind.to_owned(),
        auto_resolution_ms: None,
        presented_at: None,
        questions,
        created_at: chrono::Utc::now(),
    })
}

/// Recover an optional [`UserInputReply`] from the typed `interaction`
/// content via the `reply()` accessor (`extra["reply"]`, falling back to the
/// display `response` key).
fn interaction_reply_resource(interaction: &InteractionContent) -> Option<UserInputReply> {
    interaction.reply().map(user_input_reply_resource)
}

/// Project a typed domain request onto the API presentation resource,
/// canonicalizing the enum kind back to its wire string.
fn user_input_request_resource(request: agena_domain::UserInputRequest) -> UserInputRequest {
    UserInputRequest {
        request_id: request.request_id,
        session_id: request.session_id,
        title: request.title,
        body_markdown: request.body_markdown,
        kind: request.kind.as_str().to_owned(),
        auto_resolution_ms: request.auto_resolution_ms,
        presented_at: request.presented_at,
        questions: request
            .questions
            .into_iter()
            .map(|question| UserInputQuestion {
                header: question.header,
                question: question.question,
                options: question
                    .options
                    .into_iter()
                    .map(|option| UserInputOption {
                        label: option.label,
                        description: option.description,
                    })
                    .collect(),
                multiple: question.multiple,
                allow_custom: question.allow_custom,
            })
            .collect(),
        created_at: request.created_at,
    }
}

/// Project a typed domain reply onto the API presentation resource.
fn user_input_reply_resource(reply: agena_domain::UserInputReply) -> UserInputReply {
    UserInputReply {
        request_id: reply.request_id,
        kind: match reply.kind {
            agena_domain::UserInputReplyKind::Submit => UserInputReplyKind::Submit,
            agena_domain::UserInputReplyKind::Cancel => UserInputReplyKind::Cancel,
            agena_domain::UserInputReplyKind::Timeout => UserInputReplyKind::Timeout,
        },
        answers: reply.answers,
        reason: reply.reason,
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
pub(crate) fn role_from_string(role: &str) -> Option<RunRole> {
    match role {
        "user" => Some(RunRole::User),
        "assistant" => Some(RunRole::Assistant),
        "tool" => Some(RunRole::Tool),
        "system" => Some(RunRole::System),
        _ => None,
    }
}

/// Parse the wire part/run state string into the render model's message state.
pub(crate) fn message_status_from_string(state: &str) -> RunStatus {
    match state {
        "pending" => RunStatus::Pending,
        "in_progress" | "running" => RunStatus::InProgress,
        "completed" => RunStatus::Completed,
        "policy_denied" => RunStatus::PolicyDenied,
        "user_declined" => RunStatus::UserDeclined,
        "capability_unavailable" => RunStatus::CapabilityUnavailable,
        "tool_unavailable" => RunStatus::ToolUnavailable,
        "failed" => RunStatus::Failed,
        "cancelled" | "canceled" => RunStatus::Cancelled,
        _ => RunStatus::Pending,
    }
}

/// Parse the wire part state string into the render model's part state.
pub(crate) fn part_status_from_string(state: &str) -> PartExecutionStatusResource {
    match message_status_from_string(state) {
        RunStatus::Pending => PartExecutionStatusResource::Pending,
        RunStatus::InProgress => PartExecutionStatusResource::InProgress,
        RunStatus::Completed => PartExecutionStatusResource::Completed,
        RunStatus::PolicyDenied => PartExecutionStatusResource::PolicyDenied,
        RunStatus::UserDeclined => PartExecutionStatusResource::UserDeclined,
        RunStatus::CapabilityUnavailable => PartExecutionStatusResource::CapabilityUnavailable,
        RunStatus::ToolUnavailable => PartExecutionStatusResource::ToolUnavailable,
        RunStatus::Failed => PartExecutionStatusResource::Failed,
        RunStatus::Cancelled => PartExecutionStatusResource::Cancelled,
    }
}

fn part_status_from_state(state: RunStatus) -> PartExecutionStatusResource {
    match state {
        RunStatus::Pending => PartExecutionStatusResource::Pending,
        RunStatus::InProgress => PartExecutionStatusResource::InProgress,
        RunStatus::Completed => PartExecutionStatusResource::Completed,
        RunStatus::PolicyDenied => PartExecutionStatusResource::PolicyDenied,
        RunStatus::UserDeclined => PartExecutionStatusResource::UserDeclined,
        RunStatus::CapabilityUnavailable => PartExecutionStatusResource::CapabilityUnavailable,
        RunStatus::ToolUnavailable => PartExecutionStatusResource::ToolUnavailable,
        RunStatus::Failed => PartExecutionStatusResource::Failed,
        RunStatus::Cancelled => PartExecutionStatusResource::Cancelled,
    }
}

/// Whether a wire run/part state string is terminal. Exposed for the app's
/// optimistic-entry reconciliation, which inspects marker states directly.
pub fn part_state_is_terminal(state: &str) -> bool {
    message_status_is_terminal(message_status_from_string(state))
}

const fn message_status_is_terminal(state: RunStatus) -> bool {
    matches!(
        state,
        RunStatus::Completed
            | RunStatus::PolicyDenied
            | RunStatus::UserDeclined
            | RunStatus::CapabilityUnavailable
            | RunStatus::ToolUnavailable
            | RunStatus::Failed
            | RunStatus::Cancelled
    )
}

/// Combine two run states for a folded assistant entry: the more terminal /
/// more severe state wins. Mirrors history's `more_terminal_status` ordering
/// (Failed > Cancelled > Completed > InProgress > Pending), with the
/// denied/unavailable set at failed severity so a mid-loop rejection is
/// surfaced rather than hidden by a later successful turn.
fn fold_run_status(current: RunStatus, next: RunStatus) -> RunStatus {
    use RunStatus::*;
    let severity = |status: RunStatus| match status {
        Failed | PolicyDenied | UserDeclined | CapabilityUnavailable | ToolUnavailable => 4,
        Cancelled => 3,
        Completed => 2,
        InProgress => 1,
        Pending => 0,
    };
    if severity(next) > severity(current) {
        next
    } else {
        current
    }
}

const fn assistant_reply_state_requires_outcome(state: RunStatus) -> bool {
    matches!(
        state,
        RunStatus::Failed
            | RunStatus::Cancelled
            | RunStatus::PolicyDenied
            | RunStatus::UserDeclined
            | RunStatus::CapabilityUnavailable
            | RunStatus::ToolUnavailable
    )
}

fn assistant_reply_lifecycle(
    state: RunStatus,
    problem: Option<agena_failure::UserProblem>,
) -> TranscriptAssistantReplyLifecycle {
    match state {
        RunStatus::Failed
        | RunStatus::PolicyDenied
        | RunStatus::UserDeclined
        | RunStatus::CapabilityUnavailable
        | RunStatus::ToolUnavailable => TranscriptAssistantReplyLifecycle::Failed { problem },
        RunStatus::Cancelled => TranscriptAssistantReplyLifecycle::Cancelled,
        RunStatus::Completed => TranscriptAssistantReplyLifecycle::Completed,
        RunStatus::Pending | RunStatus::InProgress => TranscriptAssistantReplyLifecycle::Running,
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
        assert_eq!(entries[0].role, Some(RunRole::User));
        assert_eq!(entries[0].state, RunStatus::Completed);
        assert_eq!(entries[0].parts.len(), 1);
        assert!(matches!(
            entries[0].parts[0].content,
            TranscriptPartContent::Text(_)
        ));
        assert_eq!(entries[1].role, Some(RunRole::Assistant));
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
    fn consecutive_assistant_runs_fold_into_one_entry() {
        // A multi-round tool loop: one user message, then several assistant
        // runs (tool call turn, tool call turn, final answer). These must
        // render as ONE assistant block, not one per run marker.
        let parts = vec![
            run(1, "user", "completed"),
            content_part(2, "text", "user", serde_json::json!({ "text": "hi" })),
            run(3, "assistant", "in_progress"),
            content_part(
                4,
                "tool_call",
                "assistant",
                serde_json::json!({ "name": "tools_search", "operation": {} }),
            ),
            content_part(
                5,
                "tool_result",
                "tool",
                serde_json::json!({ "output": "ok" }),
            ),
            run(7, "assistant", "in_progress"),
            content_part(
                8,
                "tool_call",
                "assistant",
                serde_json::json!({ "name": "tools_help", "operation": {} }),
            ),
            content_part(
                9,
                "tool_result",
                "tool",
                serde_json::json!({ "output": "ok" }),
            ),
            run(11, "assistant", "completed"),
            content_part(
                12,
                "text",
                "assistant",
                serde_json::json!({ "text": "done" }),
            ),
        ];
        let entries = parts_entries(&parts);
        assert_eq!(
            entries.len(),
            2,
            "one user entry + one folded assistant entry"
        );
        assert_eq!(entries[0].role, Some(RunRole::User));
        assert_eq!(entries[1].role, Some(RunRole::Assistant));
        // created_at comes from the FIRST assistant marker (stable ordering),
        // state from the LAST marker (live turn shows InProgress while running).
        assert_eq!(entries[1].id, TranscriptEntryId::StoredMessage(3));
        assert_eq!(entries[1].state, RunStatus::Completed);
        // All tool calls, results and the final text live in the one block.
        let kinds = entries[1]
            .parts
            .iter()
            .map(|part| match &part.content {
                TranscriptPartContent::Activity(TranscriptActivityContent::Operation(_)) => "op",
                TranscriptPartContent::Text(_) => "text",
                _ => "other",
            })
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec!["op", "text", "op", "text", "text"]);
    }

    #[test]
    fn folded_entry_surfaces_a_mid_loop_failure() {
        // A tool turn fails but a later retry completes. The folded block must
        // still show the failure rather than hiding it behind the last state.
        let parts = vec![
            run(1, "user", "completed"),
            content_part(2, "text", "user", serde_json::json!({ "text": "hi" })),
            run(3, "assistant", "completed"),
            content_part(
                4,
                "tool_call",
                "assistant",
                serde_json::json!({ "name": "tools_search", "operation": {} }),
            ),
            run(5, "assistant", "failed"),
            run(7, "assistant", "completed"),
            content_part(
                8,
                "text",
                "assistant",
                serde_json::json!({ "text": "recovered" }),
            ),
        ];
        let entries = parts_entries(&parts);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].role, Some(RunRole::Assistant));
        assert_eq!(entries[1].state, RunStatus::Failed);
    }

    #[test]
    fn folded_entry_stays_in_progress_until_the_whole_turn_terminalizes() {
        // Intermediate tool-call turns are in_progress; only the final turn is
        // completed. The live block must read InProgress throughout.
        let parts = vec![
            run(1, "user", "completed"),
            run(3, "assistant", "in_progress"),
            content_part(
                4,
                "tool_call",
                "assistant",
                serde_json::json!({ "name": "tools_search", "operation": {} }),
            ),
            run(5, "assistant", "in_progress"),
            run(7, "assistant", "completed"),
            content_part(
                8,
                "text",
                "assistant",
                serde_json::json!({ "text": "done" }),
            ),
        ];
        let entries = parts_entries(&parts);
        assert_eq!(entries[1].role, Some(RunRole::Assistant));
        assert_eq!(entries[1].state, RunStatus::Completed);
    }

    #[test]
    fn user_run_breaks_assistant_folding() {
        // A fresh user message must start a new entry even after assistant runs.
        let parts = vec![
            run(1, "user", "completed"),
            content_part(2, "text", "user", serde_json::json!({ "text": "a" })),
            run(3, "assistant", "completed"),
            content_part(4, "text", "assistant", serde_json::json!({ "text": "r1" })),
            run(5, "user", "completed"),
            content_part(6, "text", "user", serde_json::json!({ "text": "b" })),
            run(7, "assistant", "completed"),
            content_part(8, "text", "assistant", serde_json::json!({ "text": "r2" })),
        ];
        let entries = parts_entries(&parts);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[2].id, TranscriptEntryId::StoredMessage(5));
        assert_eq!(entries[3].id, TranscriptEntryId::StoredMessage(7));
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
    fn completed_tool_call_recovers_nested_operation_output_for_expansion() {
        let parts = vec![
            run(1, "assistant", "completed"),
            content_part(
                2,
                "tool_call",
                "assistant",
                serde_json::json!({
                    "name": "tools_search",
                    "input": { "query": "status" },
                    "tool_api_call": {
                        "function": "tools_search",
                        "arguments": {
                            "fields": [{
                                "name": "query",
                                "value": { "kind": "text", "value": "status" }
                            }]
                        }
                    },
                    "operation": {
                        "call_id": 73,
                        "invocation": {
                            "name": "tools_search",
                            "input": {
                                "fields": [{
                                    "name": "query",
                                    "value": { "kind": "text", "value": "status" }
                                }]
                            },
                            "tool_api_call": {
                                "function": "tools_search",
                                "arguments": { "fields": [] }
                            }
                        },
                        "title": "tools_search · Search tools · status · 3/3",
                        "summary": "Returned 3 of 3 matching tools; no more results.",
                        "result": {
                            "state": "completed",
                            "content": [{
                                "id": "text",
                                "type": "log",
                                "stream": "stdout",
                                "text": "Matching tools for status:\n- context.status"
                            }],
                            "model_preview": {
                                "text": "Matching tools for status:\n- context.status"
                            },
                            "display": {
                                "title": "tools_search · Search tools · status · 3/3",
                                "summary": "Returned 3 of 3 matching tools; no more results."
                            }
                        },
                        "lifecycle": { "start_ms": 100, "end_ms": 200 }
                    }
                }),
            ),
        ];

        let entries = parts_entries(&parts);
        let TranscriptPartContent::Activity(TranscriptActivityContent::Operation(operation)) =
            &entries[0].parts[0].content
        else {
            panic!("expected operation activity");
        };
        assert_eq!(
            operation.invocation.gateway_function,
            Some(agena_api::part::ToolGatewayFunctionResource::Search)
        );
        assert_eq!(
            operation.model_output.text,
            "Matching tools for status:\n- context.status"
        );
        assert_eq!(operation.blocks.len(), 1);
        assert_eq!(
            operation.summary,
            "Returned 3 of 3 matching tools; no more results."
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

    #[test]
    fn canonical_interaction_part_renders_as_a_request_not_raw_json() {
        // The v2 canonical `interaction` shape (`type`/`prompt`/`options` plus
        // the lossless `request` object) must project to a Request activity,
        // not fall back to dumping its JSON as transcript body text.
        // (Regression: workflow plan review dumped `{options, prompt, request,
        // type}` verbatim because only the v1 `request_type` shape was tried.)
        let part = content_part(
            2,
            "interaction",
            "assistant",
            serde_json::json!({
                "type": "review",
                "prompt": "Approve New Plan",
                "options": [
                    {
                        "header": "Decision",
                        "question": "Choose whether this plan should move to active.",
                        "options": [{ "label": "Approve", "description": "Move it to active." }],
                        "multiple": false,
                        "allow_custom": true
                    }
                ],
                "request": {
                    "request_id": "host-input:1:2:0",
                    "session_id": 1,
                    "title": "Approve New Plan",
                    "kind": "review",
                    "auto_resolution_ms": 600000,
                    "questions": [
                        {
                            "header": "Decision",
                            "question": "Choose whether this plan should move to active.",
                            "options": [{ "label": "Approve", "description": "Move it to active." }],
                            "multiple": false,
                            "allow_custom": true
                        }
                    ],
                    "created_at": "2026-08-11T00:00:00Z"
                }
            }),
        );
        let content = entry_part(&part).content;
        let TranscriptPartContent::Activity(TranscriptActivityContent::Request(request)) = content
        else {
            panic!("v2 interaction part must project to a Request activity, got raw text/JSON");
        };
        let RequestPartResource::UserInput { request, reply } = request.as_ref();
        assert_eq!(request.request_id, "host-input:1:2:0");
        assert_eq!(request.title, "Approve New Plan");
        assert_eq!(request.kind, "review");
        assert_eq!(request.questions.len(), 1);
        assert_eq!(request.questions[0].question, "Choose whether this plan should move to active.");
        assert!(reply.is_none());
    }

    #[test]
    fn canonical_interaction_replying_reconstructs_the_reply() {
        let part = content_part(
            2,
            "interaction",
            "assistant",
            serde_json::json!({
                "type": "review",
                "prompt": "Approve New Plan",
                "request": {
                    "request_id": "host-input:1:2:0",
                    "session_id": 1,
                    "title": "Approve New Plan",
                    "kind": "review",
                    "questions": [],
                    "created_at": "2026-08-11T00:00:00Z"
                },
                "reply": {
                    "request_id": "host-input:1:2:0",
                    "kind": "submit",
                    "answers": { "decision": ["approve"] }
                }
            }),
        );
        let content = entry_part(&part).content;
        let TranscriptPartContent::Activity(TranscriptActivityContent::Request(request)) = content
        else {
            panic!("replied v2 interaction part must project to a Request activity");
        };
        let RequestPartResource::UserInput { request, reply } = request.as_ref();
        assert_eq!(request.kind, "review");
        let reply = reply.as_ref().expect("reply is recovered from the reply object");
        assert_eq!(reply.answers["decision"], ["approve"]);
    }
}
