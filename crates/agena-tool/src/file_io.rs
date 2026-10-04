//! Regular-file opening shared by tool readers. Callers retain responsibility
//! for path authorization, byte/time limits and detecting concurrent writes.
use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

/// Reject special files before and after opening. On Unix, nonblocking open
/// prevents a path swapped for a FIFO from waiting for a writer. Existing
/// symlink behavior is preserved; this is not a path-authorization boundary.
pub fn open_regular_file(path: &Path) -> io::Result<File> {
    if !path.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target must be a regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target changed to a non-regular file while opening",
        ));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regular_files_are_opened_and_directories_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file");
        std::fs::write(&path, "text").unwrap();
        assert_eq!(
            open_regular_file(&path).unwrap().metadata().unwrap().len(),
            4
        );
        assert_eq!(
            open_regular_file(root.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    #[cfg(unix)]
    #[test]
    fn fifo_is_rejected_and_regular_symlink_reads_remain_supported() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        let link = root.path().join("link");
        std::fs::write(&file, "text").unwrap();
        symlink(&file, &link).unwrap();
        assert!(open_regular_file(&link).is_ok());
        let fifo = root.path().join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert_eq!(
            open_regular_file(&fifo).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
