//! Canonical transcript snapshot projection for terminal presentation.
//!
//! This is intentionally a projection from `TurnSnapshot` / `AssistantReplySnapshot`
//! / `ContentNode` rather than an adapter through public message resources.
//! Stable domain identities therefore reach navigation and rendering without
//! fabricated integer message or part ids.

use crate::TranscriptActivityPresentation;
use agena_api::{
    message_part::{MessageTextPartResource, PartExecutionStatusResource},
    resource::{MessageRole, MessageStatus},
};
use agena_domain::{
    ActivityNode, ActivityPayload, ActivityState, AssistantReplyStatus, ComposerDocument,
    ComposerNode, ContentDocument, ContentNode, ResourceKind, ResourceReference,
    TextSegmentActivity, TranscriptSnapshot,
};
use chrono::{DateTime, Utc};

use crate::{
    TranscriptActivityContent, TranscriptAssistantReplyLifecycle, TranscriptContentId,
    TranscriptEntry, TranscriptEntryId, TranscriptEntryPart, TranscriptPartContent,
    TranscriptUserActivityStyle, TranscriptUserDocument, TranscriptUserDocumentNode,
};

pub fn transcript_entries<'a>(snapshot: &'a TranscriptSnapshot) -> Vec<TranscriptEntry<'a>> {
    let mut entries = Vec::with_capacity(
        snapshot
            .turns
            .len()
            .saturating_mul(2)
            .saturating_add(snapshot.session_activities.len()),
    );
    // Session activities (hook runs, background notices, …) are interleaved
    // into the turn timeline by their actual occurrence time instead of being
    // appended after every turn. The sort is stable, so activities recorded in
    // the same drain keep their original order.
    let mut activities = snapshot.session_activities.iter().collect::<Vec<_>>();
    activities.sort_by_key(|activity| activity.lifecycle.started_at_ms);

    let mut activity_index = 0;
    let mut pending = Vec::<&'a ActivityNode>::new();
    for turn in &snapshot.turns {
        // Activities that began before the turn's user input are placed ahead
        // of the turn (for example a session hook that fired first).
        while activity_index < activities.len()
            && activities[activity_index].lifecycle.started_at_ms < turn.created_at_ms
        {
            pending.push(activities[activity_index]);
            activity_index += 1;
        }
        entries.extend(pending.drain(..).map(session_activity_entry));
        if !turn.input.is_empty() {
            entries.push(user_document_entry(
                turn.id,
                turn.created_at_ms,
                &turn.input,
            ));
        }
        // Activities that began while the assistant was composing (before the
        // reply finished) render between the user input and the reply; the
        // finished timestamp falls back to the reply creation time while the
        // reply is still streaming.
        let reply_boundary_ms = turn.reply.finished_at_ms.unwrap_or(turn.reply.created_at_ms);
        while activity_index < activities.len()
            && activities[activity_index].lifecycle.started_at_ms <= reply_boundary_ms
        {
            pending.push(activities[activity_index]);
            activity_index += 1;
        }
        entries.extend(pending.drain(..).map(session_activity_entry));
        entries.push(assistant_reply_document_entry(
            turn.reply.id,
            assistant_reply_status(turn.reply.status),
            turn.reply.created_at_ms,
            &turn.reply.content,
            turn.reply.failure.clone(),
        ));
    }
    // Anything still pending started after the last turn's reply; keep it at
    // the end of the timeline in timestamp order.
    entries.extend(
        activities[activity_index..]
            .iter()
            .map(|activity| session_activity_entry(activity)),
    );
    entries
}

fn session_activity_entry<'a>(activity: &'a ActivityNode) -> TranscriptEntry<'a> {
    TranscriptEntry {
        id: TranscriptEntryId::SessionActivity(activity.id),
        role: None,
        state: activity_entry_status(activity.state),
        created_at: timestamp(activity.lifecycle.started_at_ms),
        parts: vec![activity_entry_part(activity)],
    }
}

fn user_document_entry(
    turn_id: agena_domain::TurnId,
    created_at_ms: i64,
    document: &ContentDocument,
) -> TranscriptEntry<'_> {
    let mut parts = document
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ContentNode::Activity { activity } => Some(activity_entry_part(activity)),
            ContentNode::Text { .. } => None,
        })
        .collect::<Vec<_>>();
    let nodes = document
        .nodes()
        .iter()
        .map(|node| match node {
            ContentNode::Text { segment } => TranscriptUserDocumentNode::Text {
                id: Some(segment.id),
                text: segment.text.clone(),
            },
            ContentNode::Activity { activity } => TranscriptUserDocumentNode::Activity {
                id: activity.id,
                placeholder: user_activity_placeholder(&activity.payload),
                style: user_activity_style(&activity.payload),
            },
        })
        .collect::<Vec<_>>();
    if !nodes.is_empty() {
        parts.push(TranscriptEntryPart {
            id: TranscriptContentId::TurnDocument(turn_id),
            status: PartExecutionStatusResource::Completed,
            content: TranscriptPartContent::UserDocument(TranscriptUserDocument { nodes }),
        });
    }
    TranscriptEntry {
        id: TranscriptEntryId::TurnInput(turn_id),
        role: Some(MessageRole::User),
        state: MessageStatus::Completed,
        created_at: timestamp(created_at_ms),
        parts,
    }
}

fn assistant_reply_document_entry(
    reply_id: agena_domain::AssistantReplyId,
    state: MessageStatus,
    created_at_ms: i64,
    document: &ContentDocument,
    failure: Option<agena_failure::UserProblem>,
) -> TranscriptEntry<'_> {
    let mut parts = assistant_reply_document_parts(document, state);
    // A reply-level failure is now persisted as a durable Error Activity in
    // the reply content (like a failed tool call), so it renders through the
    // shared canonical Activity path. Only fall back to the legacy
    // "Response failed" lifecycle row when no durable Error Activity exists
    // (for example a reply that failed before this projection was written, or
    // a failure remembered in the terminal after a recovery).
    let has_error_activity = document.nodes().iter().any(|node| {
        matches!(
            node,
            ContentNode::Activity { activity }
                if matches!(activity.payload, ActivityPayload::Error(_))
        )
    });
    let failure_shown_by_activity = matches!(state, MessageStatus::Failed) && has_error_activity;
    if document.is_empty()
        || (assistant_reply_state_requires_outcome(state) && !failure_shown_by_activity)
        || (failure.is_some() && !has_error_activity)
    {
        parts.push(assistant_reply_lifecycle_part(reply_id, state, failure));
    }
    TranscriptEntry {
        id: TranscriptEntryId::AssistantReply(reply_id),
        role: Some(MessageRole::Assistant),
        state,
        created_at: timestamp(created_at_ms),
        parts,
    }
}

fn assistant_reply_document_parts(
    document: &ContentDocument,
    state: MessageStatus,
) -> Vec<TranscriptEntryPart<'_>> {
    let nodes = document.nodes();
    // One reply can span many assistant messages: an opening paragraph, tool
    // calls, interstitial text, and a closing summary. Only the final body
    // segment is the actual answer and renders inline as plain text; every
    // earlier body segment is an intermediate step and projects as a
    // collapsible TextSegment activity so a long tool run reads as a stack of
    // blocks with a single visible reply at the end.
    //
    // "Final" is only knowable once the reply is done. Promoting whatever
    // body segment happens to be last while the reply is still streaming
    // would demote it back to an Activity the moment the document grows
    // (later text, or a tool call proving more model turns will follow) —
    // the jarring "plain text first, Activity later" flip. So:
    // * Completed replies promote exactly the last body segment to inline.
    // * In-progress replies that already issued a tool call keep every body
    //   segment as a collapsible TextSegment Activity: the tool call proves
    //   the reply continues, so no body segment is final yet.
    // * In-progress replies that have not issued a tool call keep the live
    //   body segment inline so a plain question/answer still streams inline.
    let final_text_index = if matches!(state, MessageStatus::Completed) {
        nodes.iter().rposition(is_body_text_node)
    } else if nodes.iter().any(is_tool_call_node) {
        None
    } else {
        nodes.iter().rposition(is_body_text_node)
    };
    nodes
        .iter()
        .enumerate()
        .map(|(index, node)| match node {
            ContentNode::Text { segment } => {
                let id = TranscriptContentId::Text(segment.id);
                if Some(index) == final_text_index {
                    TranscriptEntryPart {
                        id,
                        status: PartExecutionStatusResource::Completed,
                        content: TranscriptPartContent::Text(MessageTextPartResource {
                            text: segment.text.clone(),
                            synthetic: false,
                        }),
                    }
                } else {
                    // Intermediate body segments project as collapsible
                    // TextSegment activities: the owned render-model variant
                    // carries the synthesized payload without borrowing a
                    // function-local value.
                    TranscriptEntryPart {
                        id,
                        status: PartExecutionStatusResource::Completed,
                        content: TranscriptPartContent::Activity(
                            TranscriptActivityContent::TextSegment(Box::new(TextSegmentActivity {
                                text: segment.text.clone(),
                            })),
                        ),
                    }
                }
            }
            ContentNode::Activity { activity }
                if matches!(activity.payload, ActivityPayload::TextSegment(_)) =>
            {
                // A legacy persisted TextSegment that happens to be the final
                // body renders as plain text (the answer); older intermediates
                // keep their canonical Activity projection.
                let id = TranscriptContentId::Activity(activity.id);
                if Some(index) == final_text_index {
                    let text = match &activity.payload {
                        ActivityPayload::TextSegment(segment) => segment.text.clone(),
                        _ => String::new(),
                    };
                    TranscriptEntryPart {
                        id,
                        status: PartExecutionStatusResource::Completed,
                        content: TranscriptPartContent::Text(MessageTextPartResource {
                            text,
                            synthetic: false,
                        }),
                    }
                } else {
                    activity_entry_part(activity)
                }
            }
            ContentNode::Activity { activity } => activity_entry_part(activity),
        })
        .collect()
}

fn is_body_text_node(node: &ContentNode) -> bool {
    match node {
        ContentNode::Text { .. } => true,
        ContentNode::Activity { activity } => {
            matches!(activity.payload, ActivityPayload::TextSegment(_))
        }
    }
}

