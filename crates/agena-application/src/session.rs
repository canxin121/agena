//! Session-facing application services and state.

use crate::{Application, ApplicationError};
use agena_api::resource::{
    PermissionReply, PermissionReplyKind, PermissionScope, RunOptions, UserInputReply,
    UserInputReplyKind,
};
use agena_domain::{
    ComposerDocument, SessionSummary, UserInputReplyKind as DomainUserInputReplyKind,
};
use agena_runtime::{
    SessionExecutionReplyRequest, SessionExecutionRequest, SessionPermissionReplyRequest,
    SessionRunOptions, SessionUserRunRequest,
};

pub fn session_resource_from_summary(
    summary: SessionSummary,
) -> agena_api::resource::SessionResource {
    crate::service::sessions::session_resource_from_summary(summary)
}

pub async fn session_user_input_reply_request(
    state: &Application,
    session_id: i64,
    options: RunOptions,
    reply: UserInputReply,
) -> Result<SessionExecutionReplyRequest<agena_domain::UserInputReply>, ApplicationError> {
    session_execution_reply_request(
        state,
        session_id,
        options,
        agena_domain::UserInputReply {
            request_id: reply.request_id,
            kind: match reply.kind {
                UserInputReplyKind::Submit => DomainUserInputReplyKind::Submit,
                UserInputReplyKind::Cancel => DomainUserInputReplyKind::Cancel,
                UserInputReplyKind::Timeout => DomainUserInputReplyKind::Timeout,
            },
            answers: reply.answers,
            reason: reply.reason,
        },
    )
    .await
}

/// Flatten projected session runs into the v2 part transcript projection.
///
/// Each projected run is one v2 run: its id is the run marker part id, so the
/// projection emits a `run` marker part followed by the run's content parts in
/// order. `role` is the run's role; content parts carry `run_id` linking them
/// back to the marker. This is the shared transcript shape for the REST
/// `SessionExecutionResource.parts` and the JSON-RPC `messages/list` /
/// `message/submit` surfaces.
pub fn project_session_transcript(
    runs: &[agena_runtime::SessionProjectedRun],
) -> Vec<agena_api::resource::SessionTranscriptPart> {
    let mut parts = Vec::new();
    for run in runs {
        let run_id = run.id;
        parts.push(agena_api::resource::SessionTranscriptPart {
            part_id: run_id,
            kind: "run".to_owned(),
            role: run.role.to_string(),
            state: run.state.to_string(),
            content: if run.metadata.is_object() {
                run.metadata.clone()
            } else {
                serde_json::json!({})
            },
            presentation: None,
            summary: None,
            created_at_ms: run.created_at.timestamp_millis(),
            parent_part_id: None,
            run_id: None,
        });
        for part in &run.parts {
            parts.push(agena_api::resource::SessionTranscriptPart {
                part_id: part.id,
                kind: part.kind.to_string(),
                role: run.role.to_string(),
                state: part.status.to_string(),
                content: part.content.clone().unwrap_or(serde_json::Value::Null),
                presentation: None,
                summary: part.summary.clone(),
                created_at_ms: part.created_at.timestamp_millis(),
                parent_part_id: None,
                run_id: Some(run_id),
            });
        }
    }
    parts
}

pub async fn resolve_session_run_options(
    state: &Application,
    session_id: i64,
    request: RunOptions,
) -> Result<SessionRunOptions, ApplicationError> {
    let session_services = state.session_execution_services()?;
    state
        .service()
        .resolve_run_options(
            state.provider_catalog().as_ref(),
            session_services.execution_control.as_ref(),
            session_id,
            request,
        )
        .await
}

pub async fn session_execution_request(
    state: &Application,
    session_id: i64,
    request: RunOptions,
) -> Result<SessionExecutionRequest, ApplicationError> {
    Ok(SessionExecutionRequest::new(
        session_id,
        resolve_session_run_options(state, session_id, request).await?,
    ))
}

pub async fn session_execution_reply_request<T>(
    state: &Application,
    session_id: i64,
    options: RunOptions,
    reply: T,
) -> Result<SessionExecutionReplyRequest<T>, ApplicationError> {
    Ok(SessionExecutionReplyRequest::new(
        session_id,
        resolve_session_run_options(state, session_id, options).await?,
        reply,
    ))
}

