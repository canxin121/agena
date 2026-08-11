//! Shared filesystem-mutation primitives for built-in tools.
//!
//! General-purpose temporary-file creation and atomic persistence belong to
//! `tempfile`. Agena adds only its product-specific coordination here: all
//! cooperating writers lock canonical paths in sorted order, and replacements
//! preserve the target's permissions.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, Permissions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Weak};

use parking_lot::Mutex;
use path_clean::PathClean as _;
use tempfile::{Builder, NamedTempFile};

type FileLock = Mutex<()>;

static FILE_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Weak<FileLock>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Resolve every existing component while retaining a missing output suffix.
/// Permission checks and actual mutation must use the same path identity.
pub fn canonicalize_mutation_path(path: &Path) -> PathBuf {
    let original = path.clean();
    let mut current = original.clone();
    let mut missing = Vec::<OsString>::new();
    loop {
        match fs::canonicalize(&current) {
            Ok(mut resolved) => {
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return resolved;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(name) = current.file_name().map(OsString::from) else {
                    return original;
                };
                let Some(parent) = current.parent() else {
                    return original;
                };
                missing.push(name);
                current = parent.to_path_buf();
            }
            Err(_) => return original,
        }
    }
}

/// Serialize cooperating mutations of the same files. Paths are sorted and
/// deduplicated before locking, preventing AB/BA deadlocks for multi-file
/// patches. This function is synchronous by design and must run on a blocking
/// worker, never directly on a Tokio runtime worker.
pub fn with_file_mutation_locks<T>(paths: &[PathBuf], operation: impl FnOnce() -> T) -> T {
    let mut paths = paths
        .iter()
        .map(|path| canonicalize_mutation_path(path))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();

    let locks = {
        let mut registry = FILE_LOCKS.lock();
        registry.retain(|_, lock| lock.strong_count() > 0);
        paths
            .into_iter()
            .map(|path| {
                if let Some(lock) = registry.get(&path).and_then(Weak::upgrade) {
                    lock
                } else {
                    let lock = Arc::new(FileLock::new(()));
                    registry.insert(path, Arc::downgrade(&lock));
                    lock
                }
            })
            .collect::<Vec<_>>()
    };
    let _guards = locks.iter().map(|lock| lock.lock()).collect::<Vec<_>>();
    operation()
}

/// Atomically replace one regular file using a temporary file in the same
/// directory. Existing permissions are retained.
pub fn atomic_replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "atomic replacement target is not a regular file: {}",
                path.display()
            ),
        ));
    }
    if metadata.permissions().readonly() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("atomic replacement target is read-only: {}", path.display()),
        ));
    }
    let temporary = staged_file(path, bytes, Some(metadata.permissions()), false)?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

/// Atomically create one file without clobbering a target that appeared after
/// preflight. Optional permissions are used by moves and transaction rollback.
pub fn atomic_create_file(
    path: &Path,
    bytes: &[u8],
    permissions: Option<Permissions>,
) -> io::Result<()> {
    let temporary = staged_file(path, bytes, permissions, true)?;
    temporary
        .persist_noclobber(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

/// Atomically create or replace one regular file while serializing all
/// cooperating writes to the same canonical path.
pub fn atomic_write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    with_file_mutation_locks(&[path.to_path_buf()], || match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => atomic_replace_file(path, bytes),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "atomic write target is not a regular file: {}",
                path.display()
            ),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            atomic_create_file(path, bytes, None)
        }
        Err(error) => Err(error),
    })
}

fn staged_file(
    path: &Path,
    bytes: &[u8],
    permissions: Option<Permissions>,
    use_default_create_permissions: bool,
) -> io::Result<NamedTempFile> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("file path has no parent directory: {}", path.display()),
        )
    })?;
    if !parent.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("parent directory does not exist: {}", parent.display()),
        ));
    }

    let mut builder = Builder::new();
    builder.prefix(".agena-write-").suffix(".tmp");
    #[cfg(unix)]
    if permissions.is_none() && use_default_create_permissions {
        use std::os::unix::fs::PermissionsExt as _;
        // `tempfile` applies the process umask at creation time.
        builder.permissions(Permissions::from_mode(0o666));
    }

    let mut temporary = builder.tempfile_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    if let Some(permissions) = permissions {
        temporary.as_file().set_permissions(permissions)?;
        temporary.as_file().sync_all()?;
    }
    Ok(temporary)
}

#[cfg(test)]
mod tests {
    use super::{
        atomic_create_file, atomic_replace_file, atomic_write_file, with_file_mutation_locks,
    };

    #[test]
    fn create_never_clobbers_and_replace_preserves_complete_content() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("state.txt");

        atomic_create_file(&path, b"first", None).expect("atomic create");
        assert!(atomic_create_file(&path, b"stale", None).is_err());
        atomic_replace_file(&path, b"second").expect("atomic replace");

        assert_eq!(std::fs::read(&path).expect("read result"), b"second");
        assert!(
            directory
                .path()
                .read_dir()
                .expect("read temporary directory")
                .all(|entry| !entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".agena-write-"))
        );
    }

    #[test]
    fn duplicate_paths_are_locked_once() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("same.txt");
        let result = with_file_mutation_locks(&[path.clone(), path], || 42);
        assert_eq!(result, 42);
    }

    #[test]
    fn write_convenience_handles_create_and_replace() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("artifact.bin");

        atomic_write_file(&path, b"first").expect("create artifact");
        atomic_write_file(&path, b"replacement").expect("replace artifact");

        assert_eq!(std::fs::read(path).expect("read artifact"), b"replacement");
    }
}
