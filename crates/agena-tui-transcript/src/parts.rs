//! Durable parts → transcript entry projection for terminal presentation.
//!
//! The wire transcript is an ordered v2 part list (`SessionExecutionResource
//! .parts`). Each `run` marker is one turn/run:
//! it starts a [`TranscriptEntry`] whose `role`/`state`/`created_at` mirror the
//! marker, and every following content part (`run_id` backlinked) becomes a
//! part of that entry. `kind`/`role`/`state` are stable open-set strings, so
//! the mapping is defensive: well-known kinds project to the rich render-model
//! variants, and unknown or malformed parts fall back to a plain text part so
//! nothing in the transcript is silently dropped.
//!
//! This projection owns every value: parts arrive as JSON and are decoded into
//! the render model here, so entries are `'static`.

use agena_api::part::PartResource;
use agena_api::{
    part::{
        AttachmentPartResource, CommandReferencePartResource, ErrorPartResource,
        PartExecutionStatusResource, ReasoningPartResource, TextPartResource,
    },
    resource::{
        PartAttachment, PartAttachmentKind, PartAttachmentSource, PartCommandReference, RunRole,
        RunStatus, UserInputOption, UserInputQuestion, UserInputReply, UserInputReplyKind,
        UserInputRequest,
    },
};
use agena_domain::AssistantReplyId;
use agena_domain::TextSegmentActivity;
use agena_runtime_contracts::part_content::{
    CommandRefContent, CompactionContent, FileRefContent, HookContent, NoticeContent,
    PasteRefContent, SystemNotificationContent, TextContent, ThinkContent, ToolCallContent,
    attachment_source_from_file_ref, operation_from_tool_call,
};
use serde_json::Value;

use crate::{
    ToolCallView, TranscriptActivityContent, TranscriptAssistantReplyLifecycle,
    TranscriptContentId, TranscriptEntry, TranscriptEntryId, TranscriptEntryPart,
    TranscriptPartContent,
};

/// Client-owned disclosure state derived from bounded run windows.
#[derive(Debug, Clone)]
pub struct TranscriptFold {
    pub run_id: i64,
    pub run_ids: Vec<i64>,
    pub anchor_part_id: i64,
    pub hidden_count: u64,
    pub next_cursor: Option<String>,
}

pub fn folds_from_run_window(page: &agena_api::live::SessionPartsResource) -> Vec<TranscriptFold> {
    page.runs
        .iter()
        .filter_map(|run| {
            let hidden_count = run.part_count.saturating_sub(run.page.returned);
            if hidden_count == 0 {
                return None;
            }
            let anchor = page
                .parts
                .iter()
                .find(|p| p.run_id == Some(run.run_id) && p.part_id != run.run_id)
                .or_else(|| page.parts.iter().find(|p| p.run_id == Some(run.run_id)))?;
            Some(TranscriptFold {
                run_id: run.run_id,
                run_ids: vec![run.run_id],
                anchor_part_id: anchor.part_id,
                hidden_count,
                next_cursor: run.page.next_cursor.clone(),
            })
        })
        .collect()
}

/// Project an ordered v2 part list into transcript entries. Each `run` marker
/// starts an entry, except that consecutive assistant runs fold into a single
/// entry: a multi-round tool loop emits one run marker per model turn (tool
/// calls, then the final answer) and the user expects those to render as one
/// assistant block. Content ownership is resolved exclusively through
/// `run_id`, not physical row position: a background Hook may be appended much
/// later while still belonging to the assistant run that launched it.
/// Content parts whose referenced marker is absent are excluded: the current
/// wire contract has no owner reconstruction rule.
pub fn parts_entries(parts: &[PartResource]) -> Vec<TranscriptEntry<'static>> {
    parts_entries_with_folds(parts, &[])
}

