//! Recoverable multi-file memory publication.
//!
//! Writers hold the process and OS file locks. New bytes are staged first;
//! an undo journal is made durable before publication. Removing that journal
//! commits the document, navigation index and lifecycle state together.
//! Explicit authorized recovery restores the previous revision after a crash.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::*;

pub(super) const JOURNAL_NAME: &str = ".memory-transaction.json";
pub(super) const LOCK_NAME: &str = ".memory.lock";
pub(super) const MAX_JOURNAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_TARGETS: usize = 1002;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UndoEntry {
    name: String,
    previous: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UndoJournal {
    schema_version: u32,
    entries: Vec<UndoEntry>,
}

pub(super) fn stage(path: &Path, bytes: &[u8]) -> io::Result<tempfile::NamedTempFile> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "memory path has no parent"))?;
    agena_storage_layout::secure_directory(parent)?;
    let permissions = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => Some(meta.permissions()),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "memory target must be a regular file, not a symlink",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let mut builder = tempfile::Builder::new();
    builder.prefix(".agena-memory-").suffix(".tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o600));
    }
    let mut staged = builder.tempfile_in(parent)?;
    staged.write_all(bytes)?;
    staged.flush()?;
    if let Some(permissions) = permissions {
        staged.as_file().set_permissions(permissions)?;
    }
    agena_storage_layout::secure_file(staged.path())?;
    staged.as_file().sync_all()?;
    Ok(staged)
}

pub(super) fn lock(dir: &Path, exclusive: bool, create: bool) -> MemoryResult<Option<fs::File>> {
    let path = dir.join(LOCK_NAME);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {}
        Ok(_) => return Err(invalid("memory lock must be a regular file")),
        Err(error) if error.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(create).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(&path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("memory lock must be a regular file"));
    }
    if create {
        agena_storage_layout::secure_file(&path)?;
    }
    if exclusive {
        file.lock()?;
    } else {
        file.lock_shared()?;
    }
    Ok(Some(file))
}

pub(super) fn check_clean(dir: &Path) -> MemoryResult<()> {
    match fs::symlink_metadata(dir.join(JOURNAL_NAME)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "unfinished memory transaction; authorize recover_pending before accessing the store",
        )
        .into()),
    }
}

fn valid_target(name: &str) -> bool {
    if matches!(name, ENTRYPOINT_NAME | state::STATE_NAME) {
        return true;
    }
    name.strip_suffix(".md")
        .is_some_and(|base| canonical_name(base).is_ok_and(|valid| valid == base))
}

fn target_name(dir: &Path, path: &Path) -> MemoryResult<String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("memory transaction target has no UTF-8 name"))?;
    if path.parent() != Some(dir) || !valid_target(name) {
        return Err(invalid("invalid memory transaction target"));
    }
    Ok(name.to_owned())
}

