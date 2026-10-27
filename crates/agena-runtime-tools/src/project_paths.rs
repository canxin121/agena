use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

const GENERATED_IMAGE_ARTIFACTS_DIR: &str = "generated_images";
pub const MAX_GENERATED_IMAGE_BYTES: usize = 50 * 1024 * 1024;
static GENERATED_IMAGE_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

#[derive(Debug, Clone, PartialEq, Eq)]
/// A generated image artifact managed by the runtime.
pub struct ManagedGeneratedImageArtifact {
    pub path: String,
    pub filename: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, thiserror::Error)]
/// Error managing generated image artifacts.
pub enum ManagedGeneratedImageError {
    #[error("generated image payload exceeds {limit_mib} MiB limit")]
    TooLarge { limit_mib: usize },
    #[error("invalid generated image payload: {0}")]
    InvalidBase64(#[from] base64::DecodeError),
    #[error("invalid generated image content: {0}")]
    InvalidImage(String),
    #[error("failed to persist generated image artifact: {0}")]
    Io(#[from] std::io::Error),
}

/// Stable, per-workspace directory for runtime-managed state.
pub fn project_state_dir(workspace_root: &Path) -> PathBuf {
    agena_storage_layout::StorageLayout::current().workspace_dir(workspace_root)
}

/// Resolve a file inside this workspace's managed attachment stores. Returning
/// the canonical boundary lets callers keep their ordinary workspace policy
/// without permitting arbitrary files from the instance data directory.
pub fn managed_attachment_root(workspace_root: &Path, target: &Path) -> Option<PathBuf> {
    managed_attachment_root_in(
        &agena_storage_layout::StorageLayout::current(),
        workspace_root,
        target,
    )
}

fn managed_attachment_root_in(
    layout: &agena_storage_layout::StorageLayout,
    workspace_root: &Path,
    target: &Path,
) -> Option<PathBuf> {
    [
        layout.uploads_dir(workspace_root),
        layout.artifacts_dir(workspace_root),
    ]
    .into_iter()
    .filter_map(|root| {
        let canonical = root.canonicalize().ok()?;
        // A replaced managed directory must never grant access to its
        // external symlink target, even when that target contains a file.
        (canonical == root).then_some(canonical)
    })
    .find(|root| target.starts_with(root))
}

/// Stable location for a generated image that belongs to one tool call.
/// Runtime owns this process-managed artifact convention so tool/session
/// implementations do not need their own path convention.
pub fn generated_image_artifact_path(
    workspace_root: &Path,
    session_id: i64,
    call_id: &str,
    extension: &str,
) -> PathBuf {
    let stem = sanitize_component(call_id, "generated-image");
    let extension = sanitize_extension(extension);
    project_state_dir(workspace_root)
        .join(GENERATED_IMAGE_ARTIFACTS_DIR)
        .join(session_id.to_string())
        .join(format!("{stem}.{extension}"))
}

/// Copy a provider-returned image data URL into the process-managed artifact
/// store. Callers must reject `Ok(None)` when a terminal direct image API is
/// expected; conversation processors may preserve non-data provider media for
/// ordinary message rendering.
pub async fn persist_generated_image_artifact(
    workspace_root: &Path,
    session_id: i64,
    artifact_id: &str,
    media_index: usize,
    mime_type: &str,
    filename_hint: Option<&str>,
    uri: &str,
) -> Result<Option<ManagedGeneratedImageArtifact>, ManagedGeneratedImageError> {
    let workspace = workspace_root.to_owned();
    let artifact_id = artifact_id.to_owned();
    let mime_type = mime_type.to_owned();
    let filename_hint = filename_hint.map(ToOwned::to_owned);
    let uri = uri.to_owned();
    GENERATED_IMAGE_WORKERS
        .run(move || {
            persist_generated_image_artifact_sync(
                &workspace,
                session_id,
                &artifact_id,
                media_index,
                &mime_type,
                filename_hint.as_deref(),
                &uri,
            )
        })
        .await
        .map_err(|error| std::io::Error::other(format!("generated image worker failed: {error}")))?
}

fn persist_generated_image_artifact_sync(
    workspace_root: &Path,
    session_id: i64,
    artifact_id: &str,
    media_index: usize,
    mime_type: &str,
    filename_hint: Option<&str>,
    uri: &str,
) -> Result<Option<ManagedGeneratedImageArtifact>, ManagedGeneratedImageError> {
    let Some((decoded_mime, encoded)) = parse_base64_image_data_url(uri) else {
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
    let max_encoded = 4 * MAX_GENERATED_IMAGE_BYTES.div_ceil(3);
    if encoded.len() > max_encoded {
        return Err(ManagedGeneratedImageError::TooLarge {
            limit_mib: MAX_GENERATED_IMAGE_BYTES / (1024 * 1024),
        });
    }
    let bytes = STANDARD.decode(encoded.as_bytes())?;
    if bytes.len() > MAX_GENERATED_IMAGE_BYTES {
        return Err(ManagedGeneratedImageError::TooLarge {
            limit_mib: MAX_GENERATED_IMAGE_BYTES / (1024 * 1024),
        });
    }
    validate_generated_image(&bytes, effective_mime)?;
    // Provider filenames are hints, never authority for the stored format.
    let _ = filename_hint;
    let extension = generated_media_extension(None, effective_mime);
    let artifact_id = if media_index == 0 {
        artifact_id.to_owned()
    } else {
        format!("{artifact_id}-{media_index}")
    };
    let path = generated_image_artifact_path(
        workspace_root,
        session_id,
        artifact_id.as_str(),
        extension.as_str(),
    );
    if let Some(parent) = path.parent() {
        agena_storage_layout::secure_directory(parent)?;
    }
    let size_bytes = bytes.len() as u64;
    let sha256 = hex::encode(Sha256::digest(bytes.as_slice()));
    if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "generated image target cannot be a symbolic link",
        )
        .into());
    }
    // A retry can verify an already published artifact, but may not replace
    // its bytes. This also keeps the filename stable across commit retries.
    match crate::atomic_create_file(&path, bytes.as_slice(), None) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            crate::verify_file_contents(&path, bytes.as_slice())?;
        }
        Err(error) => return Err(error.into()),
    }
    agena_storage_layout::secure_file(&path)?;

    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("generated-image.{extension}"));
    Ok(Some(ManagedGeneratedImageArtifact {
        path: path.to_string_lossy().to_string(),
        filename,
        size_bytes,
        sha256,
    }))
}

