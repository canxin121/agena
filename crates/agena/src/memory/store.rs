//! Programmatic access to the persistent memory system.
//!
//! The model writes individual memory files (with `---` frontmatter) into
//! the workspace's memory directory and an index line into `MEMORY.md`. This
//! module gives the rest of agena (TUI / CLI / future `/memory` slash
//! command) a typed handle on those files: list everything, read a single
//! entry, append a new one, forget by name.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::memory::paths::MemoryDir;

const ENTRYPOINT_NAME: &str = "MEMORY.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
    #[serde(other)]
    Other,
}

impl MemoryType {
    pub fn label(self) -> &'static str {
        match self {
            MemoryType::User => "user",
            MemoryType::Feedback => "feedback",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
            MemoryType::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryFrontmatter {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub r#type: Option<MemoryType>,
}

#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub file_name: String,
    pub path: PathBuf,
    pub frontmatter: MemoryFrontmatter,
    pub body: String,
}

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("malformed memory file `{path}`: {message}")]
    Malformed { path: PathBuf, message: String },
    #[error("yaml frontmatter error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("memory `{0}` not found")]
    NotFound(String),
}

pub type MemoryResult<T> = Result<T, MemoryError>;

#[derive(Debug, Clone)]
pub struct MemoryStore {
    dir: PathBuf,
}

impl MemoryStore {
    pub fn for_workspace(workspace_root: &Path) -> Self {
        let dir = MemoryDir::from_workspace(workspace_root);
        Self {
            dir: dir.path().to_path_buf(),
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

    /// Read all memory files in the directory (excluding the `MEMORY.md`
    /// index). Files that fail to parse are logged and skipped — a single
    /// malformed entry must not prevent the rest from loading.
    pub fn list(&self) -> MemoryResult<Vec<MemoryEntry>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for dirent in fs::read_dir(&self.dir)? {
            let dirent = dirent?;
            let path = dirent.path();
            if !is_memory_file(&path) {
                continue;
            }
            match read_entry(&path) {
                Ok(entry) => entries.push(entry),
                Err(err) => {
                    tracing::warn!(
                        target: "agena::memory",
                        "skipping memory file {}: {err}",
                        path.display()
                    );
                }
            }
        }
        entries.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        Ok(entries)
    }

    pub fn get(&self, name: &str) -> MemoryResult<MemoryEntry> {
        let path = self.resolve_path(name);
        if !path.exists() {
            return Err(MemoryError::NotFound(name.to_string()));
        }
        read_entry(&path)
    }

    /// Read the `MEMORY.md` index lines (one per memory). Lines that are
    /// not list entries are returned verbatim so callers can preserve user
    /// notes interleaved with the index.
    pub fn index_lines(&self) -> MemoryResult<Vec<String>> {
        let entrypoint = self.dir.join(ENTRYPOINT_NAME);
        if !entrypoint.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&entrypoint)?;
        Ok(raw.lines().map(str::to_string).collect())
    }

    /// Remove a memory by file stem (e.g. `user_role` → deletes
    /// `user_role.md` and any matching `MEMORY.md` line).
    pub fn forget(&self, name: &str) -> MemoryResult<()> {
        let path = self.resolve_path(name);
        if !path.exists() {
            return Err(MemoryError::NotFound(name.to_string()));
        }
        fs::remove_file(&path)?;

        let entrypoint = self.dir.join(ENTRYPOINT_NAME);
        if entrypoint.exists() {
            let raw = fs::read_to_string(&entrypoint)?;
            let needle = format!("{name}.md");
            let kept: Vec<&str> = raw.lines().filter(|line| !line.contains(&needle)).collect();
            let mut updated = kept.join("\n");
            if !updated.ends_with('\n') && !updated.is_empty() {
                updated.push('\n');
            }
            fs::write(&entrypoint, updated)?;
        }
        Ok(())
    }

    /// Write a new memory file and append a one-line entry to `MEMORY.md`.
    /// Overwrites any existing file with the same name. Does not
    /// deduplicate the index line — callers should run [`list`] first if
    /// they care about uniqueness.
    pub fn save(&self, entry: NewMemory) -> MemoryResult<MemoryEntry> {
        self.ensure_exists()?;
        let file_name = format!("{}.md", entry.name.trim());
        let path = self.dir.join(&file_name);

        let mut frontmatter = String::from("---\n");
        frontmatter.push_str(&format!("name: {}\n", yaml_escape(&entry.name)));
        if !entry.description.trim().is_empty() {
            frontmatter.push_str(&format!(
                "description: {}\n",
                yaml_escape(entry.description.trim())
            ));
        }
        if let Some(memory_type) = entry.memory_type {
            frontmatter.push_str(&format!("type: {}\n", memory_type.label()));
        }
        frontmatter.push_str("---\n\n");
        frontmatter.push_str(entry.body.trim_end());
        frontmatter.push('\n');
        fs::write(&path, &frontmatter)?;

        if let Some(index_line) = entry.index_line.as_deref() {
            let entrypoint = self.dir.join(ENTRYPOINT_NAME);
            let mut existing = if entrypoint.exists() {
                fs::read_to_string(&entrypoint)?
            } else {
                String::new()
            };
            if !existing.is_empty() && !existing.ends_with('\n') {
                existing.push('\n');
            }
            existing.push_str(index_line);
            existing.push('\n');
            fs::write(&entrypoint, existing)?;
        }

        read_entry(&path)
    }

    fn resolve_path(&self, name: &str) -> PathBuf {
        let trimmed = name.trim_end_matches(".md");
        self.dir.join(format!("{trimmed}.md"))
    }
}

#[derive(Debug, Clone)]
pub struct NewMemory {
    pub name: String,
    pub description: String,
    pub memory_type: Option<MemoryType>,
    pub body: String,
    pub index_line: Option<String>,
}

fn read_entry(path: &Path) -> MemoryResult<MemoryEntry> {
    let raw = fs::read_to_string(path)?;
    let (frontmatter, body) = parse_frontmatter(&raw, path)?;
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    Ok(MemoryEntry {
        file_name,
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
    let frontmatter: MemoryFrontmatter = if yaml.trim().is_empty() {
        MemoryFrontmatter::default()
    } else {
        serde_yaml::from_str(yaml)?
    };
    Ok((frontmatter, body))
}

fn is_memory_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    if path.extension().and_then(|s| s.to_str()) != Some("md") {
        return false;
    }
    matches!(
        path.file_name().and_then(|s| s.to_str()),
        Some(name) if !name.eq_ignore_ascii_case(ENTRYPOINT_NAME)
    )
}

fn yaml_escape(s: &str) -> String {
    if s.contains(':') || s.contains('#') || s.starts_with('-') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}
