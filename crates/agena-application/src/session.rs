use crate::{Application, ApplicationError};
use agena_api::resource::{
    MessageAttachment, MessageAttachmentKind, MessageAttachmentPart, MessageAttachmentSource,
    MessagePartContent, MessageSkillReferencePart, MessageTextPart, PermissionReply,
    PermissionReplyKind, PermissionScope, RunOptions, SessionExecutionResource, UserInputReply,
    UserInputReplyKind,
};
use agena_domain::{SessionSummary, UserInputReplyKind as DomainUserInputReplyKind};
use agena_runtime::{
    SessionExecutionReplyRequest, SessionExecutionRequest, SessionPermissionReplyRequest,
    SessionRunOptions, SessionUserMessagePart, SessionUserMessageRequest,
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
            ApplicationError::BadRequest(message)
        }
        agena_provider::ProviderCatalogError::NotFound(message) => {
            ApplicationError::NotFound(message)
        }
        agena_provider::ProviderCatalogError::Operation(message) => {
            ApplicationError::Internal(message)
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
    parts: Vec<MessagePartContent>,
) -> Result<SessionUserMessageRequest<SessionUserMessagePart>, ApplicationError> {
    Ok(SessionUserMessageRequest::new(
        session_id,
        resolve_session_run_options(state, session_id, options).await?,
        parts
            .into_iter()
            .map(session_user_message_part_from_wire)
            .collect(),
    ))
}

/// Convert the public user-message write contract into the Runtime-owned input
/// contract. Terminal, HTTP, and other presentation adapters share this one
/// mapping rather than constructing Runtime-private message parts.
pub fn session_user_message_part_from_wire(value: MessagePartContent) -> SessionUserMessagePart {
    match value {
        MessagePartContent::Text(MessageTextPart { text, synthetic }) => {
            SessionUserMessagePart::Text(agena_domain::TextPart { text, synthetic })
        }
        MessagePartContent::Attachment(MessageAttachmentPart { attachments }) => {
            SessionUserMessagePart::Attachment(agena_plugin_host::sdk::attachment::AttachmentPart {
                attachments: attachments
                    .into_iter()
                    .map(message_attachment_from_wire)
                    .collect(),
            })
        }
        MessagePartContent::SkillReference(MessageSkillReferencePart { skills }) => {
            SessionUserMessagePart::SkillReference(agena_runtime::message::SkillReferencePart {
                skills: skills
                    .into_iter()
                    .map(|skill| agena_runtime::message::SkillReference {
                        name: skill.name,
                        description: skill.description,
                        instructions: skill.instructions,
                        content_hash: skill.content_hash,
                        source: skill.source,
                        aliases: skill.aliases,
                    })
                    .collect(),
            })
        }
    }
}

fn message_attachment_from_wire(
    value: MessageAttachment,
) -> agena_plugin_host::sdk::attachment::AttachmentItem {
    agena_plugin_host::sdk::attachment::AttachmentItem {
        kind: match value.kind {
            MessageAttachmentKind::Image => {
                agena_plugin_host::sdk::attachment::AttachmentKind::Image
            }
            MessageAttachmentKind::Audio => {
                agena_plugin_host::sdk::attachment::AttachmentKind::Audio
            }
            MessageAttachmentKind::Video => {
                agena_plugin_host::sdk::attachment::AttachmentKind::Video
            }
            MessageAttachmentKind::Pdf => agena_plugin_host::sdk::attachment::AttachmentKind::Pdf,
            MessageAttachmentKind::File => agena_plugin_host::sdk::attachment::AttachmentKind::File,
        },
        mime: value.mime,
        source: match value.source {
            MessageAttachmentSource::Url { url } => {
                agena_plugin_host::sdk::attachment::AttachmentSource::Url { url }
            }
            MessageAttachmentSource::DataUrl { url } => {
                agena_plugin_host::sdk::attachment::AttachmentSource::DataUrl { url }
            }
            MessageAttachmentSource::Base64 { data } => {
                agena_plugin_host::sdk::attachment::AttachmentSource::Base64 { data }
            }
            MessageAttachmentSource::FileId { file_id } => {
                agena_plugin_host::sdk::attachment::AttachmentSource::FileId { file_id }
            }
            MessageAttachmentSource::LocalPath { path } => {
                agena_plugin_host::sdk::attachment::AttachmentSource::LocalPath { path }
            }
        },
        filename: value.filename,
        title: value.title,
        size_bytes: value.size_bytes,
        sha256: value.sha256,
        width: value.width,
        height: value.height,
        duration_ms: value.duration_ms,
        page_count: value.page_count,
    }
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

#[cfg(test)]
mod tests {
    use super::{session_user_message_part_from_wire, *};

    #[test]
    fn maps_user_message_parts_without_leaking_core_types_into_the_wire_contract() {
        let text = session_user_message_part_from_wire(MessagePartContent::Text(MessageTextPart {
            text: "hello".to_owned(),
            synthetic: true,
        }));
        assert_eq!(
            text,
            SessionUserMessagePart::Text(agena_domain::TextPart {
                text: "hello".to_owned(),
                synthetic: true,
            })
        );

        let attachment = session_user_message_part_from_wire(MessagePartContent::Attachment(
            MessageAttachmentPart {
                attachments: vec![MessageAttachment {
                    kind: MessageAttachmentKind::Pdf,
                    mime: "application/pdf".to_owned(),
                    source: MessageAttachmentSource::FileId {
                        file_id: "file-123".to_owned(),
                    },
                    filename: Some("report.pdf".to_owned()),
                    title: Some("Report".to_owned()),
                    size_bytes: Some(42),
                    sha256: Some("hash".to_owned()),
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: Some(3),
                }],
            },
        ));
        assert_eq!(
            attachment,
            SessionUserMessagePart::Attachment(
                agena_plugin_host::sdk::attachment::AttachmentPart {
                    attachments: vec![agena_plugin_host::sdk::attachment::AttachmentItem {
                        kind: agena_plugin_host::sdk::attachment::AttachmentKind::Pdf,
                        mime: "application/pdf".to_owned(),
                        source: agena_plugin_host::sdk::attachment::AttachmentSource::FileId {
                            file_id: "file-123".to_owned(),
                        },
                        filename: Some("report.pdf".to_owned()),
                        title: Some("Report".to_owned()),
                        size_bytes: Some(42),
                        sha256: Some("hash".to_owned()),
                        width: None,
                        height: None,
                        duration_ms: None,
                        page_count: Some(3),
                    }],
                }
            )
        );

        let skill = session_user_message_part_from_wire(MessagePartContent::SkillReference(
            MessageSkillReferencePart {
                skills: vec![agena_api::resource::MessageSkillReference {
                    name: "review".to_owned(),
                    description: "Review changes".to_owned(),
                    instructions: "Inspect the diff.".to_owned(),
                    content_hash: "abc123".to_owned(),
                    source: "bundled".to_owned(),
                    aliases: vec!["code-review".to_owned()],
                }],
            },
        ));
        assert_eq!(
            skill,
            SessionUserMessagePart::SkillReference(agena_runtime::message::SkillReferencePart {
                skills: vec![agena_runtime::message::SkillReference {
                    name: "review".to_owned(),
                    description: "Review changes".to_owned(),
                    instructions: "Inspect the diff.".to_owned(),
                    content_hash: "abc123".to_owned(),
                    source: "bundled".to_owned(),
                    aliases: vec!["code-review".to_owned()],
                }],
            })
        );
    }
}
