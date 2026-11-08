//! Lifecycle state is host-owned and tied to exact document bytes.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::*;
use crate::{MemoryKind, MemoryMetadata, MemoryOrigin, MemorySource, MemoryStatus, MemoryType};

pub(super) const STATE_NAME: &str = ".memory-state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredMetadata {
    pub sha256: String,
    pub metadata: MemoryMetadata,
    /// Known earlier source sessions support durable forget without attributing
    /// a user's later edits to the old evidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_history: Vec<MemorySource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Tombstone {
    pub generation: u64,
    pub deleted_at_ms: i64,
    pub memory_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MemorySource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MemoryState {
    pub schema_version: u32,
    pub generation: u64,
    pub records: BTreeMap<String, StoredMetadata>,
    pub tombstones: BTreeMap<String, Tombstone>,
    /// Source suppression survives an intentional recreation of a named note.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suppressed_sources: Vec<MemorySource>,
}

impl Default for MemoryState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            generation: 0,
            records: BTreeMap::new(),
            tombstones: BTreeMap::new(),
            suppressed_sources: Vec::new(),
        }
    }
}

impl MemoryState {
    pub fn read(dir: &Path) -> MemoryResult<Self> {
        let path = dir.join(STATE_NAME);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        let state: Self = serde_json::from_slice(&read_bounded(&path)?).map_err(|error| {
            MemoryError::Malformed {
                path: path.clone(),
                message: format!("invalid lifecycle state: {error}"),
            }
        })?;
        if state.schema_version != 1 {
            return Err(MemoryError::Malformed {
                path,
                message: format!("unsupported memory state version {}", state.schema_version),
            });
        }
        Ok(state)
    }

    pub fn bytes(&self) -> MemoryResult<Vec<u8>> {
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        if bytes.len() > MAX_MEMORY_BYTES {
            return Err(invalid("memory lifecycle state exceeds the 8 MiB limit"));
        }
        Ok(bytes)
    }

    pub fn advance(&mut self) -> MemoryResult<u64> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("memory generation exhausted"))?;
        Ok(self.generation)
    }

    pub fn attach(&self, record: &mut MemoryRecord) {
        let name = record.file_name.trim_end_matches(".md");
        let mut metadata = self
            .records
            .get(name)
            .map(|stored| stored.metadata.clone())
            .unwrap_or_else(|| MemoryMetadata {
                id: memory_id(&record.path),
                kind: match record.frontmatter.r#type {
                    Some(MemoryType::User | MemoryType::Feedback) => MemoryKind::Preference,
                    _ => MemoryKind::Fact,
                },
                ..MemoryMetadata::default()
            });
        let intact = self.records.get(name).is_some_and(|stored| {
            stored.sha256 == record.sha256
                && record.frontmatter.revision == Some(stored.metadata.revision)
        });
        if !self.records.contains_key(name) && record.frontmatter.revision.is_some() {
            // A renamed managed file or lost sidecar cannot silently broaden a
            // session-only note into workspace-wide knowledge. Explicit review
            // can reactivate it with an appropriate scope.
            metadata.status = MemoryStatus::Archived;
        }
        if !intact {
            // Preserve host-controlled scope/status, but do not attribute edited
            // bytes to the source that supported an earlier revision.
            metadata.origin = MemoryOrigin::External;
            metadata.source = None;
            metadata.provenance_verified = false;
            metadata.updated_at_ms = fs::metadata(&record.path)
                .and_then(|value| value.modified())
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
                .unwrap_or(metadata.updated_at_ms);
        }
        if self.tombstones.contains_key(name) {
            metadata.status = MemoryStatus::Archived;
            metadata.source = None;
            metadata.provenance_verified = false;
        }
        // Changing bytes revokes attribution, but cannot release an automatic
        // note from a durable forget. Only an explicit host write changes its
        // stored origin to user-controlled.
        metadata.source_suppressed = self.records.get(name).is_some_and(|stored| {
            stored.metadata.origin == MemoryOrigin::Derived
                && stored
                    .metadata
                    .source
                    .iter()
                    .chain(stored.source_history.iter())
                    .any(|source| self.source_is_suppressed(source))
        });
        if metadata.source_suppressed {
            metadata.status = MemoryStatus::Archived;
        }
        record.metadata = metadata;
    }

    pub fn source_is_suppressed(&self, source: &MemorySource) -> bool {
        self.suppressed_sources
            .iter()
            .any(|old| same_source_session(old, source))
            || self
                .tombstones
                .values()
                .filter_map(|tombstone| tombstone.source.as_ref())
                .any(|old| same_source_session(old, source))
    }

    pub fn suppress_source(&mut self, source: Option<&MemorySource>) {
        if let Some(source) = source
            && source.instance_id.is_some()
            && source.session_id.is_some()
            && !self
                .suppressed_sources
                .iter()
                .any(|old| same_source_session(old, source))
        {
            self.suppressed_sources.push(source.clone());
        }
    }

    pub fn suppress_record_sources(&mut self, name: &str, source: Option<&MemorySource>) {
        let mut sources = self
            .records
            .get(name)
            .map(|stored| stored.source_history.clone())
            .unwrap_or_default();
        if let Some(stored_source) = self
            .records
            .get(name)
            .and_then(|stored| stored.metadata.source.as_ref())
        {
            sources.push(stored_source.clone());
        }
        if let Some(source) = source {
            sources.push(source.clone());
        }
        for source in sources {
            self.suppress_source(Some(&source));
        }
    }

    pub fn remove_suppressed_index_entries(&self, index: &str) -> String {
        self.records
            .iter()
            .filter(|(_, stored)| {
                stored.metadata.origin == MemoryOrigin::Derived
                    && stored
                        .metadata
                        .source
                        .as_ref()
                        .is_some_and(|source| self.source_is_suppressed(source))
            })
            .fold(index.to_owned(), |index, (name, _)| {
                index_without(&index, name)
            })
    }

    pub fn put(&mut self, name: String, record: &MemoryRecord) {
        let previous = self.records.get(&name);
        let mut source_history = previous
            .map(|old| old.source_history.clone())
            .unwrap_or_default();
        for source in previous
            .and_then(|old| old.metadata.source.as_ref())
            .into_iter()
            .chain(record.metadata.source.as_ref())
        {
            if source.instance_id.is_some()
                && source.session_id.is_some()
                && !source_history
                    .iter()
                    .any(|old| same_source_session(old, source))
            {
                source_history.push(source.clone());
            }
        }
        self.records.insert(
            name,
            StoredMetadata {
                sha256: record.sha256.clone(),
                metadata: record.metadata.clone(),
                source_history,
            },
        );
    }
}

fn same_source_session(a: &MemorySource, b: &MemorySource) -> bool {
    a.instance_id.is_some()
        && a.session_id.is_some()
        && a.instance_id == b.instance_id
        && a.session_id == b.session_id
}

pub(super) fn memory_id(path: &Path) -> String {
    // Identity is scoped to this private store and retained in state across
    // revisions. A deliberate recreate uses the same logical memory identity.
    let absolute = path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        }
    });
    format!(
        "mem_{}",
        hex::encode(Sha256::digest(absolute.as_os_str().as_encoded_bytes()))
    )
}

pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