/// Project ordered parts and client-owned disclosure state into
/// the same render model as the lossless projection. Fold markers are inserted
/// immediately before their first visible activity part; their omitted raw
/// prefix is fetched by the app when the marker is expanded.
pub fn parts_entries_with_folds(
    parts: &[PartResource],
    folds: &[TranscriptFold],
) -> Vec<TranscriptEntry<'static>> {
    // A committed checkpoint references the text stream used to create it.
    // Keep its original boundary for ownership, but present that summary only
    // through the compaction part, never again as an assistant answer.
    let checkpoint_resources = parts
        .iter()
        .filter(|part| part.content.get("compaction_through").is_some())
        .flat_map(text_resource_ids)
        .collect::<std::collections::BTreeSet<_>>();
    let summary_sources = parts
        .iter()
        .filter(|part| {
            part.role == "assistant" && matches!(part.kind.as_str(), "text" | "compaction")
        })
        .filter(|part| part.content.get("compaction_through").is_none())
        .filter(|part| {
            let resources = text_resource_ids(part);
            !resources.is_empty() && resources.iter().all(|id| checkpoint_resources.contains(id))
        })
        .map(|part| part.part_id)
        .collect::<std::collections::BTreeSet<_>>();
    let marker_ids = parts
        .iter()
        .filter(|part| part.is_message_boundary())
        .map(|part| part.part_id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut content_by_run = std::collections::BTreeMap::<i64, Vec<&PartResource>>::new();
    for part in parts {
        if !summary_sources.contains(&part.part_id)
            && let Some(run_id) = part.run_id
            && marker_ids.contains(&run_id)
        {
            content_by_run.entry(run_id).or_default().push(part);
        }
    }

    // Pending input belongs to the queue preview. A queued retry of an input
    // used by an earlier attempt remains in the conversation (queue_only=false).
    let mut markers = parts
        .iter()
        .filter(|part| part.is_message_boundary())
        .filter(|part| {
            !(part.role == "user"
                && (part.content.get("queue_only").and_then(Value::as_bool) == Some(true)
                    || part.run_state.as_deref().unwrap_or(&part.state) == "pending"))
        })
        .collect::<Vec<_>>();
    if markers.iter().any(|part| part.message_id.is_some()) {
        markers.sort_by_key(|part| (part.message_id.unwrap_or(part.part_id), part.part_id));
    }
    // Attach content only by its durable owner, after ordering the boundaries.
    // This is the same projection contract used by Web.
    let mut projected = Vec::new();
    for part in markers {
        let mut entry = run_marker_entry(part);
        if let Some(content) = content_by_run.remove(&part.part_id) {
            entry
                .parts
                .extend(content.into_iter().flat_map(entry_parts));
        }
        // Maintenance runs have their own content lifecycle, separate
        // from the assistant reply that they are displayed beneath.
        let is_reply = !summary_sources.contains(&part.part_id)
            && string_field(&part.content, "run_kind").as_deref() != Some("compaction");
        projected.push((entry, is_reply, part.message_id));
    }

    let mut entries: Vec<(TranscriptEntry<'static>, bool, Option<i64>)> = Vec::new();
    for (entry, is_reply, message_id) in projected {
        if let Some((previous, previous_is_reply, previous_message_id)) = entries.last_mut()
            && previous.role == Some(RunRole::Assistant)
            && entry.role == Some(RunRole::Assistant)
            && (message_id.is_none()
                || previous_message_id.is_none()
                || *previous_message_id == message_id)
        {
            // Fold adjacent assistant rounds only after exact ownership has
            // been resolved. Runtime/System ingress remains a hard boundary.
            if is_reply {
                previous.state = entry.state;
                previous.reply_id = entry.reply_id;
                *previous_is_reply = true;
            } else if !*previous_is_reply {
                previous.state = entry.state;
            }
            previous.parts.extend(entry.parts);
            continue;
        }
        entries.push((entry, is_reply, message_id));
    }
    let positions = parts
        .iter()
        .enumerate()
        .map(|(index, part)| (part.part_id, index))
        .collect::<std::collections::BTreeMap<_, _>>();
    for (entry, _, _) in &mut entries {
        // A run can resume after another assistant activity has started.
        // Once adjacent runs are folded, retain the durable content order.
        entry.parts.sort_by_key(|part| match part.id {
            TranscriptContentId::StoredPart(id) => {
                positions.get(&id).copied().unwrap_or(usize::MAX)
            }
            _ => usize::MAX,
        });
    }
    for fold in folds {
        let anchor = TranscriptContentId::StoredPart(fold.anchor_part_id);
        let Some((entry, _, _)) = entries
            .iter_mut()
            .find(|(entry, _, _)| entry.parts.iter().any(|part| part.id == anchor))
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
                headline: Default::default(),
                content: TranscriptPartContent::Activity(TranscriptActivityContent::Fold {
                    hidden_count: usize::try_from(fold.hidden_count).unwrap_or(usize::MAX),
                }),
            },
        );
    }
    entries
        .into_iter()
        .map(|(entry, is_reply, _)| finalize_run_entry(entry, is_reply))
        .collect()
}

