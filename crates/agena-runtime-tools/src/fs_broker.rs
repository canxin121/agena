//! Host-side publication through directory capabilities retained from staging.
//!
//! Each parent is opened without following links. After the filesystem root is
//! opened, every operation is relative to one of those handles, including
//! publication and rollback. Renaming an ancestor therefore cannot redirect a
//! staged operation into a different directory.

use std::collections::{BTreeSet, VecDeque};
use std::ffi::OsString;
use std::fs::Permissions;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::tool::ToolError;
use crate::tool::apply_patch::{
    MAX_PATCH_FILE_BYTES, MAX_PATCH_OPERATIONS, MAX_PATCH_TRANSACTION_BYTES, SandboxFileChange,
};

/// Read a bounded regular file through directory handles without following links.
///
/// The caller must first authorize this exact absolute canonical path. This
/// function does not grant access. It refuses links while opening parents and
/// the file, so replacement symlinks cannot redirect the read to a new target.
pub fn read_regular_file_nofollow(path: &Path, max_bytes: usize) -> io::Result<Vec<u8>> {
    agena_process::sandbox::validate_resource_paths(&[path.to_owned()])?;
    FileCapability::open(path, false)?
        .read_existing(max_bytes)?
        .map(|(bytes, _)| bytes)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("file does not exist: {}", path.display()),
            )
        })
}

#[derive(Clone, Debug)]
pub(crate) struct FileCapability {
    inner: Arc<FileCapabilityInner>,
}

#[derive(Debug)]
struct FileCapabilityInner {
    // A diagnostic/transaction identity only. Never use this for ambient I/O.
    path: PathBuf,
    leaf: OsString,
    create_parents: bool,
    parent: Mutex<ParentState>,
}

#[derive(Debug)]
struct ParentState {
    directory: Arc<Dir>,
    remaining: VecDeque<OsString>,
}

