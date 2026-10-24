use agena_domain::{ActivityPayload, ResourceReference};

/// Derive the compact human headline for a non-tool activity payload.
pub(crate) fn activity_presentation(
    payload: &ActivityPayload,
) -> (String, String, String, Option<agena_failure::UserProblem>) {
    match payload {
        ActivityPayload::Resource(resource) => (
            "resource".to_owned(),
            nonempty_title(Some(resource.name.as_str()), "Attachment"),
            match &resource.reference {
                ResourceReference::Artifact { uri, .. } => uri.clone(),
                ResourceReference::WorkspacePath { path } => path.clone(),
                ResourceReference::Url { url } => url.clone(),
                ResourceReference::ProviderFile { file_id, .. } => file_id.clone(),
            },
            None,
        ),
        ActivityPayload::CommandReference(command) => (
            "skill_reference".to_owned(),
            if command.name.trim().is_empty() {
                "Command".to_owned()
            } else {
                format!("Command: {}", command.name)
            },
            if command.description.trim().is_empty() {
                command.source.clone()
            } else {
                command.description.clone()
            },
            None,
        ),
        ActivityPayload::TextArtifact(artifact) => (
            "text_artifact".to_owned(),
            nonempty_title(artifact.label.as_deref(), "Pasted text"),
            artifact.text.clone(),
            None,
        ),
        ActivityPayload::Reasoning(reasoning) => (
            "reasoning".to_owned(),
            "Thinking".to_owned(),
            reasoning.content.preferred_text(),
            None,
        ),
        ActivityPayload::TextSegment(segment) => (
            "text_segment".to_owned(),
            "Text".to_owned(),
            segment.text.clone(),
            None,
        ),
        ActivityPayload::Interaction(interaction) => match interaction {
            agena_domain::InteractionActivity::UserInput { request, .. } => {
                let summary = request
                    .questions
                    .iter()
                    .map(|question| question.question.as_str())
                    .filter(|question| !question.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    "user_input".to_owned(),
                    "User input requested".to_owned(),
                    if summary.is_empty() {
                        request.title.clone()
                    } else {
                        summary
                    },
                    None,
                )
            }
        },
        ActivityPayload::Error(error) => (
            "error".to_owned(),
            "Error".to_owned(),
            error.problem.user.fallback.clone(),
            Some(error.problem.clone()),
        ),
        ActivityPayload::Notice(notice) => {
            let title = nonempty_title(
                notice.title.as_deref(),
                notice_kind_title(notice.kind.as_str()),
            );
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
        "system_notification" => "System notification",
        _ => "Notice",
    }
}

fn nonempty_title(title: Option<&str>, fallback: &str) -> String {
    title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}
