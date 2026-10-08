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

/// The canonical API envelope, preserving ownership, lifecycle and source
/// references across lists, snapshots and live delivery.
pub fn part_resource_from_fact(part: &agena_storage::store::Part) -> agena_api::part::PartResource {
    agena_api::part::PartResource {
        part_id: part.part_id,
        sections: Vec::new(),
        kind: agena_runtime_contracts::part_content::canonical_kind(&part.kind, &part.content),
        role: part.role.as_str().into(),
        state: part.state.as_str().into(),
        content: part.content.clone(),
        presentation: None,
        summary: part.summary.clone(),
        visibility: part.visibility.as_str().into(),
        parent_part_id: part.parent_part_id,
        run_id: part.run_id,
        origin_session_id: part.origin_session_id,
        revision: part.revision,
        started_at_ms: part.started_at_ms,
        finished_at_ms: part.finished_at_ms,
        created_at_ms: part.created_at_ms,
        updated_at_ms: part.updated_at_ms,
        user_message_ordinal: None,
        provider_state: part.provider_state.clone(),
    }
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
    let mut commands = 0usize;
    for node in &document.0 {
        let ComposerNode::Activity { activity } = node else {
            continue;
        };
        match &activity.payload {
            ActivityPayload::Resource(resource) => {
                resources = resources.saturating_add(1);
                if resource.delivery == agena_domain::ResourceDelivery::ModelInput
                    && (resource.kind == agena_domain::ResourceKind::Directory
                        || !matches!(resource.reference, ResourceReference::WorkspacePath { .. }))
                {
                    return Err(ApplicationError::bad_request(
                        "send-to-model resources must be explicit workspace files; directories, URLs and foreign provider IDs remain references",
                    ));
                }

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
            ActivityPayload::CommandReference(command) => {
                commands = commands.saturating_add(1);
                if command.name.trim().is_empty()
                    || command.content_hash.trim().is_empty()
                    || command.source.trim().is_empty()
                {
                    return Err(ApplicationError::bad_request(
                        "The command reference is incomplete.",
                    ));
                }
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
    if commands > 8 {
        return Err(ApplicationError::bad_request(
            "The command references exceed the per-message limit.",
        ));
    }
    Ok(())
}