impl FileCapability {
    /// Pin every existing parent without following symbolic links. Missing
    /// parents are remembered and are created only during publication.
    pub(crate) fn open(path: &Path, create_parents: bool) -> io::Result<Self> {
        let (root, mut names) = split_canonical_path(path)?;
        let leaf = names.pop_back().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "file target has no file name")
        })?;
        let root = Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
        let capability = Self {
            inner: Arc::new(FileCapabilityInner {
                path: path.to_owned(),
                leaf,
                create_parents,
                parent: Mutex::new(ParentState {
                    directory: Arc::new(root),
                    remaining: names,
                }),
            }),
        };
        capability.parent(false)?;
        Ok(capability)
    }

    /// Return a bounded snapshot from an opened regular-file handle.
    pub(crate) fn read_existing(
        &self,
        max_bytes: usize,
    ) -> io::Result<Option<(Vec<u8>, Permissions)>> {
        let Some(parent) = self.parent(false)? else {
            return Ok(None);
        };
        self.read_from(&parent, max_bytes)
    }

    pub(crate) fn verify(&self, expected: Option<&[u8]>) -> io::Result<()> {
        self.verify_with_permissions(expected).map(|_| ())
    }

    /// Publish one revision relative to its pinned parent. Success means the
    /// target mutation completed; temporary-name cleanup cannot turn a completed
    /// mutation into an error that would be missed by transaction rollback.
    pub(crate) fn publish(
        &self,
        expected: Option<&[u8]>,
        updated: Option<&[u8]>,
        permissions: Option<Permissions>,
    ) -> io::Result<()> {
        let original_permissions = self.verify_with_permissions(expected)?;
        if expected == updated {
            return Ok(());
        }
        let Some(updated) = updated else {
            let parent = self.parent(false)?.ok_or_else(|| self.changed())?;
            // remove_file unlinks the directory entry, never a symlink target.
            return parent.remove_file(&self.inner.leaf);
        };
        if original_permissions
            .as_ref()
            .is_some_and(Permissions::readonly)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "sandbox replacement target is read-only: {}",
                    self.inner.path.display()
                ),
            ));
        }

        let parent = self.parent(true)?.ok_or_else(|| self.missing_parent())?;
        let use_default_create_permissions = expected.is_none() && permissions.is_none();
        let (mut pending, mut file) =
            PendingFile::create(parent.clone(), use_default_create_permissions)?;
        file.write_all(updated)?;
        file.flush()?;
        if let Some(permissions) = permissions.or(original_permissions) {
            file.set_permissions(permissions)?;
        }
        file.sync_all()?;
        drop(file);

        // Recheck after writing the temporary file. The final rename/unlink is
        // still relative to the same parent even if its ambient path changed.
        self.verify(expected)?;
        if expected.is_some() {
            parent.rename(&pending.name, &parent, &self.inner.leaf)?;
            pending.active = false;
        } else {
            // A hard link publishes an add without replacing an entry created
            // by another writer between verification and publication.
            parent.hard_link(&pending.name, &parent, &self.inner.leaf)?;
            pending.cleanup_after_publish();
        }
        Ok(())
    }

    fn parent(&self, create: bool) -> io::Result<Option<Arc<Dir>>> {
        let mut state = self.inner.parent.lock();
        while let Some(component) = state.remaining.front().cloned() {
            let next = match state.directory.open_dir_nofollow(&component) {
                Ok(directory) => directory,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if !create {
                        return Ok(None);
                    }
                    if !self.inner.create_parents {
                        return Err(self.missing_parent());
                    }
                    match state.directory.create_dir(&component) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error),
                    }
                    // Another writer may have raced mkdir. The second open
                    // also refuses symlinks before retaining the new handle.
                    state.directory.open_dir_nofollow(&component)?
                }
                Err(error) => return Err(error),
            };
            state.directory = Arc::new(next);
            state.remaining.pop_front();
        }
        Ok(Some(state.directory.clone()))
    }

    fn read_from(
        &self,
        parent: &Dir,
        max_bytes: usize,
    ) -> io::Result<Option<(Vec<u8>, Permissions)>> {
        match parent.symlink_metadata(&self.inner.leaf) {
            Ok(metadata) if !metadata.is_file() => return Err(self.not_regular()),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No).nonblock(true);
        let mut file = match parent.open_with(&self.inner.leaf, &options) {
            Ok(file) => file.into_std(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let before = file.metadata()?;
        if !before.is_file() {
            return Err(self.not_regular());
        }
        if before.len() > max_bytes as u64 {
            return Err(self.too_large(max_bytes));
        }
        let mut bytes = Vec::with_capacity(before.len() as usize);
        Read::by_ref(&mut file)
            .take((max_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() > max_bytes {
            return Err(self.too_large(max_bytes));
        }
        let after = file.metadata()?;
        if before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || after.len() != bytes.len() as u64
        {
            return Err(self.changed());
        }
        Ok(Some((bytes, after.permissions())))
    }

    fn verify_with_permissions(&self, expected: Option<&[u8]>) -> io::Result<Option<Permissions>> {
        let Some(parent) = self.parent(false)? else {
            return if expected.is_none() {
                Ok(None)
            } else {
                Err(self.changed())
            };
        };
        if let Some(expected) = expected {
            return match self.read_from(&parent, expected.len())? {
                Some((bytes, permissions)) if bytes == expected => Ok(Some(permissions)),
                _ => Err(self.changed()),
            };
        }
        match parent.symlink_metadata(&self.inner.leaf) {
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "sandbox target appeared after staging: {}",
                    self.inner.path.display()
                ),
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn changed(&self) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "sandbox target changed after staging: {}",
                self.inner.path.display()
            ),
        )
    }

    fn not_regular(&self) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "sandbox target must be a regular file without a symbolic link: {}",
                self.inner.path.display()
            ),
        )
    }

    fn too_large(&self, max_bytes: usize) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "sandbox target exceeds its {max_bytes}-byte bound: {}",
                self.inner.path.display()
            ),
        )
    }

    fn missing_parent(&self) -> io::Error {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "sandbox target parent does not exist and parent creation is disabled: {}",
                self.inner.path.display()
            ),
        )
    }
}

/// Split only canonical lexical paths, preserving Windows drive/UNC prefixes.
/// Filesystem canonicalization belongs to authorization, before this capability
/// is opened; resolving an ambient path here would follow replaceable links.
fn split_canonical_path(path: &Path) -> io::Result<(PathBuf, VecDeque<OsString>)> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "sandbox file target must be an absolute canonical path: {}",
                path.display()
            ),
        )
    };
    if !path.is_absolute() {
        return Err(invalid());
    }
    let mut root = PathBuf::new();
    let mut reconstructed = PathBuf::new();
    let mut names = VecDeque::new();
    let mut saw_root = false;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) if !saw_root && names.is_empty() => {
                root.push(prefix.as_os_str());
                reconstructed.push(prefix.as_os_str());
            }
            Component::RootDir if !saw_root => {
                root.push(component.as_os_str());
                reconstructed.push(component.as_os_str());
                saw_root = true;
            }
            Component::Normal(name) if saw_root => {
                names.push_back(name.to_os_string());
                reconstructed.push(name);
            }
            _ => return Err(invalid()),
        }
    }
    if !saw_root || names.is_empty() || reconstructed.as_os_str() != path.as_os_str() {
        return Err(invalid());
    }
    Ok((root, names))
}