fn is_compaction_checkpoint(part: &PartResource) -> bool {
    part.kind == "compaction"
        // Servers predating the typed projection sent checkpoints as text.
        || (part.kind == "text" && part.content.get("compaction_through").is_some())
}

fn text_resource_ids(part: &PartResource) -> Vec<agena_domain::ContentId> {
    part.content
        .get("resources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| serde_json::from_value::<agena_domain::ContentRef>(value.clone()).ok())
        .filter(|resource| resource.kind == agena_domain::ContentKind::Text)
        .map(|resource| resource.resource_id)
        .collect()
}

/// Whether any run marker in the part list has a non-terminal state. The app
/// uses this as a safety net to force a refresh after a terminal execution.
pub fn parts_have_non_terminal_runs(parts: &[PartResource]) -> bool {
    parts
        .iter()
        .filter(|part| part.is_message_boundary())
        .any(|marker| !message_status_is_terminal(message_status_from_string(&marker.state)))
}

fn run_marker_entry(marker: &PartResource) -> TranscriptEntry<'static> {
    TranscriptEntry {
        id: TranscriptEntryId::StoredMessage(marker.part_id),
        role: role_from_string(&marker.role),
        state: message_status_from_string(marker.run_state.as_deref().unwrap_or(&marker.state)),
        created_at: timestamp(marker.created_at_ms),
        reply_id: string_field(&marker.content, "reply_id")
            .and_then(|reply_id| reply_id.parse::<AssistantReplyId>().ok()),
        parts: Vec::new(),
    }
}

fn finalize_run_entry(
    mut entry: TranscriptEntry<'static>,
    is_reply: bool,
) -> TranscriptEntry<'static> {
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
        // Empty active/completed envelopes may show run status. Failure
        // details are always real server Parts, including in paged histories.
        let state = entry.state;
        let has_error = entry.parts.iter().any(|part| {
            matches!(
                part.content,
                TranscriptPartContent::Activity(TranscriptActivityContent::Error(_))
            )
        });
        if is_reply
            && (entry.parts.is_empty() || (state == RunStatus::Cancelled && !has_error))
            && let Some(lifecycle) = assistant_reply_lifecycle(state)
        {
            let marker_id = match entry.id {
                TranscriptEntryId::StoredMessage(id) => id,
                _ => 0,
            };
            // The reply's own identity is the lifecycle row's presentation
            // identity; only legacy markers without one fall back to the
            // marker part id.
            let lifecycle_id = match entry.reply_id {
                Some(reply_id) => TranscriptContentId::AssistantReplyLifecycle(reply_id),
                None => TranscriptContentId::StoredPart(marker_id),
            };
            entry.parts.push(TranscriptEntryPart {
                id: lifecycle_id,
                status: part_status_from_state(state),
                headline: Default::default(),
                content: TranscriptPartContent::Activity(
                    TranscriptActivityContent::AssistantReplyLifecycle(lifecycle),
                ),
            });
        }
    }
    entry
}

