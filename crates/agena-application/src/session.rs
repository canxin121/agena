//! Session-facing application services and state.

use crate::{Application, ApplicationError};
use agena_api::resource::{
    PermissionReply, PermissionReplyKind, PermissionScope, RunOptions, SessionExecutionResource,
    UserInputReply, UserInputReplyKind,
};
use agena_domain::{
    ComposerDocument, SessionSummary, UserInputReplyKind as DomainUserInputReplyKind,
};
use agena_runtime::{
    SessionExecutionReplyRequest, SessionExecutionRequest, SessionPermissionReplyRequest,
    SessionRunOptions, SessionUserMessageRequest,
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

pub async fn resolve_session_run_options(
    state: &Application,
    session_id: i64,
    request: RunOptions,
) -> Result<SessionRunOptions, ApplicationError> {
    let default_model = state
        .provider_catalog()
        .default_model()
        .map_err(provider_catalog_error)?;
    let session_services = state.session_execution_services()?;
    state
        .service()
        .resolve_run_options(
            state.provider_catalog().as_ref(),
            default_model,
            session_services.execution_control.as_ref(),
            session_id,
            request,
        )
        .await
}

fn provider_catalog_error(error: agena_provider::ProviderCatalogError) -> ApplicationError {
    match error {
        agena_provider::ProviderCatalogError::InvalidRequest(message) => {
            ApplicationError::bad_request_with_diagnostic(
                "The provider request is invalid.",
                message,
            )
        }
        agena_provider::ProviderCatalogError::NotFound(message) => {
            ApplicationError::not_found_with_diagnostic("The provider was not found.", message)
        }
        agena_provider::ProviderCatalogError::Operation(message) => {
            ApplicationError::internal(message)
        }
    }
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

pub async fn session_user_message_request(
    state: &Application,
    session_id: i64,
    options: RunOptions,
    document: ComposerDocument,
) -> Result<SessionUserMessageRequest, ApplicationError> {
    validate_input_document(&document)?;
    Ok(SessionUserMessageRequest::new(
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
                    || skill.instructions.trim().is_empty()
                    || skill.content_hash.trim().is_empty()
                    || skill.source.trim().is_empty()
                {
                    return Err(ApplicationError::bad_request(
                        "The skill reference is incomplete.",
                    ));
                }
                if skill.instructions.len() > 64 * 1024 {
                    return Err(ApplicationError::bad_request(
                        "The skill instructions exceed the 64 KiB limit.",
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

pub async fn session_execution_resource(
    state: &Application,
    execution_control: &dyn agena_runtime::SessionExecutionControl,
    session_queries: &dyn agena_runtime::SessionQueryService,
    session_id: i64,
) -> Result<SessionExecutionResource, ApplicationError> {
    state
        .service()
        .session_execution_resource(execution_control, session_queries, session_id)
        .await
}
