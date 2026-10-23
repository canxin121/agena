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
        let root = root.into();
        Self {
            // Do not canonicalize arbitrary data-root symlinks here: doing so
            // would hide them from the private-storage checks at preparation.
            // This constructor is infallible, so retain an invalid path for
            // prepare() to reject instead of silently changing its identity.
            root: resolve_private_path(&root).unwrap_or_else(|_| {
                if root.as_os_str().is_empty() {
                    root
                } else {
                    absolute_unresolved_path(&root).unwrap_or(root)
                }
            }),
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
        let layout = Self {
            root: resolve_private_path(&root.into())?,
        };
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

/// Resolve a private storage path while rejecting user-created symlinks in
/// every existing component. Missing suffixes are kept for later creation.
///
/// macOS ships root-owned /var, /tmp and /etc symlinks into /private. Only
/// these exact aliases, with their fixed targets and protected system parents,
/// may be resolved. A user alias beneath them is still rejected. Callers must
/// perform I/O on the returned path rather than canonicalizing the input.
pub fn resolve_private_path(path: &Path) -> io::Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(unsafe_private_path());
    }
    let absolute = absolute_unresolved_path(path)?;
    let components = absolute.components().collect::<Vec<_>>();
    let mut resolved = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::Prefix(prefix) => {
                resolved.push(prefix.as_os_str());
                continue;
            }
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => continue,
            Component::ParentDir => {
                // Earlier components have already been checked. In particular,
                // an alias cannot be hidden by subsequently spelling /../.
                resolved.pop();
                continue;
            }
            Component::Normal(name) => resolved.push(name),
        }
        match std::fs::symlink_metadata(&resolved) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if let Some(target) = trusted_system_alias(&resolved, &metadata)? {
                    resolved = target;
                } else {
                    return Err(unsafe_private_path());
                }
            }
            Ok(metadata) if index + 1 < components.len() && !metadata.is_dir() => {
                return Err(unsafe_private_path());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(resolved)
}

fn absolute_unresolved_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn unsafe_private_path() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "private storage has an unsafe ancestor",
    )
}

#[cfg(target_os = "macos")]
fn trusted_system_alias(path: &Path, metadata: &std::fs::Metadata) -> io::Result<Option<PathBuf>> {
    use std::os::unix::fs::MetadataExt;

    let (relative_target, target) = match path.to_str() {
        Some("/var") => ("private/var", Path::new("/private/var")),
        Some("/tmp") => ("private/tmp", Path::new("/private/tmp")),
        Some("/etc") => ("private/etc", Path::new("/private/etc")),
        _ => return Ok(None),
    };
    if metadata.uid() != 0 {
        return Err(unsafe_private_path());
    }
    // The user must not be able to replace either the root alias or its
    // /private destination. Symlink permissions themselves are not meaningful.
    for parent in [Path::new("/"), Path::new("/private")] {
        let parent_metadata = std::fs::symlink_metadata(parent)?;
        if !parent_metadata.is_dir()
            || parent_metadata.file_type().is_symlink()
            || parent_metadata.uid() != 0
            || parent_metadata.mode() & 0o022 != 0
        {
            return Err(unsafe_private_path());
        }
    }
    let actual_target = std::fs::read_link(path)?;
    if actual_target != target && actual_target != Path::new(relative_target) {
        return Err(unsafe_private_path());
    }
    let target_metadata = std::fs::symlink_metadata(target)?;
    if !target_metadata.is_dir()
        || target_metadata.file_type().is_symlink()
        || target_metadata.uid() != 0
        || (target == Path::new("/private/tmp")
            && target_metadata.mode() & 0o022 != 0
            && target_metadata.mode() & 0o1000 == 0)
        || (target != Path::new("/private/tmp") && target_metadata.mode() & 0o022 != 0)
    {
        return Err(unsafe_private_path());
    }
    Ok(Some(target.to_path_buf()))
}

#[cfg(not(target_os = "macos"))]
fn trusted_system_alias(
    _path: &Path,
    _metadata: &std::fs::Metadata,
) -> io::Result<Option<PathBuf>> {
    Ok(None)
}

