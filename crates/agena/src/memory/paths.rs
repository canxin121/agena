use std::{
    io,
    path::{Path, PathBuf},
};

/// Returns `~/agena/projects/<sanitized-workspace-root>/memory/`.
fn memory_base_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("agena")
}

pub(crate) use agena_web::workspace_key;

pub struct MemoryDir {
    path: PathBuf,
}

impl MemoryDir {
    /// Build the memory directory path for the given workspace root.
    /// Path: `~/agena/projects/<sanitized-workspace-root>/memory/`
    pub fn from_workspace(workspace_root: &Path) -> Self {
        Self {
            path: memory_base_dir()
                .join("projects")
                .join(workspace_key(workspace_root))
                .join("memory"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entrypoint(&self) -> PathBuf {
        self.path.join("MEMORY.md")
    }

    pub fn index_dir(&self) -> PathBuf {
        self.path.join(".index")
    }

    pub fn ensure_exists(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.path)
    }
}
