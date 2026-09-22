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

mod transaction;
use sha2::{Digest, Sha256};
use std::io::Read as _;
const MAX_MEMORY_BYTES: usize = 8 * 1024 * 1024;

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
    transaction::stage(path, contents)?
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

fn read_bounded(path: &Path) -> MemoryResult<Vec<u8>> {
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "memory target is not a regular file",
        )
        .into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_MEMORY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_MEMORY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "memory file exceeds the 8 MiB limit",
        )
        .into());
    }
    Ok(bytes)
}
fn read_text(path: &Path) -> MemoryResult<String> {
    String::from_utf8(read_bounded(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error).into())
}
fn canonical_name(name: &str) -> MemoryResult<&str> {
    let name = name.trim().trim_end_matches(".md");
    if name.is_empty()
        || name.eq_ignore_ascii_case("MEMORY")
        || name.len() > 255
        || name
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
        || matches!(name, "." | "..")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid or reserved memory name",
        )
        .into());
    }
    Ok(name)
}
fn refers_to(line: &str, name: &str) -> bool {
    line.contains(&format!("({name}.md)")) || line.contains(&format!("(./{name}.md)"))
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
        let name = canonical_name(name)?;
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
        self.forget_checked(name, None)
    }
    pub fn forget_checked(&self, name: &str, expected: Option<&str>) -> MemoryResult<()> {
        let name = canonical_name(name)?;
        with_memory_write_lock(|| {
            let path = self.resolve_path(name);
            if !path.exists() {
                return Err(MemoryError::NotFound(name.into()));
            }
            check_revision(&path, expected, false)?;
            let index = self.dir.join(ENTRYPOINT_NAME);
            let updated = if index.exists() {
                Some(index_without(&read_text(&index)?, name))
            } else {
                None
            };
            transaction::mutate(
                &path,
                None,
                updated
                    .as_deref()
                    .map(|text| (index.as_path(), text.as_bytes())),
            )
        })
    }
    pub fn save(&self, entry: NewMemory) -> MemoryResult<MemoryRecord> {
        self.save_inner(entry, None, false)
    }
    /// Tool-facing create-or-CAS-update. Existing records require a revision.
    pub fn save_checked(
        &self,
        entry: NewMemory,
        expected: Option<&str>,
    ) -> MemoryResult<MemoryRecord> {
        self.save_inner(entry, expected, true)
    }
    fn save_inner(
        &self,
        entry: NewMemory,
        expected: Option<&str>,
        guarded: bool,
    ) -> MemoryResult<MemoryRecord> {
        let name = canonical_name(&entry.name)?.to_owned();
        self.ensure_exists()?;
        with_memory_write_lock(|| {
            let path = self.resolve_path(&name);
            check_revision(&path, expected, guarded)?;
            let mut raw = format!("---\nname: {}\n", yaml_escape(&name));
            if !entry.description.trim().is_empty() {
                raw.push_str(&format!(
                    "description: {}\n",
                    yaml_escape(entry.description.trim())
                ));
            }
            if let Some(kind) = entry.memory_type {
                raw.push_str(&format!("type: {}\n", kind.label()));
            }
            raw.push_str("---\n\n");
            raw.push_str(entry.body.trim_end());
            raw.push('\n');
            if raw.len() > MAX_MEMORY_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "memory result exceeds the 8 MiB limit",
                )
                .into());
            }
            // Parsing before publication prevents a YAML failure after writing.
            let (frontmatter, body) = parse_frontmatter(&raw, &path)?;
            let index = self.dir.join(ENTRYPOINT_NAME);
            let updated = if let Some(line) = entry.index_line.as_deref() {
                let previous = if index.exists() {
                    read_text(&index)?
                } else {
                    String::new()
                };
                let mut next = index_without(&previous, &name);
                next.push_str(line);
                next.push('\n');
                if next.len() > MAX_MEMORY_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "memory index exceeds the 8 MiB limit",
                    )
                    .into());
                }
                Some(next)
            } else {
                None
            };
            transaction::mutate(
                &path,
                Some(raw.as_bytes()),
                updated
                    .as_deref()
                    .map(|text| (index.as_path(), text.as_bytes())),
            )?;
            Ok(MemoryRecord {
                file_name: format!("{name}.md"),
                path,
                frontmatter,
                body,
                sha256: hex::encode(Sha256::digest(raw.as_bytes())),
            })
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

fn check_revision(path: &Path, expected: Option<&str>, required: bool) -> MemoryResult<()> {
    if path.exists() {
        let actual = hex::encode(Sha256::digest(read_bounded(path)?));
        if expected.is_some_and(|value| !value.eq_ignore_ascii_case(&actual))
            || (required && expected.is_none())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "memory revision conflict: read the record and supply expected_sha256={actual}"
                ),
            )
            .into());
        }
    } else if expected.is_some() {
        return Err(MemoryError::NotFound(path.display().to_string()));
    }
    Ok(())
}
fn index_without(previous: &str, name: &str) -> String {
    let mut text = previous
        .lines()
        .filter(|line| !refers_to(line, name))
        .collect::<Vec<_>>()
        .join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}
fn read_entry(path: &Path) -> MemoryResult<MemoryRecord> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "memory record is not a regular file",
        )
        .into());
    }
    let raw = read_text(path)?;
    let (frontmatter, body) = parse_frontmatter(&raw, path)?;
    Ok(MemoryRecord {
        file_name: path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .into(),
        path: path.into(),
        frontmatter,
        body,
        sha256: hex::encode(Sha256::digest(raw.as_bytes())),
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
    // JSON quoted strings are valid YAML scalars and preserve quotes,
    // backslashes, booleans, numbers, and embedded control characters.
    serde_json::to_string(s).expect("serializing a string cannot fail")
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
