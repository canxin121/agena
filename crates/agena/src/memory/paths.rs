use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
};

const MAX_SANITIZED_LENGTH: usize = 200;

/// Convert an absolute path to a filesystem-safe directory name.
/// Replace all non-alphanumeric chars with `-`, then if the result exceeds
/// 200 chars, truncate and append a hash.
fn sanitize_path(path: &str) -> String {
    let sanitized: String = path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    if sanitized.len() <= MAX_SANITIZED_LENGTH {
        return sanitized;
    }

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    let hash = format!("{:x}", hasher.finish());

    format!("{}-{}", &sanitized[..MAX_SANITIZED_LENGTH], hash)
}

/// Returns `~/.agena/projects/<sanitized-workspace-root>/memory/`.
fn memory_base_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".agena")
}

pub struct MemoryDir {
    path: PathBuf,
}

impl MemoryDir {
    /// Build the memory directory path for the given workspace root.
    /// Path: `~/.agena/projects/<sanitized-workspace-root>/memory/`
    pub fn from_workspace(workspace_root: &Path) -> Self {
        let workspace_str = workspace_root.to_string_lossy();
        // Normalize separators before sanitizing so the same workspace gets
        // the same key across platforms.
        let normalized = workspace_str.replace('\\', "/");
        let key = sanitize_path(&normalized);
        Self {
            path: memory_base_dir().join("projects").join(key).join("memory"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entrypoint(&self) -> PathBuf {
        self.path.join("MEMORY.md")
    }

    pub fn ensure_exists(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.path)
    }
}