fn is_tool_call_node(node: &ContentNode) -> bool {
    matches!(
        node,
        ContentNode::Activity { activity }
            if matches!(activity.payload, ActivityPayload::Operation(_))
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

fn assistant_reply_lifecycle_part(
    response_id: agena_domain::AssistantReplyId,
    state: MessageStatus,
    failure: Option<agena_failure::UserProblem>,
) -> TranscriptEntryPart<'static> {
    let (status, lifecycle) = match state {
        MessageStatus::Pending | MessageStatus::InProgress => (
            PartExecutionStatusResource::InProgress,
            TranscriptAssistantReplyLifecycle::Running,
        ),
        MessageStatus::Completed if failure.is_some() => (
            PartExecutionStatusResource::Failed,
            TranscriptAssistantReplyLifecycle::Failed { problem: failure },
        ),
        MessageStatus::Completed => (
            PartExecutionStatusResource::Completed,
            TranscriptAssistantReplyLifecycle::Completed,
        ),
        MessageStatus::Cancelled => (
            PartExecutionStatusResource::Cancelled,
            TranscriptAssistantReplyLifecycle::Cancelled,
        ),
        MessageStatus::Failed
        | MessageStatus::PolicyDenied
        | MessageStatus::UserDeclined
        | MessageStatus::CapabilityUnavailable
        | MessageStatus::ToolUnavailable => (
            PartExecutionStatusResource::Failed,
            TranscriptAssistantReplyLifecycle::Failed { problem: failure },
        ),
    };
    TranscriptEntryPart {
        id: TranscriptContentId::AssistantReplyLifecycle(response_id),
        status,
        content: TranscriptPartContent::Activity(
            TranscriptActivityContent::AssistantReplyLifecycle(lifecycle),
        ),
    }
}

pub fn pending_user_entry<'a>(
    pending_id: u64,
    confirmed: bool,
    document: &'a ComposerDocument,
) -> TranscriptEntry<'a> {
    let mut parts = document
        .0
        .iter()
        .filter_map(|node| match node {
            ComposerNode::Activity { activity } => Some(TranscriptEntryPart {
                id: TranscriptContentId::Activity(activity.id),
                status: PartExecutionStatusResource::Completed,
                content: TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(
                    &activity.payload,
                )),
            }),
            ComposerNode::Text { .. } => None,
        })
        .collect::<Vec<_>>();
    let nodes = document
        .0
        .iter()
        .map(|node| match node {
            ComposerNode::Text { text } => TranscriptUserDocumentNode::Text {
                id: None,
                text: text.clone(),
            },
            ComposerNode::Activity { activity } => TranscriptUserDocumentNode::Activity {
                id: activity.id,
                placeholder: user_activity_placeholder(&activity.payload),
                style: user_activity_style(&activity.payload),
            },
        })
        .collect::<Vec<_>>();
    if !nodes.is_empty() {
        parts.push(TranscriptEntryPart {
            id: TranscriptContentId::PendingDocument(pending_id),
            status: PartExecutionStatusResource::Completed,
            content: TranscriptPartContent::UserDocument(TranscriptUserDocument { nodes }),
        });
    }
    TranscriptEntry {
        id: TranscriptEntryId::PendingTurn(pending_id),
        role: Some(MessageRole::User),
        state: if confirmed {
            MessageStatus::Completed
        } else {
            MessageStatus::InProgress
        },
        created_at: DateTime::UNIX_EPOCH,
        parts,
    }
}

const fn user_activity_style(payload: &ActivityPayload) -> TranscriptUserActivityStyle {
    match payload {
        ActivityPayload::Resource(_) => TranscriptUserActivityStyle::Resource,
        ActivityPayload::SkillReference(_) => TranscriptUserActivityStyle::Skill,
        ActivityPayload::TextArtifact(_) => TranscriptUserActivityStyle::TextArtifact,
        _ => TranscriptUserActivityStyle::Other,
    }
}

fn user_activity_placeholder(payload: &ActivityPayload) -> String {
    match payload {
        ActivityPayload::Resource(resource) => {
            let kind = if resource.kind == ResourceKind::Directory {
                "folder"
            } else {
                "file"
            };
            format!("[{kind}: {}]", resource.name)
        }
        ActivityPayload::SkillReference(skill) => format!("[Skill: {}]", skill.name),
        ActivityPayload::TextArtifact(artifact) => format!(
            "[{}]",
            crate::text::text_artifact_display_label(
                artifact.text.as_str(),
                artifact.label.as_deref(),
            )
        ),
        _ => {
            let (_, title, _, _) = activity_presentation(payload);
            format!("[{title}]")
        }
    }
}

fn activity_entry_part<'a>(activity: &'a ActivityNode) -> TranscriptEntryPart<'a> {
    let (_schema, title, summary, problem) = activity_presentation(&activity.payload);
    let _generic = TranscriptActivityPresentation {
        title,
        summary,
        problem,
    };
    TranscriptEntryPart {
        id: TranscriptContentId::Activity(activity.id),
        status: activity_status(activity.state),
        content: TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(
            &activity.payload,
        )),
    }
}

/// Whether a compact tool summary is an approval/authorization-phase sentence
/// rather than a real tool result.
///
/// The runtime writes "Awaiting approval · <reason>" onto an Operation while it
/// blocks on permission, then records the user's decision ("Permission allowed
/// once", "Permission denied always", …) as the Operation summary when the
/// reply is persisted. Both are transcript prose about the permission gate, not
/// tool output, and must never be rendered as a fake "Output" result.
pub(crate) fn is_authorization_phase_summary(summary: &str) -> bool {
    let normalized = summary.trim().to_ascii_lowercase();
    normalized.starts_with("awaiting approval")
        || normalized.starts_with("awaiting permission")
        || normalized.starts_with("awaiting user approval")
        || normalized.starts_with("permission allowed")
        || normalized.starts_with("permission denied")
        || normalized.starts_with("permission auto-approved")
}

pub(crate) fn activity_presentation(
    payload: &ActivityPayload,
) -> (String, String, String, Option<agena_failure::UserProblem>) {
    match payload {
        ActivityPayload::Resource(resource) => (
            "resource".to_owned(),
            resource.name.clone(),
            match &resource.reference {
                ResourceReference::Artifact { uri, .. } => uri.clone(),
                ResourceReference::WorkspacePath { path } => path.clone(),
                ResourceReference::Url { url } => url.clone(),
                ResourceReference::ProviderFile { file_id, .. } => file_id.clone(),
            },
            None,
        ),
        ActivityPayload::SkillReference(skill) => (
            "skill_reference".to_owned(),
            format!("Skill: {}", skill.name),
            skill.description.clone(),
            None,
        ),
        ActivityPayload::TextArtifact(artifact) => (
            "text_artifact".to_owned(),
            artifact
                .label
                .clone()
                .unwrap_or_else(|| "Pasted text".to_owned()),
            // The full pasted text is the expandable body; the collapsed
            // headline is width-bounded by the renderer, so the expanded
            // activity shows every character with no truncation marker.
            artifact.text.clone(),
            None,
        ),
        ActivityPayload::Reasoning(reasoning) => (
            "reasoning".to_owned(),
            "Thinking".to_owned(),
            // The collapsed headline shows the first line as a natural preview;
            // the full thought trail is rendered verbatim when expanded. Never
            // ellipsize reasoning with a truncation marker.
            reasoning_first_line(reasoning.content.preferred_text().as_str()),
            None,
        ),
        ActivityPayload::TextSegment(segment) => (
            "text_segment".to_owned(),
            "Text".to_owned(),
            // The full segment is the expandable body; the collapsed headline
            // truncates it to the available width. Styled like normal body
            // text, not like thinking's muted trail.
            segment.text.clone(),
            None,
        ),
        ActivityPayload::Operation(operation) => (
            "operation".to_owned(),
            operation_activity_title(operation),
            // An approval/authorization-phase summary is permission transcript
            // prose, never a tool result. Blank it so it cannot surface as the
            // collapsed header or the Output fallback projection; the approval
            // decision stays visible in the Permissions section.
            if is_authorization_phase_summary(operation.summary.as_str()) {
                String::new()
            } else {
                operation.summary.clone()
            },
            operation.error.as_ref().map(|error| error.problem.clone()),
        ),
        ActivityPayload::Interaction(interaction) => match interaction {
            agena_domain::InteractionActivity::UserInput { request, .. } => (
                "user_input".to_owned(),
                "User input requested".to_owned(),
                request
                    .questions
                    .iter()
                    .map(|question| question.question.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                None,
            ),
        },
        ActivityPayload::Error(error) => (
            "error".to_owned(),
            "Error".to_owned(),
            error.problem.user.fallback.clone(),
            Some(error.problem.clone()),
        ),
        ActivityPayload::Notice(notice) => (
            "notice".to_owned(),
            "Notice".to_owned(),
            notice.summary.clone(),
            None,
        ),
    }
}

/// Collapsed preview for a reasoning Activity. Uses the first non-empty line
/// so the headline is a natural preview and the full reasoning body is never
/// marked as truncated — the expanded detail renders it verbatim.
fn reasoning_first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Thinking")
        .to_owned()
}

fn operation_activity_title(operation: &agena_domain::OperationActivity) -> String {
    // The activity headline is the composed tool title the runtime produced:
    // "fs.read · Read README.md", "tools.list · List tools · 2/133". Fall
    // back to the direct execution-tool name only when no title was generated
    // yet (very early streaming or a malformed call).
    let title = operation.title.trim();
    if title.is_empty() {
        operation.invocation.name.clone()
    } else {
        title.to_owned()
    }
}
fn timestamp(value: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(value).unwrap_or(DateTime::UNIX_EPOCH)
}

const fn activity_status(state: ActivityState) -> PartExecutionStatusResource {
    match state {
        ActivityState::Pending => PartExecutionStatusResource::Pending,
        ActivityState::InProgress => PartExecutionStatusResource::InProgress,
        ActivityState::Completed => PartExecutionStatusResource::Completed,
        ActivityState::Failed => PartExecutionStatusResource::Failed,
        ActivityState::Cancelled => PartExecutionStatusResource::Cancelled,
    }
}

const fn activity_entry_status(state: ActivityState) -> MessageStatus {
    match state {
        ActivityState::Pending => MessageStatus::Pending,
        ActivityState::InProgress => MessageStatus::InProgress,
        ActivityState::Completed => MessageStatus::Completed,
        ActivityState::Failed => MessageStatus::Failed,
        ActivityState::Cancelled => MessageStatus::Cancelled,
    }
}

