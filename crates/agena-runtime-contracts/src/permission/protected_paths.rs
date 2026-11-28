//! Host-owned paths that retain their protections when a surrounding directory is granted.

use agena_domain::{AccessKind, PermissionDecision};
use agena_storage_layout::{StorageLayout, project_config_path};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use crate::authorization::canonical_resource_path;

use super::{PermissionPolicy, normalize_path_string, path_identity};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProtectedPaths {
    private_paths: Vec<PathBuf>,
    workspaces: PathBuf,
    own_state: Option<PathBuf>,
    readonly_paths: Vec<PathBuf>,
    git_directories: Vec<PathBuf>,
}

impl ProtectedPaths {
    pub(crate) fn for_workspace(
        workspace: &Path,
        layout: &StorageLayout,
        base: Option<&PermissionPolicy>,
    ) -> Self {
        let database = layout.database_path();
        let mut private_paths = vec![database.clone(), layout.config_path(), layout.secrets_dir()];
        for suffix in ["-journal", "-shm", "-wal"] {
            let mut sidecar = OsString::from(database.as_os_str());
            sidecar.push(suffix);
            private_paths.push(PathBuf::from(sidecar));
        }
        let mut readonly_paths = Vec::new();
        let own_state = workspace
            .is_absolute()
            .then(|| layout.workspace_dir(workspace));
        let mut git_paths = Vec::new();
        let mut git_directories = Vec::new();
        if own_state.is_some() {
            readonly_paths.push(project_config_path(workspace));
            for root in std::iter::once(workspace).chain(
                base.into_iter()
                    .flat_map(|policy| &policy.additional_working_directories)
                    .map(PathBuf::as_path),
            ) {
                let (paths, directories) = git_protections(root);
                git_paths.extend(paths);
                git_directories.extend(directories);
            }
        }
        for path in git_paths {
            let aliases = resolved_spellings(vec![path]);
            if !base.is_some_and(|base| {
                aliases
                    .iter()
                    .any(|path| base.has_explicit_write_grant(workspace, path))
            }) {
                readonly_paths.extend(aliases);
            }
        }
        Self {
            private_paths: resolved_spellings(private_paths),
            workspaces: normalized(&layout.workspaces_dir()),
            own_state: own_state.as_deref().map(normalized),
            readonly_paths: resolved_spellings(readonly_paths),
            git_directories: git_directories
                .iter()
                .filter_map(|path| canonical_resource_path(path).ok())
                .map(|path| normalized(&path))
                .collect(),
        }
    }

    pub(crate) fn check(&self, access: AccessKind, target: &Path) -> Option<PermissionDecision> {
        self.denial_reason(access, &normalized(target))
            .or_else(|| {
                let resolved = canonical_resource_path(target).ok()?;
                self.denial_reason(access, &normalized(&resolved))
            })
            .map(|reason| PermissionDecision::Deny {
                reason: reason.into(),
            })
    }

    fn denial_reason(&self, access: AccessKind, target: &Path) -> Option<&'static str> {
        if self
            .private_paths
            .iter()
            .any(|path| path_identity::within(target, path))
            || (path_identity::within(target, &self.workspaces)
                && self
                    .own_state
                    .as_ref()
                    .is_none_or(|path| !path_identity::within(target, path)))
        {
            return Some("Agena instance storage is private to the host or another workspace");
        }
        if access == AccessKind::Write
            && self
                .readonly_paths
                .iter()
                .any(|path| path_identity::within(target, path))
        {
            return Some("Agena project settings and Git configuration are read-only to the agent");
        }
        None
    }

    pub(super) fn private_roots(&self) -> impl Iterator<Item = &PathBuf> {
        self.private_paths
            .iter()
            .chain((!self.workspaces.as_os_str().is_empty()).then_some(&self.workspaces))
    }

    pub(super) fn own_state(&self) -> Option<&Path> {
        self.own_state.as_deref()
    }

    pub(super) fn native_readonly_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.readonly_paths.iter().filter(|path| {
            path.exists()
                && canonical_resource_path(path)
                    .is_ok_and(|resolved| path_identity::equal(path, &normalized(&resolved)))
        })
    }

    // Binding an existing Git directory separately pins its directory entry on
    // mount-based backends while still allowing index and reference updates.
    pub(super) fn git_directories(&self) -> impl Iterator<Item = &PathBuf> {
        self.git_directories.iter()
    }
}

fn git_protections(root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let Some(marker) = root
        .ancestors()
        .map(|directory| directory.join(".git"))
        .find(|marker| fs::symlink_metadata(marker).is_ok())
    else {
        return (Vec::new(), Vec::new());
    };
    let (mut paths, mut directories) = if marker.is_dir() {
        (Vec::new(), vec![marker])
    } else {
        let gitdir = read_git_path(&marker, "gitdir:");
        (vec![marker], gitdir.into_iter().collect())
    };
    if let Some(directory) = directories.first()
        && let Some(common) = read_git_path(&directory.join("commondir"), "")
    {
        directories.push(common);
    }
    for directory in &directories {
        paths.extend(["config", "config.worktree", "hooks"].map(|name| directory.join(name)));
    }
    (paths, directories)
}

fn normalized(path: &Path) -> PathBuf {
    PathBuf::from(normalize_path_string(path))
}

fn resolved_spellings(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for path in paths.into_iter().filter(|path| path.is_absolute()) {
        if let Ok(resolved) = canonical_resource_path(&path) {
            result.push(normalized(&resolved));
        }
        result.push(normalized(&path));
    }
    result.sort();
    result.dedup();
    result
}

fn read_git_path(file: &Path, prefix: &str) -> Option<PathBuf> {
    const MAX_PATH_FILE: u64 = 16 * 1024;
    if !file.is_file() {
        return None;
    }
    let mut text = String::new();
    File::open(file)
        .ok()?
        .take(MAX_PATH_FILE + 1)
        .read_to_string(&mut text)
        .ok()?;
    let value = text.strip_prefix(prefix)?.trim();
    if text.len() as u64 > MAX_PATH_FILE || value.is_empty() || value.contains(['\n', '\r', '\0']) {
        return None;
    }
    let target = file.parent()?.join(value);
    target.canonicalize().ok().filter(|path| path.is_dir())
}
