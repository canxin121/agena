//! Filesystem implementation of the portable memory repository contract.

use std::{
    collections::BTreeSet,
    fs, io,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

use crate::{
    MemoryDir, MemoryError, MemoryFrontmatter, MemoryMetadata, MemoryOrigin, MemoryRecord,
    MemoryRepository, MemoryResult, MemoryScope, MemoryStatus, MemoryWriteOptions,
    MemoryWriteSnapshot, NewMemory,
};
use sha2::{Digest, Sha256};

mod state;
#[cfg(test)]
mod tests;
mod transaction;

const MAX_MEMORY_BYTES: usize = 8 * 1024 * 1024;
const MEMORY_SCHEMA_VERSION: u32 = 2;
const ENTRYPOINT_NAME: &str = "MEMORY.md";
static MEMORY_FILE_WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn with_memory_lock<T>(operation: impl FnOnce() -> T) -> T {
    let _guard = MEMORY_FILE_WRITE_LOCK.lock().unwrap_or_else(|error| {
        tracing::error!(
            diagnostic = %error,
            "memory file lock is poisoned; recovering serialized access"
        );
        error.into_inner()
    });
    operation()
}

fn invalid(message: impl Into<String>) -> MemoryError {
    io::Error::new(io::ErrorKind::InvalidInput, message.into()).into()
}

fn read_bounded(path: &Path) -> MemoryResult<Vec<u8>> {
    read_bounded_limit(path, MAX_MEMORY_BYTES)
}

fn read_bounded_limit(path: &Path, limit: usize) -> MemoryResult<Vec<u8>> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(invalid("memory file cannot be a symbolic link"));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("memory target is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("memory file exceeds the {limit}-byte limit"),
        )
        .into());
    }
    Ok(bytes)
}

fn read_text(path: &Path) -> MemoryResult<String> {
    String::from_utf8(read_bounded(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error).into())
}

/// Normalize one memory name using the shared storage, tool, and management API rules.
pub fn normalize_memory_name(name: &str) -> MemoryResult<&str> {
    let name = name.trim().trim_end_matches(".md");
    if name.is_empty()
        || name.eq_ignore_ascii_case("MEMORY")
        || name.len() > 255
        || name
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
        || matches!(name, "." | "..")
    {
        return Err(invalid("invalid or reserved memory name"));
    }
    Ok(name)
}

fn refers_to(line: &str, name: &str) -> bool {
    line.contains(&format!("({name}.md)")) || line.contains(&format!("(./{name}.md)"))
}

/// A parsed document becomes a public record only after loading its ledger metadata.
struct MemoryDocument {
    sha256: String,
    file_name: String,
    path: PathBuf,
    frontmatter: MemoryFrontmatter,
    body: String,
}

