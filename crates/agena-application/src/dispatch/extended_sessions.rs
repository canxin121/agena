use crate::{Application, ApplicationError};
use agena_api::extended_sessions::*;
use agena_api::live::{ToolDetailResource, ToolDetailSection};
use agena_runtime_contracts::part_content::ToolCallContent;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;

fn value<T: Serialize>(data: T) -> Result<Value, ApplicationError> {
    serde_json::to_value(data).map_err(|error| ApplicationError::internal_error(&error))
}

impl Application {
    pub(crate) async fn dispatch_extended_session_query(
        &self,
        query: SessionQuery,
    ) -> Result<Value, ApplicationError> {
        match query {
            SessionQuery::Overview {
                workspace_id,
                recent_limit,
            } => value(self.session_overview(workspace_id, recent_limit).await?),
            SessionQuery::ExecutionAttempt {
                session_id,
                execution_id,
                attempt_generation,
            } => value(
                self.execution_attempt(session_id, execution_id, attempt_generation)
                    .await?,
            ),
            SessionQuery::PromptPreview { session_id } => value(
                self.session_query_service()?
                    .prompt_preview(session_id)
                    .await
                    .map_err(|error| ApplicationError::from_failure(*error.failure))?,
            ),
            SessionQuery::Plan { session_id } => {
                if self.service().get_session(session_id).await?.is_none() {
                    return Err(ApplicationError::not_found("The session was not found."));
                }
                value(
                    self.invoke_plugin_tool(
                        "agena.plan",
                        "get",
                        json!({"view":"full"}),
                        Some(session_id),
                    )
                    .await?,
                )
            }
            SessionQuery::ToolDetail {
                session_id,
                part_id,
                section,
                locale,
            } => value(
                self.tool_detail(session_id, part_id, section, locale.as_deref())
                    .await?,
            ),
            SessionQuery::QueuedMessage {
                session_id,
                execution_id,
            } => value(
                self.queued_message_for_edit(session_id, execution_id)
                    .await?,
            ),
            SessionQuery::InlineUi {
                session_id,
                reference,
            } => value(
                self.read_inline_ui_reference(session_id, &reference)
                    .await?,
            ),
            SessionQuery::InlineUiSnapshot {
                session_id,
                reference,
            } => value(self.read_inline_ui_snapshot(session_id, &reference).await?),
            SessionQuery::FileChanges { session_id, query } => {
                value(self.session_file_changes(session_id, query).await?)
            }
            SessionQuery::WorkspaceStats { mut ids } => {
                if ids.len() > 128 || ids.iter().any(|id| *id <= 0) {
                    return Err(ApplicationError::bad_request(
                        "At most 128 positive workspace ids are allowed",
                    ));
                }
                ids.sort_unstable();
                ids.dedup();
                let stats = self
                    .session_kernel()?
                    .reader
                    .workspace_statistics(&ids)
                    .await
                    .map_err(crate::kernel_error)?;
                Ok(json!({"items": ids.into_iter().map(|id| {
                    let row = stats.get(&id).copied().unwrap_or_default();
                    json!({"workspace_id":id,"stats":{"total":row.total,"roots":row.roots,"pinned":row.pinned,"running":row.running,"attention":row.attention}})
                }).collect::<Vec<_>>()}))
            }
            SessionQuery::ReadStream(request) => {
                value(self.operation_io.read_stream(request).await?)
            }
            SessionQuery::ReadTransfer(request) => {
                value(self.operation_io.read_transfer(request).await?)
            }
        }
    }