const fn assistant_reply_status(status: AssistantReplyStatus) -> MessageStatus {
    match status {
        AssistantReplyStatus::Pending => MessageStatus::Pending,
        AssistantReplyStatus::InProgress => MessageStatus::InProgress,
        AssistantReplyStatus::Completed => MessageStatus::Completed,
        AssistantReplyStatus::Failed => MessageStatus::Failed,
        AssistantReplyStatus::Cancelled => MessageStatus::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use agena_domain::{
        ActivityActor, ActivityLifecycle, ActivityOwner, ActivityProvenance, ContentPosition,
        OperationActivity, OperationActivityError, OperationAuthorization, OperationPermission,
        PermissionAction, PermissionReply, PermissionReplyKind, PermissionRequest,
        ResourceActivity, SkillReferenceActivity, StructuredObject, ToolCallId, ToolInvocation,
    };
    use agena_failure::{
        Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility,
        RecoveryDirective, RetryDirective, UserPresentation,
    };

    use super::*;

    #[test]
    fn text_artifact_expansion_does_not_mutate_user_document_body() {
        let turn_id = agena_domain::TurnId::new();
        let activity_id = agena_domain::ActivityId::new();
        let pasted = format!(
            "The quick brown fox jumps over the lazy dog. {}",
            "x".repeat(2_000)
        );
        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::TurnInput { turn_id },
            actor: ActivityActor::User,
            state: ActivityState::Completed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::TextArtifact(agena_domain::TextArtifactActivity {
                text: pasted.clone(),
                language: None,
                label: Some(format!("paste {} chars", pasted.chars().count())),
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry = user_document_entry(turn_id, 1, &document);
        let defaults = crate::TranscriptDetailDefaults {
            activity_expanded: false,
        };
        let activity_key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::TurnInput(turn_id),
            content_id: TranscriptContentId::Activity(activity_id),
        };
        let collapsed = crate::render_entry_detailed(
            &entry,
            80,
            &agena_tui::i18n::I18n::english(),
            defaults,
            &Default::default(),
        );
        let expanded = crate::render_entry_detailed(
            &entry,
            80,
            &agena_tui::i18n::I18n::english(),
            defaults,
            &std::collections::BTreeMap::from([(activity_key, true)]),
        );
        let body_of = |rendered: &crate::renderer::RenderedMessageBlock| -> Vec<String> {
            rendered
                .lines
                .iter()
                .filter(|line| !line.text.starts_with("user"))
                .map(|line| line.text.clone())
                .collect::<Vec<_>>()
        };
        let collapsed_body = body_of(&collapsed);
        let expanded_body = body_of(&expanded);
        let placeholder_line = |lines: &[String]| -> String {
            lines
                .iter()
                .find(|line| line.contains('[') && line.contains("chars]"))
                .cloned()
                .expect("user document placeholder body row must render")
        };
        let collapsed_placeholder = placeholder_line(&collapsed_body);
        let expanded_placeholder = placeholder_line(&expanded_body);
        assert_eq!(
            collapsed_placeholder, expanded_placeholder,
            "expanding a TextArtifact must not change the user message body"
        );
        // The placeholder is the first 12 chars of the pasted text plus the
        // remaining character count, never a generic `paste N chars` label.
        let placeholder = format!(
            "[The quick br\u{2026} +{} chars]",
            pasted.chars().count() - 12
        );
        assert!(
            collapsed_placeholder.contains(placeholder.as_str()),
            "placeholder should be `{placeholder}` but was `{collapsed_placeholder}`"
        );

        // The expanded activity renders the complete pasted content with no
        // truncation marker, while the collapsed row keeps it hidden.
        let expanded_text = expanded_body.join("\n");
        assert!(
            expanded_text.contains("The quick brown fox jumps over the lazy dog.")
                && expanded_text.contains(&"x".repeat(2_000)),
            "expanded activity must contain the complete pasted content"
        );
        assert!(
            !expanded_text.contains("truncated"),
            "expanded activity must not be truncated"
        );
        let collapsed_text = collapsed_body.join("\n");
        assert!(
            !collapsed_text.contains(&"x".repeat(2_000)),
            "collapsed activity must not leak the full pasted content"
        );
    }

    fn invalid_tool_input_error(message: &str) -> OperationActivityError {
        OperationActivityError {
            problem: agena_failure::UserProblem::from(Failure::new(
                FailureCode::new("tool.invalid_input"),
                FailureCategory::InvalidInput,
                FailureResponsibility::Caller,
                RetryDirective::CorrectInput,
                RecoveryDirective::None,
                FailureImpact::RequestRejected,
                UserPresentation::validated("tool-invalid-input", message),
            )),
        }
    }

    #[test]
    fn empty_running_reply_projects_lifecycle_as_activity_not_empty_message() {
        let response_id = agena_domain::AssistantReplyId::new();
        let document = ContentDocument::default();
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::InProgress,
            1,
            &document,
            None,
        );
        assert!(matches!(
            entry.parts.as_slice(),
            [TranscriptEntryPart {
                id: TranscriptContentId::AssistantReplyLifecycle(id),
                content: TranscriptPartContent::Activity(
                    TranscriptActivityContent::AssistantReplyLifecycle(
                        TranscriptAssistantReplyLifecycle::Running
                    )
                ),
                ..
            }] if *id == response_id
        ));
        let rendered = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line.text.contains("Generating response"))
        );
        assert!(
            rendered
                .lines
                .iter()
                .all(|line| !line.text.contains("empty"))
        );
        assert!(rendered.nodes.iter().any(|node| {
            node.kind == crate::TranscriptNodeKind::Activity
                && node.key
                    == crate::TranscriptNodeKey::Activity {
                        entry_id: TranscriptEntryId::AssistantReply(response_id),
                        content_id: TranscriptContentId::AssistantReplyLifecycle(response_id),
                    }
        }));
    }

    #[test]
    fn one_canonical_assistant_reply_projects_all_permission_continuation_content_once() {
        let turn_id = agena_domain::TurnId::new();
        let reply_id = agena_domain::AssistantReplyId::new();
        let operation_activity_id = agena_domain::ActivityId::new();
        let request_id = "permission-fs-write".to_owned();
        let snapshot = TranscriptSnapshot {
            session_id: 7,
            seq_session: 3,
            turns: vec![agena_domain::TurnSnapshot {
                id: turn_id,
                session_id: 7,
                sequence: 1,
                input: ContentDocument::new(vec![ContentNode::text("real user input")]),
                reply: agena_domain::AssistantReplySnapshot {
                    id: reply_id,
                    turn_id,
                    status: AssistantReplyStatus::Completed,
                    content: ContentDocument::new(vec![
                        ContentNode::text("before permission"),
                        ContentNode::activity(ActivityNode {
                            id: operation_activity_id,
                            owner: ActivityOwner::AssistantReply { reply_id },
                            actor: ActivityActor::Tool,
                            state: ActivityState::Completed,
                            position: ContentPosition { index: 1 },
                            revision_seq: 2,
                            lifecycle: ActivityLifecycle::default(),
                            payload: ActivityPayload::Operation(OperationActivity {
                                call_id: ToolCallId::new("call-fs-write"),
                                invocation: ToolInvocation::new(
                                    "fs.write",
                                    StructuredObject::try_from(serde_json::json!({
                                        "path": "config.json"
                                    }))
                                    .expect("structured write input"),
                                ),
                                title: "fs.write".to_owned(),
                                summary: "Updated config.json".to_owned(),
                                data: serde_json::Value::Null,
                                markdown: String::new(),
                                authorization: OperationAuthorization {
                                    permissions: vec![OperationPermission {
                                        request: PermissionRequest {
                                            request_id: request_id.clone(),
                                            session_id: Some(7),
                                            action: PermissionAction::Tool {
                                                tool_name: "fs.write".to_owned(),
                                                qualifier: None,
                                            },
                                            related_actions: Vec::new(),
                                            requested_actions: Vec::new(),
                                            reason: "write access requires approval".to_owned(),
                                            explanation: String::new(),
                                            source: Some("static_policy".to_owned()),
                                            scope: None,
                                            operator: None,
                                            trace: Vec::new(),
                                            created_at: Utc::now(),
                                        },
                                        reply: Some(PermissionReply {
                                            request_id,
                                            kind: PermissionReplyKind::AllowOnce,
                                            reason: Some("approved for this edit".to_owned()),
                                            scope: None,
                                        }),
                                        replied_at_ms: Some(2),
                                    }],
                                },
                                error: None,
                            }),
                            provenance: ActivityProvenance::default(),
                        }),
                        ContentNode::text("continued after permission"),
                    ]),
                    revision_seq: 3,
                    created_at_ms: 1,
                    finished_at_ms: Some(3),
                    failure: None,
                },
                created_at_ms: 1,
            }],
            session_activities: Vec::new(),
        };

        let entries = transcript_entries(&snapshot);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, TranscriptEntryId::TurnInput(turn_id));
        assert_eq!(entries[1].id, TranscriptEntryId::AssistantReply(reply_id));
        assert_eq!(entries[1].parts.len(), 3);
        let rendered = crate::render_entry_detailed(
            &entries[1],
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        assert!(rendered.nodes.iter().any(|node| {
            node.key
                == crate::TranscriptNodeKey::Activity {
                    entry_id: TranscriptEntryId::AssistantReply(reply_id),
                    content_id: TranscriptContentId::Activity(operation_activity_id),
                }
        }));
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("before permission"), "{text}");
        assert!(text.contains("continued after permission"), "{text}");
        assert!(text.contains("fs.write"), "{text}");
        assert!(!text.contains("Awaiting permission"), "{text}");

        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(reply_id),
            content_id: TranscriptContentId::Activity(operation_activity_id),
        };
        let permissions_key = crate::TranscriptNodeKey::ActivitySection {
            entry_id: TranscriptEntryId::AssistantReply(reply_id),
            content_id: TranscriptContentId::Activity(operation_activity_id),
            section: crate::TranscriptActivitySection::Permissions,
        };
        let expanded = crate::render_entry_detailed(
            &entries[1],
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key.clone(), true)]),
        );
        let expanded_text = expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            expanded_text.contains("▸ Permissions · 1 permission"),
            "{expanded_text}"
        );
        assert!(
            !expanded_text.contains("Allowed once · fs.write"),
            "{expanded_text}"
        );
        assert!(
            !expanded_text.contains("approved for this edit"),
            "{expanded_text}"
        );
        let permissions_node = expanded
            .nodes
            .iter()
            .find(|node| node.key == permissions_key)
            .expect("collapsed Permissions section");
        assert!(permissions_node.toggleable);
        assert!(!permissions_node.expanded);
        assert!(
            permissions_node
                .copy_text
                .contains("Allowed once · fs.write")
        );

        let permissions_expanded = crate::render_entry_detailed(
            &entries[1],
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key, true), (permissions_key.clone(), true)]),
        );
        let permissions_expanded_text = permissions_expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            permissions_expanded_text.contains("▾ Permissions"),
            "{permissions_expanded_text}"
        );
        assert!(
            permissions_expanded_text.contains("Allowed once · fs.write"),
            "{permissions_expanded_text}"
        );
        assert!(
            permissions_expanded_text.contains("approved for this edit"),
            "{permissions_expanded_text}"
        );
        let permissions_node = permissions_expanded
            .nodes
            .iter()
            .find(|node| node.key == permissions_key)
            .expect("expanded Permissions section");
        assert!(permissions_node.toggleable);
        assert!(permissions_node.expanded);
    }

    #[test]
    fn permission_decision_summary_is_never_rendered_as_tool_output() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let request_id = "permission-shell-run".to_owned();
        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::AssistantReply {
                reply_id: response_id,
            },
            actor: ActivityActor::Tool,
            state: ActivityState::Completed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Operation(OperationActivity {
                call_id: ToolCallId::new("call-shell-run"),
                invocation: ToolInvocation::new(
                    "shell.run",
                    StructuredObject::try_from(serde_json::json!({
                        "command": "ls"
                    }))
                    .expect("structured shell.run input"),
                ),
                title: "shell.run".to_owned(),
                // The runtime records the approval decision as the Operation
                // summary when the tool produced no result (replies.rs). This
                // is permission transcript prose, never tool output.
                summary: "Permission allowed once".to_owned(),
                data: serde_json::Value::Null,
                markdown: String::new(),
                authorization: OperationAuthorization {
                    permissions: vec![OperationPermission {
                        request: PermissionRequest {
                            request_id: request_id.clone(),
                            session_id: Some(7),
                            action: PermissionAction::Tool {
                                tool_name: "shell.run".to_owned(),
                                qualifier: None,
                            },
                            related_actions: Vec::new(),
                            requested_actions: Vec::new(),
                            reason: "shell.run requires approval".to_owned(),
                            explanation: String::new(),
                            source: Some("static_policy".to_owned()),
                            scope: None,
                            operator: None,
                            trace: Vec::new(),
                            created_at: Utc::now(),
                        },
                        reply: Some(PermissionReply {
                            request_id,
                            kind: PermissionReplyKind::AllowOnce,
                            reason: None,
                            scope: None,
                        }),
                        replied_at_ms: Some(2),
                    }],
                },
                error: None,
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &document,
            None,
        );
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
        };
        let collapsed = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        let collapsed_text = collapsed
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(collapsed_text.contains("shell.run"), "{collapsed_text}");
        // The approval sentence is not tool output: it must never surface as
        // the collapsed header summary either.
        assert!(
            !collapsed_text.contains("Permission allowed once"),
            "{collapsed_text}"
        );

        let expanded = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key.clone(), true)]),
        );
        let expanded_text = expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(expanded_text.contains("shell.run"), "{expanded_text}");
        assert!(
            expanded_text.contains("▸ Permissions · 1 permission"),
            "{expanded_text}"
        );
        assert!(
            expanded_text.contains("▸ Input · 1 field"),
            "{expanded_text}"
        );
        // The approval sentence must never be wrapped in an "Output" section.
        assert!(!expanded_text.contains("Output"), "{expanded_text}");
        assert!(
            !expanded_text.contains("Permission allowed once"),
            "{expanded_text}"
        );
        let node = expanded
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("shell.run Activity node");
        assert!(!node.copy_text.contains("Output"), "{}", node.copy_text);
        assert!(
            !node.copy_text.contains("Permission allowed once"),
            "{}",
            node.copy_text
        );
        // The approval decision remains visible in the Permissions section.
        let permissions_key = crate::TranscriptNodeKey::ActivitySection {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
            section: crate::TranscriptActivitySection::Permissions,
        };
        let permissions_node = expanded
            .nodes
            .iter()
            .find(|node| node.key == permissions_key)
            .expect("collapsed Permissions section");
        assert!(
            permissions_node
                .copy_text
                .contains("Allowed once · shell.run"),
            "{}",
            permissions_node.copy_text
        );
    }

    #[test]
    fn session_activity_is_a_top_level_activity_without_a_fake_system_message() {
        let activity_id = agena_domain::ActivityId::new();
        let snapshot = TranscriptSnapshot {
            session_id: 7,
            seq_session: 1,
            turns: Vec::new(),
            session_activities: vec![ActivityNode {
                id: activity_id,
                owner: ActivityOwner::Session { session_id: 7 },
                actor: ActivityActor::Runtime,
                state: ActivityState::Completed,
                position: ContentPosition { index: 0 },
                revision_seq: 1,
                lifecycle: ActivityLifecycle::default(),
                payload: ActivityPayload::Notice(agena_domain::NoticeActivity {
                    kind: "session_notice".to_owned(),
                    summary: "Session notice".to_owned(),
                    detail: Some("Background state changed".to_owned()),
                }),
                provenance: ActivityProvenance::default(),
            }],
        };
        let entries = transcript_entries(&snapshot);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].role, None);
        let rendered = crate::render_entry_detailed(
            &entries[0],
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line.text.contains("Session notice"))
        );
        assert!(
            rendered
                .lines
                .iter()
                .all(|line| !line.text.starts_with("system"))
        );
        assert!(rendered.nodes.iter().any(|node| {
            node.key
                == crate::TranscriptNodeKey::Activity {
                    entry_id: TranscriptEntryId::SessionActivity(activity_id),
                    content_id: TranscriptContentId::Activity(activity_id),
                }
        }));
    }

    fn session_notice_activity(id: agena_domain::ActivityId, started_at_ms: i64) -> ActivityNode {
        ActivityNode {
            id,
            owner: ActivityOwner::Session { session_id: 7 },
            actor: ActivityActor::Runtime,
            state: ActivityState::Completed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle {
                started_at_ms,
                finished_at_ms: Some(started_at_ms),
            },
            payload: ActivityPayload::Notice(agena_domain::NoticeActivity {
                kind: "hook".to_owned(),
                summary: format!("hook@{started_at_ms}"),
                detail: None,
            }),
            provenance: ActivityProvenance::default(),
        }
    }

    #[test]
    fn session_activities_interleave_into_the_turn_timeline_by_timestamp() {
        let turn_id = agena_domain::TurnId::new();
        let reply_id = agena_domain::AssistantReplyId::new();
        let before = agena_domain::ActivityId::new();
        let mid = agena_domain::ActivityId::new();
        let after = agena_domain::ActivityId::new();
        let snapshot = TranscriptSnapshot {
            session_id: 7,
            seq_session: 1,
            turns: vec![agena_domain::TurnSnapshot {
                id: turn_id,
                session_id: 7,
                sequence: 1,
                input: ContentDocument::new(vec![ContentNode::text("user input")]),
                reply: agena_domain::AssistantReplySnapshot {
                    id: reply_id,
                    turn_id,
                    status: AssistantReplyStatus::Completed,
                    content: ContentDocument::new(vec![ContentNode::text("reply")]),
                    revision_seq: 1,
                    created_at_ms: 100,
                    finished_at_ms: Some(300),
                    failure: None,
                },
                created_at_ms: 100,
            }],
            // Deliberately out of timestamp order: the projection must sort by
            // started_at_ms before interleaving.
            session_activities: vec![
                session_notice_activity(after, 400),
                session_notice_activity(before, 50),
                session_notice_activity(mid, 200),
            ],
        };

        let entries = transcript_entries(&snapshot);
        let ids = entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                TranscriptEntryId::SessionActivity(before),
                TranscriptEntryId::TurnInput(turn_id),
                TranscriptEntryId::SessionActivity(mid),
                TranscriptEntryId::AssistantReply(reply_id),
                TranscriptEntryId::SessionActivity(after),
            ]
        );
    }

    #[test]
    fn session_activities_land_between_turns_and_streaming_reply_falls_back_to_created() {
        let first_turn = agena_domain::TurnId::new();
        let first_reply = agena_domain::AssistantReplyId::new();
        let second_turn = agena_domain::TurnId::new();
        let second_reply = agena_domain::AssistantReplyId::new();
        let between = agena_domain::ActivityId::new();
        let inside_streaming = agena_domain::ActivityId::new();
        let tail = agena_domain::ActivityId::new();
        let snapshot = TranscriptSnapshot {
            session_id: 7,
            seq_session: 1,
            turns: vec![
                agena_domain::TurnSnapshot {
                    id: first_turn,
                    session_id: 7,
                    sequence: 1,
                    input: ContentDocument::new(vec![ContentNode::text("first")]),
                    reply: agena_domain::AssistantReplySnapshot {
                        id: first_reply,
                        turn_id: first_turn,
                        status: AssistantReplyStatus::Completed,
                        content: ContentDocument::new(vec![ContentNode::text("first reply")]),
                        revision_seq: 1,
                        created_at_ms: 100,
                        finished_at_ms: Some(300),
                        failure: None,
                    },
                    created_at_ms: 100,
                },
                agena_domain::TurnSnapshot {
                    id: second_turn,
                    session_id: 7,
                    sequence: 2,
                    input: ContentDocument::new(vec![ContentNode::text("second")]),
                    // Still streaming: finished_at_ms is None, so the reply
                    // creation time is the boundary for inside-turn placement.
                    reply: agena_domain::AssistantReplySnapshot {
                        id: second_reply,
                        turn_id: second_turn,
                        status: AssistantReplyStatus::InProgress,
                        content: ContentDocument::new(vec![ContentNode::text("second reply")]),
                        revision_seq: 1,
                        created_at_ms: 500,
                        finished_at_ms: None,
                        failure: None,
                    },
                    created_at_ms: 500,
                },
            ],
            session_activities: vec![
                // Started after the first reply finished but before the second
                // turn was created: lands between the two turns.
                session_notice_activity(between, 400),
                // Started at the streaming reply's creation time: inside the
                // second turn (before its reply entry).
                session_notice_activity(inside_streaming, 500),
                // Started after the last reply boundary: stays at the tail.
                session_notice_activity(tail, 900),
            ],
        };

        let entries = transcript_entries(&snapshot);
        let ids = entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                TranscriptEntryId::TurnInput(first_turn),
                TranscriptEntryId::AssistantReply(first_reply),
                TranscriptEntryId::SessionActivity(between),
                TranscriptEntryId::TurnInput(second_turn),
                TranscriptEntryId::SessionActivity(inside_streaming),
                TranscriptEntryId::AssistantReply(second_reply),
                TranscriptEntryId::SessionActivity(tail),
            ]
        );
    }

    #[test]
    fn canonical_tools_list_activity_expands_text_without_structured_output() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::AssistantReply {
                reply_id: response_id,
            },
            actor: ActivityActor::Tool,
            state: ActivityState::Completed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Operation(OperationActivity {
                call_id: ToolCallId::new("call-tools-list"),
                invocation: ToolInvocation {
                    tool_api_call: Some(agena_domain::ToolApiCall {
                        function: agena_domain::ToolApiFunction::List,
                        arguments: StructuredObject::try_from(serde_json::json!({
                            "limit": 33,
                            "offset": 100
                        }))
                        .expect("structured tools_list provider input"),
                    }),
                    name: "tools_list".to_owned(),
                    plugin_name: None,
                    input: StructuredObject::try_from(serde_json::json!({
                        "limit": 33,
                        "offset": 100
                    }))
                    .expect("structured tools_list input"),
                },
                title: "List tools · 2/133".to_owned(),
                summary: "Returned 2 of 133 tools; continue at offset 102.".to_owned(),
                data: serde_json::json!({
                    "tool": "tool_search",
                    "results": ["fs.read", "repo.status"]
                }),
                markdown: String::new(),
                authorization: Default::default(),
                error: None,
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &document,
            None,
        );
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
        };
        let input_key = crate::TranscriptNodeKey::ActivitySection {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
            section: crate::TranscriptActivitySection::Input,
        };
        let collapsed = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        let node = collapsed
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("tools_list Activity node");
        assert!(node.toggleable);
        assert!(!node.expanded);
        // The collapsed headline is the composed operation title; the bare
        // execution-tool name is not repeated.
        assert!(
            collapsed
                .lines
                .iter()
                .any(|line| line.text.contains("List tools · 2/133"))
        );
        assert!(
            collapsed
                .lines
                .iter()
                .all(|line| !line.text.contains("tools_list"))
        );
        assert!(
            collapsed
                .lines
                .iter()
                .all(|line| !line.text.contains("repo.status"))
        );

        let expanded = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key.clone(), true)]),
        );
        let node = expanded
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("expanded tools_list Activity node");
        assert!(node.expanded);
        assert_eq!(node.end_line.saturating_sub(node.start_line), 1);
        let expanded_text = expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(expanded_text.contains("Output\n"), "{expanded_text}");
        assert!(
            expanded_text.contains("▸ Input · 2 fields"),
            "{expanded_text}"
        );
        assert!(!expanded_text.contains("\"limit\": 33"), "{expanded_text}");
        assert!(
            !expanded_text.contains("Structured result"),
            "{expanded_text}"
        );
        assert!(
            expanded
                .lines
                .iter()
                .any(|line| line.text.contains("repo.status"))
        );
        assert!(node.copy_text.contains("fs.read"));
        assert!(!node.copy_text.contains("Structured result"));
        let input_node = expanded
            .nodes
            .iter()
            .find(|node| node.key == input_key)
            .expect("collapsed nested Input node");
        assert!(input_node.toggleable);
        assert!(!input_node.expanded);
        assert!(input_node.copy_text.contains("\"limit\": 33"));

        let input_expanded = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key, true), (input_key.clone(), true)]),
        );
        let input_expanded_text = input_expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            input_expanded_text.contains("▾ Input · 2 fields")
                || input_expanded_text.contains("▾ Input"),
            "{input_expanded_text}"
        );
        assert!(
            input_expanded_text.contains("\"limit\": 33"),
            "{input_expanded_text}"
        );
        let input_node = input_expanded
            .nodes
            .iter()
            .find(|node| node.key == input_key)
            .expect("expanded nested Input node");
        assert!(input_node.expanded);
    }

    #[test]
    fn activity_copy_text_mirrors_the_expansion_state() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::AssistantReply {
                reply_id: response_id,
            },
            actor: ActivityActor::Tool,
            state: ActivityState::Completed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Operation(OperationActivity {
                call_id: ToolCallId::new("call-tools-list"),
                invocation: ToolInvocation {
                    tool_api_call: Some(agena_domain::ToolApiCall {
                        function: agena_domain::ToolApiFunction::List,
                        arguments: StructuredObject::try_from(serde_json::json!({
                            "limit": 33,
                            "offset": 100
                        }))
                        .expect("structured tools_list provider input"),
                    }),
                    name: "tools_list".to_owned(),
                    plugin_name: None,
                    input: StructuredObject::try_from(serde_json::json!({
                        "limit": 33,
                        "offset": 100
                    }))
                    .expect("structured tools_list input"),
                },
                title: "List tools · 2/133".to_owned(),
                summary: "Returned 2 of 133 tools; continue at offset 102.".to_owned(),
                data: serde_json::json!({
                    "tool": "tool_search",
                    "results": ["fs.read", "repo.status"]
                }),
                markdown: String::new(),
                authorization: Default::default(),
                error: None,
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &document,
            None,
        );
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
        };
        let input_key = crate::TranscriptNodeKey::ActivitySection {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
            section: crate::TranscriptActivitySection::Input,
        };

        let collapsed = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        let collapsed_node = collapsed
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("collapsed Activity node");
        assert!(!collapsed_node.expanded);
        assert!(
            collapsed_node.copy_text.is_empty(),
            "a collapsed Activity must not leak its expanded content into copy text: {}",
            collapsed_node.copy_text
        );
        assert!(!collapsed_node.contributes_to_aggregate_copy());

        let expanded = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key.clone(), true)]),
        );
        let expanded_node = expanded
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("expanded Activity node");
        assert!(expanded_node.expanded);
        assert!(
            expanded_node.copy_text.contains("fs.read"),
            "expanded Output section belongs in copy text: {}",
            expanded_node.copy_text
        );
        assert!(
            !expanded_node.copy_text.contains("\"limit\": 33"),
            "a collapsed Input section must not leak into copy text: {}",
            expanded_node.copy_text
        );
        assert!(expanded_node.contributes_to_aggregate_copy());

        let input_expanded = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key.clone(), true), (input_key, true)]),
        );
        let input_expanded_node = input_expanded
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("expanded Activity node");
        assert!(
            input_expanded_node.copy_text.contains("\"limit\": 33"),
            "an expanded Input section belongs in copy text: {}",
            input_expanded_node.copy_text
        );
    }

    #[test]
    fn canonical_operation_uses_rich_markdown_for_folded_and_expanded_activity_content() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let markdown_result = concat!(
            "## Checks\n\n",
            "- **Passed**: `cargo test`\n",
            "- [x] Snapshots\n\n",
            "| Metric | Value |\n",
            "| --- | ---: |\n",
            "| Passed | 94 |\n\n",
            "```rust\n",
            "fn main() {}\n",
            "```",
        );
        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::AssistantReply {
                reply_id: response_id,
            },
            actor: ActivityActor::Tool,
            state: ActivityState::Completed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Operation(OperationActivity {
                call_id: ToolCallId::new("call-markdown"),
                invocation: ToolInvocation::new(
                    "shell.run",
                    StructuredObject::try_from(serde_json::json!({
                        "command": "cargo test -p agena-tui-transcript"
                    }))
                    .expect("structured Markdown test input"),
                ),
                title: "Inspect `shell.run`".to_owned(),
                summary: "Found **3** checks".to_owned(),
                data: serde_json::Value::Null,
                // Runtime-derived human detail attached to the snapshot.
                markdown: markdown_result.to_owned(),
                authorization: Default::default(),
                error: None,
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &document,
            None,
        );
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
        };
        let input_key = crate::TranscriptNodeKey::ActivitySection {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
            section: crate::TranscriptActivitySection::Input,
        };
        let result_key = crate::TranscriptNodeKey::ActivitySection {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
            section: crate::TranscriptActivitySection::Result,
        };

        let collapsed = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        let collapsed_text = collapsed
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(collapsed_text.contains("shell.run"), "{collapsed_text}");
        assert!(
            collapsed_text.contains("Found 3 checks"),
            "{collapsed_text}"
        );
        assert!(!collapsed_text.contains('`'), "{collapsed_text}");
        assert!(!collapsed_text.contains("**"), "{collapsed_text}");
        assert!(collapsed.lines.iter().any(|line| {
            line.text.contains("shell.run")
                && line
                    .rich_line
                    .as_ref()
                    .is_some_and(|line| line.spans.len() >= 2)
        }));

        let expanded = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key.clone(), true)]),
        );
        let expanded_text = expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let node = expanded
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("Markdown Operation Activity node");
        assert!(
            expanded_text.contains("▸ Input · 1 field"),
            "{expanded_text}"
        );
        assert!(!expanded_text.contains("┌─ json"), "{expanded_text}");
        assert!(expanded_text.contains("▾ Output"), "{expanded_text}");
        assert!(expanded_text.contains("── Checks"), "{expanded_text}");
        assert!(
            expanded_text.contains("Passed: cargo test"),
            "{expanded_text}"
        );
        assert!(expanded_text.contains("┌─ rust"), "{expanded_text}");
        assert!(expanded_text.contains("│ Metric"), "{expanded_text}");
        assert!(!expanded_text.contains("**Passed**"), "{expanded_text}");
        assert!(!expanded_text.contains("```"), "{expanded_text}");
        assert!(node.copy_text.contains(markdown_result));
        let result_node = expanded
            .nodes
            .iter()
            .find(|node| node.key == result_key)
            .expect("default-expanded Output section");
        assert!(result_node.toggleable);
        assert!(result_node.expanded);

        let result_collapsed = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key.clone(), true), (result_key.clone(), false)]),
        );
        let result_collapsed_text = result_collapsed
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            result_collapsed_text.contains("▸ Output · Checks"),
            "{result_collapsed_text}"
        );
        assert!(
            !result_collapsed_text.contains("##"),
            "{result_collapsed_text}"
        );
        assert!(
            !result_collapsed_text.contains("Passed: cargo test"),
            "{result_collapsed_text}"
        );
        assert!(
            !result_collapsed_text.contains("┌─ rust"),
            "{result_collapsed_text}"
        );
        let result_node = result_collapsed
            .nodes
            .iter()
            .find(|node| node.key == result_key)
            .expect("collapsed Output section");
        assert!(result_node.toggleable);
        assert!(!result_node.expanded);

        let input_expanded = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key, true), (input_key, true)]),
        );
        let input_expanded_text = input_expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            input_expanded_text.contains("▾ Input"),
            "{input_expanded_text}"
        );
        assert!(
            input_expanded_text.contains("┌─ json"),
            "{input_expanded_text}"
        );
        assert!(
            input_expanded_text.contains("cargo test -p agena-tui-transcript"),
            "{input_expanded_text}"
        );
    }

    #[test]
    fn failed_canonical_operation_renders_the_real_error_once_without_a_fake_result() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let full_error = "shell.run must declare every accessed path in reads/writes because the command provably mutates or reads the filesystem: invokes mutating command 'rm'";
        let truncated_summary = agena_tool::normalize_tool_summary(full_error);
        assert!(truncated_summary.ends_with('…'));
        assert_ne!(truncated_summary, full_error);

        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::AssistantReply {
                reply_id: response_id,
            },
            actor: ActivityActor::Tool,
            state: ActivityState::Failed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Operation(OperationActivity {
                call_id: ToolCallId::new("call-shell-run"),
                invocation: ToolInvocation::new(
                    "shell.run",
                    StructuredObject::try_from(serde_json::json!({
                        "command": "rm -rf /tmp/agena-snapshot-test",
                        "description": "Compute pi with Python",
                        "reads": [],
                        "writes": [],
                        "network": [],
                        "shell": "bash",
                        "timeout_ms": 60_000
                    }))
                    .expect("structured shell.run input"),
                ),
                title: "shell.run".to_owned(),
                summary: truncated_summary.clone(),
                data: serde_json::Value::Null,
                markdown: String::new(),
                authorization: Default::default(),
                error: Some(invalid_tool_input_error(full_error)),
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry =
            assistant_reply_document_entry(response_id, MessageStatus::Failed, 1, &document, None);
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
        };
        let error_key = crate::TranscriptNodeKey::ActivitySection {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
            section: crate::TranscriptActivitySection::Error,
        };
        let rendered = crate::render_entry_detailed(
            &entry,
            400,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key.clone(), true)]),
        );
        let expanded_text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let node = rendered
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("failed shell.run Activity node");

        assert!(
            expanded_text.contains("▸ Input · 7 fields"),
            "{expanded_text}"
        );
        assert!(!expanded_text.contains("┌─ json"), "{expanded_text}");
        assert!(expanded_text.contains("▾ Error\n"), "{expanded_text}");
        assert!(!expanded_text.contains("Output\n"), "{expanded_text}");
        assert_eq!(
            expanded_text
                .matches("shell.run must declare every accessed path in reads/writes")
                .count(),
            1,
            "{expanded_text}"
        );
        assert_eq!(
            expanded_text
                .matches("provably mutates or reads the filesystem")
                .count(),
            1,
            "{expanded_text}"
        );
        assert_eq!(
            node.copy_text.matches(full_error).count(),
            1,
            "{}",
            node.copy_text
        );
        assert!(
            !node.copy_text.lines().any(|line| line == truncated_summary),
            "{}",
            node.copy_text
        );
        let error_node = rendered
            .nodes
            .iter()
            .find(|node| node.key == error_key)
            .expect("default-expanded Error section");
        assert!(error_node.toggleable);
        assert!(error_node.expanded);

        let error_collapsed = crate::render_entry_detailed(
            &entry,
            400,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key, true), (error_key.clone(), false)]),
        );
        let error_collapsed_text = error_collapsed
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            error_collapsed_text.contains("▸ Error ·"),
            "{error_collapsed_text}"
        );
        assert!(
            !error_collapsed_text.contains("which may read or write local files"),
            "{error_collapsed_text}"
        );
        let error_node = error_collapsed
            .nodes
            .iter()
            .find(|node| node.key == error_key)
            .expect("collapsed Error section");
        assert!(error_node.toggleable);
        assert!(!error_node.expanded);
        assert_eq!(error_node.copy_text, full_error);
    }

    #[test]
    fn failed_canonical_operation_keeps_distinct_partial_output_before_the_error() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let partial_output = "Compiled 18 modules before the linker stopped.";
        let full_error = "The linker could not resolve symbol `agena_runtime_start`.";
        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::AssistantReply {
                reply_id: response_id,
            },
            actor: ActivityActor::Tool,
            state: ActivityState::Failed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Operation(OperationActivity {
                call_id: ToolCallId::new("call-build"),
                invocation: ToolInvocation::new(
                    "shell.run",
                    StructuredObject::try_from(serde_json::json!({
                        "command": "cargo build"
                    }))
                    .expect("structured build input"),
                ),
                title: "Build workspace".to_owned(),
                summary: agena_tool::normalize_tool_summary(full_error),
                data: serde_json::Value::Null,
                // Partial output the tool produced before failing.
                markdown: partial_output.to_owned(),
                authorization: Default::default(),
                error: Some(invalid_tool_input_error(full_error)),
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry =
            assistant_reply_document_entry(response_id, MessageStatus::Failed, 1, &document, None);
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
        };
        let rendered = crate::render_entry_detailed(
            &entry,
            200,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key.clone(), true)]),
        );
        let expanded_text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let node = rendered
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("partially completed shell.run Activity node");

        assert!(expanded_text.contains("Output\n"), "{expanded_text}");
        assert!(expanded_text.contains("Error\n"), "{expanded_text}");
        assert_eq!(
            expanded_text.matches(partial_output).count(),
            1,
            "{expanded_text}"
        );
        assert_eq!(
            expanded_text
                .matches("The linker could not resolve symbol")
                .count(),
            1,
            "{expanded_text}"
        );
        assert_eq!(
            expanded_text.matches("agena_runtime_start").count(),
            1,
            "{expanded_text}"
        );
        assert_eq!(node.copy_text.matches(partial_output).count(), 1);
        assert_eq!(node.copy_text.matches(full_error).count(), 1);
    }

    #[test]
    fn canonical_error_activity_renders_its_problem_once_in_the_error_section() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let full_error = "The provider response ended before the requested item was complete.";
        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::AssistantReply {
                reply_id: response_id,
            },
            actor: ActivityActor::Runtime,
            state: ActivityState::Failed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Error(agena_domain::ErrorActivity {
                problem: invalid_tool_input_error(full_error).problem,
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry =
            assistant_reply_document_entry(response_id, MessageStatus::Failed, 1, &document, None);
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
        };
        let rendered = crate::render_entry_detailed(
            &entry,
            160,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key.clone(), true)]),
        );
        let expanded_text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let node = rendered
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("canonical Error Activity node");

        assert!(expanded_text.contains("Error\n"), "{expanded_text}");
        assert_eq!(
            expanded_text.matches(full_error).count(),
            1,
            "{expanded_text}"
        );
        assert_eq!(node.copy_text.matches(full_error).count(), 1);
    }

    #[test]
    fn canonical_hook_activity_renders_headline_summary_and_expanded_detail() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::AssistantReply {
                reply_id: response_id,
            },
            actor: ActivityActor::Runtime,
            state: ActivityState::Completed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Notice(agena_domain::NoticeActivity {
                kind: "hook".to_owned(),
                summary: "agent.stop hook blocked stop: workflow plan autorun".to_owned(),
                detail: Some("Continue: next plan step".to_owned()),
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &document,
            None,
        );
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
        };
        let defaults = crate::TranscriptDetailDefaults {
            activity_expanded: false,
        };

        // Collapsed: the row carries the hook headline and human summary; the
        // detail body stays folded.
        let collapsed = crate::render_entry_detailed(
            &entry,
            120,
            &agena_tui::i18n::I18n::english(),
            defaults,
            &Default::default(),
        );
        let collapsed_text = collapsed
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(collapsed_text.contains("Notice"), "{collapsed_text}");
        assert!(
            collapsed_text.contains("agent.stop hook blocked stop: workflow plan autorun"),
            "{collapsed_text}"
        );
        assert!(
            !collapsed_text.contains("Continue: next plan step"),
            "{collapsed_text}"
        );

        // Expanded: the detail renders in the Notice section.
        let expanded = crate::render_entry_detailed(
            &entry,
            120,
            &agena_tui::i18n::I18n::english(),
            defaults,
            &std::collections::BTreeMap::from([(key, true)]),
        );
        let expanded_text = expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            expanded_text.contains("Continue: next plan step"),
            "{expanded_text}"
        );
    }

    #[test]
    fn canonical_operation_renders_named_sections_without_repeating_raw_result() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::AssistantReply {
                reply_id: response_id,
            },
            actor: ActivityActor::Tool,
            state: ActivityState::Completed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Operation(OperationActivity {
                call_id: ToolCallId::new("call-search"),
                invocation: ToolInvocation::new(
                    "repo.search",
                    StructuredObject::try_from(serde_json::json!({"query": "Activity"}))
                        .expect("structured search input"),
                ),
                title: "Search repository".to_owned(),
                summary: "2 matches".to_owned(),
                data: serde_json::Value::Null,
                // Runtime-derived search result detail attached to the snapshot.
                markdown: "### Matches\n\n- `src/activity.rs`\n- `src/tool_result.rs`".to_owned(),
                authorization: Default::default(),
                error: None,
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &document,
            None,
        );
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
        };

        let collapsed = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        let collapsed_text = collapsed
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        // The collapsed headline is the composed operation title; the bare
        // execution-tool name is not repeated.
        assert!(collapsed_text.contains("Search repository"));
        assert!(!collapsed_text.contains("repo.search"));
        assert!(collapsed_text.contains("2 matches"));
        assert!(!collapsed_text.contains("src/activity.rs"));

        let expanded = crate::render_entry_detailed(
            &entry,
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &std::collections::BTreeMap::from([(key, true)]),
        );
        let expanded_text = expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(expanded_text.contains("Matches"), "{expanded_text}");
        assert!(expanded_text.contains("src/activity.rs"), "{expanded_text}");
        assert_eq!(
            expanded_text.matches("src/activity.rs").count(),
            1,
            "{expanded_text}"
        );
        assert!(
            !expanded_text.contains("Structured result"),
            "{expanded_text}"
        );
        // The summary is collapsed-only; expanded shows the derived detail.
        assert!(
            expanded_text.contains("src/tool_result.rs"),
            "{expanded_text}"
        );
    }

    #[test]
    fn canonical_activity_headlines_show_composed_title_and_spend_remaining_width_on_summary() {
        let response_id = agena_domain::AssistantReplyId::new();
        let operation_document = |invocation_name: &str, summary: &str| {
            ContentDocument::new(vec![ContentNode::activity(ActivityNode {
                id: agena_domain::ActivityId::new(),
                owner: ActivityOwner::AssistantReply {
                    reply_id: response_id,
                },
                actor: ActivityActor::Tool,
                state: ActivityState::Completed,
                position: ContentPosition { index: 0 },
                revision_seq: 1,
                lifecycle: ActivityLifecycle::default(),
                payload: ActivityPayload::Operation(OperationActivity {
                    call_id: ToolCallId::new("call-title"),
                    invocation: ToolInvocation::new(invocation_name, StructuredObject::default()),
                    // The composed operation title is the headline; it spends
                    // the remaining width on the summary.
                    title: "Process run Create a tiny test PNG with pure python".to_owned(),
                    summary: summary.to_owned(),
                    data: serde_json::Value::Null,
                    markdown: String::new(),
                    authorization: Default::default(),
                    error: None,
                }),
                provenance: ActivityProvenance::default(),
            })])
        };

        let ordinary_document = operation_document("shell.run", &"PNG result details ".repeat(30));
        let ordinary = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &ordinary_document,
            None,
        );
        let ordinary_rendered = crate::render_entry_detailed(
            &ordinary,
            80,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        let ordinary_headline = ordinary_rendered
            .lines
            .iter()
            .find(|line| line.text.contains("Process run"))
            .expect("ordinary Operation headline");
        assert!(ordinary_headline.text.contains("Process run"));
        assert!(!ordinary_headline.text.contains("shell.run"));
        assert!(ordinary_headline.text.contains("PNG result details"));
        assert!(ordinary_headline.text.ends_with('…'));
        assert!(unicode_width::UnicodeWidthStr::width(ordinary_headline.text.as_str()) <= 80);

        let long_document = operation_document("shell.run", &"complete ".repeat(60));
        let long = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &long_document,
            None,
        );
        let long_rendered = crate::render_entry_detailed(
            &long,
            120,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        let long_headline = long_rendered
            .lines
            .iter()
            .find(|line| line.text.contains("Process run"))
            .expect("long Operation headline");
        assert!(long_headline.text.ends_with('…'));
        assert!(unicode_width::UnicodeWidthStr::width(long_headline.text.as_str()) <= 120);
    }

    #[test]
    fn operation_activity_titles_use_the_composed_operation_title() {
        let operation = |name: &str, input: serde_json::Value, title: &str| {
            let provider_arguments =
                StructuredObject::try_from(input).expect("structured tool input");
            let invocation = match agena_domain::ToolApiFunction::from_function_name(name) {
                Some(agena_domain::ToolApiFunction::Call) => {
                    let target = provider_arguments
                        .get("tool")
                        .and_then(agena_domain::StructuredValue::as_text)
                        .expect("tools_call target")
                        .to_owned();
                    let target_input = provider_arguments
                        .get("input")
                        .cloned()
                        .map(serde_json::Value::from)
                        .and_then(|value| StructuredObject::try_from(value).ok())
                        .expect("tools_call target input");
                    ToolInvocation {
                        tool_api_call: Some(agena_domain::ToolApiCall {
                            function: agena_domain::ToolApiFunction::Call,
                            arguments: provider_arguments,
                        }),
                        name: target,
                        plugin_name: None,
                        input: target_input,
                    }
                }
                Some(function) => ToolInvocation {
                    tool_api_call: Some(agena_domain::ToolApiCall {
                        function,
                        arguments: provider_arguments.clone(),
                    }),
                    name: function.function_name().to_owned(),
                    plugin_name: None,
                    input: provider_arguments,
                },
                None => ToolInvocation::new(name, provider_arguments),
            };
            OperationActivity {
                call_id: ToolCallId::new(format!("call-{name}")),
                invocation,
                // A non-empty composed title is the headline; an empty title
                // falls back to the invocation name.
                title: title.to_owned(),
                summary: String::new(),
                data: serde_json::Value::Null,
                markdown: String::new(),
                authorization: Default::default(),
                error: None,
            }
        };

        assert_eq!(
            operation_activity_title(&operation(
                "tools_list",
                serde_json::json!({}),
                "List tools"
            )),
            "List tools"
        );
        assert_eq!(
            operation_activity_title(&operation(
                "tools_list",
                serde_json::json!({"offset": 20}),
                "List tools · 20/133"
            )),
            "List tools · 20/133"
        );
        assert_eq!(
            operation_activity_title(&operation(
                "tools_search",
                serde_json::json!({"query": "filesystem"}),
                "Search tools · filesystem",
            )),
            "Search tools · filesystem"
        );
        assert_eq!(
            operation_activity_title(&operation(
                "tools_search",
                serde_json::json!({"query": "filesystem"}),
                "Search tools · filesystem · 5/12",
            )),
            "Search tools · filesystem · 5/12"
        );
        assert_eq!(
            operation_activity_title(&operation(
                "tools_help",
                serde_json::json!({"tool": "fs.read"}),
                "Inspect fs.read",
            )),
            "Inspect fs.read"
        );
        // An empty title falls back to the direct execution-tool name.
        assert_eq!(
            operation_activity_title(&operation(
                "tools_call",
                serde_json::json!({"tool": "fs.read", "input": {"path": "README.md"}}),
                "",
            )),
            "fs.read"
        );
        assert_eq!(
            operation_activity_title(&operation(
                "tools_call",
                serde_json::json!({"tool": "fs.read", "input": {"path": "README.md"}}),
                "Read README.md",
            )),
            "Read README.md"
        );
        assert_eq!(
            operation_activity_title(&operation(
                "agena.repo.status",
                serde_json::json!({}),
                "Repository status",
            )),
            "Repository status"
        );
    }

    #[test]
    fn user_entry_keeps_editor_like_inline_activity_placeholders() {
        let turn_id = agena_domain::TurnId::new();
        let response_id = agena_domain::AssistantReplyId::new();
        let first_text_id = agena_domain::TextSegmentId::new();
        let second_text_id = agena_domain::TextSegmentId::new();
        let skill_id = agena_domain::ActivityId::new();
        let git_id = agena_domain::ActivityId::new();
        let activity = |id, position, payload| ActivityNode {
            id,
            owner: ActivityOwner::TurnInput { turn_id },
            actor: ActivityActor::User,
            state: ActivityState::Completed,
            position: ContentPosition { index: position },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload,
            provenance: ActivityProvenance::default(),
        };
        let snapshot = TranscriptSnapshot {
            session_id: 1,
            seq_session: 1,
            turns: vec![agena_domain::TurnSnapshot {
                id: turn_id,
                session_id: 1,
                sequence: 1,
                input: ContentDocument::new(vec![
                    ContentNode::text_at(first_text_id, "hi", 0, 1),
                    ContentNode::activity(activity(
                        skill_id,
                        1,
                        ActivityPayload::SkillReference(SkillReferenceActivity {
                            name: "batch".to_owned(),
                            description: String::new(),
                            instructions: "Use it".to_owned(),
                            content_hash: "sha256:test".to_owned(),
                            source: "test".to_owned(),
                            aliases: Vec::new(),
                        }),
                    )),
                    ContentNode::text_at(second_text_id, "/asd/", 2, 1),
                    ContentNode::activity(activity(
                        git_id,
                        3,
                        ActivityPayload::Resource(ResourceActivity {
                            kind: ResourceKind::Directory,
                            reference: ResourceReference::WorkspacePath {
                                path: ".git".to_owned(),
                            },
                            name: ".git".to_owned(),
                            media_type: None,
                            size_bytes: None,
                            width: None,
                            height: None,
                            duration_ms: None,
                            page_count: None,
                        }),
                    )),
                ]),
                reply: agena_domain::AssistantReplySnapshot {
                    id: response_id,
                    turn_id,
                    status: AssistantReplyStatus::Pending,
                    content: ContentDocument::default(),
                    revision_seq: 1,
                    created_at_ms: 1,
                    finished_at_ms: None,
                    failure: None,
                },
                created_at_ms: 1,
            }],
            session_activities: Vec::new(),
        };

        let entries = transcript_entries(&snapshot);
        assert_eq!(entries[0].id, TranscriptEntryId::TurnInput(turn_id));
        assert_eq!(
            entries[1].id,
            TranscriptEntryId::AssistantReply(response_id)
        );
        assert_eq!(entries[0].parts.len(), 3);
        assert_eq!(
            entries[0]
                .parts
                .iter()
                .map(|part| part.id)
                .collect::<Vec<_>>(),
            vec![
                TranscriptContentId::Activity(skill_id),
                TranscriptContentId::Activity(git_id),
                TranscriptContentId::TurnDocument(turn_id),
            ]
        );
        assert!(matches!(
            &entries[0].parts[0].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload))
                if matches!(payload, ActivityPayload::SkillReference(_))
        ));
        assert!(matches!(
            &entries[0].parts[1].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload))
                if matches!(payload, ActivityPayload::Resource(_))
        ));
        let TranscriptPartContent::UserDocument(document) = &entries[0].parts[2].content else {
            panic!("user input must project as one inline document");
        };
        assert!(matches!(
            document.nodes.as_slice(),
            [
                TranscriptUserDocumentNode::Text { id, text },
                TranscriptUserDocumentNode::Activity { id: first_activity, placeholder: first_placeholder, .. },
                TranscriptUserDocumentNode::Text { id: second_id, text: second_text },
                TranscriptUserDocumentNode::Activity { id: second_activity, placeholder: second_placeholder, .. },
            ] if *id == Some(first_text_id)
                && text == "hi"
                && *first_activity == skill_id
                && first_placeholder == "[Skill: batch]"
                && *second_id == Some(second_text_id)
                && second_text == "/asd/"
                && *second_activity == git_id
                && second_placeholder == "[folder: .git]"
        ));

        let rendered = crate::render_entry_detailed(
            &entries[0],
            100,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        let inline_line = rendered
            .lines
            .iter()
            .find(|line| line.text == "  hi[Skill: batch]/asd/[folder: .git]")
            .expect("inline document follows the detailed activities");
        assert_eq!(inline_line.copy_text, document.plain_text());
        assert!(rendered.nodes.iter().any(|node| node.key
            == crate::TranscriptNodeKey::Activity {
                entry_id: TranscriptEntryId::TurnInput(turn_id),
                content_id: TranscriptContentId::Activity(skill_id),
            }));
        assert!(rendered.nodes.iter().any(|node| node.key
            == crate::TranscriptNodeKey::Activity {
                entry_id: TranscriptEntryId::TurnInput(turn_id),
                content_id: TranscriptContentId::Activity(git_id),
            }));
        assert!(rendered.nodes.iter().any(|node| node.key
            == crate::TranscriptNodeKey::Content {
                entry_id: TranscriptEntryId::TurnInput(turn_id),
                content_id: Some(TranscriptContentId::TurnDocument(turn_id)),
            }));

        let narrow = crate::render_entry_detailed(
            &entries[0],
            18,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        assert!(
            narrow
                .lines
                .iter()
                .any(|line| line.text.contains("[Skill: batch]"))
        );
        assert!(
            narrow
                .lines
                .iter()
                .any(|line| line.text.contains("[folder: .git]"))
        );
    }

    #[test]
    fn text_artifact_inline_placeholder_uses_text_prefix_and_remaining_count() {
        // Short pasted text is shown verbatim; the persisted label is ignored
        // because the content itself fits the row.
        let short = ActivityPayload::TextArtifact(agena_domain::TextArtifactActivity {
            text: "pasted body".to_owned(),
            language: None,
            label: Some("paste 1000 chars".to_owned()),
        });
        assert_eq!(user_activity_placeholder(&short), "[pasted body]");

        // Long pasted text keeps the first 12 chars plus the remaining count,
        // never a generic `paste N chars` label.
        let long = ActivityPayload::TextArtifact(agena_domain::TextArtifactActivity {
            text: "abcdefghijklmnopqrstuvwxyz".to_owned(),
            language: None,
            label: Some("paste 26 chars".to_owned()),
        });
        assert_eq!(
            user_activity_placeholder(&long),
            "[abcdefghijkl… +14 chars]"
        );
        assert!(user_activity_placeholder(&long).chars().count() < 40);
    }

    #[test]
    fn failed_reply_with_structured_problem_renders_summary_and_expandable_detail() {
        let response_id = agena_domain::AssistantReplyId::new();
        let problem = agena_failure::UserProblem::from(agena_failure::Failure::new(
            agena_failure::FailureCode::new("internal.event_store"),
            agena_failure::FailureCategory::Internal,
            agena_failure::FailureResponsibility::System,
            agena_failure::RetryDirective::ImmediateOnce,
            agena_failure::RecoveryDirective::Retry,
            agena_failure::FailureImpact::OperationFailed,
            agena_failure::UserPresentation::new(
                "internal-event-store",
                "The reply was interrupted because the runtime restarted. Try again.",
            ),
        ));
        let document = ContentDocument::default();
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::Failed,
            1,
            &document,
            Some(problem),
        );
        let defaults = crate::TranscriptDetailDefaults {
            activity_expanded: false,
        };

        // Collapsed: the row carries the readable failure summary.
        let collapsed = crate::render_entry_detailed(
            &entry,
            120,
            &agena_tui::i18n::I18n::english(),
            defaults,
            &Default::default(),
        );
        let collapsed_text = collapsed
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        assert!(
            collapsed_text.contains("Response failed"),
            "{collapsed_text}"
        );
        assert!(
            collapsed_text
                .contains("The reply was interrupted because the runtime restarted. Try again."),
            "{collapsed_text}"
        );

        // Expanded: the structured public failure fields are visible.
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::AssistantReplyLifecycle(response_id),
        };
        let expanded = crate::render_entry_detailed(
            &entry,
            120,
            &agena_tui::i18n::I18n::english(),
            defaults,
            &std::collections::BTreeMap::from([(key, true)]),
        );
        let expanded_text = expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        assert!(
            expanded_text.contains("Error code: internal.event_store"),
            "{expanded_text}"
        );
        assert!(
            expanded_text.contains("Category: Internal error"),
            "{expanded_text}"
        );
        assert!(
            expanded_text.contains("Responsibility: The system"),
            "{expanded_text}"
        );
        assert!(expanded_text.contains("Recovery: Retry"), "{expanded_text}");
        assert!(
            expanded_text.contains("Retry: Retry once immediately"),
            "{expanded_text}"
        );
        assert!(!expanded_text.contains("Reference:"), "{expanded_text}");
    }
    #[test]
    fn durable_error_activity_renders_once_without_a_lifecycle_duplicate() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let full_error = "The provider response ended unexpectedly.";
        let problem = invalid_tool_input_error(full_error).problem;
        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::AssistantReply {
                reply_id: response_id,
            },
            actor: ActivityActor::Runtime,
            state: ActivityState::Failed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Error(agena_domain::ErrorActivity {
                problem: problem.clone(),
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let defaults = crate::TranscriptDetailDefaults {
            activity_expanded: false,
        };

        // A reply that recovered keeps its durable Error Activity in content
        // while the runtime failure projection is cleared; the error must
        // still render, and without a duplicate lifecycle row.
        let recovered = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            2,
            &document,
            None,
        );
        let rendered = crate::render_entry_detailed(
            &recovered,
            120,
            &agena_tui::i18n::I18n::english(),
            defaults,
            &Default::default(),
        );
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        assert!(text.contains("Error"), "{text}");
        assert!(text.contains(full_error), "{text}");
        assert!(!text.contains("Response failed"), "{text}");

        // While the reply is still marked failed, the durable error activity
        // is the single representation of the failure.
        let failed = assistant_reply_document_entry(
            response_id,
            MessageStatus::Failed,
            1,
            &document,
            Some(problem),
        );
        let failed_text = crate::render_entry_detailed(
            &failed,
            120,
            &agena_tui::i18n::I18n::english(),
            defaults,
            &Default::default(),
        )
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join(
            "
",
        );
        assert!(failed_text.contains(full_error), "{failed_text}");
        assert!(!failed_text.contains("Response failed"), "{failed_text}");
    }
    #[test]
    fn text_segment_activity_renders_collapsible_with_normal_body_color() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let body = "Second paragraph after the tool call.