struct PendingFile {
    parent: Arc<Dir>,
    name: OsString,
    active: bool,
}

impl PendingFile {
    fn create(
        parent: Arc<Dir>,
        use_default_create_permissions: bool,
    ) -> io::Result<(Self, std::fs::File)> {
        let name = OsString::from(format!(".agena-write-{}.tmp", Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            // Match ordinary creates (0666 filtered by umask). Replacements
            // and moves stage privately until their retained mode is applied.
            options.mode(if use_default_create_permissions {
                0o666
            } else {
                0o600
            });
        }
        #[cfg(not(unix))]
        let _ = use_default_create_permissions;
        let file = parent.open_with(&name, &options)?.into_std();
        Ok((
            Self {
                parent,
                name,
                active: true,
            },
            file,
        ))
    }

    fn cleanup_after_publish(&mut self) {
        match self.parent.remove_file(&self.name) {
            Ok(()) => self.active = false,
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.active = false,
            Err(error) => tracing::warn!(
                %error,
                "sandbox target published; temporary link cleanup will be retried"
            ),
        }
    }
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        if self.active {
            if let Err(error) = self.parent.remove_file(&self.name) {
                if error.kind() != io::ErrorKind::NotFound {
                    tracing::warn!(%error, "could not remove sandbox publication temporary file");
                }
            }
        }
    }
}

/// Verify all snapshots before mutation, publish destinations before deletes,
/// and restore every completed mutation if a later publication fails.
pub(crate) fn commit(changes: Vec<(FileCapability, SandboxFileChange)>) -> Result<(), ToolError> {
    commit_with_file_limit(changes, MAX_PATCH_FILE_BYTES as usize)
}

