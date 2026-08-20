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

use agena_api::live::SessionTranscriptFoldResource;
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
use agena_domain::TextSegmentActivity;
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
/// assistant block. Content ownership is resolved exclusively through
/// `run_id`, not physical row position: a background Hook may be appended much
/// later while still belonging to the assistant run that launched it.
/// Content parts whose referenced marker is absent become their own entry.
pub fn parts_entries(parts: &[SessionTranscriptPart]) -> Vec<TranscriptEntry<'static>> {
    parts_entries_with_folds(parts, &[])
}

/// Project ordered parts and server-provided presentation fold metadata into
/// the same render model as the lossless projection. Fold markers are inserted
/// immediately before their first visible activity part; their omitted raw
/// prefix is fetched by the app when the marker is expanded.
pub fn parts_entries_with_folds(
    parts: &[SessionTranscriptPart],
    folds: &[SessionTranscriptFoldResource],
) -> Vec<TranscriptEntry<'static>> {
    let marker_ids = parts
        .iter()
        .filter(|part| part.kind == "run")
        .map(|part| part.part_id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut content_by_run = std::collections::BTreeMap::<i64, Vec<&SessionTranscriptPart>>::new();
    for part in parts.iter().filter(|part| part.kind != "run") {
        if let Some(run_id) = part.run_id
            && marker_ids.contains(&run_id)
        {
            content_by_run.entry(run_id).or_default().push(part);
        }
    }

    // Build entries in marker/orphan order, but attach content by its durable
    // owner. This is the same projection contract used by Web.
    let mut projected = Vec::new();
    let mut latest_marker_index = None;
    for part in parts {
        if part.kind == "run" {
            let mut entry = run_marker_entry(part);
            if let Some(content) = content_by_run.remove(&part.part_id) {
                entry.parts.extend(content.into_iter().map(entry_part));
            }
            projected.push(entry);
            latest_marker_index = Some(projected.len() - 1);
        } else if part.run_id.is_none() {
            // Compatibility for pre-v2/test projections that omitted run_id:
            // physical adjacency is used only when no owner was recorded at
            // all. A present run_id is always authoritative, even when its
            // part arrives much later than another run marker.
            if let Some(index) = latest_marker_index {
                projected[index].parts.push(entry_part(part));
            } else {
                projected.push(orphan_entry(part));
            }
        } else if part
            .run_id
            .is_some_and(|run_id| !marker_ids.contains(&run_id))
        {
            projected.push(orphan_entry(part));
        }
    }

    let mut entries: Vec<TranscriptEntry<'static>> = Vec::new();
    for entry in projected {
        if let Some(previous) = entries.last_mut()
            && previous.role == Some(RunRole::Assistant)
            && entry.role == Some(RunRole::Assistant)
        {
            // Fold adjacent assistant rounds only after exact ownership has
            // been resolved. Runtime/System ingress remains a hard boundary.
            previous.state = fold_run_status(previous.state, entry.state);
            previous.parts.extend(entry.parts);
            continue;
        }
        entries.push(entry);
    }
    for fold in folds {
        let anchor = TranscriptContentId::StoredPart(fold.anchor_part_id);
        let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.parts.iter().any(|part| part.id == anchor))
        else {
            continue;
        };
        let Some(index) = entry.parts.iter().position(|part| part.id == anchor) else {
            continue;
        };
        entry.parts.insert(
            index,
            TranscriptEntryPart {
                id: TranscriptContentId::TranscriptFold {
                    run_id: fold.run_id,
                    anchor_part_id: fold.anchor_part_id,
                },
                status: PartExecutionStatusResource::Completed,
                content: TranscriptPartContent::Activity(TranscriptActivityContent::Fold {
                    hidden_count: usize::try_from(fold.hidden_count).unwrap_or(usize::MAX),
                }),
            },
        );
    }
    entries.into_iter().map(finalize_run_entry).collect()
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
    if entry.role == Some(RunRole::Assistant) {
        // The final assistant text part is the answer and must render as a
        // default-expanded part ([`TranscriptActivityContent::Answer`]); every
        // earlier text part is a working note and stays a collapsible
        // TextSegment. A text followed by a tool call is interstitial (the
        // reply continues past it), so only the last text with no tool call
        // after it is promoted.
        if let Some(index) = entry
            .parts
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, part)| {
                let is_text_segment = matches!(
                    part.content,
                    TranscriptPartContent::Activity(TranscriptActivityContent::TextSegment(_))
                );
                let tool_call_follows = entry.parts[index + 1..].iter().any(|later| {
                    matches!(
                        later.content,
                        TranscriptPartContent::Activity(TranscriptActivityContent::Operation(_))
                    )
                });
                (is_text_segment && !tool_call_follows).then_some(index)
            })
        {
            let part = &mut entry.parts[index];
            let segment = match &part.content {
                TranscriptPartContent::Activity(TranscriptActivityContent::TextSegment(
                    segment,
                )) => segment.text.clone(),
                _ => unreachable!("promotion target filtered to TextSegment"),
            };
            part.content = TranscriptPartContent::Activity(TranscriptActivityContent::Answer(
                Box::new(TextSegmentActivity { text: segment }),
            ));
        }
        // Mirror the v1 reply projection: an assistant run renders a trailing
        // lifecycle row when it has no content yet (the active envelope) or when
        // it ended without a durable Error part carrying the outcome (failed,
        // cancelled, denied). A completed run with body content needs no row.
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
        "text" => {
            let text = string_field(content, "text").unwrap_or_default();
            if part.role == "assistant" && !text.trim().is_empty() {
                // Assistant reply text projects as an owned activity so it
                // renders as a toggleable part. [`finalize_run_entry`] promotes
                // the final non-tool-followed text part to `Answer` (default
                // expanded); earlier turn texts stay collapsible TextSegments.
                TranscriptPartContent::Activity(TranscriptActivityContent::TextSegment(Box::new(
                    TextSegmentActivity { text },
                )))
            } else {
                TranscriptPartContent::Text(TextPartResource {
                    text,
                    synthetic: bool_field(content, "synthetic").unwrap_or(false),
                })
            }
        }
        "think" => TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(
            ReasoningPartResource {
                summary: string_array_field(content, "summary"),
                raw_content: string_array_field(content, "raw"),
                encrypted_content: string_field(content, "encrypted_content"),
            },
        )),
        "tool_call" => {
            if let Some(operation) = operation_resource_from_part(part) {
                return TranscriptPartContent::Activity(TranscriptActivityContent::Operation(
                    Box::new(operation),
                ));
            }
            TranscriptPartContent::Text(TextPartResource {
                text: format!("invalid tool_call content: {}", fallback_json_text(content)),
                synthetic: false,
            })
        }
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
                message: string_field(content, "message"),
            }),
        )),
        "compaction" => TranscriptPartContent::Activity(TranscriptActivityContent::Hook(Box::new(
            HookPartResource {
                hook: "compaction".to_owned(),
                plugin_id: None,
                summary: string_field(content, "summary").unwrap_or_default(),
                detail: content.get("window").map(|window| window.to_string()),
                message: None,
            },
        ))),
        // A background-operation completion/event delivered to the model as a
        // System message (the agena analog of Claude's task-notification chip).
        // Reuses the Hook activity renderer so it draws as a compact
        // notification row with the operation identity in the hook slot.
        "system_notification" => {
            let operation_kind =
                string_field(content, "operation_kind").unwrap_or_else(|| "background".to_string());
            let operation_id = string_field(content, "operation_id").unwrap_or_default();
            let status = string_field(content, "status").unwrap_or_default();
            let hook = if operation_id.is_empty() {
                operation_kind
            } else {
                format!("{operation_kind}:{operation_id}:{status}")
            };
            let summary = string_field(content, "summary")
                .or_else(|| string_field(content, "body"))
                .unwrap_or_default();
            let detail = string_field(content, "detail").or_else(|| string_field(content, "body"));
            TranscriptPartContent::Activity(TranscriptActivityContent::Hook(Box::new(
                HookPartResource {
                    hook,
                    plugin_id: None,
                    summary,
                    detail,
                    message: None,
                },
            )))
        }
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
/// Rewrite one stored `ViewBlock` JSON object into the API
/// `OperationBlockResource` wire shape in place:
///
/// - `table`: `columns: ["name"]` → `[{"key":"name","label":null}]`
/// - `search_results`: `items: [{title,url,snippet}]` → `results: [{title,uri,snippet}]`
/// - `custom`: expose `presentation`/`schema` as the required `value`
///
/// Known-compatible kinds (text/markdown/json/log/command/diff/file_changes/
/// media) are left untouched. Unknown kinds stay as-is so the typed decode can
/// still represent them.
fn operation_resource_from_part(part: &SessionTranscriptPart) -> Option<OperationPartResource> {
    use agena_runtime_contracts::part_content::ToolCallContent;

    let content = ToolCallContent::try_from(&part.content).ok()?;
    let name = content.name.clone();
    let input = match &content.input {
        Value::Object(map) => StructuredObjectResource {
            fields: map
                .iter()
                .map(|(name, value)| StructuredFieldResource {
                    name: name.clone(),
                    value: structured_value(value),
                })
                .collect(),
        },
        _ => return None,
    };
    let presentation = part.presentation.clone().unwrap_or_else(|| {
        agena_api::live::ToolHumanPresentationResource {
            title: name.clone(),
            summary: String::new(),
            blocks: Vec::new(),
        }
    });
    let title = presentation.title;
    let summary = presentation.summary;
    let blocks = presentation
        .blocks
        .iter()
        .filter_map(operation_block_from_view)
        .collect::<Vec<_>>();
    let state = serde_json::from_value::<agena_api::part::ToolResultStateResource>(
        serde_json::to_value(content.state).ok()?,
    )
    .unwrap_or(agena_api::part::ToolResultStateResource::Pending);
    let gateway_name = content
        .tool_api_call
        .as_ref()
        .map(|call| call.function.function_name())
        .unwrap_or(name.as_str());
    let lifecycle = serde_json::from_value::<agena_api::part::TimeRangeResource>(
        serde_json::to_value(&content.lifecycle).ok()?,
    )
    .unwrap_or_default();
    let display = agena_api::part::ToolResultDisplayResource {
        title: title.clone(),
        summary: summary.clone(),
        sections: Vec::new(),
    };
    let result = agena_api::part::ToolResultEnvelopeResource {
        state,
        structured: None,
        content: blocks.clone(),
        display,
        ..Default::default()
    };
    Some(OperationPartResource {
        call_id: content.call_id,
        invocation: ToolInvocationResource {
            gateway_function: gateway_function(gateway_name),
            name,
            plugin_name: content.plugin,
            input,
        },
        title,
        summary,
        authorization: content.authorization,
        user_input: content.user_input,
        blocks,
        attachments: Vec::new(),
        structured: None,
        result,
        lifecycle,
        error: content
            .error
            .map(|error| agena_api::part::OperationErrorResource {
                failure: (&error.failure).into(),
            }),
        metadata: content.metadata,
        ..Default::default()
    })
}