/// The summary the runtime already projected for this part. Presentation
/// data is carried by the part, so it must win over re-probing the content
/// JSON with client-side field guesses.
fn part_summary(part: &PartResource) -> Option<String> {
    part.summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Attachment kind as the part stores it. The runtime writes the resolved
/// kind; rebuilds that guess from the file name lose it.
fn attachment_kind_from_value(content: &serde_json::Value) -> Option<PartAttachmentKind> {
    match string_field(content, "kind")?.as_str() {
        "image" => Some(PartAttachmentKind::Image),
        "audio" => Some(PartAttachmentKind::Audio),
        "video" => Some(PartAttachmentKind::Video),
        "pdf" => Some(PartAttachmentKind::Pdf),
        _ => Some(PartAttachmentKind::File),
    }
}

fn composer_resource_attachment(resource: &agena_domain::ResourceActivity) -> PartAttachment {
    let kind = match resource.kind {
        agena_domain::ResourceKind::Image => PartAttachmentKind::Image,
        agena_domain::ResourceKind::Audio => PartAttachmentKind::Audio,
        agena_domain::ResourceKind::Video => PartAttachmentKind::Video,
        agena_domain::ResourceKind::Pdf => PartAttachmentKind::Pdf,
        agena_domain::ResourceKind::File
        | agena_domain::ResourceKind::Directory
        | agena_domain::ResourceKind::Url
        | agena_domain::ResourceKind::Artifact => PartAttachmentKind::File,
    };
    let (source, sha256) = match &resource.reference {
        agena_domain::ResourceReference::Artifact { sha256, uri } => (
            PartAttachmentSource::FileId {
                file_id: uri.clone(),
            },
            Some(sha256.clone()),
        ),
        agena_domain::ResourceReference::WorkspacePath { path } => {
            (PartAttachmentSource::LocalPath { path: path.clone() }, None)
        }
        agena_domain::ResourceReference::Url { url } => {
            (PartAttachmentSource::Url { url: url.clone() }, None)
        }
        agena_domain::ResourceReference::ProviderFile { file_id, .. } => (
            PartAttachmentSource::FileId {
                file_id: file_id.clone(),
            },
            None,
        ),
    };
    PartAttachment {
        kind,
        mime: resource.media_type.clone().unwrap_or_default(),
        source,
        filename: Some(resource.name.clone()),
        title: None,
        size_bytes: resource.size_bytes,
        sha256,
        width: resource.width,
        height: resource.height,
        duration_ms: resource.duration_ms,
        page_count: resource.page_count,
    }
}

fn entry_parts(part: &PartResource) -> Vec<TranscriptEntryPart<'static>> {
    let mut projected = vec![entry_part(part)];
    if part.role != "user" {
        return projected;
    }
    let Some(document) = part
        .content
        .get("document")
        .cloned()
        .and_then(|value| serde_json::from_value::<agena_domain::ComposerDocument>(value).ok())
    else {
        return projected;
    };
    for node in document.0 {
        let agena_domain::ComposerNode::Activity { activity } = node else {
            continue;
        };
        let content = match activity.payload {
            agena_domain::ActivityPayload::Resource(resource) => {
                TranscriptActivityContent::Attachment(AttachmentPartResource {
                    attachments: vec![composer_resource_attachment(&resource)],
                })
            }
            agena_domain::ActivityPayload::CommandReference(command) => {
                TranscriptActivityContent::CommandReference(CommandReferencePartResource {
                    commands: vec![PartCommandReference {
                        name: command.name,
                        description: command.description,
                        content_hash: command.content_hash,
                        source: command.source,
                        skill_id: command.skill_id,
                        aliases: command.aliases,
                    }],
                })
            }
            agena_domain::ActivityPayload::TextArtifact(artifact) => {
                TranscriptActivityContent::TextArtifact(Box::new(artifact))
            }
            _ => continue,
        };
        projected.push(TranscriptEntryPart {
            id: TranscriptContentId::Activity(activity.id),
            status: PartExecutionStatusResource::Completed,
            headline: Default::default(),
            content: TranscriptPartContent::Activity(content),
        });
    }
    projected
}

