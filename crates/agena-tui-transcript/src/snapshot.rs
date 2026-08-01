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
) -> TranscriptEntry {
    let mut parts = assistant_reply_document_parts(document);
    if document.is_empty() || assistant_reply_state_requires_outcome(state) {
        parts.push(assistant_reply_lifecycle_part(reply_id, state));
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
            TranscriptAssistantReplyLifecycle::Failed,
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
            if operation.summary.trim().is_empty() {
                operation.model_output_text.clone()
            } else {
                operation.summary.clone()
            },
            operation.error.as_ref().map(|error| error.problem.clone()),
        ),
        ActivityPayload::Interaction(interaction) => match interaction {
            agena_domain::InteractionActivity::Permission { request, .. } => (
                "permission".to_owned(),
                permission_request_title(&request.action),
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

fn permission_request_title(action: &agena_domain::PermissionAction) -> String {
    match action {
        agena_domain::PermissionAction::Tool { tool_name, .. } => {
            format!("Permission: {tool_name}")
        }
        agena_domain::PermissionAction::PathAccess {
            access_kind,
            target_path,
            ..
        } => format!("Permission: {access_kind} {target_path}"),
        agena_domain::PermissionAction::NetworkAccess { target, .. } => {
            format!("Permission: network access to {target}")
        }
    }
}

fn operation_activity_title(operation: &agena_domain::OperationActivity) -> String {
    use agena_domain::ToolApiFunction;

    let function = operation
        .invocation
        .tool_api_call
        .as_ref()
        .map(|call| call.function);
    let input = serde_json::Value::from(operation.invocation.input.clone());
    let input_text = |name: &str| {
        input
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let persisted_tool_api_title = |fallback: String| {
        let configured = operation_presentation_title(operation);
        if configured == fallback
            || configured
                .strip_prefix(fallback.as_str())
                .is_some_and(|suffix| suffix.starts_with(" · "))
        {
            configured.to_owned()
        } else {
            fallback
        }
    };

    match function {
        Some(ToolApiFunction::List) => persisted_tool_api_title("List tools".to_owned()),
        Some(ToolApiFunction::Search) => {
            let label = input_text("query")
                .map(|query| format!("Search tools · {query}"))
                .unwrap_or_else(|| "Search tools".to_owned());
            persisted_tool_api_title(label)
        }
        Some(ToolApiFunction::Help) => persisted_tool_api_title(
            input_text("tool")
                .map(|tool| format!("Inspect {tool}"))
                .unwrap_or_else(|| "Inspect tool".to_owned()),
        ),
        Some(ToolApiFunction::Tags) => persisted_tool_api_title("List tool tags".to_owned()),
        Some(ToolApiFunction::Call) => {
            let configured = operation_presentation_title(operation);
            if !configured.is_empty()
                && configured != operation.invocation.name
                && configured != format!("Tool {}", operation.invocation.name)
                && configured != format!("Run {}", operation.invocation.name)
            {
                format!("{} · {configured}", operation.invocation.name)
            } else {
                operation.invocation.name.clone()
            }
        }
        None => {
            let configured = operation_presentation_title(operation);
            if configured.is_empty()
                || configured == operation.invocation.name
                || configured == format!("Tool {}", operation.invocation.name)
                || configured == format!("Run {}", operation.invocation.name)
            {
                operation.invocation.name.clone()
            } else {
                configured.to_owned()
            }
        }
    }
}

fn operation_presentation_title(operation: &agena_domain::OperationActivity) -> &str {
    let title = operation.title.trim();
    if !title.is_empty() {
        return title;
    }
    operation.summary.trim()
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
        CustomActivity, OperationActivity, ResourceActivity, SkillReferenceActivity,
        StructuredObject, ToolCallId, ToolInvocation, ToolOutput,
    };

    use super::*;

    #[test]
    fn empty_running_reply_projects_lifecycle_as_activity_not_empty_message() {
        let response_id = agena_domain::AssistantReplyId::new();
        let entry = assistant_reply_document_entry(
            response_id,
            MessageStatus::InProgress,
            1,
            &ContentDocument::default(),
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
        let permission_activity_id = agena_domain::ActivityId::new();
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
                            id: permission_activity_id,
                            owner: ActivityOwner::AssistantReply { reply_id },
                            actor: ActivityActor::Runtime,
                            state: ActivityState::Completed,
                            position: ContentPosition { index: 1 },
                            revision_seq: 2,
                            lifecycle: ActivityLifecycle::default(),
                            payload: ActivityPayload::Custom(CustomActivity {
                                schema: "permission_decision".to_owned(),
                                schema_version: 1,
                                data: serde_json::json!({"decision": "allow"}),
                                presentation: Default::default(),
                            }),
                            provenance: ActivityProvenance::default(),
                        }),
                        ContentNode::text("continued after permission"),
                    ]),
                    revision_seq: 3,
                    created_at_ms: 1,
                    finished_at_ms: Some(3),
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
                    content_id: TranscriptContentId::Activity(permission_activity_id),
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
                error: None,
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry =
            assistant_reply_document_entry(response_id, MessageStatus::Completed, 1, &document);
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
        let expanded_text = expanded
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(expanded_text.contains("Result\n"), "{expanded_text}");
        assert!(expanded_text.contains("Input\n"), "{expanded_text}");
        assert!(expanded_text.contains("\"limit\": 33"), "{expanded_text}");
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
                error: None,
            }),
            provenance: ActivityProvenance::default(),
        })]);
        let entry =
            assistant_reply_document_entry(response_id, MessageStatus::Completed, 1, &document);
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
        assert!(collapsed_text.contains("Search repository · 2 matches"));
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
    }

    #[test]
    fn operation_activity_titles_describe_the_action_instead_of_the_gateway_function() {
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
                error: None,
            }
        };

        assert_eq!(
            operation_activity_title(&operation(
                "tools_list",
                serde_json::json!({}),
                "tools_list"
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
                "Tool tools_search",
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
                "Tool tools_help",
            )),
            "Inspect fs.read"
        );
        assert_eq!(
            operation_activity_title(&operation(
                "tools_call",
                serde_json::json!({"tool": "fs.read", "input": {"path": "README.md"}}),
                "fs.read",
            )),
            "fs.read"
        );
        assert_eq!(
            operation_activity_title(&operation(
                "tools_call",
                serde_json::json!({"tool": "fs.read", "input": {"path": "README.md"}}),
                "Read README.md",
            )),
            "fs.read · Read README.md"
        );
        assert_eq!(
            operation_activity_title(&operation(
                "agena.repo.status",
                serde_json::json!({}),
                "Tool agena.repo.status",
            )),
            "agena.repo.status"
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
}
