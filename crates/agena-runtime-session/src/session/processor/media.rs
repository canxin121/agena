use base64::Engine;
use sha2::{Digest, Sha256};

use super::{AppError, BASE64_STANDARD, Path};

/// A provider-native image is copied into the process-managed artifact store,
/// not streamed directly to a renderer. Keep the same 50 MiB ceiling as the
/// explicit filesystem image viewer so an untrusted provider payload cannot
/// force unbounded decode allocation in the session processor.
const MAX_GENERATED_IMAGE_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct PersistedMediaArtifact {
    pub(crate) path: String,
    pub(crate) filename: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
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

    // Base64 expands a payload by roughly 4/3. Reject before decoding when
    // the lower-bound decoded size already exceeds the artifact limit.
    let decoded_lower_bound = encoded.len().saturating_mul(3) / 4;
    if decoded_lower_bound > MAX_GENERATED_IMAGE_BYTES {
        return Err(AppError::Internal(format!(
            "generated image payload exceeds {} MiB limit",
            MAX_GENERATED_IMAGE_BYTES / (1024 * 1024)
        )));
    }

    let bytes = BASE64_STANDARD
        .decode(encoded.as_bytes())
        .map_err(|err| AppError::Internal(format!("invalid generated image payload: {err}")))?;
    if bytes.len() > MAX_GENERATED_IMAGE_BYTES {
        return Err(AppError::Internal(format!(
            "generated image payload exceeds {} MiB limit",
            MAX_GENERATED_IMAGE_BYTES / (1024 * 1024)
        )));
    }
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
    let sha256 = hex::encode(Sha256::digest(bytes.as_slice()));
    Ok(Some(PersistedMediaArtifact {
        path: path.to_string_lossy().to_string(),
        filename,
        size_bytes: bytes.len() as u64,
        sha256,
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{generated_media_extension, parse_base64_data_url};

    #[test]
    fn data_url_parser_requires_base64_media() {
        assert_eq!(
            parse_base64_data_url("data:image/png;base64,aGVsbG8="),
            Some(("image/png".to_string(), "aGVsbG8=".to_string()))
        );
        assert_eq!(
            parse_base64_data_url("https://example.test/image.png"),
            None
        );
        assert_eq!(parse_base64_data_url("data:image/png,raw"), None);
    }

    #[test]
    fn generated_extension_prefers_filename_then_mime() {
        assert_eq!(
            generated_media_extension(Some("output.JPEG"), "image/png"),
            "jpeg"
        );
        assert_eq!(generated_media_extension(None, "image/webp"), "webp");
        assert_eq!(generated_media_extension(Some("."), "image/png"), "png");
        assert_eq!(
            Path::new("generated-image.webp").extension().unwrap(),
            "webp"
        );
    }
}