fn entry_part(part: &PartResource) -> TranscriptEntryPart<'static> {
    TranscriptEntryPart {
        id: TranscriptContentId::StoredPart(part.part_id),
        status: part_status_from_string(&part.state),
        headline: crate::TranscriptPartHeadline {
            title: part
                .presentation
                .as_ref()
                .map(|view| view.title.clone())
                .unwrap_or_default(),
            summary: part
                .presentation
                .as_ref()
                .map(|view| view.summary.as_str())
                .filter(|summary| !summary.trim().is_empty())
                .map(str::to_owned)
                .or_else(|| part_summary(part))
                .unwrap_or_default(),
        },
        content: part_content(part),
    }
}

fn part_content(part: &PartResource) -> TranscriptPartContent<'static> {
    if part.kind == "text"
        && part.role == "assistant"
        && let Some(refs) = &part.inline_ui
        && let Some(source) = part.content.get("text").and_then(Value::as_str)
        && !agena_domain::inline_reference_spans(source, !part_state_is_terminal(&part.state))
            .is_empty()
    {
        return TranscriptPartContent::InlineUi(Box::new(crate::inline_ui::InlineReply {
            source: source.to_owned(),
            documents: refs.clone(),
            streaming: !part_state_is_terminal(&part.state),
            ..Default::default()
        }));
    }
    if part.kind == "text" && is_compaction_checkpoint(part) {
        return match decode_part_content::<CompactionContent>(part) {
            Some(content) => TranscriptPartContent::Activity(
                TranscriptActivityContent::Compaction(Box::new(CompactionContent {
                    summary: part_summary(part).or(content.summary),
                    ..content
                })),
            ),
            None => fallback_text_part(part),
        };
    }
    // Every kind decodes through its canonical contract shape: no field is
    // guessed from raw JSON and no kind is re-encoded as another. Content that
    // cannot be decoded degrades to a readable text part, never silently
    // disappears.
    match part.kind.as_str() {
        "text" => {
            if part.role == "user"
                && let Some(document) = part.content.get("document").cloned().and_then(|value| {
                    serde_json::from_value::<agena_domain::ComposerDocument>(value).ok()
                })
            {
                return TranscriptPartContent::UserDocument(crate::TranscriptUserDocument {
                    nodes: document
                        .0
                        .into_iter()
                        .map(|node| match node {
                            agena_domain::ComposerNode::Text { text } => {
                                crate::TranscriptUserDocumentNode::Text { id: None, text }
                            }
                            agena_domain::ComposerNode::Activity { activity } => {
                                let (placeholder, style) = match activity.payload {
                                    agena_domain::ActivityPayload::Resource(resource) => (
                                        format!("[{}]", resource.name),
                                        crate::TranscriptUserActivityStyle::Resource,
                                    ),
                                    agena_domain::ActivityPayload::CommandReference(command) => (
                                        format!("[{}]", command.name),
                                        crate::TranscriptUserActivityStyle::Command,
                                    ),
                                    agena_domain::ActivityPayload::TextArtifact(artifact) => (
                                        format!(
                                            "[{}]",
                                            artifact.label.unwrap_or_else(|| "text".into())
                                        ),
                                        crate::TranscriptUserActivityStyle::TextArtifact,
                                    ),
                                    _ => (
                                        "[activity]".into(),
                                        crate::TranscriptUserActivityStyle::Other,
                                    ),
                                };
                                crate::TranscriptUserDocumentNode::Activity {
                                    id: activity.id,
                                    placeholder,
                                    style,
                                }
                            }
                        })
                        .collect(),
                });
            }
            let Some(content) = decode_part_content::<TextContent>(part) else {
                return fallback_text_part(part);
            };
            if part.role == "assistant"
                && (!content.text.trim().is_empty() || !content.resources.is_empty())
            {
                // Assistant reply text projects as an owned activity so it
                // renders as a toggleable part. [`finalize_run_entry`] promotes
                // the final non-tool-followed text part to `Answer` (default
                // expanded); earlier turn texts stay collapsible TextSegments.
                TranscriptPartContent::Activity(TranscriptActivityContent::TextSegment(Box::new(
                    TextSegmentActivity { text: content.text },
                )))
            } else {
                TranscriptPartContent::Text(TextPartResource {
                    text: content.text,
                    synthetic: content.synthetic,
                })
            }
        }
        "think" => {
            let Some(content) = decode_part_content::<ThinkContent>(part) else {
                return fallback_text_part(part);
            };
            TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(
                ReasoningPartResource {
                    summary: content.summary,
                    raw_content: content.raw,
                    encrypted_content: content.encrypted_content,
                },
            ))
        }
        "tool_call" => {
            if let Some(operation) = tool_call_view_from_part(part) {
                return TranscriptPartContent::Activity(TranscriptActivityContent::Operation(
                    Box::new(operation),
                ));
            }
            TranscriptPartContent::Text(TextPartResource {
                text: format!(
                    "invalid tool_call content: {}",
                    fallback_json_text(&part.content)
                ),
                synthetic: false,
            })
        }
        "file_ref" => {
            let Some(content) = decode_part_content::<FileRefContent>(part) else {
                return fallback_text_part(part);
            };
            // The canonical reader understands every source shape the composer
            // can persist, including a provider-bound upload or one that has
            // since degraded. Rebuilding the source from `path` alone dropped
            // those attachments from the transcript entirely.
            let source = PartAttachmentSource::from(&attachment_source_from_file_ref(&content));
            TranscriptPartContent::Activity(TranscriptActivityContent::Attachment(
                AttachmentPartResource {
                    attachments: vec![PartAttachment {
                        kind: attachment_kind_from_value(&part.content)
                            .unwrap_or(PartAttachmentKind::File),
                        mime: content.mime.clone().unwrap_or_default(),
                        source,
                        filename: content.name,
                        title: string_field(&part.content, "title"),
                        size_bytes: part
                            .content
                            .get("size_bytes")
                            .and_then(serde_json::Value::as_u64),
                        sha256: content.sha,
                        width: part
                            .content
                            .get("width")
                            .and_then(serde_json::Value::as_u64)
                            .map(|value| value as u32),
                        height: part
                            .content
                            .get("height")
                            .and_then(serde_json::Value::as_u64)
                            .map(|value| value as u32),
                        duration_ms: None,
                        page_count: None,
                    }],
                },
            ))
        }
        "paste_ref" => {
            let Some(content) = decode_part_content::<PasteRefContent>(part) else {
                return fallback_text_part(part);
            };
            TranscriptPartContent::Activity(TranscriptActivityContent::TextArtifact(Box::new(
                agena_domain::TextArtifactActivity {
                    text: content.text,
                    label: string_field(&part.content, "label"),
                    language: string_field(&part.content, "language"),
                },
            )))
        }
        "skill_ref" => {
            let Some(content) = decode_part_content::<CommandRefContent>(part) else {
                return fallback_text_part(part);
            };
            TranscriptPartContent::Activity(TranscriptActivityContent::CommandReference(
                CommandReferencePartResource {
                    commands: vec![PartCommandReference {
                        name: content.command.unwrap_or_default(),
                        description: string_field(&part.content, "description").unwrap_or_default(),
                        content_hash: string_field(&part.content, "content_hash")
                            .unwrap_or_default(),
                        source: string_field(&part.content, "source").unwrap_or_default(),
                        skill_id: string_field(&part.content, "skill_id"),
                        aliases: Vec::new(),
                    }],
                },
            ))
        }
        "notice" => match decode_part_content::<NoticeContent>(part) {
            Some(content) => TranscriptPartContent::Activity(TranscriptActivityContent::Notice(
                Box::new(NoticeContent {
                    summary: part_summary(part).unwrap_or(content.summary),
                    ..content
                }),
            )),
            None => fallback_text_part(part),
        },
        "hook" => match decode_part_content::<HookContent>(part) {
            Some(mut content) => {
                if content.hook.is_empty() {
                    content.hook =
                        string_field(&part.content, "kind").unwrap_or_else(|| part.kind.clone());
                }
                TranscriptPartContent::Activity(TranscriptActivityContent::Hook(Box::new(
                    HookContent {
                        summary: part_summary(part).unwrap_or(content.summary),
                        ..content
                    },
                )))
            }
            None => fallback_text_part(part),
        },
        "compaction" => match decode_part_content::<CompactionContent>(part) {
            Some(content) => TranscriptPartContent::Activity(
                TranscriptActivityContent::Compaction(Box::new(CompactionContent {
                    summary: part_summary(part).or(content.summary),
                    ..content
                })),
            ),
            None => fallback_text_part(part),
        },
        // A background-operation completion/event delivered to the model as a
        // System message (the agena analog of Claude's task-notification chip).
        "system_notification" => match decode_part_content::<SystemNotificationContent>(part) {
            Some(content) => {
                TranscriptPartContent::Activity(TranscriptActivityContent::SystemNotification(
                    Box::new(SystemNotificationContent {
                        summary: part_summary(part).unwrap_or(content.summary),
                        ..content
                    }),
                ))
            }
            None => fallback_text_part(part),
        },
        "error" => match serde_json::from_value::<ErrorPartResource>(part.content.clone()) {
            Ok(error) => TranscriptPartContent::Activity(TranscriptActivityContent::Error(error)),
            Err(error) => {
                tracing::warn!(
                    part_id = part.part_id,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to decode a persisted transcript error part",
                        &error,
                    ),
                    "rendering malformed transcript error content as plain text"
                );
                TranscriptPartContent::Text(TextPartResource {
                    text: string_field(&part.content, "message")
                        .or_else(|| string_field(&part.content, "summary"))
                        .unwrap_or_else(|| fallback_json_text(&part.content)),
                    synthetic: false,
                })
            }
        },
        _ => fallback_text_part(part),
    }
}