/// Create an application-owned directory without following user symlinks,
/// then restrict it to the current account.
pub fn secure_directory(path: &Path) -> io::Result<()> {
    let path = resolve_private_path(path)?;
    if path.parent().is_none() || is_system_storage_anchor(&path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a system directory cannot be a private storage directory",
        ));
    }
    let mut missing = Vec::new();
    let mut existing = path.as_path();
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
    restrict(&path, true)
}

pub fn secure_file(path: &Path) -> io::Result<()> {
    let path = resolve_private_path(path)?;
    let metadata = std::fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "private storage file is not a regular file",
        ));
    }
    restrict(&path, false)
}

#[cfg(target_os = "macos")]
fn is_system_storage_anchor(path: &Path) -> bool {
    [
        "/var",
        "/tmp",
        "/etc",
        "/private",
        "/private/var",
        "/private/tmp",
        "/private/etc",
    ]
    .iter()
    .any(|anchor| path == Path::new(anchor))
}

#[cfg(not(target_os = "macos"))]
fn is_system_storage_anchor(_path: &Path) -> bool {
    false
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

    #[cfg(unix)]
    #[test]
    fn private_paths_do_not_hide_user_symlinks_in_layouts_or_parent_components() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("target");
        std::fs::create_dir_all(target.join("existing")).unwrap();
        let alias = fixture.path().join("alias");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        let private_file = target.join("existing/key");
        std::fs::write(&private_file, "unchanged").unwrap();

        assert!(StorageLayout::new(&alias).prepare().is_err());
        assert!(resolve_private_path(&alias.join("../new-storage")).is_err());
        assert!(secure_file(&alias.join("existing/key")).is_err());
        assert_eq!(std::fs::read_to_string(private_file).unwrap(), "unchanged");
        assert!(!fixture.path().join("new-storage").exists());
    }

    #[test]
    fn an_empty_private_root_cannot_select_the_current_working_directory() {
        assert!(resolve_private_path(Path::new("")).is_err());
        assert!(StorageLayout::new("").prepare().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_paths_reject_user_aliases_even_when_they_target_system_directories() {
        let fixture = tempfile::tempdir().unwrap();
        for name in ["tmp", "var", "etc"] {
            let alias = fixture.path().join(name);
            std::os::unix::fs::symlink(Path::new("/").join(name), &alias).unwrap();
            assert!(resolve_private_path(&alias.join("agena-private")).is_err());
            assert!(secure_directory(&alias.join("agena-private")).is_err());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_temp_aliases_allow_private_descendants_but_not_user_links() {
        use std::os::unix::fs::PermissionsExt;

        for base in ["/tmp", "/var/tmp"] {
            let fixture = tempfile::tempdir_in(base).unwrap();
            let directory = fixture.path().join("nested/private");
            secure_directory(&directory).unwrap();
            let resolved = resolve_private_path(&directory).unwrap();
            assert_eq!(resolved, directory.canonicalize().unwrap());
            assert!(resolved.starts_with("/private"));
            assert_eq!(
                std::fs::metadata(&resolved).unwrap().permissions().mode() & 0o777,
                0o700
            );

            let file = directory.join("key");
            std::fs::write(&file, "private-value").unwrap();
            secure_file(&file).unwrap();
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );

            let alias = fixture.path().join("user-alias");
            std::os::unix::fs::symlink(&directory, &alias).unwrap();
            assert!(secure_directory(&alias.join("new")).is_err());
            assert!(secure_file(&alias.join("key")).is_err());
        }
        assert_eq!(
            resolve_private_path(Path::new("/etc/agena-nonexistent-private-entry")).unwrap(),
            PathBuf::from("/private/etc/agena-nonexistent-private-entry")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_aliases_and_their_targets_never_become_private_storage_roots() {
        use std::os::unix::fs::PermissionsExt;

        for path in [
            "/tmp",
            "/var",
            "/etc",
            "/private",
            "/private/tmp",
            "/private/var",
            "/private/etc",
        ] {
            let path = Path::new(path);
            let before = std::fs::metadata(path).unwrap().permissions().mode();
            assert!(secure_directory(path).is_err());
            assert!(StorageLayout::new(path).prepare().is_err());
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode(),
                before
            );
        }
    }
}
