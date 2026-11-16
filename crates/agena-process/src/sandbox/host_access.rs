//! Keep derived filesystem grants within the invoking user's host access.

use std::{fs, io, path::Path};

/// Probe an existing regular file or directory using MXC's delegation semantics.
/// Missing or inaccessible derived roots are omitted instead of invalidating a
/// broad filesystem grant. Unexpected host errors remain visible to the caller.
pub(super) fn can_delegate(path: &Path, write: bool) -> io::Result<bool> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => return classify_error(error),
    };
    // In particular, do not open a FIFO, socket, or device to test its access.
    if !metadata.is_file() && !metadata.is_dir() {
        return Ok(false);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DEVICE;

        if metadata.file_attributes() & FILE_ATTRIBUTE_DEVICE != 0 {
            return Ok(false);
        }
    }
    user_can_access(path, write)
}

#[cfg(unix)]
fn user_can_access(path: &Path, write: bool) -> io::Result<bool> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let mask = libc::R_OK | if write { libc::W_OK } else { 0 };
    // SAFETY: path remains a valid NUL-terminated string for the call. access
    // checks real UID/GID, matching MXC rather than the process's effective IDs.
    if unsafe { libc::access(path.as_ptr(), mask) } == 0 {
        Ok(true)
    } else {
        classify_error(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn user_can_access(path: &Path, write: bool) -> io::Result<bool> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    match fs::OpenOptions::new()
        .read(true)
        .write(write)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
    {
        Ok(_) => Ok(true),
        Err(error) => classify_error(error),
    }
}

#[cfg(not(any(unix, windows)))]
fn user_can_access(_path: &Path, _write: bool) -> io::Result<bool> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "filesystem delegation is unsupported on this platform",
    ))
}

fn classify_error(error: io::Error) -> io::Result<bool> {
    #[cfg(unix)]
    if matches!(
        error.raw_os_error(),
        Some(libc::ENOENT | libc::ENOTDIR | libc::EACCES | libc::EPERM | libc::EROFS)
    ) {
        return Ok(false);
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{
            ERROR_ACCESS_DENIED, ERROR_DIRECTORY, ERROR_FILE_NOT_FOUND, ERROR_INVALID_NAME,
            ERROR_PATH_NOT_FOUND, ERROR_WRITE_PROTECT,
        };

        if error.raw_os_error().is_some_and(|code| {
            matches!(
                code as u32,
                ERROR_FILE_NOT_FOUND
                    | ERROR_PATH_NOT_FOUND
                    | ERROR_DIRECTORY
                    | ERROR_ACCESS_DENIED
                    | ERROR_INVALID_NAME
                    | ERROR_WRITE_PROTECT
            )
        }) {
            return Ok(false);
        }
    }
    match error.kind() {
        io::ErrorKind::NotFound
        | io::ErrorKind::PermissionDenied
        | io::ErrorKind::ReadOnlyFilesystem => Ok(false),
        _ => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_files_and_directories_can_be_delegated() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("input.txt");
        fs::write(&file, "input").unwrap();
        for path in [root.path(), file.as_path()] {
            assert!(can_delegate(path, false).unwrap());
            assert!(can_delegate(path, true).unwrap());
        }
    }

    #[test]
    fn absent_roots_and_children_of_files_are_omitted() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        fs::write(&file, "input").unwrap();
        for path in [root.path().join("absent"), file.join("child")] {
            assert!(!can_delegate(&path, false).unwrap());
            assert!(!can_delegate(&path, true).unwrap());
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_modes_limit_delegation_for_unprivileged_users() {
        use std::os::unix::fs::PermissionsExt;

        // SAFETY: getuid has no preconditions. Root bypasses these mode checks.
        if unsafe { libc::getuid() } == 0 {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("input.txt");
        fs::write(&file, "input").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o400)).unwrap();
        let readonly = (can_delegate(&file, false), can_delegate(&file, true));
        fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
        let inaccessible = (can_delegate(&file, false), can_delegate(&file, true));
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(readonly.0.unwrap());
        assert!(!readonly.1.unwrap());
        assert!(!inaccessible.0.unwrap());
        assert!(!inaccessible.1.unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlinks_are_omitted() {
        let root = tempfile::tempdir().unwrap();
        let link = root.path().join("dangling");
        std::os::unix::fs::symlink(root.path().join("absent"), &link).unwrap();
        assert!(!can_delegate(&link, false).unwrap());
        assert!(!can_delegate(&link, true).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn fifos_are_omitted_without_opening_them() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};

        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("fifo");
        let path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: path is valid and the temporary directory is owned by the test.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(!can_delegate(&fifo, false).unwrap());
        assert!(!can_delegate(&fifo, true).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn unexpected_host_failures_are_not_treated_as_missing_permissions() {
        let error = classify_error(io::Error::from_raw_os_error(libc::EIO)).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EIO));
        assert!(!classify_error(io::Error::from_raw_os_error(libc::EROFS)).unwrap());
    }
}