/// Validate both the actual raster format and bounded decompression before
/// publishing provider bytes as a trusted image attachment.
fn validate_generated_image(
    bytes: &[u8],
    declared_mime: &str,
) -> Result<(), ManagedGeneratedImageError> {
    let invalid = |message: &str| ManagedGeneratedImageError::InvalidImage(message.into());
    let format = image::guess_format(bytes).map_err(|_| invalid("unrecognized raster image"))?;
    let actual = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        _ => return Err(invalid("unsupported generated raster format")),
    };
    let declared = if declared_mime == "image/jpg" {
        "image/jpeg"
    } else {
        declared_mime
    };
    if declared != actual {
        return Err(invalid("declared MIME does not match image bytes"));
    }
    let size = imagesize::blob_size(bytes).map_err(|_| invalid("invalid image dimensions"))?;
    if size.width == 0
        || size.height == 0
        || size.width > 16384
        || size.height > 16384
        || (size.width as u64).saturating_mul(size.height as u64)
            > crate::media_input::MAX_IMAGE_PIXELS
    {
        return Err(invalid("image exceeds the 40 megapixel/16384 pixel limit"));
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(crate::media_input::MAX_IMAGE_PIXELS * 4);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| invalid("corrupt image or decoded-memory limit exceeded"))?;
    if u64::from(decoded.width()) * u64::from(decoded.height())
        > crate::media_input::MAX_IMAGE_PIXELS
    {
        return Err(invalid("image exceeds the 40 megapixel limit"));
    }
    Ok(())
}

pub fn parse_base64_image_data_url(url: &str) -> Option<(String, String)> {
    let payload = url.trim().strip_prefix("data:")?;
    let (metadata, encoded) = payload.split_once(',')?;
    let mime = metadata.trim().strip_suffix(";base64")?.trim();
    let encoded = encoded.trim();
    if encoded.is_empty() {
        return None;
    }
    Some((mime.to_owned(), encoded.to_owned()))
}

pub fn generated_media_extension(filename_hint: Option<&str>, mime_type: &str) -> String {
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

/// Root directory for Agena's process-managed local state.
pub fn agena_home_dir() -> PathBuf {
    agena_storage_layout::StorageLayout::current()
        .root()
        .to_path_buf()
}

fn sanitize_component(value: &str, fallback: &str) -> String {
    let sanitized = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

fn sanitize_extension(value: &str) -> String {
    let sanitized = value
        .trim()
        .trim_start_matches('.')
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase();
    if sanitized.is_empty() {
        "png".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod generated_attachment_tests {
    use super::*;
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4/5/hPwAH/QL+ppTFtAAAAABJRU5ErkJggg==";
    #[test]
    fn generated_bytes_must_be_a_real_image_of_the_declared_mime() {
        let bytes = STANDARD.decode(PNG).unwrap();
        assert!(validate_generated_image(&bytes, "image/png").is_ok());
        assert!(validate_generated_image(&bytes, "image/jpeg").is_err());
        assert!(validate_generated_image(b"<html>not a PNG</html>", "image/png").is_err());
        assert!(validate_generated_image(&bytes[..20], "image/png").is_err());
    }
    #[tokio::test]
    async fn repeated_publication_never_overwrites_a_completed_image() {
        let workspace = std::env::temp_dir().join(format!(
            "agena-generated-attachment-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let url = format!("data:image/png;base64,{PNG}");
        let first = persist_generated_image_artifact(
            &workspace,
            43,
            "call",
            0,
            "image/png",
            Some("wrong.html"),
            &url,
        )
        .await
        .unwrap()
        .unwrap();
        assert!(first.path.ends_with(".png"));
        let same =
            persist_generated_image_artifact(&workspace, 43, "call", 0, "image/png", None, &url)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(same, first);
        let mut changed = STANDARD.decode(PNG).unwrap();
        changed.push(1);
        let changed_url = format!("data:image/png;base64,{}", STANDARD.encode(&changed));
        assert!(
            persist_generated_image_artifact(
                &workspace,
                43,
                "call",
                0,
                "image/png",
                None,
                &changed_url
            )
            .await
            .is_err()
        );
        assert_eq!(
            std::fs::read(&first.path).unwrap(),
            STANDARD.decode(PNG).unwrap()
        );
        std::fs::remove_dir_all(project_state_dir(&workspace)).unwrap();
        std::fs::remove_dir_all(workspace).unwrap();
    }
}