#[derive(Debug, Clone)]
/// Private filesystem implementation of the workspace memory contract.
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

    /// Host-owned state; callers authorize this together with the record/index.
    pub fn metadata_path(&self) -> PathBuf {
        self.dir.join(state::STATE_NAME)
    }

    /// Internal files touched by initialization and lifecycle-aware writes.
    pub fn internal_paths(&self) -> [PathBuf; 3] {
        [
            self.metadata_path(),
            self.dir.join(transaction::JOURNAL_NAME),
            self.dir.join(transaction::LOCK_NAME),
        ]
    }

    pub fn ensure_exists(&self) -> io::Result<()> {
        agena_storage_layout::secure_directory(&self.dir)
    }

    /// A read-only, bounded inventory for asynchronous recovery authorization.
    /// An absent journal returns an empty list without creating any files.
    pub fn pending_recovery_paths(&self) -> MemoryResult<Vec<PathBuf>> {
        with_memory_lock(|| {
            let _lock = transaction::lock(&self.dir, false, false)?;
            let Some(mut paths) = transaction::recovery_paths(&self.dir)? else {
                return Ok(Vec::new());
            };
            paths.push(self.dir.clone());
            paths.extend(self.internal_paths());
            paths.sort();
            paths.dedup();
            Ok(paths)
        })
    }

    /// Restore an interrupted transaction only after authorizing every target.
    /// The actual journal is re-read and checked while holding the exclusive
    /// lock, so approval of an earlier inventory cannot authorize new targets.
    pub fn recover_pending(&self, allowed: impl Fn(&Path) -> bool) -> MemoryResult<bool> {
        with_memory_lock(|| {
            let Some(paths) = transaction::recovery_paths(&self.dir)? else {
                return Ok(false);
            };
            for path in std::iter::once(self.dir.clone())
                .chain(self.internal_paths())
                .chain(paths)
            {
                if !allowed(&path) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!("memory recovery is not authorized for {}", path.display()),
                    )
                    .into());
                }
            }
            let _lock = transaction::lock(&self.dir, true, true)?;
            transaction::recover_authorized(&self.dir, allowed)
        })
    }

    fn read_locked<T>(&self, operation: impl FnOnce() -> MemoryResult<T>) -> MemoryResult<T> {
        with_memory_lock(|| {
            let _lock = transaction::lock(&self.dir, false, false)?;
            transaction::check_clean(&self.dir)?;
            operation()
        })
    }

    fn write_locked<T>(&self, operation: impl FnOnce() -> MemoryResult<T>) -> MemoryResult<T> {
        self.ensure_exists()?;
        with_memory_lock(|| {
            let _lock = transaction::lock(&self.dir, true, true)?;
            transaction::check_clean(&self.dir)?;
            operation()
        })
    }

    /// Initialize the navigation index after any authorized recovery.
    pub fn ensure_index(&self) -> MemoryResult<PathBuf> {
        self.write_locked(|| {
            let path = self.dir.join(ENTRYPOINT_NAME);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    transaction::mutate_many(&self.dir, vec![(path.clone(), Some(Vec::new()))])?;
                }
                Err(error) => return Err(error.into()),
                Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                    return Err(invalid("memory index must be a regular file"));
                }
                Ok(_) => {}
            }
            Ok(path)
        })
    }

    /// Includes inactive history. Retrieval callers filter with is_active_in_context.
    pub fn list(&self) -> MemoryResult<Vec<MemoryRecord>> {
        self.list_filtered(|_| true)
    }

    /// Check every concrete record before reading it; approving the directory
    /// must not bypass a deny rule for an individual memory.
    pub fn list_filtered(
        &self,
        allowed: impl Fn(&Path) -> bool,
    ) -> MemoryResult<Vec<MemoryRecord>> {
        self.read_locked(|| {
            if !self.dir.exists() {
                return Ok(Vec::new());
            }
            let state = state::MemoryState::read(&self.dir)?;
            let mut entries = Vec::new();
            for dirent in fs::read_dir(&self.dir)? {
                let path = dirent?.path();
                if !is_memory_file(&path) || !allowed(&path) {
                    continue;
                }
                match read_entry(&path).and_then(|entry| state.attach(entry)) {
                    Ok(entry) => entries.push(entry),
                    Err(error) => {
                        tracing::warn!(target: "agena::memory", "skipping memory file {}: {error}", path.display())
                    }
                }
            }
            entries.sort_by(|a, b| a.file_name.cmp(&b.file_name));
            Ok(entries)
        })
    }

    pub fn get(&self, name: &str) -> MemoryResult<MemoryRecord> {
        let name = normalize_memory_name(name)?;
        self.read_locked(|| {
            let state = state::MemoryState::read(&self.dir)?;
            self.read_optional(name, &state)?
                .ok_or_else(|| MemoryError::NotFound(name.to_owned()))
        })
    }

    pub fn index_lines(&self) -> MemoryResult<Vec<String>> {
        self.read_locked(|| {
            let path = self.dir.join(ENTRYPOINT_NAME);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
                Err(error) => Err(error.into()),
                Ok(_) => Ok(read_text(&path)?.lines().map(str::to_string).collect()),
            }
        })
    }

    pub fn source_is_suppressed(&self, source: &crate::MemorySource) -> MemoryResult<bool> {
        self.read_locked(|| Ok(state::MemoryState::read(&self.dir)?.source_is_suppressed(source)))
    }

    pub fn write_snapshot(&self, name: &str) -> MemoryResult<MemoryWriteSnapshot> {
        let name = normalize_memory_name(name)?.to_owned();
        self.read_locked(|| {
            let state = state::MemoryState::read(&self.dir)?;
            let previous = self.read_optional(&name, &state)?;
            Ok(MemoryWriteSnapshot {
                expected_sha256: previous.map(|record| record.sha256),
                is_tombstoned: state.tombstones.contains_key(&name),
                generation: state.generation,
                store_key: self.store_key(),
                name,
            })
        })
    }

    pub fn forget_checked(&self, name: &str, expected: &str) -> MemoryResult<()> {
        let name = normalize_memory_name(name)?.to_owned();
        self.write_locked(|| {
            let mut state = state::MemoryState::read(&self.dir)?;
            let previous = self
                .read_optional(&name, &state)?
                .ok_or_else(|| MemoryError::NotFound(name.clone()))?;
            check_revision(Some(&previous), Some(expected))?;
            let generation = state.advance()?;
            let source = previous.metadata.source.clone().or_else(|| {
                state
                    .records
                    .get(&name)
                    .and_then(|stored| stored.metadata.source.clone())
            });
            state.suppress_record_sources(&name, source.as_ref());
            state.records.remove(&name);
            state.tombstones.insert(
                name.clone(),
                state::Tombstone {
                    generation,
                    deleted_at_ms: state::now_ms(),
                    memory_id: previous.metadata.id,
                    source,
                },
            );
            let index = self.dir.join(ENTRYPOINT_NAME);
            let previous_index = self.read_index()?;
            let next_index =
                state.remove_suppressed_index_entries(&index_without(&previous_index, &name));
            transaction::mutate_many(
                &self.dir,
                vec![
                    (previous.path, None),
                    (index, Some(next_index.into_bytes())),
                    (self.metadata_path(), Some(state.bytes()?)),
                ],
            )
        })
    }

    /// Compare every selected revision under one lock and publish one deletion.
    pub fn forget_many_checked(&self, records: &[crate::MemoryRevision]) -> MemoryResult<()> {
        if records.is_empty() || records.len() > 1000 {
            return Err(invalid("select between 1 and 1000 memory entries"));
        }
        self.write_locked(|| {
            let mut state = state::MemoryState::read(&self.dir)?;
            let generation = state.advance()?;
            let now = state::now_ms();
            let mut names = BTreeSet::new();
            let mut changes = Vec::with_capacity(records.len() + 2);
            let mut next_index = self.read_index()?;
            let mut total_bytes = 0usize;
            for requested in records {
                let name = normalize_memory_name(&requested.name)?;
                if !names.insert(name.to_owned()) {
                    return Err(invalid("duplicate memory deletion"));
                }
                let previous = self
                    .read_optional(name, &state)?
                    .ok_or_else(|| MemoryError::NotFound(name.to_owned()))?;
                check_revision(Some(&previous), Some(&requested.sha256))?;
                total_bytes = total_bytes.saturating_add(previous.body.len());
                if total_bytes > transaction::MAX_JOURNAL_BYTES / 2 {
                    return Err(invalid(
                        "selected memories exceed the transaction size budget",
                    ));
                }
                let source = previous.metadata.source.clone().or_else(|| {
                    state
                        .records
                        .get(name)
                        .and_then(|stored| stored.metadata.source.clone())
                });
                state.suppress_record_sources(name, source.as_ref());
                state.records.remove(name);
                state.tombstones.insert(
                    name.to_owned(),
                    state::Tombstone {
                        generation,
                        deleted_at_ms: now,
                        memory_id: previous.metadata.id,
                        source,
                    },
                );
                next_index = index_without(&next_index, name);
                changes.push((previous.path, None));
            }
            let next_index = state.remove_suppressed_index_entries(&next_index);
            changes.push((
                self.dir.join(ENTRYPOINT_NAME),
                Some(next_index.into_bytes()),
            ));
            changes.push((self.metadata_path(), Some(state.bytes()?)));
            transaction::mutate_many(&self.dir, changes)
        })
    }

    /// Explicit update, with host provenance and optional checked supersession.
    pub fn save_with_options(
        &self,
        entry: NewMemory,
        expected: Option<&str>,
        options: MemoryWriteOptions,
    ) -> MemoryResult<MemoryRecord> {
        self.save_inner(entry, expected, options, None)
    }

    /// Background writes cannot recreate forgotten records or overwrite a
    /// newer generation, explicit correction, externally edited file or source.
    pub fn save_derived(
        &self,
        entry: NewMemory,
        snapshot: &MemoryWriteSnapshot,
        options: MemoryWriteOptions,
    ) -> MemoryResult<MemoryRecord> {
        self.save_inner(
            entry,
            snapshot.expected_sha256.as_deref(),
            options,
            Some(snapshot),
        )
    }

    fn save_inner(
        &self,
        entry: NewMemory,
        expected: Option<&str>,
        options: MemoryWriteOptions,
        derived: Option<&MemoryWriteSnapshot>,
    ) -> MemoryResult<MemoryRecord> {
        let name = normalize_memory_name(&entry.name)?.to_owned();
        validate_options(&options)?;
        self.write_locked(|| {
            let mut state = state::MemoryState::read(&self.dir)?;
            let previous = self.read_optional(&name, &state)?;
            if let Some(snapshot) = derived {
                if snapshot.name != name || snapshot.store_key != self.store_key() {
                    return Err(invalid("memory snapshot belongs to a different record or store"));
                }
                if snapshot.generation != state.generation {
                    return Err(invalid("memory changed while extraction was running; discard stale result"));
                }
                if snapshot.is_tombstoned || state.tombstones.contains_key(&name) {
                    return Err(invalid("forgotten memory cannot be recreated by background extraction"));
                }
                if !options.supersedes.is_empty() {
                    return Err(invalid("background extraction cannot supersede other memories"));
                }
                let source = options.source.as_ref()
                    .filter(|source| source.entry_id.is_some())
                    .ok_or_else(|| invalid("derived memory requires a source session and durable entry"))?;
                if state.source_is_suppressed(source) {
                    return Err(invalid("forgotten source conversation cannot generate derived memories"));
                }
                if let Some(record) = previous.as_ref() {
                    if record.metadata.origin != MemoryOrigin::Derived
                        || !record.metadata.provenance_verified
                        || record.metadata.status != MemoryStatus::Active
                    {
                        return Err(invalid("background extraction cannot overwrite an explicit or external edit"));
                    }
                    if let Some(old) = record.metadata.source.as_ref() {
                        if old.session_id != source.session_id || old.instance_id != source.instance_id {
                            return Err(invalid("derived memory source session cannot change"));
                        }
                        if let (Some(old_entry), Some(new_entry)) = (old.entry_id, source.entry_id) {
                            if new_entry < old_entry {
                                return Err(invalid("derived memory source watermark cannot move backward"));
                            }
                            if new_entry == old_entry {
                                check_revision(previous.as_ref(), expected)?;
                                return Ok(record.clone());
                            }
                        }
                    }
                }
            }
            check_revision(previous.as_ref(), expected)?;
            let generation = state.advance()?;
            let now = state::now_ms();
            let mut metadata = previous.as_ref().map(|record| record.metadata.clone())
                .unwrap_or_else(|| MemoryMetadata {
                    id: state::memory_id(&self.resolve_path(&name)),
                    kind: entry.kind,
                    created_at_ms: now,
                    ..MemoryMetadata::default()
                });
            metadata.revision = generation;
            metadata.updated_at_ms = now;
            metadata.origin = if derived.is_some() { MemoryOrigin::Derived } else { MemoryOrigin::Explicit };
            metadata.kind = entry.kind;
            if let Some(scope) = options.scope.clone() { metadata.scope = scope; }
            if let Some(status) = options.status {
                metadata.status = status;
                if status != MemoryStatus::Superseded {
                    metadata.superseded_by = None;
                }
            }
            if let Some(importance) = options.importance { metadata.importance = importance; }
            metadata.source = options.source.clone();
            metadata.provenance_verified = metadata.source.is_some();
            metadata.source_suppressed = false;
            if let MemoryScope::Session { session_id, instance_id } = &metadata.scope
                && derived.is_some()
                && metadata.source.as_ref().is_none_or(|source| {
                    source.session_id != *session_id || source.instance_id != *instance_id
                })
            {
                return Err(invalid("session-scoped derived memory must cite that session and instance"));
            }
            let mut changes = Vec::new();
            let mut next_index = index_without(&self.read_index()?, &name);
            let mut superseded_names = BTreeSet::new();
            for replaced in &options.supersedes {
                let old_name = normalize_memory_name(&replaced.name)?;
                if old_name == name || !superseded_names.insert(old_name.to_owned()) {
                    return Err(invalid("cannot supersede the same record or duplicate a supersession"));
                }
                let mut old = self.read_optional(old_name, &state)?
                    .ok_or_else(|| MemoryError::NotFound(old_name.into()))?;
                check_revision(Some(&old), Some(&replaced.sha256))?;
                if old.metadata.scope != metadata.scope {
                    return Err(invalid("superseded memory must have the same scope"));
                }
                if old.metadata.status == MemoryStatus::Superseded
                    && old.metadata.superseded_by.as_deref() != Some(metadata.id.as_str())
                {
                    return Err(invalid("memory already superseded by another record; inspect its current replacement"));
                }
                if !metadata.supersedes.contains(&old.metadata.id) {
                    metadata.supersedes.push(old.metadata.id.clone());
                }
                old.metadata.status = MemoryStatus::Superseded;
                old.metadata.superseded_by = Some(metadata.id.clone());
                old.metadata.updated_at_ms = now;
                old.metadata.revision = generation;
                let old_input = NewMemory {
                    name: old_name.into(),
                    description: old.frontmatter.description.clone(),
                    kind: old.metadata.kind,
                    body: old.body.clone(),
                };
                let (old, raw) = render_record(&old.path, &old_input, old.metadata)?;
                changes.push((old.path.clone(), Some(raw)));
                state.put(old_name.into(), &old);
                next_index = index_without(&next_index, old_name);
            }
            let path = self.resolve_path(&name);
            let (record, raw) = render_record(&path, &entry, metadata)?;
            if record.metadata.status == MemoryStatus::Active {
                let label = if entry.description.trim().is_empty() {
                    entry.body.lines().find(|line| !line.trim().is_empty()).unwrap_or("")
                } else {
                    &entry.description
                };
                let description = label.split_whitespace().collect::<Vec<_>>().join(" ");
                let description: String = description.chars().take(240).collect();
                let line = format!("- [{name}]({name}.md) — {description}");
                next_index.push_str(&line);
                next_index.push('\n');
            }
            if next_index.len() > MAX_MEMORY_BYTES {
                return Err(invalid("memory index exceeds the 8 MiB limit"));
            }
            state.tombstones.remove(&name);
            state.put(name.clone(), &record);
            changes.push((path, Some(raw)));
            changes.push((self.dir.join(ENTRYPOINT_NAME), Some(next_index.into_bytes())));
            changes.push((self.metadata_path(), Some(state.bytes()?)));
            transaction::mutate_many(&self.dir, changes)?;
            Ok(record)
        })
    }

    fn resolve_path(&self, name: &str) -> PathBuf {
        self.dir
            .join(format!("{}.md", name.trim_end_matches(".md")))
    }

    fn read_optional(
        &self,
        name: &str,
        state: &state::MemoryState,
    ) -> MemoryResult<Option<MemoryRecord>> {
        let path = self.resolve_path(name);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
            Ok(_) => read_entry(&path)
                .and_then(|entry| state.attach(entry))
                .map(Some),
        }
    }

    fn read_index(&self) -> MemoryResult<String> {
        let path = self.dir.join(ENTRYPOINT_NAME);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error.into()),
            Ok(_) => read_text(&path),
        }
    }

    fn store_key(&self) -> String {
        state::memory_id(&self.metadata_path())
    }
}

