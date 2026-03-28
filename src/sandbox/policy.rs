use std::path::{Path, PathBuf};

use super::cap_fs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxPolicy {
    DangerFullAccess,
    ReadOnly,
    WorkspaceWrite {
        writable_roots: Vec<PathBuf>,
        network_access: bool,
        exclude_tmpdir_env_var: bool,
        exclude_slash_tmp: bool,
        enforce_world_writable_audit: bool,
        reject_reparse_points: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritableRoot {
    pub root: PathBuf,
    pub read_only_subpaths: Vec<PathBuf>,
}

impl SandboxPolicy {
    pub fn new_read_only_policy() -> Self {
        Self::ReadOnly
    }

    pub fn new_workspace_write_policy() -> Self {
        Self::WorkspaceWrite {
            writable_roots: Vec::new(),
            network_access: false,
            exclude_tmpdir_env_var: false,
            exclude_slash_tmp: false,
            enforce_world_writable_audit: true,
            reject_reparse_points: true,
        }
    }

    pub fn has_full_disk_write_access(&self) -> bool {
        matches!(self, Self::DangerFullAccess)
    }

    pub fn has_full_network_access(&self) -> bool {
        match self {
            Self::DangerFullAccess => true,
            Self::ReadOnly => false,
            Self::WorkspaceWrite { network_access, .. } => *network_access,
        }
    }

    pub fn enforce_world_writable_audit(&self) -> bool {
        match self {
            Self::WorkspaceWrite {
                enforce_world_writable_audit,
                ..
            } => *enforce_world_writable_audit,
            Self::ReadOnly => true,
            Self::DangerFullAccess => false,
        }
    }

    pub fn reject_reparse_points(&self) -> bool {
        match self {
            Self::WorkspaceWrite {
                reject_reparse_points,
                ..
            } => *reject_reparse_points,
            Self::ReadOnly => true,
            Self::DangerFullAccess => false,
        }
    }

    pub fn with_additional_writable_roots(
        mut self,
        roots: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        if let Self::WorkspaceWrite { writable_roots, .. } = &mut self {
            writable_roots.extend(roots);
        }
        self
    }

    pub fn with_network_access(mut self, enabled: bool) -> Self {
        if let Self::WorkspaceWrite { network_access, .. } = &mut self {
            *network_access = enabled;
        }
        self
    }

    pub fn with_world_writable_audit(mut self, enabled: bool) -> Self {
        if let Self::WorkspaceWrite {
            enforce_world_writable_audit,
            ..
        } = &mut self
        {
            *enforce_world_writable_audit = enabled;
        }
        self
    }

    pub fn with_reparse_point_rejection(mut self, enabled: bool) -> Self {
        if let Self::WorkspaceWrite {
            reject_reparse_points,
            ..
        } = &mut self
        {
            *reject_reparse_points = enabled;
        }
        self
    }

    pub fn writable_roots_with_workspace(&self, workspace_root: &Path) -> Vec<WritableRoot> {
        match self {
            Self::DangerFullAccess | Self::ReadOnly => Vec::new(),
            Self::WorkspaceWrite {
                writable_roots,
                exclude_tmpdir_env_var,
                exclude_slash_tmp,
                ..
            } => {
                let mut roots = writable_roots.clone();
                roots.push(workspace_root.to_path_buf());

                if cfg!(unix) && !exclude_slash_tmp {
                    let slash_tmp = PathBuf::from("/tmp");
                    if cap_fs::is_dir(&slash_tmp) {
                        roots.push(slash_tmp);
                    }
                }

                if !exclude_tmpdir_env_var
                    && let Some(tmpdir) = std::env::var_os("TMPDIR")
                    && !tmpdir.is_empty()
                {
                    roots.push(PathBuf::from(tmpdir));
                }

                if cfg!(windows) {
                    for key in ["TEMP", "TMP"] {
                        if let Some(value) = std::env::var_os(key)
                            && !value.is_empty()
                        {
                            roots.push(PathBuf::from(value));
                        }
                    }
                }

                let mut out = Vec::new();
                let canonical_roots = cap_fs::PathPolicy::exact().normalize_paths(roots);
                for canonical_root in canonical_roots {
                    let mut read_only_subpaths = Vec::new();
                    let git_path = canonical_root.join(".git");
                    if cap_fs::is_dir(&git_path) {
                        read_only_subpaths.push(git_path);
                    }
                    out.push(WritableRoot {
                        root: canonical_root,
                        read_only_subpaths,
                    });
                }
                out
            }
        }
    }

    pub fn is_path_writable(&self, path: &Path, workspace_root: &Path) -> bool {
        if matches!(self, Self::DangerFullAccess) {
            return true;
        }

        for root in self.writable_roots_with_workspace(workspace_root) {
            if !path.starts_with(&root.root) {
                continue;
            }
            let inside_read_only_subpath = root
                .read_only_subpaths
                .iter()
                .any(|read_only_subpath| path.starts_with(read_only_subpath));
            if !inside_read_only_subpath {
                return true;
            }
        }
        false
    }
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self::new_workspace_write_policy()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::SandboxPolicy;

    #[test]
    fn workspace_write_contains_workspace_root() {
        let workspace = if cfg!(windows) {
            PathBuf::from(r"C:\workspace\repo")
        } else {
            PathBuf::from("/workspace/repo")
        };

        let roots =
            SandboxPolicy::new_workspace_write_policy().writable_roots_with_workspace(&workspace);
        assert!(roots.iter().any(|root| root.root == workspace));
    }

    #[test]
    fn read_only_has_no_writable_roots() {
        let workspace = if cfg!(windows) {
            PathBuf::from(r"C:\workspace\repo")
        } else {
            PathBuf::from("/workspace/repo")
        };

        let roots = SandboxPolicy::new_read_only_policy().writable_roots_with_workspace(&workspace);
        assert!(roots.is_empty());
    }
}
