use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

const MAX_WORKSPACE_KEY_LEN: usize = 80;

pub(crate) fn agena_home_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("agena")
}

pub(crate) fn project_state_dir(workspace_root: &Path) -> PathBuf {
    agena_home_dir()
        .join("projects")
        .join(workspace_key(workspace_root))
}

fn workspace_key(workspace_root: &Path) -> String {
    let normalized = workspace_root.to_string_lossy().replace('\\', "/");
    sanitize_path(&normalized)
}

fn sanitize_path(value: &str) -> String {
    let mut sanitized = value
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
    value.hash(&mut hasher);
    let hash = format!("{:x}", hasher.finish());
    format!("{}-{hash}", &sanitized[..MAX_WORKSPACE_KEY_LEN])
}
