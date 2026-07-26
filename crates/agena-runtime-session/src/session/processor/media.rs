use super::{AppError, Path};
type PersistedMediaArtifact = agena_runtime_tools::ManagedGeneratedImageArtifact;

pub(crate) async fn persist_generated_media_artifact(
    workspace_root: &Path,
    session_id: i64,
    call_id: &str,
    media_index: usize,
    mime_type: &str,
    filename_hint: Option<&str>,
    uri: &str,
) -> Result<Option<PersistedMediaArtifact>, AppError> {
    agena_runtime_tools::persist_generated_image_artifact(
        workspace_root,
        session_id,
        call_id,
        media_index,
        mime_type,
        filename_hint,
        uri,
    )
    .await
    .map_err(|error| AppError::Internal(error.to_string()))
}
