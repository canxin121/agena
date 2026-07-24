use base64::Engine;

use super::{AppError, BASE64_STANDARD, Path};

#[derive(Debug)]
pub(crate) struct PersistedMediaArtifact {
    pub(crate) path: String,
    pub(crate) filename: String,
    pub(crate) size_bytes: u64,
}

pub(crate) async fn persist_generated_media_artifact(
    workspace_root: &Path,
    session_id: i64,
    call_id: &str,
    media_index: usize,
    mime_type: &str,
    filename_hint: Option<&str>,
    uri: &str,
) -> Result<Option<PersistedMediaArtifact>, AppError> {
    let Some((decoded_mime, encoded)) = parse_base64_data_url(uri) else {
        return Ok(None);
    };
    let effective_mime = if decoded_mime.is_empty() {
        mime_type.trim()
    } else {
        decoded_mime.as_str()
    };
    if !effective_mime.starts_with("image/") {
        return Ok(None);
    }

    let bytes = BASE64_STANDARD
        .decode(encoded.as_bytes())
        .map_err(|err| AppError::Internal(format!("invalid generated image payload: {err}")))?;
    let extension = generated_media_extension(filename_hint, effective_mime);
    let artifact_id = if media_index == 0 {
        call_id.to_string()
    } else {
        format!("{call_id}-{media_index}")
    };
    let path = agena_runtime::generated_image_artifact_path(
        workspace_root,
        session_id,
        artifact_id.as_str(),
        extension.as_str(),
    );
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, bytes.as_slice()).await?;

    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("generated-image.{extension}"));
    Ok(Some(PersistedMediaArtifact {
        path: path.to_string_lossy().to_string(),
        filename,
        size_bytes: bytes.len() as u64,
    }))
}

pub(crate) fn parse_base64_data_url(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim();
    let payload = trimmed.strip_prefix("data:")?;
    let (metadata, encoded) = payload.split_once(',')?;
    let metadata = metadata.trim();
    let encoded = encoded.trim();
    if encoded.is_empty() {
        return None;
    }
    let mime = metadata.strip_suffix(";base64")?.trim().to_owned();
    Some((mime, encoded.to_owned()))
}

pub(crate) fn generated_media_extension(filename_hint: Option<&str>, mime_type: &str) -> String {
    if let Some(extension) = filename_hint
        .and_then(|value| Path::new(value).extension())
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return extension.to_ascii_lowercase();
    }

    mime_type
        .trim()
        .strip_prefix("image/")
        .filter(|value| !value.is_empty())
        .unwrap_or("png")
        .to_ascii_lowercase()
}