/// Trusted callers may carry their operation's file bound through staging and
/// publication. The limit is never supplied by worker output or tool JSON;
/// ordinary filesystem/patch callers retain commit's 16 MiB default.
pub(crate) fn commit_with_file_limit(
    changes: Vec<(FileCapability, SandboxFileChange)>,
    max_file_bytes: usize,
) -> Result<(), ToolError> {
    if changes.len() > MAX_PATCH_OPERATIONS * 2 {
        return Err(ToolError::invalid_patch(format!(
            "sandbox transaction exceeds {} file targets",
            MAX_PATCH_OPERATIONS * 2
        )));
    }
    let mut paths = BTreeSet::new();
    let mut transaction_bytes = 0_usize;
    let mut rollback_permissions = Vec::with_capacity(changes.len());
    for (capability, change) in &changes {
        if capability.inner.path.as_os_str() != change.path.as_os_str() {
            return Err(ToolError::invalid_patch(
                "sandbox target does not match its retained file capability",
            ));
        }
        if !paths.insert(change.path.clone()) {
            return Err(ToolError::invalid_patch(format!(
                "sandbox transaction repeats a file target: {}",
                change.path.display()
            )));
        }
        for bytes in [&change.original, &change.updated].into_iter().flatten() {
            if bytes.len() > max_file_bytes {
                return Err(ToolError::invalid_patch(format!(
                    "sandbox target exceeds the {} MiB limit: {}",
                    max_file_bytes / 1024 / 1024,
                    change.path.display()
                )));
            }
            transaction_bytes = transaction_bytes.saturating_add(bytes.len());
        }
        if transaction_bytes > MAX_PATCH_TRANSACTION_BYTES {
            return Err(ToolError::invalid_patch(format!(
                "sandbox transaction exceeds the {} MiB limit",
                MAX_PATCH_TRANSACTION_BYTES / 1024 / 1024
            )));
        }
        rollback_permissions.push(capability.verify_with_permissions(change.original.as_deref())?);
    }

    let order = (0..changes.len())
        .filter(|&index| changes[index].1.updated.is_some())
        .chain((0..changes.len()).filter(|&index| changes[index].1.updated.is_none()));
    let mut completed: Vec<usize> = Vec::new();
    for index in order {
        let (capability, change) = &changes[index];
        if change.original == change.updated {
            continue;
        }
        if let Err(error) = capability.publish(
            change.original.as_deref(),
            change.updated.as_deref(),
            change.permissions.clone(),
        ) {
            let mut failures = Vec::new();
            for &completed_index in completed.iter().rev() {
                let (capability, change) = &changes[completed_index];
                if let Err(rollback_error) = capability.publish(
                    change.updated.as_deref(),
                    change.original.as_deref(),
                    rollback_permissions[completed_index].clone(),
                ) {
                    failures.push(format!("{}: {rollback_error}", change.path.display()));
                }
            }
            let mut message = format!(
                "sandbox publication failed for {}: {error}",
                change.path.display()
            );
            if !failures.is_empty() {
                message.push_str("; rollback failed for ");
                message.push_str(&failures.join("; "));
            }
            return Err(ToolError::Io(io::Error::new(error.kind(), message)));
        }
        completed.push(index);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn canonical(path: &Path) -> PathBuf {
        // The test fixture root exists; its descendant may intentionally not.
        let root = path.parent().unwrap();
        if root.exists() {
            std::fs::canonicalize(root)
                .unwrap()
                .join(path.file_name().unwrap())
        } else {
            path.to_owned()
        }
    }

    fn change(path: &Path, original: Option<&[u8]>, updated: Option<&[u8]>) -> SandboxFileChange {
        SandboxFileChange {
            path: path.to_owned(),
            original: original.map(Vec::from),
            updated: updated.map(Vec::from),
            permissions: None,
        }
    }

    #[test]
    fn fs_broker_rejects_noncanonical_targets() {
        assert!(FileCapability::open(Path::new("relative/file"), true).is_err());
        let directory = tempdir();
        assert!(FileCapability::open(&directory.path().join("../file"), true).is_err());
        assert!(FileCapability::open(&directory.path().join("./file"), true).is_err());
        assert!(FileCapability::open(directory.path().ancestors().last().unwrap(), true).is_err());
    }

    #[test]
    fn fs_broker_reads_are_bounded_and_new_files_do_not_clobber() {
        let directory = tempdir();
        let path = canonical(&directory.path().join("file"));
        let capability = FileCapability::open(&path, false).unwrap();
        assert!(capability.read_existing(8).unwrap().is_none());
        assert_eq!(
            read_regular_file_nofollow(&path, 8).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        std::fs::write(&path, b"external").unwrap();
        assert!(capability.publish(None, Some(b"ours"), None).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"external");
        assert!(capability.read_existing(7).is_err());
        assert_eq!(capability.read_existing(8).unwrap().unwrap().0, b"external");
        assert_eq!(read_regular_file_nofollow(&path, 8).unwrap(), b"external");
    }

    #[test]
    fn fs_broker_missing_parents_are_created_only_at_publication() {
        let directory = tempdir();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let path = root.join("missing/deep/file");
        let capability = FileCapability::open(&path, true).unwrap();
        assert!(!root.join("missing").exists());
        assert!(capability.read_existing(4).unwrap().is_none());
        capability.verify(None).unwrap();
        assert!(!root.join("missing").exists());
        capability.publish(None, Some(b"data"), None).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"data");
    }

    #[test]
    fn fs_broker_refuses_replacing_readonly_files() {
        let directory = tempdir();
        let path = canonical(&directory.path().join("readonly"));
        std::fs::write(&path, b"before").unwrap();
        let writable = std::fs::metadata(&path).unwrap().permissions();
        let mut readonly = writable.clone();
        readonly.set_readonly(true);
        std::fs::set_permissions(&path, readonly).unwrap();
        let capability = FileCapability::open(&path, false).unwrap();
        let error = capability
            .publish(Some(b"before"), Some(b"after"), None)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read(&path).unwrap(), b"before");
        capability
            .publish(Some(b"before"), Some(b"before"), None)
            .unwrap();
        std::fs::set_permissions(&path, writable).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn fs_broker_creates_files_with_ordinary_umask_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempdir();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let ordinary = root.join("ordinary");
        std::fs::write(&ordinary, b"data").unwrap();
        let target = root.join("target");
        FileCapability::open(&target, false)
            .unwrap()
            .publish(None, Some(b"data"), None)
            .unwrap();
        let mode = |path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&target), mode(&ordinary));
    }

    #[test]
    fn fs_broker_validates_all_files_before_first_mutation() {
        let directory = tempdir();
        let first = canonical(&directory.path().join("first"));
        let stale = canonical(&directory.path().join("stale"));
        std::fs::write(&first, b"before").unwrap();
        std::fs::write(&stale, b"external").unwrap();
        let result = commit(vec![
            (
                FileCapability::open(&first, false).unwrap(),
                change(&first, Some(b"before"), Some(b"after")),
            ),
            (
                FileCapability::open(&stale, false).unwrap(),
                change(&stale, Some(b"old"), Some(b"after")),
            ),
        ]);
        assert!(result.is_err());
        assert_eq!(std::fs::read(first).unwrap(), b"before");
        assert_eq!(std::fs::read(stale).unwrap(), b"external");
    }

    #[test]
    fn fs_broker_default_limit_stays_16_mib_for_notebook_filenames() {
        let directory = tempdir();
        let path = canonical(&directory.path().join("notebook.ipynb"));
        std::fs::write(&path, b"before").unwrap();
        let error = commit(vec![(
            FileCapability::open(&path, false).unwrap(),
            SandboxFileChange {
                path: path.clone(),
                original: Some(b"before".to_vec()),
                updated: Some(vec![b'x'; MAX_PATCH_FILE_BYTES as usize + 1]),
                permissions: None,
            },
        )])
        .unwrap_err();
        assert!(error.to_string().contains("16 MiB limit"));
        assert_eq!(std::fs::read(path).unwrap(), b"before");
    }

    #[test]
    fn fs_broker_notebook_limit_accepts_32_mib_and_preserves_revision_checks() {
        let maximum = 32 * 1024 * 1024;
        let directory = tempdir();
        let path = canonical(&directory.path().join("notebook.ipynb"));
        let original = vec![b'a'; maximum];
        std::fs::write(&path, &original).unwrap();
        let mut updated = original.clone();
        updated[maximum - 1] = b'b';
        commit_with_file_limit(
            vec![(
                FileCapability::open(&path, false).unwrap(),
                SandboxFileChange {
                    path: path.clone(),
                    original: Some(original),
                    updated: Some(updated),
                    permissions: None,
                },
            )],
            maximum,
        )
        .unwrap();
        let published = std::fs::read(&path).unwrap();
        assert_eq!(published.len(), maximum);
        assert!(published[..maximum - 1].iter().all(|byte| *byte == b'a'));
        assert_eq!(published[maximum - 1], b'b');

        assert!(
            commit_with_file_limit(
                vec![(
                    FileCapability::open(&path, false).unwrap(),
                    change(&path, Some(b"stale revision"), Some(b"must not replace")),
                )],
                maximum,
            )
            .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), published);
    }

    #[test]
    fn fs_broker_notebook_limit_rejects_oversized_originals_and_outputs() {
        let maximum = 32 * 1024 * 1024;
        let directory = tempdir();
        let path = canonical(&directory.path().join("notebook.ipynb"));
        std::fs::write(&path, b"before").unwrap();
        for oversized_original in [true, false] {
            let change = SandboxFileChange {
                path: path.clone(),
                original: Some(if oversized_original {
                    vec![b'x'; maximum + 1]
                } else {
                    b"before".to_vec()
                }),
                updated: Some(if oversized_original {
                    b"after".to_vec()
                } else {
                    vec![b'x'; maximum + 1]
                }),
                permissions: None,
            };
            let error = commit_with_file_limit(
                vec![(FileCapability::open(&path, false).unwrap(), change)],
                maximum,
            )
            .unwrap_err();
            assert!(error.to_string().contains("32 MiB limit"));
            assert_eq!(std::fs::read(&path).unwrap(), b"before");
        }
    }

    #[test]
    fn fs_broker_rolls_back_updates_and_adds_before_deleting_sources() {
        let directory = tempdir();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let first = root.join("first");
        let added = root.join("added");
        let missing = root.join("missing/file");
        let deleted = root.join("source");
        std::fs::write(&first, [0xff, 0x00, 0x01]).unwrap();
        std::fs::write(&deleted, b"source").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&first, Permissions::from_mode(0o751)).unwrap();
        }
        let result = commit(vec![
            (
                FileCapability::open(&deleted, false).unwrap(),
                change(&deleted, Some(b"source"), None),
            ),
            (
                FileCapability::open(&first, false).unwrap(),
                change(&first, Some(&[0xff, 0x00, 0x01]), Some(b"updated")),
            ),
            (
                FileCapability::open(&added, false).unwrap(),
                change(&added, None, Some(b"new")),
            ),
            (
                FileCapability::open(&missing, false).unwrap(),
                change(&missing, None, Some(b"fails")),
            ),
        ]);
        assert!(result.is_err());
        assert_eq!(std::fs::read(&first).unwrap(), [0xff, 0x00, 0x01]);
        assert!(!added.exists());
        assert!(!root.join("missing").exists());
        assert_eq!(std::fs::read(deleted).unwrap(), b"source");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(first).unwrap().permissions().mode() & 0o777,
                0o751
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn fs_broker_ancestor_exchange_keeps_reads_and_writes_on_pinned_directory() {
        use std::os::unix::fs::symlink;
        let directory = tempdir();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let allowed = root.join("allowed");
        let outside = root.join("outside");
        std::fs::create_dir(&allowed).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let target = allowed.join("file");
        std::fs::write(&target, b"before").unwrap();
        std::fs::write(outside.join("file"), b"secret").unwrap();
        let capability = FileCapability::open(&target, false).unwrap();
        assert_eq!(capability.read_existing(16).unwrap().unwrap().0, b"before");

        let retained = root.join("retained");
        std::fs::rename(&allowed, &retained).unwrap();
        symlink(&outside, &allowed).unwrap();

        assert_eq!(capability.read_existing(16).unwrap().unwrap().0, b"before");
        capability
            .publish(Some(b"before"), Some(b"after"), None)
            .unwrap();
        assert_eq!(std::fs::read(retained.join("file")).unwrap(), b"after");
        assert_eq!(std::fs::read(outside.join("file")).unwrap(), b"secret");
    }

    #[cfg(unix)]
    #[test]
    fn fs_broker_rejects_existing_ancestor_and_leaf_symlinks() {
        use std::os::unix::fs::symlink;
        let directory = tempdir();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let real = root.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("file"), b"secret").unwrap();
        symlink(&real, root.join("linked")).unwrap();
        assert!(FileCapability::open(&root.join("linked/file"), false).is_err());
        assert!(read_regular_file_nofollow(&root.join("linked/file"), 16).is_err());

        let target = root.join("target");
        std::fs::write(&target, b"before").unwrap();
        let capability = FileCapability::open(&target, false).unwrap();
        std::fs::remove_file(&target).unwrap();
        symlink(real.join("file"), &target).unwrap();
        assert!(capability.read_existing(16).is_err());
        assert!(read_regular_file_nofollow(&target, 16).is_err());
        assert!(capability.verify(Some(b"before")).is_err());
        assert!(
            capability
                .publish(Some(b"before"), Some(b"after"), None)
                .is_err()
        );
        assert!(capability.publish(Some(b"before"), None, None).is_err());
        assert_eq!(std::fs::read(real.join("file")).unwrap(), b"secret");
    }

    #[cfg(unix)]
    #[test]
    fn fs_broker_creates_missing_parents_below_retained_handle_after_exchange() {
        use std::os::unix::fs::symlink;
        let directory = tempdir();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let allowed = root.join("allowed");
        let outside = root.join("outside");
        std::fs::create_dir(&allowed).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let capability = FileCapability::open(&allowed.join("missing/deep/file"), true).unwrap();
        let retained = root.join("retained");
        std::fs::rename(&allowed, &retained).unwrap();
        symlink(&outside, &allowed).unwrap();

        capability.publish(None, Some(b"new"), None).unwrap();
        assert_eq!(
            std::fs::read(retained.join("missing/deep/file")).unwrap(),
            b"new"
        );
        assert!(!outside.join("missing").exists());
    }

    #[cfg(unix)]
    #[test]
    fn fs_broker_refuses_a_symlink_inserted_into_a_missing_parent_chain() {
        use std::os::unix::fs::symlink;
        let directory = tempdir();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("file"), b"secret").unwrap();
        let missing = root.join("missing");
        let capability = FileCapability::open(&missing.join("file"), true).unwrap();
        assert!(capability.read_existing(16).unwrap().is_none());
        symlink(&outside, &missing).unwrap();
        assert!(capability.read_existing(16).is_err());
        assert!(capability.publish(None, Some(b"overwrite"), None).is_err());
        assert_eq!(std::fs::read(outside.join("file")).unwrap(), b"secret");
    }
}