fn permission_reply_from_wire(value: PermissionReply) -> agena_domain::PermissionReply {
    agena_domain::PermissionReply {
        request_id: value.request_id,
        kind: match value.kind {
            PermissionReplyKind::AllowOnce => agena_domain::PermissionReplyKind::AllowOnce,
            PermissionReplyKind::AllowAlways => agena_domain::PermissionReplyKind::AllowAlways,
            PermissionReplyKind::DenyOnce => agena_domain::PermissionReplyKind::DenyOnce,
            PermissionReplyKind::DenyAlways => agena_domain::PermissionReplyKind::DenyAlways,
            PermissionReplyKind::AutoApprove => agena_domain::PermissionReplyKind::AutoApprove,
        },
        reason: value.reason,
        scope: value.scope.map(|scope| match scope {
            PermissionScope::Session => agena_domain::PermissionScope::Session,
            PermissionScope::Workspace => agena_domain::PermissionScope::Workspace,
            PermissionScope::Global => agena_domain::PermissionScope::Global,
        }),
    }
}

pub async fn session_permission_reply_request(
    state: &Application,
    session_id: i64,
    options: RunOptions,
    reply: PermissionReply,
    source: Option<String>,
) -> Result<SessionPermissionReplyRequest, ApplicationError> {
    Ok(SessionPermissionReplyRequest::new(
        session_id,
        resolve_session_run_options(state, session_id, options).await?,
        permission_reply_from_wire(reply),
        source,
    ))
}

pub async fn session_user_run_request(
    state: &Application,
    session_id: i64,
    options: RunOptions,
    document: ComposerDocument,
) -> Result<SessionUserRunRequest, ApplicationError> {
    validate_input_document(&document)?;
    Ok(SessionUserRunRequest::new(
        session_id,
        resolve_session_run_options(state, session_id, options).await?,
        document,
    ))
}

pub fn validate_input_document(document: &ComposerDocument) -> Result<(), ApplicationError> {
    use agena_domain::{ActivityPayload, ComposerNode, ResourceReference};

    if document.is_empty() {
        return Err(ApplicationError::bad_request(
            "The message must contain text or an attachment.",
        ));
    }
    let mut resources = 0usize;
    let mut skills = 0usize;
    let mut skill_bytes = 0usize;
    for node in &document.0 {
        let ComposerNode::Activity { activity } = node else {
            continue;
        };
        match &activity.payload {
            ActivityPayload::Resource(resource) => {
                resources = resources.saturating_add(1);
                match &resource.reference {
                    ResourceReference::Artifact { sha256, uri }
                        if sha256.trim().is_empty() || uri.trim().is_empty() =>
                    {
                        return Err(ApplicationError::bad_request(
                            "The artifact attachment is incomplete.",
                        ));
                    }
                    ResourceReference::WorkspacePath { path } if path.trim().is_empty() => {
                        return Err(ApplicationError::bad_request(
                            "The workspace attachment needs a relative path.",
                        ));
                    }
                    ResourceReference::WorkspacePath { path }
                        if std::path::Path::new(path).is_absolute()
                            || path.split('/').any(|part| part == "..") =>
                    {
                        return Err(ApplicationError::bad_request(
                            "The workspace attachment path must be relative and normalized.",
                        ));
                    }
                    ResourceReference::Url { url } if url.trim().is_empty() => {
                        return Err(ApplicationError::bad_request(
                            "The URL attachment needs a URL.",
                        ));
                    }
                    ResourceReference::ProviderFile {
                        provider_id,
                        file_id,
                    } if provider_id.trim().is_empty() || file_id.trim().is_empty() => {
                        return Err(ApplicationError::bad_request(
                            "The provider attachment is incomplete.",
                        ));
                    }
                    _ => {}
                }
            }
            ActivityPayload::SkillReference(skill) => {
                skills = skills.saturating_add(1);
                if skill.name.trim().is_empty()
                    || skill.content_hash.trim().is_empty()
                    || skill.source.trim().is_empty()
                {
                    return Err(ApplicationError::bad_request(
                        "The skill reference is incomplete.",
                    ));
                }
                if skill.instructions.len() > 64 * 1024 {
                    return Err(ApplicationError::bad_request(
                        "The legacy skill instructions exceed the 64 KiB limit.",
                    ));
                }
                skill_bytes = skill_bytes.saturating_add(skill.instructions.len());
            }
            ActivityPayload::TextArtifact(artifact) => {
                if artifact.text.is_empty() {
                    return Err(ApplicationError::bad_request(
                        "The text attachment cannot be empty.",
                    ));
                }
            }
            _ => {
                return Err(ApplicationError::bad_request(
                    "This activity type cannot be sent as message input.",
                ));
            }
        }
    }
    if resources > 8 {
        return Err(ApplicationError::bad_request(
            "A message cannot contain more than 8 attachments.",
        ));
    }
    if skills > 8 || skill_bytes > 256 * 1024 {
        return Err(ApplicationError::bad_request(
            "The skill references exceed the per-message limit.",
        ));
    }
    Ok(())
}
