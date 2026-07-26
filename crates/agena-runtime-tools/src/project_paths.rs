use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

const MAX_WORKSPACE_KEY_LEN: usize = 80;
const GENERATED_IMAGE_ARTIFACTS_DIR: &str = "generated_images";
pub const MAX_GENERATED_IMAGE_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedGeneratedImageArtifact {
    pub path: String,
    pub filename: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ManagedGeneratedImageError {
    #[error("generated image payload exceeds {limit_mib} MiB limit")]
    TooLarge { limit_mib: usize },
    #[error("invalid generated image payload: {0}")]
    InvalidBase64(#[from] base64::DecodeError),
    #[error("failed to persist generated image artifact: {0}")]
    Io(#[from] std::io::Error),
}

/// Stable, per-workspace directory for runtime-managed state.
pub fn project_state_dir(workspace_root: &Path) -> PathBuf {
    agena_home_dir()
        .join("projects")
        .join(workspace_key(workspace_root))
}

pub fn snapshot_managed_dir(workspace_root: &Path) -> PathBuf {
    project_state_dir(workspace_root).join("snapshots")
}

pub fn snapshot_rift_database_path(workspace_root: &Path) -> PathBuf {
    project_state_dir(workspace_root).join("rift.sqlite")
}

/// Stable location for a generated image that belongs to one tool call.
/// Runtime owns this process-managed artifact convention so tool/session
/// implementations do not need a compatibility path shim.
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
/// backward-compatible rendering.
pub async fn persist_generated_image_artifact(
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
    let decoded_lower_bound = encoded.len().saturating_mul(3) / 4;
    if decoded_lower_bound > MAX_GENERATED_IMAGE_BYTES {
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
    let extension = generated_media_extension(filename_hint, effective_mime);
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
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, bytes.as_slice()).await?;

    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("generated-image.{extension}"));
    Ok(Some(ManagedGeneratedImageArtifact {
        path: path.to_string_lossy().to_string(),
        filename,
        size_bytes: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(bytes.as_slice())),
    }))
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
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("agena")
}

fn workspace_key(workspace_root: &Path) -> String {
    let normalized = workspace_root.to_string_lossy().replace('\\', "/");
    let mut sanitized = normalized
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    while sanitized.contains("--") {
        sanitized = sanitized.replace("--", "-");
    }
    sanitized = sanitized.trim_matches('-').to_string();
    if sanitized.is_empty() {
        sanitized = "workspace".to_string();
    }
    if sanitized.len() <= MAX_WORKSPACE_KEY_LEN {
        return sanitized;
    }

    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    format!(
        "{}-{:x}",
        &sanitized[..MAX_WORKSPACE_KEY_LEN],
        hasher.finish()
    )
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
mod tests {
    use std::path::Path;

    use super::{
        generated_image_artifact_path, generated_media_extension, parse_base64_image_data_url,
    };

    #[test]
    fn generated_image_path_sanitizes_untrusted_call_parts() {
        let path = generated_image_artifact_path(
            Path::new("workspace"),
            42,
            "  call/with spaces  ",
            ".WEBP!",
        );
        assert!(path.ends_with("generated_images/42/call-with-spaces.webp"));
    }

    #[test]
    fn data_url_parser_requires_base64_media() {
        assert_eq!(
            parse_base64_image_data_url("data:image/png;base64,aGVsbG8="),
            Some(("image/png".to_string(), "aGVsbG8=".to_string()))
        );
        assert_eq!(
            parse_base64_image_data_url("https://example.test/image.png"),
            None
        );
        assert_eq!(parse_base64_image_data_url("data:image/png,raw"), None);
    }

    #[test]
    fn generated_extension_prefers_filename_then_mime() {
        assert_eq!(
            generated_media_extension(Some("output.JPEG"), "image/png"),
            "jpeg"
        );
        assert_eq!(generated_media_extension(None, "image/webp"), "webp");
        assert_eq!(generated_media_extension(Some("."), "image/png"), "png");
    }
}