It has two lines.";
        let document = ContentDocument::new(vec![
            ContentNode::activity(ActivityNode {
                id: activity_id,
                owner: ActivityOwner::AssistantReply {
                    reply_id: response_id,
                },
                actor: ActivityActor::Assistant,
                state: ActivityState::Completed,
                position: ContentPosition { index: 0 },
                revision_seq: 1,
                lifecycle: ActivityLifecycle::default(),
                payload: ActivityPayload::TextSegment(agena_domain::TextSegmentActivity {
                    text: body.to_owned(),
                }),
                provenance: ActivityProvenance::default(),
            }),
            // A final body segment after the interstitial text renders inline
            // as plain text, never as a collapsed Activity.
            ContentNode::text_at(agena_domain::TextSegmentId::new(), "final answer", 1, 2),
        ]);
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &document,
            None,
        );
        assert_eq!(entry.parts.len(), 2);
        assert!(matches!(
            &entry.parts[0].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload))
                if matches!(payload, ActivityPayload::TextSegment(_))
        ));
        assert!(matches!(
            &entry.parts[1].content,
            TranscriptPartContent::Text(text) if text.text == "final answer"
        ));
        let defaults = crate::TranscriptDetailDefaults {
            activity_expanded: false,
        };
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::AssistantReply(response_id),
            content_id: TranscriptContentId::Activity(activity_id),
        };
        let collapsed = crate::render_entry_detailed(
            &entry,
            120,
            &agena_tui::i18n::I18n::english(),
            defaults,
            &Default::default(),
        );
        let collapsed_lines = collapsed
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        // Interstitial segments default to collapsed: the headline shows the
        // label with a preview and the body stays folded, while the final
        // answer is visible inline.
        assert!(
            collapsed_lines.iter().any(|line| line.contains("Text")),
            "{collapsed_lines:?}"
        );
        assert!(
            collapsed_lines
                .iter()
                .any(|line| line.contains("final answer")),
            "{collapsed_lines:?}"
        );
        let collapsed_node = collapsed
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("collapsed TextSegment Activity node");
        assert!(collapsed_node.toggleable);
        assert!(!collapsed_node.expanded);
        let rendered = crate::render_entry_detailed(
            &entry,
            120,
            &agena_tui::i18n::I18n::english(),
            defaults,
            &std::collections::BTreeMap::from([(key, true)]),
        );
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        assert!(text.contains("Text"), "{text}");
        assert!(
            text.contains("Second paragraph after the tool call."),
            "{text}"
        );
        assert!(text.contains("It has two lines."), "{text}");

        // The expanded body must use the normal text style, not thinking's
        // muted color.
        let muted = agena_tui_components::theme::muted_color();
        let body_styles = rendered
            .lines
            .iter()
            .filter(|line| line.text.contains("Second paragraph after the tool call."))
            .filter_map(|line| line.style.fg)
            .collect::<Vec<_>>();
        assert!(
            body_styles.iter().all(|fg| *fg != muted),
            "text segment body must not be muted: {body_styles:?}"
        );
    }

    #[test]
    fn interstitial_plain_text_projects_as_owned_text_segment_activity() {
        let response_id = agena_domain::AssistantReplyId::new();
        let first_text_id = agena_domain::TextSegmentId::new();
        let second_text_id = agena_domain::TextSegmentId::new();
        let document = ContentDocument::new(vec![
            ContentNode::text_at(first_text_id, "Let me inspect the file.", 0, 1),
            ContentNode::text_at(second_text_id, "That is the answer.", 1, 2),
        ]);
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &document,
            None,
        );
        assert_eq!(entry.parts.len(), 2);
        // The opening body segment is not the answer: it projects as an
        // owned TextSegment activity (collapsible), never as inline text.
        assert!(matches!(
            &entry.parts[0].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::TextSegment(segment))
                if segment.text == "Let me inspect the file."
        ));
        // The final body segment is the answer and stays inline plain text.
        assert!(matches!(
            &entry.parts[1].content,
            TranscriptPartContent::Text(text) if text.text == "That is the answer."
        ));

        let rendered = crate::render_entry_detailed(
            &entry,
            120,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        // Collapsed headline shows the interstitial label; the answer is
        // visible inline without any label.
        assert!(text.contains("Text"), "{text}");
        assert!(text.contains("That is the answer."), "{text}");
    }

    #[test]
    fn in_progress_reply_after_tool_call_keeps_every_body_segment_collapsible() {
        let response_id = agena_domain::AssistantReplyId::new();
        let first_text_id = agena_domain::TextSegmentId::new();
        let operation_id = agena_domain::ActivityId::new();
        let document = ContentDocument::new(vec![
            ContentNode::text_at(first_text_id, "Let me inspect the file.", 0, 1),
            ContentNode::activity(ActivityNode {
                id: operation_id,
                owner: ActivityOwner::AssistantReply {
                    reply_id: response_id,
                },
                actor: ActivityActor::Tool,
                state: ActivityState::Completed,
                position: ContentPosition { index: 1 },
                revision_seq: 2,
                lifecycle: ActivityLifecycle::default(),
                payload: ActivityPayload::Operation(OperationActivity {
                    call_id: ToolCallId::new("call-fs-read"),
                    invocation: ToolInvocation::new("fs.read", StructuredObject::default()),
                    title: "fs.read".to_owned(),
                    summary: String::new(),
                    data: serde_json::Value::Null,
                    markdown: String::new(),
                    authorization: Default::default(),
                    error: None,
                }),
                provenance: ActivityProvenance::default(),
            }),
        ]);
        // While the reply is still running after a tool call, the opening
        // body segment is a working note, not the answer: it must project as
        // a collapsible TextSegment activity and never as inline text, so it
        // does not flip from plain text to an Activity when the reply grows.
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::InProgress,
            1,
            &document,
            None,
        );
        assert_eq!(entry.parts.len(), 2);
        assert!(matches!(
            &entry.parts[0].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::TextSegment(segment))
                if segment.text == "Let me inspect the file."
        ));
        assert!(matches!(
            &entry.parts[1].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload))
                if matches!(payload, ActivityPayload::Operation(_))
        ));

        let rendered = crate::render_entry_detailed(
            &entry,
            120,
            &agena_tui::i18n::I18n::english(),
            crate::TranscriptDetailDefaults {
                activity_expanded: false,
            },
            &Default::default(),
        );
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Text"), "{text}");
        assert!(text.contains("fs.read"), "{text}");
    }

    #[test]
    fn in_progress_reply_without_tool_call_keeps_live_text_inline() {
        let response_id = agena_domain::AssistantReplyId::new();
        let first_text_id = agena_domain::TextSegmentId::new();
        let document = ContentDocument::new(vec![ContentNode::text_at(
            first_text_id,
            "live answer",
            0,
            1,
        )]);
        // A plain question/answer still streams inline: with no tool call in
        // the document, the current body segment is the (candidate) final
        // text and must not collapse into an Activity while streaming.
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::InProgress,
            1,
            &document,
            None,
        );
        assert!(matches!(
            entry.parts.as_slice(),
            [TranscriptEntryPart {
                content: TranscriptPartContent::Text(text),
                ..
            }] if text.text == "live answer"
        ));
    }
}
