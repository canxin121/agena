//! Process-wide coordination and atomic persistence for configuration files.

use std::fs;
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::{LazyLock, Mutex};

static CONFIG_FILE_WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Serialize process-local configuration read/modify/write transactions.
///
/// Configuration edits are infrequent and can target overlapping global and
/// workspace files from several services. One lock avoids stale snapshots and
/// has no async suspension point, so it cannot participate in Tokio deadlocks.
pub fn with_config_file_write_lock<T>(operation: impl FnOnce() -> T) -> T {
    let _guard = CONFIG_FILE_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    operation()
}

/// Persist a configuration document through a unique same-directory tempfile.
/// Callers performing a read/modify/write transaction must hold
/// [`with_config_file_write_lock`] across the read and this write.
pub fn write_config_file_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("configuration path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;
    let permissions = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Some(metadata.permissions()),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "configuration target is not a regular file: {}",
                    path.display()
                ),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    let mut builder = tempfile::Builder::new();
    builder.prefix(".agena-config-").suffix(".tmp");
    #[cfg(unix)]
    if permissions.is_none() {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(fs::Permissions::from_mode(0o600));
    }
    let mut staged = builder.tempfile_in(parent)?;
    staged.write_all(bytes)?;
    staged.flush()?;
    staged.as_file().sync_all()?;
    if let Some(permissions) = permissions {
        staged.as_file().set_permissions(permissions)?;
        staged.as_file().sync_all()?;
    }
    staged
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

#[cfg(test)]
mod tests {
    use super::{with_config_file_write_lock, write_config_file_atomically};

    #[test]
    fn config_write_replaces_atomically_without_fixed_temp_file() {
        let directory = tempfile::tempdir().expect("configuration directory");
        let path = directory.path().join("config.json");

        with_config_file_write_lock(|| write_config_file_atomically(&path, b"first"))
            .expect("first config");
        with_config_file_write_lock(|| write_config_file_atomically(&path, b"replacement"))
            .expect("replacement config");

        assert_eq!(fs::read(&path).expect("read config"), b"replacement");
        assert_eq!(
            directory.path().read_dir().expect("read directory").count(),
            1
        );
    }

    use std::fs;
}
