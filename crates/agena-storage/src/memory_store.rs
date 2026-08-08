//! Filesystem implementation of the portable memory repository contract.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::{
    MemoryDir, MemoryError, MemoryFrontmatter, MemoryRecord, MemoryRepository, MemoryResult,
    NewMemory,
};

const ENTRYPOINT_NAME: &str = "MEMORY.md";

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
        if !path.exists() {
            fs::write(&path, "")?;
        }
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
            fs::write(index, updated)?;
        }
        Ok(())
    }
    pub fn save(&self, entry: NewMemory) -> MemoryResult<MemoryRecord> {
        self.ensure_exists()?;
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
        fs::write(&path, raw)?;
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
            fs::write(index, existing)?;
        }
        read_entry(&path)
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
    use std::{fs, time::SystemTime};

    use super::MemoryStore;
    use crate::NewMemory;

    #[test]
    fn save_replaces_a_memory_and_its_index_line() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "agena-memory-store-{}/{}",
            std::process::id(),
            nonce
        ));
        let store = MemoryStore::at(&dir);
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
        fs::remove_dir_all(dir).expect("remove temporary memory directory");
    }
}
