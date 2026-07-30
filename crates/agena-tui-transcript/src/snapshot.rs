//! Canonical transcript snapshot projection for terminal presentation.
//!
//! This is intentionally a projection from `TurnSnapshot` / `ResponseSnapshot`
//! / `ContentNode` rather than an adapter through public message resources.
//! Stable domain identities therefore reach navigation and rendering without
//! fabricated integer message or part ids.

use agena_api::{
    message_part::{
        MessageAttachmentPartResource, MessageErrorPartResource, MessageReasoningPartResource,
        MessageSkillReferencePartResource, MessageTextPartResource, PartExecutionStatusResource,
    },
    resource::{
        MessageAttachment, MessageAttachmentKind, MessageAttachmentSource, MessageRole,
        MessageSkillReference, MessageStatus,
    },
};
use agena_domain::{
    ActivityNode, ActivityPayload, ActivityState, ContentDocument, ContentNode,
    MaintenanceActivity, ResourceKind, ResourceReference, ResponseStatus, TranscriptSnapshot,
};
use chrono::{DateTime, Utc};

use crate::{
    TranscriptActivityPresentation, TranscriptContentId, TranscriptEntry, TranscriptEntryId,
    TranscriptEntryPart, TranscriptPartContent, TranscriptUserActivityStyle,
    TranscriptUserDocument, TranscriptUserDocumentNode,
};

pub fn transcript_entries(snapshot: &TranscriptSnapshot) -> Vec<TranscriptEntry> {
    let mut entries = Vec::with_capacity(
        snapshot
            .turns
            .len()
            .saturating_mul(2)
            .saturating_add(snapshot.session_activities.len()),
    );
    for turn in &snapshot.turns {
        entries.push(user_document_entry(
            turn.id,
            turn.created_at_ms,
            &turn.input,
        ));
        entries.push(response_document_entry(
            turn.response.id,
            response_status(turn.response.status),
            turn.response.created_at_ms,
            &turn.response.content,
        ));
    }
    entries.extend(
        snapshot
            .session_activities
            .iter()
            .map(|activity| TranscriptEntry {
                id: TranscriptEntryId::SessionActivity(activity.id),
                role: MessageRole::System,
                state: activity_entry_status(activity.state),
                created_at: timestamp(activity.lifecycle.started_at_ms),
                parts: vec![activity_entry_part(activity)],
            }),
    );
    entries
}

fn user_document_entry(
    turn_id: agena_domain::TurnId,
    created_at_ms: i64,
    document: &ContentDocument,
) -> TranscriptEntry {
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
                id: segment.id,
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
            name: Some("user_document".to_owned()),
            operation_id: None,
            content: TranscriptPartContent::UserDocument(TranscriptUserDocument { nodes }),
        });
    }
    TranscriptEntry {
        id: TranscriptEntryId::TurnInput(turn_id),
        role: MessageRole::User,
        state: MessageStatus::Completed,
        created_at: timestamp(created_at_ms),
        parts,
    }
}

fn response_document_entry(
    response_id: agena_domain::ResponseId,
    state: MessageStatus,
    created_at_ms: i64,
    document: &ContentDocument,
) -> TranscriptEntry {
    TranscriptEntry {
        id: TranscriptEntryId::Response(response_id),
        role: MessageRole::Assistant,
        state,
        created_at: timestamp(created_at_ms),
        parts: document
            .nodes()
            .iter()
            .map(|node| match node {
                ContentNode::Text { segment } => TranscriptEntryPart {
                    id: TranscriptContentId::Text(segment.id),
                    status: PartExecutionStatusResource::Completed,
                    name: Some("text".to_owned()),
                    operation_id: None,
                    content: TranscriptPartContent::Text(MessageTextPartResource {
                        text: segment.text.clone(),
                        synthetic: false,
                    }),
                },
                ContentNode::Activity { activity } => activity_entry_part(activity),
            })
            .collect(),
    }
}

const fn user_activity_style(payload: &ActivityPayload) -> TranscriptUserActivityStyle {
    match payload {
        ActivityPayload::Resource(_) => TranscriptUserActivityStyle::Resource,
        ActivityPayload::SkillReference(_) | ActivityPayload::SkillExecution(_) => {
            TranscriptUserActivityStyle::Skill
        }
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
        ActivityPayload::SkillExecution(skill) => format!("[Skill: {}]", skill.skill_name),
        ActivityPayload::TextArtifact(artifact) => {
            let label = artifact
                .label
                .clone()
                .unwrap_or_else(|| format!("paste {} chars", artifact.text.chars().count()));
            format!("[{label}]")
        }
        _ => {
            let (_, title, _, _) = activity_presentation(payload);
            format!("[{title}]")
        }
    }
}

