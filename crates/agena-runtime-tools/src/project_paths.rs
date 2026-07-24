use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

const MAX_WORKSPACE_KEY_LEN: usize = 80;
const GENERATED_IMAGE_ARTIFACTS_DIR: &str = "generated_images";

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

    use super::generated_image_artifact_path;

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
}
