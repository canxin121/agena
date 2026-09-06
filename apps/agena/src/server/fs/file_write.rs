//! Publish complete Workbench file writes using same-directory staging.

use std::{
    collections::HashMap,
    fs,
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, RwLock, Weak},
};

use axum::body::Bytes;

use super::{ApiResult, AppError};

#[cfg(test)]
mod tests;

// Keep file syncs and other blocking filesystem work off Tokio's workers.
// The permit lives in the blocking task even if the HTTP waiter is dropped.
static FILE_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);

// Namespace changes need exclusive access, while independent file edits can
// proceed together. Every participating operation takes this before a path lock.
static NAMESPACE: RwLock<()> = RwLock::new(());
static PATH_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, thiserror::Error)]
#[error("File changed during replacement; run search again before replacing")]
struct FileChanged;

async fn file_worker<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> io::Result<T> {
    let permit = FILE_WORKERS.acquire().await.map_err(io::Error::other)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await
    .map_err(io::Error::other)
}

fn path_lock(path: &Path) -> Arc<Mutex<()>> {
    let mut locks = PATH_LOCKS.lock().unwrap_or_else(|error| error.into_inner());
    // Retain only live guards/waiters, rather than growing with every edited path.
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}

fn lock_key(path: &Path) -> io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| io::Error::other("file path has no parent"))?;
            let name = path
                .file_name()
                .ok_or_else(|| io::Error::other("file path has no name"))?;
            Ok(fs::canonicalize(parent)?.join(name))
        }
        Err(error) => Err(error),
    }
}

pub(super) enum FileRead {
    Contents(Vec<u8>),
    NotFile,
    TooLarge,
}

impl FileRead {
    pub(super) fn into_contents(self) -> Option<Vec<u8>> {
        match self {
            Self::Contents(bytes) => Some(bytes),
            Self::NotFile | Self::TooLarge => None,
        }
    }
}

pub(super) async fn read_file_bounded(path: PathBuf, limit: u64) -> io::Result<FileRead> {
    file_worker(move || read_blocking_bounded(&path, limit)).await?
}

fn read_blocking_bounded(path: &Path, limit: u64) -> io::Result<FileRead> {
    // Some platforms reject opening a directory before we can inspect its
    // handle. Also check the opened handle below to cover path replacement.
    let preliminary = fs::metadata(path)?;
    if !preliminary.is_file() {
        return Ok(FileRead::NotFile);
    }
    if preliminary.len() > limit {
        return Ok(FileRead::TooLarge);
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Ok(FileRead::NotFile);
    }
    if metadata.len() > limit {
        return Ok(FileRead::TooLarge);
    }
    read_opened_bounded(file, metadata.len(), limit)
}

fn read_opened_bounded(file: fs::File, initial_len: u64, limit: u64) -> io::Result<FileRead> {
    let mut bytes = Vec::with_capacity(initial_len as usize);
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    Ok(if bytes.len() as u64 <= limit {
        FileRead::Contents(bytes)
    } else {
        FileRead::TooLarge
    })
}

pub(super) async fn transform_file_atomically<T: Send + 'static>(
    path: PathBuf,
    read_limit: u64,
    transform: impl FnOnce(Option<&[u8]>) -> ApiResult<(Option<Bytes>, T)> + Send + 'static,
) -> ApiResult<T> {
    file_worker(move || {
        let _namespace = NAMESPACE.read().unwrap_or_else(|error| error.into_inner());
        let path = fs::canonicalize(&path).map_err(replacement_io_error)?;
        let lock = path_lock(&path);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        let original = read_blocking_bounded(&path, read_limit)
            .map_err(replacement_io_error)?
            .into_contents();
        let (updated, result) = transform(original.as_deref())?;
        if let Some(updated) = updated {
            let original = original
                .ok_or_else(|| AppError::bad_request("Target file is not searchable text"))?;
            write_blocking(&path, &updated, true, Some(&original)).map_err(replacement_io_error)?;
        }
        Ok(result)
    })
    .await
    .map_err(replacement_io_error)?
}

fn replacement_io_error(error: io::Error) -> AppError {
    if error
        .get_ref()
        .is_some_and(|cause| cause.is::<FileChanged>())
    {
        return AppError::conflict(error.to_string());
    }
    match error.kind() {
        io::ErrorKind::NotFound => AppError::not_found("File not found"),
        io::ErrorKind::PermissionDenied => {
            AppError::forbidden_error("replace file contents", &error)
        }
        _ => AppError::internal_error_with_context("replace file contents", &error),
    }
}

