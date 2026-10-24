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
