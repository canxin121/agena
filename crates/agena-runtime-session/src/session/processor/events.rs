use super::{SessionProcessor, persist_generated_media_artifact};

impl SessionProcessor {
    pub(crate) async fn persist_provider_native_tool_media(
        &self,
        session_id: i64,
        call_id: &str,
        blocks: Vec<agena_domain::ViewBlock>,
    ) -> Vec<agena_domain::ViewBlock> {
        let workspace_root = self.workspace_root.as_path();

        let mut media_index = 0usize;
        let mut persisted = Vec::with_capacity(blocks.len());
        for block in blocks {
            match block {
                agena_domain::ViewBlock::Media { id, mut artifact } => {
                    let mime_type = artifact.mime.clone();
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
                            agena_domain::ViewBlock::Media { id, artifact }
                        }
                        Ok(None) => agena_domain::ViewBlock::Media { id, artifact },
                        Err(err) => {
                            tracing::warn!(
                                session_id,
                                call_id,
                                "failed to persist provider media artifact: {err}"
                            );
                            agena_domain::ViewBlock::Media { id, artifact }
                        }
                    };
                    persisted.push(next_block);
                }
                other => persisted.push(other),
            }
        }
        persisted
    }
}