    pub(crate) async fn dispatch_extended_session_command(
        &self,
        command: SessionCommand,
    ) -> Result<Value, ApplicationError> {
        match command {
            SessionCommand::UpdateMetadata {
                session_id,
                update,
                expected_version,
            } => {
                if let Some(version) = expected_version {
                    self.service()
                        .assert_session_version(session_id, version)
                        .await?;
                }
                value(self.service().replace_session(session_id, update).await?)
            }
            SessionCommand::UpdatePermission {
                session_id,
                permission,
                expected_version,
            } => {
                if let Some(version) = expected_version {
                    self.service()
                        .assert_session_version(session_id, version)
                        .await?;
                }
                value(
                    self.set_session_permission(session_id, permission.into())
                        .await?,
                )
            }
            SessionCommand::CancelExecutionAttempt {
                session_id,
                execution_id,
                attempt_generation,
            } => value(
                self.cancel_execution_attempt(session_id, execution_id, attempt_generation)
                    .await?,
            ),
            SessionCommand::EditQueuedMessage {
                request_id,
                session_id,
                execution_id,
                expected_revision,
                document,
            } => value(
                self.edit_queued_message(agena_runtime::session::SessionQueuedMessageEditRequest {
                    request_id,
                    session_id,
                    execution_id,
                    expected_revision,
                    document,
                })
                .await?,
            ),
            SessionCommand::DeleteQueuedMessage {
                request_id,
                session_id,
                execution_id,
            } => value(
                self.delete_queued_message(request_id, session_id, execution_id)
                    .await?,
            ),
            SessionCommand::TakeQueuedMessage {
                request_id,
                session_id,
                execution_id,
            } => value(
                self.take_queued_message(request_id, session_id, execution_id)
                    .await?,
            ),
            SessionCommand::PauseQueue { session_id, paused } => {
                value(self.set_message_queue_paused(session_id, paused).await?)
            }
            SessionCommand::ClearQueue {
                request_id,
                session_id,
            } => value(self.clear_message_queue(request_id, session_id).await?),
            SessionCommand::StartBtw {
                session_id,
                question,
                options,
            } => {
                let answers = self
                    .start_btw(
                        session_id,
                        agena_api::resource::BtwRequest { question, options },
                    )
                    .await?;
                value(self.register_serialized_stream("btw_answer", answers))
            }
            SessionCommand::OpenStream(source) => value(self.open_operation_stream(source).await?),
            SessionCommand::CloseStream { stream_id } => {
                self.operation_io.close_stream(stream_id);
                Ok(json!({"closed":true}))
            }
            SessionCommand::Export { session_id } => {
                let temp = self
                    .run_blocking(|_| tempfile::NamedTempFile::new())
                    .await?
                    .map_err(|e| ApplicationError::internal_error(&e))?;
                let file = temp
                    .reopen()
                    .map_err(|e| ApplicationError::internal_error(&e))?;
                let mut file = tokio::fs::File::from_std(file);
                self.export_session_to(session_id, &mut file).await?;
                file.flush()
                    .await
                    .map_err(|e| ApplicationError::internal_error(&e))?;
                drop(file);
                value(
                    self.operation_io
                        .register_transfer(temp, None, Some(format!("session-{session_id}.ndjson")))
                        .await?,
                )
            }
            SessionCommand::OpenMedia { session_id, path } => {
                let (filename, bytes) = self
                    .service()
                    .read_session_media_file(
                        session_id,
                        crate::dto::WorkspaceFileDownloadQuery { path },
                    )
                    .await?;
                let temp = self
                    .run_blocking(|_| tempfile::NamedTempFile::new())
                    .await?
                    .map_err(|e| ApplicationError::internal_error(&e))?;
                let mut file = tokio::fs::File::from_std(
                    temp.reopen()
                        .map_err(|e| ApplicationError::internal_error(&e))?,
                );
                file.write_all(&bytes)
                    .await
                    .map_err(|e| ApplicationError::internal_error(&e))?;
                file.flush()
                    .await
                    .map_err(|e| ApplicationError::internal_error(&e))?;
                drop(file);
                value(
                    self.operation_io
                        .register_transfer(temp, None, Some(filename))
                        .await?,
                )
            }
            SessionCommand::BeginUpload(purpose) => {
                if let UploadPurpose::Attachment { session_id, .. } = &purpose {
                    if self.service().get_session(*session_id).await?.is_none() {
                        return Err(ApplicationError::not_found("The session was not found."));
                    }
                }
                let temp = self
                    .run_blocking(|_| tempfile::NamedTempFile::new())
                    .await?
                    .map_err(|e| ApplicationError::internal_error(&e))?;
                value(
                    self.operation_io
                        .register_transfer(temp, Some(purpose), None)
                        .await?,
                )
            }
            SessionCommand::WriteTransfer {
                transfer_id,
                offset,
                data_base64,
            } => Ok(
                json!({"next_offset":self.operation_io.write_transfer(transfer_id, offset, data_base64).await?}),
            ),
            SessionCommand::FinishUpload { transfer_id } => {
                self.operation_io.finish_upload(self, transfer_id).await
            }
            SessionCommand::CloseTransfer { transfer_id } => {
                self.operation_io.close_transfer(transfer_id);
                Ok(json!({"closed":true}))
            }
        }
    }

    pub async fn tool_detail(
        &self,
        session_id: i64,
        part_id: i64,
        section: ToolDetailSection,
        locale: Option<&str>,
    ) -> Result<ToolDetailResource, ApplicationError> {
        let page = self
            .read_parts(agena_api::queries::ReadPartsParams {
                session_id,
                ids: vec![part_id],
                sections: ToolDetailSection::ALL.to_vec(),
                ..Default::default()
            })
            .await?;
        let part = page
            .parts
            .into_iter()
            .find(|part| part.part_id == part_id)
            .ok_or_else(|| ApplicationError::not_found("The tool entry was not found."))?;
        self.project_tool_detail(&part, section, locale)
            .await
            .ok_or_else(|| ApplicationError::not_found("The tool part was not found."))
    }

    pub async fn project_tool_detail(
        &self,
        part: &agena_api::part::PartResource,
        section: ToolDetailSection,
        locale: Option<&str>,
    ) -> Option<ToolDetailResource> {
        if part.kind != ToolCallContent::kind() {
            return None;
        }
        let content = ToolCallContent::try_from(&part.content).ok()?;
        let value = match section {
            ToolDetailSection::Metadata => serde_json::to_value(content.metadata).ok()?,
            ToolDetailSection::Input => content.input,
            ToolDetailSection::Output => serde_json::to_value(content.output).ok()?,
            ToolDetailSection::Presentation => {
                let mut document = self.part_document(part).await;
                if let Some(document) = document.as_mut() {
                    crate::part_locale::localize_part_document(document, locale);
                }
                serde_json::to_value(document).ok()?
            }
        };
        Some(ToolDetailResource {
            part_id: part.part_id,
            revision: part.revision,
            updated_at_ms: part.updated_at_ms,
            section,
            part_state: (section == ToolDetailSection::Output).then(|| part.state.clone()),
            value,
        })
    }
}
