use agena_api::{
    part::{PartExecutionStatusResource, TextPartResource},
    resource::{RunRole, RunStatus},
};
use agena_domain::{ActivityPayload, ComposerDocument, ComposerNode, ResourceReference};
use chrono::{DateTime, Utc};

use crate::{
    TranscriptActivityContent, TranscriptContentId, TranscriptEntry, TranscriptEntryId,
    TranscriptEntryPart, TranscriptPartContent,
};

/// Project an optimistic composer document into the same part shape used by
/// the persisted v2 parts transcript.
pub fn pending_user_entry<'a>(
    pending_id: u64,
    confirmed: bool,
    document: &'a ComposerDocument,
) -> TranscriptEntry<'a> {
    let status = if confirmed {
        PartExecutionStatusResource::Completed
    } else {
        PartExecutionStatusResource::InProgress
    };
    let parts = document
        .0
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let id = TranscriptContentId::PendingPart {
                pending_id,
                index: index as u32,
            };
            match node {
                ComposerNode::Text { text } => TranscriptEntryPart {
                    id,
                    status,
                    content: TranscriptPartContent::Text(TextPartResource {
                        text: text.clone(),
                        synthetic: false,
                    }),
                },
                ComposerNode::Activity { activity } => TranscriptEntryPart {
                    id,
                    status: PartExecutionStatusResource::Completed,
                    content: TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(
                        &activity.payload,
                    )),
                },
            }
        })
        .collect::<Vec<_>>();
    TranscriptEntry {
        id: TranscriptEntryId::PendingTurn(pending_id),
        role: Some(RunRole::User),
        state: if confirmed {
            RunStatus::Completed
        } else {
            RunStatus::InProgress
        },
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        parts,
    }
}

/// Derive the compact human headline for a non-tool activity payload.
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
            artifact.text.clone(),
            None,
        ),
        ActivityPayload::Reasoning(reasoning) => (
            "reasoning".to_owned(),
            "Thinking".to_owned(),
            reasoning_first_line(reasoning.content.preferred_text().as_str()),
            None,
        ),
        ActivityPayload::TextSegment(segment) => (
            "text_segment".to_owned(),
            "Text".to_owned(),
            segment.text.clone(),
            None,
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
        ActivityPayload::Notice(notice) => {
            let title = notice
                .title
                .clone()
                .unwrap_or_else(|| notice_kind_title(notice.kind.as_str()).to_owned());
            ("notice".to_owned(), title, notice.summary.clone(), None)
        }
    }
}

fn notice_kind_title(kind: &str) -> &'static str {
    match kind {
        "hook" => "Hook",
        "compaction" => "Compaction",
        "provider_retry" => "Provider retry",
        "max_turns_exhausted" => "Turn limit",
        _ => "Notice",
    }
}

fn reasoning_first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Thinking")
        .to_owned()
}