fn activity_entry_part(activity: &ActivityNode) -> TranscriptEntryPart {
    let (schema, title, summary, error) = activity_presentation(&activity.payload);
    let generic = TranscriptActivityPresentation {
        title,
        summary,
        error,
    };
    TranscriptEntryPart {
        id: TranscriptContentId::Activity(activity.id),
        status: activity_status(activity.state),
        name: Some(schema),
        operation_id: match &activity.payload {
            ActivityPayload::Operation(operation) => Some(operation.call_id.to_string()),
            _ => None,
        },
        content: activity_detail(activity, generic),
    }
}

fn activity_detail(
    activity: &ActivityNode,
    generic: TranscriptActivityPresentation,
) -> TranscriptPartContent {
    match &activity.payload {
        ActivityPayload::Resource(resource) => {
            let kind = match resource.kind {
                ResourceKind::Image => MessageAttachmentKind::Image,
                ResourceKind::Audio => MessageAttachmentKind::Audio,
                ResourceKind::Video => MessageAttachmentKind::Video,
                ResourceKind::Pdf => MessageAttachmentKind::Pdf,
                ResourceKind::File
                | ResourceKind::Directory
                | ResourceKind::Url
                | ResourceKind::Artifact => MessageAttachmentKind::File,
            };
            let (source, sha256) = match &resource.reference {
                ResourceReference::Artifact { sha256, uri } => (
                    MessageAttachmentSource::FileId {
                        file_id: uri.clone(),
                    },
                    Some(sha256.clone()),
                ),
                ResourceReference::WorkspacePath { path } => (
                    MessageAttachmentSource::LocalPath { path: path.clone() },
                    None,
                ),
                ResourceReference::Url { url } => {
                    (MessageAttachmentSource::Url { url: url.clone() }, None)
                }
                ResourceReference::ProviderFile { file_id, .. } => (
                    MessageAttachmentSource::FileId {
                        file_id: file_id.clone(),
                    },
                    None,
                ),
            };
            TranscriptPartContent::Attachment(MessageAttachmentPartResource {
                attachments: vec![MessageAttachment {
                    kind,
                    mime: resource.media_type.clone().unwrap_or_else(|| {
                        if resource.kind == ResourceKind::Directory {
                            "inode/directory".to_owned()
                        } else {
                            String::new()
                        }
                    }),
                    source,
                    filename: Some(resource.name.clone()),
                    title: None,
                    size_bytes: resource.size_bytes,
                    sha256,
                    width: resource.width,
                    height: resource.height,
                    duration_ms: resource.duration_ms,
                    page_count: resource.page_count,
                }],
            })
        }
        ActivityPayload::SkillReference(skill) => {
            TranscriptPartContent::SkillReference(MessageSkillReferencePartResource {
                skills: vec![MessageSkillReference {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    instructions: skill.instructions.clone(),
                    content_hash: skill.content_hash.clone(),
                    source: skill.source.clone(),
                    aliases: skill.aliases.clone(),
                }],
            })
        }
        ActivityPayload::Reasoning(reasoning) => {
            TranscriptPartContent::Reasoning(MessageReasoningPartResource {
                summary: reasoning.content.summary.clone(),
                raw_content: reasoning.content.raw_content.clone(),
                encrypted_content: reasoning.content.encrypted_content.clone(),
            })
        }
        ActivityPayload::Error(error) => TranscriptPartContent::Error(MessageErrorPartResource {
            code: error.code.clone(),
            message: error.message.clone(),
        }),
        _ => TranscriptPartContent::Activity(generic),
    }
}

