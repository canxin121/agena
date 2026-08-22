//! Atomic file persistence for Git helper endpoints.
//!
//! The endpoints own repository/global mutation semantics; `tempfile` owns
//! collision-safe same-directory staging and atomic rename behavior.

use std::{
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use tempfile::{Builder, NamedTempFile};

const FILE_WORKER_LIMIT: usize = 16;
static FILE_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(FILE_WORKER_LIMIT);

pub(super) async fn write_file_atomically(path: PathBuf, bytes: Vec<u8>) -> io::Result<()> {
    let permit = FILE_WORKERS.acquire().await.map_err(|error| {
        io::Error::other(agena_failure::diagnostic::format_error_chain_with_context(
            "acquire a Git atomic-file worker",
            &error,
        ))
    })?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        write_file_atomically_blocking(&path, &bytes)
    })
    .await
    .map_err(|error| {
        io::Error::other(agena_failure::diagnostic::format_error_chain_with_context(
            "Git atomic-file worker failed",
            &error,
        ))
    })?
}

fn write_file_atomically_blocking(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let path = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error),
    };
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("atomic write target is not a file: {}", path.display()),
                ));
            }
            Some(metadata)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("atomic write path has no parent: {}", path.display()),
        )
    })?;
    if !parent.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("atomic write parent does not exist: {}", parent.display()),
        ));
    }

    let permissions = metadata.as_ref().map(fs::Metadata::permissions);
    let mut builder = Builder::new();
    builder.prefix(".agena-git-write-").suffix(".tmp");
    #[cfg(unix)]
    if permissions.is_none() {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut staged: NamedTempFile = builder.tempfile_in(parent)?;
    staged.write_all(bytes)?;
    staged.flush()?;
    staged.as_file().sync_all()?;
    if let Some(permissions) = permissions {
        staged.as_file().set_permissions(permissions)?;
        staged.as_file().sync_all()?;
    }

    if metadata.is_some() {
        staged
            .persist(&path)
            .map(|_| ())
            .map_err(|error| error.error)
    } else {
        staged
            .persist_noclobber(&path)
            .map(|_| ())
            .map_err(|error| error.error)
    }
}

#[cfg(test)]
mod tests {
    use super::write_file_atomically;

    #[tokio::test]
    async fn writes_and_replaces_without_leaving_staging_files() {
        let directory = tempfile::tempdir().expect("Git atomic directory");
        let path = directory.path().join("config");

        write_file_atomically(path.clone(), b"first".to_vec())
            .await
            .expect("create file");
        write_file_atomically(path.clone(), b"second".to_vec())
            .await
            .expect("replace file");

        assert_eq!(std::fs::read(path).expect("read file"), b"second");
        assert_eq!(
            directory.path().read_dir().expect("read directory").count(),
            1
        );
    }
}
