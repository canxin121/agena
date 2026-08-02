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
    ComposerNode, ContentDocument, ContentNode, MaintenanceActivity, ResourceKind,
    ResourceReference, TranscriptSnapshot,
};
use chrono::{DateTime, Utc};

use crate::{
    TranscriptActivityContent, TranscriptAssistantReplyLifecycle, TranscriptContentId,
    TranscriptEntry, TranscriptEntryId, TranscriptEntryPart, TranscriptPartContent,
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
        if !turn.input.is_empty() {
            entries.push(user_document_entry(
                turn.id,
                turn.created_at_ms,
                &turn.input,
            ));
        }
        entries.push(assistant_reply_document_entry(
            turn.reply.id,
            assistant_reply_status(turn.reply.status),
            turn.reply.created_at_ms,
            &turn.reply.content,
            turn.reply.failure.clone(),
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

fn assistant_reply_document_entry(
    reply_id: agena_domain::AssistantReplyId,
    state: MessageStatus,
    created_at_ms: i64,
    document: &ContentDocument,
    failure: Option<agena_failure::UserProblem>,
) -> TranscriptEntry {
    let mut parts = assistant_reply_document_parts(document);
    if document.is_empty() || assistant_reply_state_requires_outcome(state) {
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

fn assistant_reply_document_parts(document: &ContentDocument) -> Vec<TranscriptEntryPart> {
    document
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
        .collect()
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
) -> TranscriptEntryPart {
    let (status, lifecycle) = match state {
        MessageStatus::Pending | MessageStatus::InProgress => (
            PartExecutionStatusResource::InProgress,
            TranscriptAssistantReplyLifecycle::Running,
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
    let (_schema, title, summary, problem) = activity_presentation(&activity.payload);
    let _generic = TranscriptActivityPresentation {
        title,
        summary,
        problem,
    };
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
            operation_activity_title(operation),
            operation.summary.clone(),
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
            "Error".to_owned(),
            error.problem.user.fallback.clone(),
            Some(error.problem.clone()),
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

fn operation_activity_title(operation: &agena_domain::OperationActivity) -> String {
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
        CustomActivity, OperationActivity, OperationActivityError, OperationAuthorization,
        OperationPermission, PermissionAction, PermissionReply, PermissionReplyKind,
        PermissionRequest, PermissionRiskLevel, ResourceActivity, SkillReferenceActivity,
        StructuredObject, ToolCallId, ToolInvocation, ToolOutput,
    };
    use agena_failure::{
        Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility,
        RecoveryDirective, RetryDirective, UserPresentation,
    };

    use super::*;

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
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::InProgress,
            1,
            &ContentDocument::default(),
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
                                sections: Vec::new(),
                                model_output_text: "Updated config.json".to_owned(),
                                details: ToolOutput::default(),
                                resource_activity_ids: Vec::new(),
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
                                            risk: PermissionRiskLevel::Medium,
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
    fn canonical_tools_list_activity_expands_text_without_structured_output() {
        let response_id = agena_domain::AssistantReplyId::new();
        let activity_id = agena_domain::ActivityId::new();
        let output_text = "Available tools: returned 2 of 133 starting at offset 100.\n- fs.read [read_only]: Read a file\n- repo.status [read_only]: Inspect repository status\nMore available: yes. Continue with `tools_list` using `offset: 102`.";
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
                sections: Vec::new(),
                model_output_text: output_text.to_owned(),
                details: ToolOutput::default(),
                resource_activity_ids: Vec::new(),
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
        assert!(expanded_text.contains("Result\n"), "{expanded_text}");
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
                sections: Vec::new(),
                model_output_text: markdown_result.to_owned(),
                details: ToolOutput::default(),
                resource_activity_ids: Vec::new(),
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
        assert!(
            collapsed_text.contains("Inspect shell.run"),
            "{collapsed_text}"
        );
        assert!(
            collapsed_text.contains("Found 3 checks"),
            "{collapsed_text}"
        );
        assert!(!collapsed_text.contains('`'), "{collapsed_text}");
        assert!(!collapsed_text.contains("**"), "{collapsed_text}");
        assert!(collapsed.lines.iter().any(|line| {
            line.text.contains("Inspect shell.run")
                && line
                    .rich_line
                    .as_ref()
                    .is_some_and(|line| line.spans.len() >= 7)
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
        assert!(expanded_text.contains("▾ Result"), "{expanded_text}");
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
            .expect("default-expanded Result section");
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
            result_collapsed_text.contains("▸ Result · Checks"),
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
            .expect("collapsed Result section");
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
        let full_error = "shell.run filesystem_effects must declare every accessed path because the command appears to touch the filesystem: invokes 'python3' which may read or write local files";
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
                        "command": "python3 - <<'EOF'\nprint('pi')\nEOF",
                        "description": "Compute pi with Python",
                        "filesystem_effects": [],
                        "network_effects": [],
                        "shell": "bash",
                        "timeout_ms": 60_000
                    }))
                    .expect("structured shell.run input"),
                ),
                title: "shell.run".to_owned(),
                summary: truncated_summary.clone(),
                sections: Vec::new(),
                model_output_text: full_error.to_owned(),
                details: ToolOutput::default(),
                resource_activity_ids: Vec::new(),
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
            expanded_text.contains("▸ Input · 6 fields"),
            "{expanded_text}"
        );
        assert!(!expanded_text.contains("┌─ json"), "{expanded_text}");
        assert!(expanded_text.contains("▾ Error\n"), "{expanded_text}");
        assert!(!expanded_text.contains("Result\n"), "{expanded_text}");
        assert_eq!(
            expanded_text
                .matches("shell.run filesystem_effects must declare every accessed path")
                .count(),
            1,
            "{expanded_text}"
        );
        assert_eq!(
            expanded_text
                .matches("which may read or write local files")
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
                sections: Vec::new(),
                model_output_text: partial_output.to_owned(),
                details: ToolOutput::default(),
                resource_activity_ids: Vec::new(),
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

        assert!(expanded_text.contains("Result\n"), "{expanded_text}");
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
                sections: vec![agena_domain::ToolPresentationSection {
                    title: "Matches".to_owned(),
                    text: "src/activity.rs\nsrc/tool_result.rs".to_owned(),
                }],
                model_output_text: "2 matches".to_owned(),
                details: ToolOutput::from_json_payload(Some(&serde_json::json!({
                    "matches": ["src/activity.rs", "src/tool_result.rs"]
                })))
                .expect("structured result"),
                resource_activity_ids: Vec::new(),
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
        assert!(collapsed_text.contains("Search repository"));
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
        assert!(expanded_text.contains("2 matches"), "{expanded_text}");
    }

    #[test]
    fn canonical_activity_headlines_keep_titles_and_use_remaining_width_for_summaries() {
        let response_id = agena_domain::AssistantReplyId::new();
        let operation_document = |title: &str, summary: &str| {
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
                    invocation: ToolInvocation::new("shell.run", StructuredObject::default()),
                    title: title.to_owned(),
                    summary: summary.to_owned(),
                    sections: Vec::new(),
                    model_output_text: summary.to_owned(),
                    details: ToolOutput::default(),
                    resource_activity_ids: Vec::new(),
                    authorization: Default::default(),
                    error: None,
                }),
                provenance: ActivityProvenance::default(),
            })])
        };

        let ordinary_title = "Process run Create a tiny test PNG with pure python";
        let ordinary = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &operation_document(ordinary_title, &"PNG result details ".repeat(30)),
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
        assert!(ordinary_headline.text.contains(ordinary_title));
        assert!(ordinary_headline.text.contains("PNG result details"));
        assert!(ordinary_headline.text.ends_with('…'));
        assert!(unicode_width::UnicodeWidthStr::width(ordinary_headline.text.as_str()) <= 80);

        let long_title = format!("Inspect {}", "very-long-component-".repeat(8));
        let long = assistant_reply_document_entry(
            response_id,
            MessageStatus::Completed,
            1,
            &operation_document(long_title.as_str(), "complete"),
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
            .find(|line| line.text.contains("Inspect"))
            .expect("long Operation headline");
        assert!(long_headline.text.ends_with('…'));
        assert!(unicode_width::UnicodeWidthStr::width(long_headline.text.as_str()) <= 120);
    }

    #[test]
    fn operation_activity_titles_use_the_producer_contract_without_ui_inference() {
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
                title: title.to_owned(),
                summary: String::new(),
                sections: Vec::new(),
                model_output_text: String::new(),
                details: ToolOutput::default(),
                resource_activity_ids: Vec::new(),
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
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::Failed,
            1,
            &ContentDocument::default(),
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
        assert!(expanded_text.contains("Reference:"), "{expanded_text}");
    }
}