fn activity_presentation(payload: &ActivityPayload) -> (String, String, String, Option<String>) {
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
        ActivityPayload::SkillExecution(skill) => (
            "skill_execution".to_owned(),
            format!("Skill: {}", skill.skill_name),
            String::new(),
            None,
        ),
        ActivityPayload::TextArtifact(artifact) => (
            "text_artifact".to_owned(),
            artifact
                .label
                .clone()
                .unwrap_or_else(|| "Pasted text".to_owned()),
            artifact.text.clone(),
            None,
        ),
        ActivityPayload::Reasoning(reasoning) => (
            "reasoning".to_owned(),
            "Thinking".to_owned(),
            reasoning.content.preferred_text(),
            None,
        ),
        ActivityPayload::Operation(operation) => (
            "operation".to_owned(),
            if operation.title.trim().is_empty() {
                operation.invocation.name.clone()
            } else {
                operation.title.clone()
            },
            if operation.summary.trim().is_empty() {
                operation.model_output_text.clone()
            } else {
                operation.summary.clone()
            },
            operation.error.as_ref().map(|error| error.message.clone()),
        ),
        ActivityPayload::Interaction(interaction) => match interaction {
            agena_domain::InteractionActivity::Permission { request, .. } => (
                "permission".to_owned(),
                "Permission request".to_owned(),
                request.reason.clone(),
                None,
            ),
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
        ActivityPayload::Progress(progress) => (
            "progress".to_owned(),
            progress.title.clone(),
            progress.detail.clone(),
            None,
        ),
        ActivityPayload::Checklist(checklist) => (
            "checklist".to_owned(),
            "Checklist".to_owned(),
            format!("{} item(s)", checklist.items.len()),
            None,
        ),
        ActivityPayload::Search(search) => (
            "search".to_owned(),
            "Search".to_owned(),
            search.query.clone(),
            None,
        ),
        ActivityPayload::FileChanges(changes) => (
            "file_changes".to_owned(),
            "File changes".to_owned(),
            format!("{} file(s)", changes.changes.len()),
            None,
        ),
        ActivityPayload::NestedTask(task) => (
            "nested_task".to_owned(),
            task.title.clone().unwrap_or_else(|| task.task_id.clone()),
            format!("{:?}", task.status).to_ascii_lowercase(),
            None,
        ),
        ActivityPayload::Maintenance(maintenance) => match maintenance {
            MaintenanceActivity::Compaction { activity, .. } => (
                "compaction".to_owned(),
                "Context compacted".to_owned(),
                format!(
                    "Reduced context from {} to {} tokens",
                    activity.before_tokens, activity.after_tokens
                ),
                None,
            ),
            MaintenanceActivity::Process { process } => (
                "process".to_owned(),
                "Process".to_owned(),
                format!("{process:?}"),
                None,
            ),
        },
        ActivityPayload::Error(error) => (
            "error".to_owned(),
            if error.code.trim().is_empty() {
                "Error".to_owned()
            } else {
                error.code.clone()
            },
            error.message.clone(),
            Some(error.message.clone()),
        ),
        ActivityPayload::Custom(custom) => (
            custom.schema.clone(),
            custom
                .presentation
                .get("title")
                .cloned()
                .unwrap_or_else(|| custom.schema.clone()),
            custom
                .presentation
                .get("summary")
                .cloned()
                .unwrap_or_default(),
            None,
        ),
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

const fn response_status(status: ResponseStatus) -> MessageStatus {
    match status {
        ResponseStatus::Pending => MessageStatus::Pending,
        ResponseStatus::InProgress => MessageStatus::InProgress,
        ResponseStatus::Completed => MessageStatus::Completed,
        ResponseStatus::Failed => MessageStatus::Failed,
        ResponseStatus::Cancelled => MessageStatus::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use agena_domain::{
        ActivityActor, ActivityLifecycle, ActivityOwner, ActivityProvenance, ContentPosition,
        ResourceActivity, SkillReferenceActivity,
    };

    use super::*;

    #[test]
    fn user_entry_keeps_editor_like_inline_activity_placeholders() {
        let turn_id = agena_domain::TurnId::new();
        let response_id = agena_domain::ResponseId::new();
        let first_text_id = agena_domain::ResponseSegmentId::new();
        let second_text_id = agena_domain::ResponseSegmentId::new();
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
                response: agena_domain::ResponseSnapshot {
                    id: response_id,
                    turn_id,
                    execution_id: agena_domain::ExecutionId::new(),
                    status: ResponseStatus::Pending,
                    content: ContentDocument::default(),
                    revision_seq: 1,
                    created_at_ms: 1,
                    finished_at_ms: None,
                },
                created_at_ms: 1,
            }],
            session_activities: Vec::new(),
        };

        let entries = transcript_entries(&snapshot);
        assert_eq!(entries[0].id, TranscriptEntryId::TurnInput(turn_id));
        assert_eq!(entries[1].id, TranscriptEntryId::Response(response_id));
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
            entries[0].parts[0].content,
            TranscriptPartContent::SkillReference(_)
        ));
        assert!(matches!(
            entries[0].parts[1].content,
            TranscriptPartContent::Attachment(_)
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
            ] if *id == first_text_id
                && text == "hi"
                && *first_activity == skill_id
                && first_placeholder == "[Skill: batch]"
                && *second_id == second_text_id
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
}