impl MemoryRepository for MemoryStore {
    fn directory(&self) -> PathBuf {
        self.dir.clone()
    }
    fn ensure_index(&self) -> MemoryResult<PathBuf> {
        Self::ensure_index(self)
    }
    fn pending_recovery_paths(&self) -> MemoryResult<Vec<PathBuf>> {
        Self::pending_recovery_paths(self)
    }
    fn recover_pending(&self, allowed: &dyn Fn(&Path) -> bool) -> MemoryResult<bool> {
        Self::recover_pending(self, allowed)
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
    fn save_with_options(
        &self,
        entry: NewMemory,
        expected: Option<&str>,
        options: MemoryWriteOptions,
    ) -> MemoryResult<MemoryRecord> {
        Self::save_with_options(self, entry, expected, options)
    }
    fn forget_checked(&self, name: &str, expected: &str) -> MemoryResult<()> {
        Self::forget_checked(self, name, expected)
    }
    fn forget_many_checked(&self, records: &[crate::MemoryRevision]) -> MemoryResult<()> {
        Self::forget_many_checked(self, records)
    }
}

fn validate_options(options: &MemoryWriteOptions) -> MemoryResult<()> {
    if options.importance.is_some_and(|value| value > 100) {
        return Err(invalid("memory importance must be between 0 and 100"));
    }
    if options.status == Some(MemoryStatus::Superseded) {
        return Err(invalid(
            "use a revision-checked supersedes relationship to replace a memory",
        ));
    }
    if options.supersedes.len() > 32 {
        return Err(invalid(
            "at most 32 memories can be superseded in one write",
        ));
    }
    if let Some(MemoryScope::Session {
        session_id,
        instance_id,
    }) = &options.scope
        && (*session_id <= 0 || instance_id.trim().is_empty() || instance_id.len() > 256)
    {
        return Err(invalid("invalid memory session scope or runtime instance"));
    }
    if let Some(source) = options.source.as_ref()
        && (source.session_id <= 0
            || source.entry_id.is_some_and(|value| value <= 0)
            || source.instance_id.trim().is_empty()
            || source.instance_id.len() > 256
            || source
                .call_id
                .as_ref()
                .is_some_and(|value| value.trim().is_empty() || value.len() > 2048)
            || source
                .task_id
                .as_ref()
                .is_some_and(|value| value.trim().is_empty() || value.len() > 2048))
    {
        return Err(invalid("invalid memory source identity"));
    }
    Ok(())
}

fn check_revision(previous: Option<&MemoryRecord>, expected: Option<&str>) -> MemoryResult<()> {
    if let Some(previous) = previous {
        if expected.is_none_or(|value| !value.eq_ignore_ascii_case(&previous.sha256)) {
            return Err(invalid(format!(
                "memory revision conflict: read the record and supply expected_sha256={}",
                previous.sha256
            )));
        }
    } else if expected.is_some() {
        return Err(MemoryError::NotFound(
            "record required by expected_sha256".into(),
        ));
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

fn render_record(
    path: &Path,
    entry: &NewMemory,
    metadata: MemoryMetadata,
) -> MemoryResult<(MemoryRecord, Vec<u8>)> {
    let name = normalize_memory_name(&entry.name)?;
    let mut raw = format!(
        "---\nschema_version: {MEMORY_SCHEMA_VERSION}\nname: {}\ndescription: {}\nrevision: {}\n---\n\n",
        yaml_escape(name),
        yaml_escape(entry.description.trim()),
        metadata.revision,
    );
    raw.push_str(entry.body.trim_end());
    raw.push('\n');
    if raw.len() > MAX_MEMORY_BYTES {
        return Err(invalid("memory result exceeds the 8 MiB limit"));
    }
    let (frontmatter, body) = parse_frontmatter(&raw, path)?;
    let sha256 = hex::encode(Sha256::digest(raw.as_bytes()));
    Ok((
        MemoryRecord {
            file_name: format!("{name}.md"),
            path: path.into(),
            frontmatter,
            body,
            sha256,
            metadata,
        },
        raw.into_bytes(),
    ))
}

fn read_entry(path: &Path) -> MemoryResult<MemoryDocument> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid("memory record is not a regular file"));
    }
    let raw = read_text(path)?;
    let (frontmatter, body) = parse_frontmatter(&raw, path)?;
    Ok(MemoryDocument {
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
        return Err(MemoryError::Malformed {
            path: path.to_path_buf(),
            message: "memory document requires current-format frontmatter".into(),
        });
    };
    let Some(end) = stripped
        .find("\n---")
        .filter(|end| stripped[end + 4..].is_empty() || stripped[end + 4..].starts_with('\n'))
    else {
        return Err(MemoryError::Malformed {
            path: path.to_path_buf(),
            message: "frontmatter missing closing '---'".into(),
        });
    };
    let yaml = &stripped[..end];
    let body = stripped[end + 4..].trim_start_matches('\n').to_string();
    let frontmatter: MemoryFrontmatter = serde_yaml::from_str(yaml)?;
    if frontmatter.schema_version != MEMORY_SCHEMA_VERSION || frontmatter.revision == 0 {
        return Err(MemoryError::Malformed {
            path: path.to_path_buf(),
            message: "unsupported memory document schema or invalid revision".into(),
        });
    }
    if normalize_memory_name(&frontmatter.name).ok() != Some(frontmatter.name.as_str())
        || path.file_stem().and_then(|stem| stem.to_str()) != Some(frontmatter.name.as_str())
    {
        return Err(MemoryError::Malformed {
            path: path.to_path_buf(),
            message: "memory document name must be canonical and match its file name".into(),
        });
    }
    Ok((frontmatter, body))
}

fn is_memory_file(path: &Path) -> bool {
    path.is_file()
        && path.extension().and_then(|s| s.to_str()) == Some("md")
        && matches!(path.file_name().and_then(|s| s.to_str()), Some(name) if !name.eq_ignore_ascii_case(ENTRYPOINT_NAME))
}

fn yaml_escape(s: &str) -> String {
    // JSON quoted strings are valid YAML scalars.
    serde_json::to_string(s).expect("serializing a string cannot fail")
}
