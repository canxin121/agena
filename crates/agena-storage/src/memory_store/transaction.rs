//! Stage all fallible I/O before publication. If index publication fails,
//! restore the record and preserve the primary failure. MEMORY.md is derived
//! data; this is an in-process transaction, not a cross-process crash journal.
use super::*;

pub(super) fn stage(path: &Path, bytes: &[u8]) -> io::Result<tempfile::NamedTempFile> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "memory path has no parent"))?;
    fs::create_dir_all(parent)?;
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
    staged.as_file().sync_all()?;
    Ok(staged)
}

pub(super) fn mutate(
    path: &Path,
    new: Option<&[u8]>,
    index: Option<(&Path, &[u8])>,
) -> MemoryResult<()> {
    let previous = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            Some((read_bounded(path)?, meta.permissions()))
        }
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "memory target is not a regular file",
            )
            .into());
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let staged_record = new.map(|bytes| stage(path, bytes)).transpose()?;
    let staged_index = index.map(|(path, bytes)| stage(path, bytes)).transpose()?;
    if let Some(record) = staged_record {
        record.persist(path).map_err(|error| error.error)?;
    } else {
        fs::remove_file(path)?;
    }
    let published = index
        .zip(staged_index)
        .map_or(Ok(()), |((path, _), staged)| {
            staged
                .persist(path)
                .map(|_| ())
                .map_err(|error| error.error)
        });
    recover_index_failure(path, previous, published)
}

fn recover_index_failure(
    path: &Path,
    previous: Option<(Vec<u8>, fs::Permissions)>,
    result: io::Result<()>,
) -> MemoryResult<()> {
    if let Err(primary) = result {
        let rollback = match previous {
            Some((bytes, permissions)) => stage(path, &bytes).and_then(|staged| {
                staged.as_file().set_permissions(permissions)?;
                staged
                    .persist(path)
                    .map(|_| ())
                    .map_err(|error| error.error)
            }),
            None => fs::remove_file(path),
        };
        if let Err(secondary) = rollback {
            return Err(io::Error::new(primary.kind(),format!("memory index publication failed: {primary}; record rollback also failed: {secondary}; record may have changed, inspect before retrying")).into());
        }
        return Err(primary.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_index_publication_restores_complete_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("record.md");
        fs::write(&path, "original").unwrap();
        let permissions = fs::metadata(&path).unwrap().permissions();
        fs::write(&path, "replacement").unwrap();
        assert!(
            recover_index_failure(
                &path,
                Some((b"original".to_vec(), permissions)),
                Err(io::Error::other("injected index failure"))
            )
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"original");
    }
}
