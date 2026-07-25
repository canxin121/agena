use super::{
    AppError, EventKind, Message, MessagePartCheckpointedEvent, MessagePartDeltaEvent,
    OperationBlock, PartDeltaField, PublishContext, SessionProcessor, SessionRunRequest, Utc,
    persist_generated_media_artifact,
};

impl SessionProcessor {
    pub(crate) async fn persist_provider_native_tool_media(
        &self,
        session_id: i64,
        call_id: &str,
        blocks: Vec<OperationBlock>,
    ) -> Vec<OperationBlock> {
        let workspace_root = self.workspace_root.as_path();

        let mut media_index = 0usize;
        let mut persisted = Vec::with_capacity(blocks.len());
        for block in blocks {
            match block {
                OperationBlock::Media {
                    mime_type,
                    mut artifact,
                } => {
                    let next_block = match persist_generated_media_artifact(
                        workspace_root,
                        session_id,
                        call_id,
                        media_index,
                        mime_type.as_str(),
                        artifact.name.as_deref(),
                        artifact.uri.as_str(),
                    )
                    .await
                    {
                        Ok(Some(saved)) => {
                            media_index += 1;
                            artifact.uri = saved.path;
                            artifact.size_bytes = Some(saved.size_bytes);
                            artifact.sha256 = Some(saved.sha256);
                            if artifact.name.is_none() {
                                artifact.name = Some(saved.filename);
                            }
                            OperationBlock::Media {
                                mime_type,
                                artifact,
                            }
                        }
                        Ok(None) => OperationBlock::Media {
                            mime_type,
                            artifact,
                        },
                        Err(err) => {
                            tracing::warn!(
                                session_id,
                                call_id,
                                "failed to persist provider media artifact: {err}"
                            );
                            OperationBlock::Media {
                                mime_type,
                                artifact,
                            }
                        }
                    };
                    persisted.push(next_block);
                }
                other => persisted.push(other),
            }
        }
        persisted
    }

    pub(crate) async fn checkpoint_part(
        &self,
        run: &SessionRunRequest,
        assistant: &Message,
        part_id: i64,
    ) -> Result<(), AppError> {
        let Some(publisher) = run.event_publisher.as_ref() else {
            return Ok(());
        };

        let part = assistant
            .parts
            .iter()
            .find(|part| part.id == part_id)
            .cloned()
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "part snapshot not found for stream event: {part_id}"
                ))
            })?;
        let kind = EventKind::MessagePartCheckpointed(MessagePartCheckpointedEvent {
            session_id: run.session_id,
            execution_id: Some(run.execution_id),
            run_id: Some(run.run_id),
            message_id: assistant.id,
            message_role: assistant.role,
            message_state: assistant.state,
            message_created_at: assistant.created_at,
            message_metadata: assistant.metadata.clone(),
            part,
            ts_ms: Utc::now().timestamp_millis(),
        });
        publisher
            .publish(PublishContext::for_session(run.session_id), kind)
            .await
            .map_err(|err| AppError::Internal(format!("publish part checkpoint failed: {err}")))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn emit_part_delta(
        &self,
        run: &SessionRunRequest,
        assistant: &Message,
        part_id: i64,
        call_id: Option<i64>,
        field: PartDeltaField,
        delta: String,
        seq: u64,
    ) -> Result<(), AppError> {
        let Some(publisher) = run.event_publisher.as_ref() else {
            return Ok(());
        };

        let _ = assistant; // assistant snapshot is no longer needed: events
        // carry their own routing context.
        let kind = EventKind::MessagePartDelta(MessagePartDeltaEvent {
            session_id: run.session_id,
            execution_id: Some(run.execution_id),
            run_id: Some(run.run_id),
            message_id: assistant.id,
            part_id,
            call_id,
            field,
            delta,
            seq,
            ts_ms: Utc::now().timestamp_millis(),
        });
        publisher
            .publish(PublishContext::for_session(run.session_id), kind)
            .await
            .map_err(|err| AppError::Internal(format!("publish part-delta failed: {err}")))?;
        Ok(())
    }
}
