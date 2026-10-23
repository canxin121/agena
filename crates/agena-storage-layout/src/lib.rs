//! One location policy for an Agena service instance and its local clients.
//!
//! Process entrypoints select the layout once before composing services.
//! Infrastructure receives explicit paths, and embedders can construct an
//! independent layout without changing process environment variables.

use std::{
    io,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

static PROCESS_LAYOUT: OnceLock<StorageLayout> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageLayout {
    root: PathBuf,
}

impl StorageLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: absolute_path(&root.into()),
        }
    }

    /// The sole environment override for application-managed local storage.
    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var_os(name))
    }

    /// Resolve the same location policy against an injected environment.
    pub fn from_lookup(mut lookup: impl FnMut(&str) -> Option<std::ffi::OsString>) -> Self {
        let root = lookup("AGENA_DATA_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                lookup("HOME")
                    .or_else(|| lookup("USERPROFILE"))
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("agena")
            });
        Self::new(root)
    }

    pub fn current() -> Self {
        PROCESS_LAYOUT.get().cloned().unwrap_or_else(Self::from_env)
    }

    /// Bind one immutable root to a running process. Repeated composition of
    /// the same instance is harmless; selecting another instance is an error.
    pub fn initialize_process(root: impl Into<PathBuf>) -> io::Result<Self> {
        let layout = Self::new(root);
        let installed = PROCESS_LAYOUT.get_or_init(|| layout.clone());
        if installed != &layout {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "this process already owns a different Agena data directory",
            ));
        }
        Ok(installed.clone())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn database_path(&self) -> PathBuf {
        self.root.join("agena.db")
    }
    pub fn config_path(&self) -> PathBuf {
        self.root.join("agena.json")
    }
    pub fn secrets_dir(&self) -> PathBuf {
        self.root.join("secrets")
    }
    pub fn workspace_dir(&self, workspace: &Path) -> PathBuf {
        self.root.join("workspaces").join(workspace_key(workspace))
    }
    pub fn memory_dir(&self, workspace: &Path) -> PathBuf {
        self.workspace_dir(workspace).join("memory")
    }
    pub fn memory_index_dir(&self, workspace: &Path) -> PathBuf {
        self.cache_dir()
            .join("memory")
            .join(workspace_key(workspace))
    }
    pub fn uploads_dir(&self, workspace: &Path) -> PathBuf {
        self.workspace_dir(workspace).join("uploads")
    }
    pub fn client_scope_dir(&self, scope: &str) -> PathBuf {
        self.client_dir()
            .join(blake3::hash(scope.as_bytes()).to_hex().to_string())
    }
    pub fn artifacts_dir(&self, workspace: &Path) -> PathBuf {
        self.workspace_dir(workspace).join("artifacts")
    }
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }
    pub fn temp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }
    pub fn run_dir(&self) -> PathBuf {
        self.root.join("run")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn marketplace_dir(&self) -> PathBuf {
        self.root.join("marketplace")
    }
    pub fn plugin_storage_dir(&self) -> PathBuf {
        self.root.join("plugin-data")
    }
    pub fn client_dir(&self) -> PathBuf {
        self.root.join("clients")
    }
    pub fn terminal_ui_state_path(&self) -> PathBuf {
        self.root.join("terminal-ui.json")
    }

    pub fn prepare(&self) -> io::Result<()> {
        secure_directory(&self.root)
    }
}

/// A full deterministic digest avoids lossy path sanitization and truncated
/// hash collisions. Existing symlink aliases resolve to the same workspace.
pub fn workspace_key(workspace: &Path) -> String {
    let normalized = absolute_path(workspace);
    #[cfg(windows)]
    let bytes = normalized
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("//?/")
        .to_lowercase()
        .into_bytes();
    #[cfg(not(windows))]
    let bytes = normalized.as_os_str().as_encoded_bytes().to_vec();
    blake3::hash(&bytes).to_hex().to_string()
}