fn sync_directory(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn read_journal(dir: &Path) -> MemoryResult<Option<UndoJournal>> {
    let journal_path = dir.join(JOURNAL_NAME);
    match fs::symlink_metadata(&journal_path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let raw = read_bounded_limit(&journal_path, MAX_JOURNAL_BYTES)?;
    let journal: UndoJournal =
        serde_json::from_slice(&raw).map_err(|error| MemoryError::Malformed {
            path: journal_path,
            message: format!("invalid memory recovery journal: {error}"),
        })?;
    if journal.schema_version != 1
        || journal.entries.is_empty()
        || journal.entries.len() > MAX_TARGETS
    {
        return Err(invalid("unsupported or oversized memory recovery journal"));
    }
    let mut names = BTreeSet::new();
    for entry in &journal.entries {
        if !valid_target(&entry.name) || !names.insert(entry.name.as_str()) {
            return Err(invalid("invalid or duplicate recovery target"));
        }
        if entry
            .previous
            .as_ref()
            .is_some_and(|text| text.len() > MAX_MEMORY_BYTES)
        {
            return Err(invalid("recovery document exceeds memory size limit"));
        }
        match fs::symlink_metadata(dir.join(&entry.name)) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err(invalid("recovery target must be a regular file")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(journal))
}

/// Read and validate the bounded journal without staging or publishing files.
pub(super) fn recovery_paths(dir: &Path) -> MemoryResult<Option<Vec<PathBuf>>> {
    Ok(read_journal(dir)?.map(|journal| {
        journal
            .entries
            .into_iter()
            .map(|entry| dir.join(entry.name))
            .collect()
    }))
}

pub(super) fn recover(dir: &Path) -> MemoryResult<()> {
    recover_authorized(dir, |_| true).map(|_| ())
}

/// Authorize the exact parsed journal before staging any previous contents.
pub(super) fn recover_authorized(
    dir: &Path,
    allowed: impl Fn(&Path) -> bool,
) -> MemoryResult<bool> {
    let Some(journal) = read_journal(dir)? else {
        return Ok(false);
    };
    for entry in &journal.entries {
        let path = dir.join(&entry.name);
        if !allowed(&path) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("memory recovery is not authorized for {}", path.display()),
            )
            .into());
        }
    }
    let mut staged = Vec::with_capacity(journal.entries.len());
    for entry in journal.entries {
        let path = dir.join(entry.name);
        let replacement = entry
            .previous
            .as_ref()
            .map(|bytes| stage(&path, bytes.as_bytes()))
            .transpose()?;
        staged.push((path, replacement));
    }
    for (path, replacement) in staged {
        if let Some(replacement) = replacement {
            replacement.persist(&path).map_err(|error| error.error)?;
        } else {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    sync_directory(dir)?;
    fs::remove_file(dir.join(JOURNAL_NAME))?;
    sync_directory(dir)?;
    Ok(true)
}

pub(super) fn mutate_many(
    dir: &Path,
    changes: Vec<(PathBuf, Option<Vec<u8>>)>,
) -> MemoryResult<()> {
    check_clean(dir)?;
    if changes.is_empty() {
        return Ok(());
    }
    if changes.len() > MAX_TARGETS {
        return Err(invalid("too many memory transaction targets"));
    }
    let mut names = BTreeSet::new();
    let mut undo = Vec::with_capacity(changes.len());
    let mut staged = Vec::with_capacity(changes.len());
    for (path, new) in changes {
        let name = target_name(dir, &path)?;
        if !names.insert(name.clone()) {
            return Err(invalid("duplicate memory transaction target"));
        }
        let previous = match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => Some(read_text(&path)?),
            Ok(_) => return Err(invalid("memory target must be a regular file")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if new
            .as_ref()
            .is_some_and(|bytes| bytes.len() > MAX_MEMORY_BYTES)
        {
            return Err(invalid("memory transaction document exceeds 8 MiB"));
        }
        let next = new.as_ref().map(|bytes| stage(&path, bytes)).transpose()?;
        undo.push(UndoEntry { name, previous });
        staged.push((path, next));
    }
    let journal = serde_json::to_vec(&UndoJournal {
        schema_version: 1,
        entries: undo,
    })
    .map_err(io::Error::other)?;
    if journal.len() > MAX_JOURNAL_BYTES {
        return Err(invalid("memory transaction journal exceeds 64 MiB"));
    }
    let journal_path = dir.join(JOURNAL_NAME);
    stage(&journal_path, &journal)?
        .persist(&journal_path)
        .map_err(|error| error.error)?;
    sync_directory(dir)?;
    let publish = || -> io::Result<()> {
        for (path, next) in staged {
            if let Some(next) = next {
                next.persist(&path).map_err(|error| error.error)?;
            } else {
                fs::remove_file(&path)?;
            }
        }
        sync_directory(dir)
    };
    if let Err(primary) = publish() {
        if let Err(rollback) = recover(dir) {
            return Err(io::Error::new(
                primary.kind(),
                format!("memory publication failed: {primary}; recovery also failed: {rollback}"),
            )
            .into());
        }
        return Err(primary.into());
    }
    fs::remove_file(journal_path)?;
    sync_directory(dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_restores_all_files_and_rejects_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("note.md");
        let index = dir.path().join(ENTRYPOINT_NAME);
        fs::write(&doc, "new bytes").unwrap();
        fs::write(&index, "new index").unwrap();
        let pending = UndoJournal {
            schema_version: 1,
            entries: vec![
                UndoEntry {
                    name: "note.md".into(),
                    previous: Some("old bytes".into()),
                },
                UndoEntry {
                    name: ENTRYPOINT_NAME.into(),
                    previous: Some("old index".into()),
                },
                UndoEntry {
                    name: "created.md".into(),
                    previous: None,
                },
            ],
        };
        fs::write(dir.path().join("created.md"), "uncommitted").unwrap();
        fs::write(
            dir.path().join(JOURNAL_NAME),
            serde_json::to_vec(&pending).unwrap(),
        )
        .unwrap();
        assert!(check_clean(dir.path()).is_err());
        recover(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(doc).unwrap(), "old bytes");
        assert_eq!(fs::read_to_string(index).unwrap(), "old index");
        assert!(!dir.path().join("created.md").exists());
        assert!(check_clean(dir.path()).is_ok());
        let malicious = UndoJournal {
            schema_version: 1,
            entries: vec![UndoEntry {
                name: "../escape.md".into(),
                previous: Some("bad".into()),
            }],
        };
        fs::write(
            dir.path().join(JOURNAL_NAME),
            serde_json::to_vec(&malicious).unwrap(),
        )
        .unwrap();
        assert!(recover(dir.path()).is_err());
        assert!(!dir.path().parent().unwrap().join("escape.md").exists());
    }

    #[test]
    fn staging_failure_publishes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("note.md");
        fs::write(&doc, "old").unwrap();
        fs::create_dir(dir.path().join(ENTRYPOINT_NAME)).unwrap();
        let result = mutate_many(
            dir.path(),
            vec![
                (doc.clone(), Some(b"new".to_vec())),
                (dir.path().join(ENTRYPOINT_NAME), Some(b"index".to_vec())),
            ],
        );
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(doc).unwrap(), "old");
        assert!(!dir.path().join(JOURNAL_NAME).exists());
    }
}
