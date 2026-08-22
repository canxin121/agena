//! Filesystem implementation of the portable memory repository contract.

use std::{
    fs, io,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

use crate::{
    MemoryDir, MemoryError, MemoryFrontmatter, MemoryRecord, MemoryRepository, MemoryResult,
    NewMemory,
};

const ENTRYPOINT_NAME: &str = "MEMORY.md";
static MEMORY_FILE_WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn with_memory_write_lock<T>(operation: impl FnOnce() -> T) -> T {
    let _guard = MEMORY_FILE_WRITE_LOCK.lock().unwrap_or_else(|error| {
        tracing::error!(
            diagnostic = %error,
            "memory file write lock is poisoned; recovering serialized write access"
        );
        error.into_inner()
    });
    operation()
}

fn write_memory_file_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("memory path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;
    let permissions = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Some(metadata.permissions()),
        Ok(_) => None,
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let mut builder = tempfile::Builder::new();
    builder.prefix(".agena-memory-").suffix(".tmp");
    #[cfg(unix)]
    if permissions.is_none() {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(fs::Permissions::from_mode(0o600));
    }
    let mut staged = builder.tempfile_in(parent)?;
    staged.write_all(contents)?;
    staged.flush()?;
    staged.as_file().sync_all()?;
    if let Some(permissions) = permissions {
        staged.as_file().set_permissions(permissions)?;
    }
    staged
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

#[derive(Debug, Clone)]
/// In-memory implementation of the storage contracts for tests and small deployments.
pub struct MemoryStore {
    dir: PathBuf,
}

impl MemoryStore {
    pub fn for_workspace(workspace_root: &Path) -> Self {
        Self {
            dir: MemoryDir::from_workspace(workspace_root)
                .path()
                .to_path_buf(),
        }
    }
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn ensure_exists(&self) -> io::Result<()> {
        fs::create_dir_all(&self.dir)
    }

    pub fn ensure_index(&self) -> MemoryResult<PathBuf> {
        self.ensure_exists()?;
        let path = self.dir.join(ENTRYPOINT_NAME);
        with_memory_write_lock(|| {
            if !path.exists() {
                write_memory_file_atomically(&path, b"")?;
            }
            Ok::<(), MemoryError>(())
        })?;
        Ok(path)
    }

    pub fn list(&self) -> MemoryResult<Vec<MemoryRecord>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for dirent in fs::read_dir(&self.dir)? {
            let path = dirent?.path();
            if !is_memory_file(&path) {
                continue;
            }
            match read_entry(&path) {
                Ok(entry) => entries.push(entry),
                Err(err) => {
                    tracing::warn!(target: "agena::memory", "skipping memory file {}: {err}", path.display())
                }
            }
        }
        entries.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        Ok(entries)
    }
    pub fn get(&self, name: &str) -> MemoryResult<MemoryRecord> {
        let path = self.resolve_path(name);
        if !path.exists() {
            return Err(MemoryError::NotFound(name.to_string()));
        }
        read_entry(&path)
    }
    pub fn index_lines(&self) -> MemoryResult<Vec<String>> {
        let path = self.dir.join(ENTRYPOINT_NAME);
        if !path.exists() {
            return Ok(Vec::new());
        }
        Ok(fs::read_to_string(path)?
            .lines()
            .map(str::to_string)
            .collect())
    }
    pub fn forget(&self, name: &str) -> MemoryResult<()> {
        with_memory_write_lock(|| {
            let path = self.resolve_path(name);
            if !path.exists() {
                return Err(MemoryError::NotFound(name.to_string()));
            }
            fs::remove_file(path)?;
            let index = self.dir.join(ENTRYPOINT_NAME);
            if index.exists() {
                let needle = format!("{name}.md");
                let mut updated = fs::read_to_string(&index)?
                    .lines()
                    .filter(|line| !line.contains(&needle))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !updated.is_empty() {
                    updated.push('\n');
                }
                write_memory_file_atomically(&index, updated.as_bytes())?;
            }
            Ok(())
        })
    }
    pub fn save(&self, entry: NewMemory) -> MemoryResult<MemoryRecord> {
        self.ensure_exists()?;
        with_memory_write_lock(|| {
            let file_name = format!("{}.md", entry.name.trim());
            let path = self.dir.join(&file_name);
            let mut raw = format!("---\nname: {}\n", yaml_escape(&entry.name));
            if !entry.description.trim().is_empty() {
                raw.push_str(&format!(
                    "description: {}\n",
                    yaml_escape(entry.description.trim())
                ));
            }
            if let Some(memory_type) = entry.memory_type {
                raw.push_str(&format!("type: {}\n", memory_type.label()));
            }
            raw.push_str("---\n\n");
            raw.push_str(entry.body.trim_end());
            raw.push('\n');
            write_memory_file_atomically(&path, raw.as_bytes())?;
            if let Some(index_line) = entry.index_line.as_deref() {
                let index = self.dir.join(ENTRYPOINT_NAME);
                let needle = format!("{}.md", entry.name.trim());
                let mut existing = if index.exists() {
                    fs::read_to_string(&index)?
                } else {
                    String::new()
                };
                existing = existing
                    .lines()
                    .filter(|line| !line.contains(&needle))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !existing.is_empty() {
                    existing.push('\n');
                }
                existing.push_str(index_line);
                existing.push('\n');
                write_memory_file_atomically(&index, existing.as_bytes())?;
            }
            read_entry(&path)
        })
    }
    fn resolve_path(&self, name: &str) -> PathBuf {
        self.dir
            .join(format!("{}.md", name.trim_end_matches(".md")))
    }
}

impl MemoryRepository for MemoryStore {
    fn directory(&self) -> PathBuf {
        self.dir.clone()
    }

    fn ensure_index(&self) -> MemoryResult<PathBuf> {
        Self::ensure_index(self)
    }

    fn list(&self) -> MemoryResult<Vec<MemoryRecord>> {
        Self::list(self)
    }
    fn get(&self, name: &str) -> MemoryResult<MemoryRecord> {
        Self::get(self, name)
    }
    fn index_lines(&self) -> MemoryResult<Vec<String>> {
        Self::index_lines(self)
    }
    fn forget(&self, name: &str) -> MemoryResult<()> {
        Self::forget(self, name)
    }
    fn save(&self, entry: NewMemory) -> MemoryResult<MemoryRecord> {
        Self::save(self, entry)
    }
}

fn read_entry(path: &Path) -> MemoryResult<MemoryRecord> {
    let raw = fs::read_to_string(path)?;
    let (frontmatter, body) = parse_frontmatter(&raw, path)?;
    Ok(MemoryRecord {
        file_name: path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string(),
        path: path.to_path_buf(),
        frontmatter,
        body,
    })
}
fn parse_frontmatter(raw: &str, path: &Path) -> MemoryResult<(MemoryFrontmatter, String)> {
    let normalized = raw.replace("\r\n", "\n");
    let Some(stripped) = normalized.strip_prefix("---\n") else {
        return Ok((MemoryFrontmatter::default(), normalized.trim().to_string()));
    };
    let Some(end) = stripped.find("\n---") else {
        return Err(MemoryError::Malformed {
            path: path.to_path_buf(),
            message: "frontmatter missing closing '---'".into(),
        });
    };
    let yaml = &stripped[..end];
    let body = stripped[end + 4..].trim_start_matches('\n').to_string();
    Ok((
        if yaml.trim().is_empty() {
            MemoryFrontmatter::default()
        } else {
            serde_yaml::from_str(yaml)?
        },
        body,
    ))
}
fn is_memory_file(path: &Path) -> bool {
    path.is_file()
        && path.extension().and_then(|s| s.to_str()) == Some("md")
        && matches!(path.file_name().and_then(|s| s.to_str()), Some(name) if !name.eq_ignore_ascii_case(ENTRYPOINT_NAME))
}
fn yaml_escape(s: &str) -> String {
    if s.contains(':') || s.contains('#') || s.starts_with('-') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::MemoryStore;
    use crate::NewMemory;

    #[test]
    fn save_replaces_a_memory_and_its_index_line() {
        let directory = tempfile::tempdir().expect("memory directory");
        let store = MemoryStore::at(directory.path());
        for (description, body) in [("old", "first"), ("new", "second")] {
            store
                .save(NewMemory {
                    name: "decision".into(),
                    description: description.into(),
                    body: body.into(),
                    memory_type: None,
                    index_line: Some("- [decision](decision.md)".into()),
                })
                .expect("save memory");
        }
        assert_eq!(store.list().expect("list memory").len(), 1);
        assert!(
            store
                .get("decision")
                .expect("read memory")
                .body
                .contains("second")
        );
        assert_eq!(
            store.index_lines().expect("read index"),
            vec!["- [decision](decision.md)"]
        );
    }

    #[test]
    fn concurrent_saves_keep_every_index_entry() {
        let directory = tempfile::tempdir().expect("memory directory");
        let store = Arc::new(MemoryStore::at(directory.path()));
        let barrier = Arc::new(Barrier::new(2));
        let handles = [("first", "First"), ("second", "Second")].map(|(name, label)| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store
                    .save(NewMemory {
                        name: name.to_string(),
                        description: String::new(),
                        body: label.to_string(),
                        memory_type: None,
                        index_line: Some(format!("- [{label}]({name}.md)")),
                    })
                    .expect("concurrent memory save");
            })
        });
        for handle in handles {
            handle.join().expect("memory writer");
        }

        let index = store.index_lines().expect("memory index");
        assert_eq!(index.len(), 2);
        assert!(index.iter().any(|line| line.contains("first.md")));
        assert!(index.iter().any(|line| line.contains("second.md")));
    }
}