fn operation_block_from_view(
    block: &agena_domain::ViewBlock,
) -> Option<agena_api::part::OperationBlockResource> {
    let mut value = serde_json::to_value(block).ok()?;
    let object = value.as_object_mut()?;
    match object.get("type").and_then(Value::as_str) {
        Some("table") => {
            if let Some(columns) = object.get_mut("columns").and_then(Value::as_array_mut) {
                for column in columns {
                    if let Some(key) = column.as_str().map(ToOwned::to_owned) {
                        *column = serde_json::json!({"key": key});
                    }
                }
            }
        }
        Some("search_results") => {
            if let Some(items) = object.remove("items") {
                let mut items = items.as_array().cloned().unwrap_or_default();
                for item in &mut items {
                    if let Some(item) = item.as_object_mut()
                        && let Some(url) = item.remove("url")
                    {
                        item.insert("uri".to_owned(), url);
                    }
                }
                object.insert("results".to_owned(), Value::Array(items));
            }
            object.remove("total");
        }
        Some("custom") => {
            let schema = object.remove("schema").unwrap_or(Value::Null);
            let presentation = object.remove("presentation").unwrap_or(Value::Null);
            object.insert(
                "value".to_owned(),
                serde_json::json!({"schema": schema, "presentation": presentation}),
            );
            object.insert("schema".to_owned(), Value::Null);
            object.remove("kind");
        }
        _ => {}
    }
    object.remove("id");
    serde_json::from_value(value).ok()
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
        source: interaction.source(),
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
/// canonicalizing the enum kind back to its wire string. Shared with the
/// renderer, which converts a tool operation's `user_input` record request to
/// the resource it draws from.
pub(crate) fn user_input_request_resource(
    request: agena_domain::UserInputRequest,
) -> UserInputRequest {
    UserInputRequest {
        request_id: request.request_id,
        session_id: request.session_id,
        title: request.title,
        body_markdown: request.body_markdown,
        kind: request.kind.as_str().to_owned(),
        source: request.source,
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
pub(crate) fn user_input_reply_resource(reply: agena_domain::UserInputReply) -> UserInputReply {
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

fn timestamp(created_at_ms: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(created_at_ms).unwrap_or_default()
}

/// Parse the wire role string. Runtime ingress is intentionally rendered with
/// System identity; assistant-owned typed notifications retain Assistant via
/// their owning run instead of being relabelled from their content kind.
pub(crate) fn role_from_string(role: &str) -> Option<RunRole> {
    match role {
        "user" => Some(RunRole::User),
        "assistant" => Some(RunRole::Assistant),
        "tool" => Some(RunRole::Tool),
        "system" | "runtime" => Some(RunRole::System),
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
    // A cancellation is an interruption, not a result: when a newer run
    // continues the same reply (the `/continue` case reuses the reply
    // identity), the cancelled attempt must not keep the block marked
    // cancelled — the reply is generating again, or has since completed.
    if current == Cancelled && next != Cancelled {
        return next;
    }
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
    use crate::interaction_request_id_for_part;
    use agena_api::resource::SessionTranscriptPart;

    fn run(part_id: i64, role: &str, state: &str) -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id,
            kind: "run".to_owned(),
            role: role.to_owned(),
            state: state.to_owned(),
            content: serde_json::json!({ "run_kind": "user_send" }),
            presentation: None,
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
        content_part_for(1, part_id, kind, role, content)
    }

    fn content_part_for(
        run_id: i64,
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
            presentation: None,
            summary: None,
            created_at_ms: part_id * 10,
            parent_part_id: None,
            run_id: Some(run_id),
        }
    }

    #[test]
    fn run_markers_group_content_parts_into_entries() {
        let parts = vec![
            run(1, "user", "completed"),
            content_part(2, "text", "user", serde_json::json!({ "text": "hello" })),
            run(3, "assistant", "completed"),
            content_part_for(
                3,
                4,
                "think",
                "assistant",
                serde_json::json!({ "summary": ["thinking"] }),
            ),
            content_part_for(
                3,
                5,
                "text",
                "assistant",
                serde_json::json!({ "text": "hi" }),
            ),
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
        // The final assistant text is the answer: a default-expanded part.
        assert!(matches!(
            entries[1].parts[1].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::Answer(_))
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
            content_part_for(
                3,
                4,
                "tool_call",
                "assistant",
                serde_json::json!({
                    "name": "tools_search",
                    "input": {},
                    "state": "completed",
                    "output": {"payload": {"text": "ok"}}
                }),
            ),
            run(7, "assistant", "in_progress"),
            content_part_for(
                7,
                8,
                "tool_call",
                "assistant",
                serde_json::json!({
                    "name": "tools_help",
                    "input": {},
                    "state": "completed",
                    "output": {"payload": {"text": "ok"}}
                }),
            ),
            run(11, "assistant", "completed"),
            content_part_for(
                11,
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
        // All tool calls, results and the final text live in the one block;
        // the final assistant text is the answer part, tool results stay plain.
        let kinds = entries[1]
            .parts
            .iter()
            .map(|part| match &part.content {
                TranscriptPartContent::Activity(TranscriptActivityContent::Operation(_)) => "op",
                TranscriptPartContent::Activity(TranscriptActivityContent::Answer(_)) => "answer",
                TranscriptPartContent::Text(_) => "text",
                _ => "other",
            })
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec!["op", "text", "op", "text", "answer"]);
    }

    #[test]
    fn folded_entry_surfaces_a_mid_loop_failure() {
        // A tool turn fails but a later retry completes. The folded block must
        // still show the failure rather than hiding it behind the last state.
        let parts = vec![
            run(1, "user", "completed"),
            content_part(2, "text", "user", serde_json::json!({ "text": "hi" })),
            run(3, "assistant", "completed"),
            content_part_for(
                3,
                4,
                "tool_call",
                "assistant",
                serde_json::json!({ "name": "tools_search", "operation": {} }),
            ),
            run(5, "assistant", "failed"),
            run(7, "assistant", "completed"),
            content_part_for(
                7,
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
    fn cancelled_then_continued_reply_shows_the_live_continue_not_cancelled() {
        // Regression: the user cancels a reply, then runs `/continue`. The new
        // continue run reuses the reply identity and folds onto the cancelled
        // marker; the block must read as the live continue (InProgress), not
        // keep the stale "reply cancelled" lifecycle row.
        let parts = vec![
            run(1, "user", "completed"),
            content_part(2, "text", "user", serde_json::json!({ "text": "hi" })),
            run(3, "assistant", "cancelled"), // cancelled before any content
            run(5, "assistant", "in_progress"), // /continue regenerating
        ];
        let entries = parts_entries(&parts);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].role, Some(RunRole::Assistant));
        assert_eq!(entries[1].state, RunStatus::InProgress);
        assert!(!entries[1].parts.iter().any(|part| matches!(
            &part.content,
            TranscriptPartContent::Activity(TranscriptActivityContent::AssistantReplyLifecycle(
                TranscriptAssistantReplyLifecycle::Cancelled
            ))
        )));
    }

    #[test]
    fn completed_continue_supersedes_the_cancelled_attempt() {
        // The `/continue` finished with a real answer: the block is Completed
        // and renders the answer, never the stale cancellation.
        let parts = vec![
            run(1, "user", "completed"),
            content_part(2, "text", "user", serde_json::json!({ "text": "hi" })),
            run(3, "assistant", "cancelled"),
            run(5, "assistant", "completed"),
            content_part_for(
                5,
                6,
                "text",
                "assistant",
                serde_json::json!({ "text": "answer" }),
            ),
        ];
        let entries = parts_entries(&parts);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].state, RunStatus::Completed);
        assert!(!entries[1].parts.iter().any(|part| matches!(
            &part.content,
            TranscriptPartContent::Activity(TranscriptActivityContent::AssistantReplyLifecycle(
                TranscriptAssistantReplyLifecycle::Cancelled
            ))
        )));
        assert!(entries[1].parts.iter().any(|part| matches!(
            &part.content,
            TranscriptPartContent::Activity(TranscriptActivityContent::Answer(_))
        )));
    }

    #[test]
    fn folded_entry_stays_in_progress_until_the_whole_turn_terminalizes() {
        // Intermediate tool-call turns are in_progress; only the final turn is
        // completed. The live block must read InProgress throughout.
        let parts = vec![
            run(1, "user", "completed"),
            run(3, "assistant", "in_progress"),
            content_part_for(
                3,
                4,
                "tool_call",
                "assistant",
                serde_json::json!({ "name": "tools_search", "operation": {} }),
            ),
            run(5, "assistant", "in_progress"),
            run(7, "assistant", "completed"),
            content_part_for(
                7,
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
            content_part_for(
                3,
                4,
                "text",
                "assistant",
                serde_json::json!({ "text": "r1" }),
            ),
            run(5, "user", "completed"),
            content_part_for(5, 6, "text", "user", serde_json::json!({ "text": "b" })),
            run(7, "assistant", "completed"),
            content_part_for(
                7,
                8,
                "text",
                "assistant",
                serde_json::json!({ "text": "r2" }),
            ),
        ];
        let entries = parts_entries(&parts);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[2].id, TranscriptEntryId::StoredMessage(5));
        assert_eq!(entries[3].id, TranscriptEntryId::StoredMessage(7));
    }

    #[test]
    fn late_assistant_hook_stays_on_its_launch_run_and_runtime_ingress_stays_system() {
        let notification = |operation_id: &str| {
            serde_json::json!({
                "type": "system_notification",
                "operation_id": operation_id,
                "operation_kind": "shell",
                "status": "completed",
                "summary": "background work completed",
                "body": "background work completed"
            })
        };
        let parts = vec![
            run(1, "assistant", "completed"),
            content_part(
                2,
                "text",
                "assistant",
                serde_json::json!({ "text": "launched" }),
            ),
            run(3, "runtime", "completed"),
            content_part_for(
                3,
                4,
                "system_notification",
                "runtime",
                notification("external"),
            ),
            run(5, "assistant", "completed"),
            content_part_for(
                5,
                6,
                "text",
                "assistant",
                serde_json::json!({ "text": "later" }),
            ),
            // This row arrives after runs 3 and 5, but its durable owner is
            // still run 1. Physical adjacency must never relabel it System.
            content_part_for(
                1,
                7,
                "system_notification",
                "assistant",
                notification("proc_1"),
            ),
        ];

        let entries = parts_entries(&parts);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].role, Some(RunRole::Assistant));
        assert!(
            entries[0]
                .parts
                .iter()
                .any(|part| part.id == TranscriptContentId::StoredPart(7))
        );
        assert_eq!(entries[1].role, Some(RunRole::System));
        assert!(
            entries[1]
                .parts
                .iter()
                .any(|part| part.id == TranscriptContentId::StoredPart(4))
        );
        assert_eq!(entries[2].role, Some(RunRole::Assistant));
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
    fn completed_tool_call_uses_the_ephemeral_human_presentation() {
        let mut tool = content_part(
            2,
            "tool_call",
            "assistant",
            serde_json::json!({
                "name": "tools_search",
                "input": { "query": "model" },
                "tool_api_call": {
                    "function": "tools_search",
                    "arguments": {
                        "fields": [{
                            "name": "query",
                            "value": { "kind": "text", "value": "model" }
                        }]
                    }
                },
                "call_id": 73,
                "state": "completed",
                "output": {"payload": {"text": "raw result"}},
                "lifecycle": { "start_ms": 100, "end_ms": 200 }
            }),
        );
        tool.presentation = Some(agena_api::live::ToolHumanPresentationResource {
            title: "Search tools".to_owned(),
            summary: "Returned 3 matching tools.".to_owned(),
            blocks: vec![agena_domain::ViewBlock::Log {
                id: Some("result".to_owned()),
                stream: agena_domain::CommandOutputStream::Stdout,
                text: "Matching tools for model:\n- session.model".to_owned(),
            }],
        });
        let parts = vec![run(1, "assistant", "completed"), tool];

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
        assert!(operation.model_output.text.is_empty());
        assert_eq!(operation.blocks.len(), 1);
        assert_eq!(operation.title, "Search tools");
        assert_eq!(operation.summary, "Returned 3 matching tools.");
    }

    #[test]
    fn runtime_role_entries_render_with_explicit_system_identity() {
        let parts = vec![run(1, "runtime", "completed")];
        let entries = parts_entries(&parts);
        assert_eq!(entries[0].role, Some(RunRole::System));
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
            content_part_for(
                3,
                4,
                "text",
                "assistant",
                serde_json::json!({ "text": "b" }),
            ),
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
            presentation: None,
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
        assert_eq!(
            request.questions[0].question,
            "Choose whether this plan should move to active."
        );
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
        let reply = reply
            .as_ref()
            .expect("reply is recovered from the reply object");
        assert_eq!(reply.answers["decision"], ["approve"]);
    }

    #[test]
    fn interaction_request_id_is_only_exposed_for_pending_parts() {
        // `interaction_request_id_for_part` is the correlation key the app
        // uses to attach a live `UserInputPresentation` to an expanded pending
        // interaction part. It must return the request_id only while the part
        // is unanswered; once a reply lands the part is no longer interactive
        // and the id must be withheld so key routing can never target it.
        let pending = content_part(
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
                }
            }),
        );
        let entries = parts_entries(&[pending]);
        let part = &entries[0].parts[0];
        assert_eq!(
            interaction_request_id_for_part(part),
            Some("host-input:1:2:0")
        );

        let answered = content_part(
            3,
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
        let entries = parts_entries(&[answered]);
        let part = &entries[0].parts[0];
        assert_eq!(interaction_request_id_for_part(part), None);

        // Non-interaction parts never expose a request id.
        let text = content_part(4, "text", "assistant", serde_json::json!({ "text": "hi" }));
        let entries = parts_entries(&[text]);
        assert_eq!(interaction_request_id_for_part(&entries[0].parts[0]), None);
    }
}
