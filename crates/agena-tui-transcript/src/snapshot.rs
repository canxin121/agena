//! Canonical transcript snapshot projection for terminal presentation.
//!
//! This is intentionally a projection from `TurnSnapshot` / `ResponseSnapshot`
//! / `ContentNode` rather than an adapter through public message resources.
//! Stable domain identities therefore reach navigation and rendering without
//! fabricated integer message or part ids.

use agena_api::{
    message_part::{MessageTextPartResource, PartExecutionStatusResource},
    resource::{MessageRole, MessageStatus},
};
use agena_domain::{
    ActivityNode, ActivityPayload, ActivityState, ComposerDocument, ComposerNode, ContentDocument,
    ContentNode, MaintenanceActivity, ResourceKind, ResourceReference, ResponseStatus,
    TranscriptSnapshot,
};
use chrono::{DateTime, Utc};

use crate::{
    TranscriptActivityContent, TranscriptContentId, TranscriptEntry, TranscriptEntryId,
    TranscriptEntryPart, TranscriptPartContent, TranscriptResponseLifecycle,
    TranscriptUserActivityStyle, TranscriptUserDocument, TranscriptUserDocumentNode,
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
                role: None,
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

fn response_document_entry(
    response_id: agena_domain::ResponseId,
    state: MessageStatus,
    created_at_ms: i64,
    document: &ContentDocument,
) -> TranscriptEntry {
    let mut parts = document
        .nodes()
        .iter()
        .map(|node| match node {
            ContentNode::Text { segment } => TranscriptEntryPart {
                id: TranscriptContentId::Text(segment.id),
                status: PartExecutionStatusResource::Completed,
                content: TranscriptPartContent::Text(MessageTextPartResource {
                    text: segment.text.clone(),
                    synthetic: false,
                }),
            },
            ContentNode::Activity { activity } => activity_entry_part(activity),
        })
        .collect::<Vec<_>>();
    if document.is_empty() || matches!(state, MessageStatus::Failed | MessageStatus::Cancelled) {
        let (part_status, outcome) = match state {
            MessageStatus::Pending | MessageStatus::InProgress => (
                PartExecutionStatusResource::InProgress,
                TranscriptResponseLifecycle::Running,
            ),
            MessageStatus::Completed => (
                PartExecutionStatusResource::Completed,
                TranscriptResponseLifecycle::Completed,
            ),
            MessageStatus::Failed => (
                PartExecutionStatusResource::Failed,
                TranscriptResponseLifecycle::Failed,
            ),
            MessageStatus::Cancelled => (
                PartExecutionStatusResource::Cancelled,
                TranscriptResponseLifecycle::Cancelled,
            ),
        };
        parts.push(TranscriptEntryPart {
            id: TranscriptContentId::ResponseLifecycle(response_id),
            status: part_status,
            content: TranscriptPartContent::Activity(TranscriptActivityContent::ResponseLifecycle(
                outcome,
            )),
        });
    }
    TranscriptEntry {
        id: TranscriptEntryId::Response(response_id),
        role: Some(MessageRole::Assistant),
        state,
        created_at: timestamp(created_at_ms),
        parts,
    }
}

pub fn pending_user_entry(
    pending_id: u64,
    confirmed: bool,
    document: &ComposerDocument,
) -> TranscriptEntry {
    let mut parts = document
        .0
        .iter()
        .filter_map(|node| match node {
            ComposerNode::Activity { activity } => Some(TranscriptEntryPart {
                id: TranscriptContentId::Activity(activity.id),
                status: PartExecutionStatusResource::Completed,
                content: TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(
                    Box::new(activity.payload.clone()),
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
    TranscriptEntryPart {
        id: TranscriptContentId::Activity(activity.id),
        status: activity_status(activity.state),
        content: TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(Box::new(
            activity.payload.clone(),
        ))),
    }
}

pub(crate) fn activity_presentation(
    payload: &ActivityPayload,
) -> (String, String, String, Option<String>) {
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
        CustomActivity, OperationActivity, ResourceActivity, SkillReferenceActivity,
        StructuredObject, ToolCallId, ToolInvocation, ToolOutput,
    };

    use super::*;

    #[test]
    fn empty_running_response_projects_lifecycle_as_activity_not_empty_message() {
        let response_id = agena_domain::ResponseId::new();
        let entry = response_document_entry(
            response_id,
            MessageStatus::InProgress,
            1,
            &ContentDocument::default(),
        );
        assert!(matches!(
            entry.parts.as_slice(),
            [TranscriptEntryPart {
                id: TranscriptContentId::ResponseLifecycle(id),
                content: TranscriptPartContent::Activity(
                    TranscriptActivityContent::ResponseLifecycle(
                        TranscriptResponseLifecycle::Running
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
                        entry_id: TranscriptEntryId::Response(response_id),
                        content_id: TranscriptContentId::ResponseLifecycle(response_id),
                    }
        }));
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
                payload: ActivityPayload::Custom(CustomActivity {
                    schema: "session_notice".to_owned(),
                    schema_version: 1,
                    data: serde_json::json!({"kind": "notice"}),
                    presentation: std::collections::BTreeMap::from([
                        ("title".to_owned(), "Session notice".to_owned()),
                        ("summary".to_owned(), "Background state changed".to_owned()),
                    ]),
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

    #[test]
    fn canonical_tools_list_activity_retains_expandable_structured_output() {
        let response_id = agena_domain::ResponseId::new();
        let activity_id = agena_domain::ActivityId::new();
        let details = ToolOutput::from_json_payload(Some(&serde_json::json!({
            "tools": [
                {"name": "fs.read", "description": "Read a file"},
                {"name": "repo.status", "description": "Inspect repository status"}
            ]
        })))
        .expect("structured tools_list output");
        let document = ContentDocument::new(vec![ContentNode::activity(ActivityNode {
            id: activity_id,
            owner: ActivityOwner::Response { response_id },
            actor: ActivityActor::Tool,
            state: ActivityState::Completed,
            position: ContentPosition { index: 0 },
            revision_seq: 1,
            lifecycle: ActivityLifecycle::default(),
            payload: ActivityPayload::Operation(OperationActivity {
                call_id: ToolCallId::new("call-tools-list"),
                invocation: ToolInvocation::new("tools_list", StructuredObject::default()),
                title: "tools_list".to_owned(),
                summary: "Listed 2 tools".to_owned(),
                model_output_text: String::new(),
                details,
                resource_activity_ids: Vec::new(),
                error: None,
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry = response_document_entry(response_id, MessageStatus::Completed, 1, &document);
        let key = crate::TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::Response(response_id),
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
        let node = collapsed
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("tools_list Activity node");
        assert!(node.toggleable);
        assert!(!node.expanded);
        assert!(collapsed.lines.iter().any(|line| line.text.contains("▸")));
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
        assert!(
            expanded
                .lines
                .iter()
                .any(|line| line.text.contains("repo.status"))
        );
        assert!(node.copy_text.contains("fs.read"));
    }

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
            &entries[0].parts[0].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload))
                if matches!(payload.as_ref(), ActivityPayload::SkillReference(_))
        ));
        assert!(matches!(
            &entries[0].parts[1].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload))
                if matches!(payload.as_ref(), ActivityPayload::Resource(_))
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
}