pub(super) async fn remove_path(path: PathBuf) -> io::Result<()> {
    file_worker(move || {
        let _namespace = NAMESPACE.write().unwrap_or_else(|error| error.into_inner());
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
            Ok(_) => fs::remove_file(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    })
    .await?
}

pub(super) async fn rename_path(from: PathBuf, to: PathBuf) -> io::Result<()> {
    file_worker(move || {
        let _namespace = NAMESPACE.write().unwrap_or_else(|error| error.into_inner());
        fs::rename(from, to)
    })
    .await?
}

pub(super) async fn write_file_atomically(
    path: PathBuf,
    bytes: Bytes,
    overwrite: bool,
) -> io::Result<()> {
    file_worker(move || {
        let _namespace = NAMESPACE.read().unwrap_or_else(|error| error.into_inner());
        let key = lock_key(&path)?;
        let lock = path_lock(&key);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        write_blocking(&path, &bytes, overwrite, None)
    })
    .await?
}

fn write_blocking(
    path: &Path,
    bytes: &[u8],
    overwrite: bool,
    expected: Option<&[u8]>,
) -> io::Result<()> {
    let existing = match fs::symlink_metadata(path) {
        Ok(metadata) if !overwrite => {
            return Err(io::Error::new(
                if metadata.is_dir() {
                    io::ErrorKind::IsADirectory
                } else {
                    io::ErrorKind::AlreadyExists
                },
                "upload target already exists",
            ));
        }
        Ok(_) => {
            // Preserve writes through an existing symbolic link. Canonicalize
            // before staging so atomic rename replaces the referent, not the
            // link itself. A dangling link is an error, never a missing target
            // that may be silently replaced.
            let resolved = fs::canonicalize(path)?;
            let metadata = fs::metadata(&resolved)?;
            if !metadata.is_file() {
                return Err(io::Error::new(
                    if metadata.is_dir() {
                        io::ErrorKind::IsADirectory
                    } else {
                        io::ErrorKind::InvalidInput
                    },
                    "file write target is not a regular file",
                ));
            }
            // A writable parent must not let atomic replacement bypass the
            // existing file's explicit read-only permissions.
            if metadata.permissions().readonly() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "file write target is read-only",
                ));
            }
            // Keep the operating system's write-access checks (including
            // ACLs) even though publication itself uses rename permissions.
            let mut options = fs::OpenOptions::new();
            options.write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.custom_flags(libc::O_NONBLOCK);
            }
            let file = options.open(&resolved)?;
            if !file.metadata()?.is_file() {
                return Err(io::Error::other(FileChanged));
            }
            drop(file);
            Some((resolved, metadata.permissions()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let target = existing.as_ref().map_or(path, |(path, _)| path.as_path());
    let parent = target.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "file write target has no parent directory",
        )
    })?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".agena-write-").suffix(".tmp");
    #[cfg(unix)]
    if existing.is_none() {
        use std::os::unix::fs::PermissionsExt as _;
        // Keep ordinary new-file permissions, with the process umask applied
        // by tempfile at creation. Replacements inherit the old permissions.
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut staged = builder.tempfile_in(parent)?;
    staged.write_all(bytes)?;
    staged.flush()?;
    if let Some((_, permissions)) = &existing {
        staged.as_file().set_permissions(permissions.clone())?;
    }
    staged.as_file().sync_all()?;

    if let Some(expected) = expected {
        // Check external editors again after the potentially expensive transform
        // and staging write. The path/namespace locks coordinate Workbench operations;
        // uncooperative external writers still have a final check/rename race.
        let current = match read_blocking_bounded(target, expected.len() as u64) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(io::Error::other(FileChanged));
            }
            result => result?,
        };
        if current.into_contents().as_deref() != Some(expected) {
            return Err(io::Error::other(FileChanged));
        }
    }

    if overwrite {
        staged.persist(target)
    } else {
        // This is the deciding existence check. A concurrent creator cannot
        // be clobbered even if preflight observed no target.
        staged.persist_noclobber(target)
    }
    .map(|_| ())
    .map_err(|error| error.error)
}