/// Decode one part's content through its canonical contract shape. A part whose
/// content cannot be decoded is reported and degraded by the caller.
fn decode_part_content<T>(part: &PartResource) -> Option<T>
where
    T: for<'de> TryFrom<&'de serde_json::Value, Error = String>,
{
    match T::try_from(&part.content) {
        Ok(content) => Some(content),
        Err(diagnostic) => {
            tracing::warn!(
                part_id = part.part_id,
                kind = part.kind.as_str(),
                diagnostic,
                "malformed persisted transcript part content"
            );
            None
        }
    }
}

/// A readable stand-in for content that could not be decoded: the raw payload
/// stays visible instead of the part disappearing from the transcript.
fn fallback_text_part(part: &PartResource) -> TranscriptPartContent<'static> {
    TranscriptPartContent::Text(TextPartResource {
        text: fallback_json_text(&part.content),
        synthetic: false,
    })
}

/// Decode the canonical `tool_call` facts and keep the human presentation in
/// its native `ViewBlock` form. No API envelope or flattened mirror is built.
fn tool_call_view_from_part(part: &PartResource) -> Option<ToolCallView> {
    // Transcript snapshots intentionally omit collapsed tool detail sections.
    // The canonical durable decoder requires `input`, so provide only the
    // empty structural default needed to decode a presentation-only snapshot;
    // the actual input is inserted when the TUI's lazy section request lands.
    let mut projected_content = part.content.clone();
    if let Some(object) = projected_content.as_object_mut() {
        object
            .entry("input".to_owned())
            .or_insert_with(|| serde_json::json!({}));
    }
    let content = match ToolCallContent::try_from(&projected_content) {
        Ok(content) => content,
        Err(error) => {
            tracing::warn!(
                part_id = part.part_id,
                diagnostic = %error,
                "malformed persisted tool call could not be rendered as an operation"
            );
            return None;
        }
    };
    let operation = operation_from_tool_call(&content);
    let mut view = ToolCallView::from_operation(operation, part.presentation.clone());
    let loaded = |section, field| {
        part.content.get(field).is_some()
            || part.sections.iter().any(|loaded| loaded.section == section)
    };
    view.input_loaded = loaded(agena_api::live::ToolDetailSection::Input, "input");
    view.output_loaded = loaded(agena_api::live::ToolDetailSection::Output, "output");
    view.metadata_loaded = loaded(agena_api::live::ToolDetailSection::Metadata, "metadata");
    Some(view)
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
                        custom: option.custom,
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

/// Whether a wire run/part state string is terminal.
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

fn assistant_reply_lifecycle(state: RunStatus) -> Option<TranscriptAssistantReplyLifecycle> {
    match state {
        RunStatus::Failed
        | RunStatus::PolicyDenied
        | RunStatus::UserDeclined
        | RunStatus::CapabilityUnavailable
        | RunStatus::ToolUnavailable => None,
        RunStatus::Cancelled => Some(TranscriptAssistantReplyLifecycle::Cancelled),
        RunStatus::Completed => Some(TranscriptAssistantReplyLifecycle::Completed),
        RunStatus::Pending | RunStatus::InProgress => {
            Some(TranscriptAssistantReplyLifecycle::Running)
        }
    }
}

#[cfg(test)]
mod compaction_tests {
    use super::*;
    use serde_json::json;

    fn part(id: i64, run_id: i64, kind: &str, content: Value) -> PartResource {
        PartResource {
            part_id: id,
            kind: kind.into(),
            role: "assistant".into(),
            state: "completed".into(),
            run_state: (id == run_id).then(|| "completed".into()),
            run_id: Some(run_id),
            message_id: Some(1),
            message_boundary: id == run_id,
            content,
            ..Default::default()
        }
    }

    #[test]
    fn committed_compaction_has_one_part_and_preserves_the_original_answer() {
        for (checkpoint_kind, summary_kind) in [
            ("compaction", "text"),
            ("text", "text"),
            ("compaction", "compaction"),
        ] {
            let resource = agena_domain::ContentRef {
                resource_id: agena_domain::ContentId::new(),
                kind: agena_domain::ContentKind::Text,
            };
            let parts = vec![
                part(2, 2, "text", json!({"text": "original answer"})),
                part(
                    3,
                    3,
                    summary_kind,
                    json!({"text": "", "resources": [resource]}),
                ),
                part(
                    4,
                    3,
                    checkpoint_kind,
                    json!({"resources": [resource], "compaction_through": 2}),
                ),
            ];
            let entries = parts_entries(&parts);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].parts.len(), 2, "{checkpoint_kind}");
            assert!(matches!(
                &entries[0].parts[0].content,
                TranscriptPartContent::Activity(TranscriptActivityContent::Answer(body))
                    if body.text == "original answer"
            ));
            assert_eq!(entries[0].parts[1].id, TranscriptContentId::StoredPart(4));
            assert!(matches!(
                entries[0].parts[1].content,
                TranscriptPartContent::Activity(TranscriptActivityContent::Compaction(_))
            ));
            let rendered = crate::render_entry_detailed(
                &entries[0],
                120,
                &agena_tui::i18n::I18n::english(),
                &crate::TranscriptDetailDefaults::default(),
                &Default::default(),
            );
            let marker = rendered
                .nodes
                .iter()
                .find(|node| {
                    matches!(
                        node.key,
                        crate::TranscriptNodeKey::Activity {
                            content_id: TranscriptContentId::StoredPart(4),
                            ..
                        }
                    )
                })
                .unwrap();
            assert!(
                marker.toggleable,
                "a resource-only checkpoint must be expandable"
            );
            assert!(
                rendered
                    .lines
                    .iter()
                    .any(|line| line.text.contains("Compaction"))
            );
        }
    }

    #[test]
    fn compaction_summary_is_not_an_answer_when_its_original_boundary_is_the_source() {
        let resource = agena_domain::ContentRef {
            resource_id: agena_domain::ContentId::new(),
            kind: agena_domain::ContentKind::Text,
        };
        let entries = parts_entries(&[
            part(3, 3, "text", json!({"text": "", "resources": [resource]})),
            part(
                4,
                3,
                "compaction",
                json!({"resources": [resource], "compaction_through": 2}),
            ),
        ]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].parts.len(), 1);
        assert!(matches!(
            entries[0].parts[0].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::Compaction(_))
        ));
    }
}