fn absolute_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    // Canonicalize the existing prefix as well (e.g. macOS /var), even when
    // the selected instance directory has not been created yet.
    let mut suffix = Vec::new();
    let mut ancestor = normalized.as_path();
    while let Some(parent) = ancestor.parent() {
        if let Ok(mut canonical) = ancestor.canonicalize() {
            for component in suffix.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }
        if let Some(name) = ancestor.file_name() {
            suffix.push(name.to_os_string());
        }
        ancestor = parent;
    }
    normalized
}

/// Create an application-owned directory without following a symlink at the
/// final component, then restrict it to the current account.
pub fn secure_directory(path: &Path) -> io::Result<()> {
    if path.parent().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a filesystem root cannot be a private storage directory",
        ));
    }
    // Validate every existing component, including parents of an already
    // existing leaf. Do not silently follow a replaced managed directory.
    for ancestor in path
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
    {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private storage has an unsafe ancestor",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let mut missing = Vec::new();
    let mut existing = path;
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private storage directory is not a regular directory",
                ));
            }
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(existing.to_path_buf());
                existing = existing.parent().ok_or_else(|| {
                    io::Error::other("private directory has no existing ancestor")
                })?;
            }
            Err(error) => return Err(error),
        }
    }
    for directory in missing.into_iter().rev() {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = std::fs::symlink_metadata(&directory)?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(io::Error::other(
                        "private directory was replaced during creation",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
        restrict(&directory, true)?;
    }
    restrict(path, true)
}

pub fn secure_file(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "private storage file is not a regular file",
        ));
    }
    restrict(path, false)
}

#[cfg(unix)]
fn restrict(path: &Path, directory: bool) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        path,
        std::fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
    )
}

#[cfg(windows)]
fn restrict(path: &Path, directory: bool) -> io::Result<()> {
    windows_acl::restrict(path, directory)
}

#[cfg(not(any(unix, windows)))]
fn restrict(_path: &Path, _directory: bool) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
mod windows_acl;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_managed_location_stays_inside_the_selected_root() {
        let fixture = tempfile::tempdir().unwrap();
        let layout = StorageLayout::new(fixture.path().join("instance"));
        for path in [
            layout.database_path(),
            layout.config_path(),
            layout.secrets_dir(),
            layout.memory_dir(Path::new("/project")),
            layout.memory_index_dir(Path::new("/project")),
            layout.uploads_dir(Path::new("/project")),
            layout.artifacts_dir(Path::new("/project")),
            layout.marketplace_dir(),
            layout.plugin_storage_dir(),
            layout.client_dir(),
            layout.terminal_ui_state_path(),
        ] {
            assert!(path.starts_with(layout.root()));
        }
    }

    #[test]
    fn workspace_keys_do_not_collapse_distinct_paths() {
        assert_ne!(
            workspace_key(Path::new("/tmp/a/b")),
            workspace_key(Path::new("/tmp/a-b"))
        );
        assert_eq!(
            workspace_key(Path::new("/tmp/a/../b")),
            workspace_key(Path::new("/tmp/b"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn workspace_keys_preserve_non_unicode_path_identity() {
        use std::os::unix::ffi::OsStrExt;
        let first = Path::new(std::ffi::OsStr::from_bytes(b"/missing-workspace/\xff"));
        let second = Path::new(std::ffi::OsStr::from_bytes(b"/missing-workspace/\xfe"));
        assert_ne!(workspace_key(first), workspace_key(second));
    }

    #[cfg(unix)]
    #[test]
    fn private_directory_rejects_symlink_parents_of_existing_leaves() {
        let fixture = tempfile::tempdir().unwrap();
        let actual = fixture.path().join("actual");
        std::fs::create_dir_all(actual.join("existing")).unwrap();
        let alias = fixture.path().join("alias");
        std::os::unix::fs::symlink(&actual, &alias).unwrap();
        assert!(secure_directory(&alias.join("existing")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_directory_rejects_symlinks() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let link = fixture.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(secure_directory(&link).is_err());
    }
}
